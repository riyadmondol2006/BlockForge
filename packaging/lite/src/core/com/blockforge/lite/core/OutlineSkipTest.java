package com.blockforge.lite.core;

import java.util.HashMap;
import java.util.Map;
import com.blockforge.lite.core.Tests.Game;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;

/** Self-test of outline-skip on a stand-in of LevelRenderer's outline code (and PostChain, RenderTarget). */
final class OutlineSkipTest extends PatchTest {
    private static final String LR = "net/minecraft/client/renderer/LevelRenderer";
    private static final String PCH = "net/minecraft/client/renderer/PostChain";
    private static final String RT = "com/mojang/blaze3d/pipeline/RenderTarget";

    void outlineSkip() throws Exception {
        for (int version : new int[] {V1_6, V1_8}) {
            String report = reportPath("outline");
            Game g = load(factory.apply("quiet,report=" + report), levelRenderer(version, null));
            expect(g.transformed.contains(LR), "LevelRenderer stand-in patched (class version " + version + ")");
            expect(line(report, "outline-skip").contains("applied"), "reported as applied");
            Class<?> c = g.loader.loadClass(LR.replace('/', '.'));
            Class<?> rt = g.loader.loadClass(RT.replace('/', '.'));
            Object lr = c.newInstance();
            c.getMethod("init").invoke(lr);
            int[] blits = new int[3];
            boolean[] glow = {false, true, false};
            for (int i = 0; i < 3; i++) {
                c.getField("glowing").setBoolean(lr, glow[i]);
                c.getMethod("renderLevel", float.class).invoke(lr, 0f);
                c.getMethod("doEntityOutline").invoke(lr);
                blits[i] = rt.getField("blits").getInt(null);
            }
            expect(blits[0] == 0, "nothing glows: no outline pass");
            expect(blits[1] == 1, "something glows: the outline pass is drawn");
            expect(blits[2] == 1, "nothing glows again: no pass");
        }
        Game g = load(null, levelRenderer(V1_8, null));
        Class<?> c = g.loader.loadClass(LR.replace('/', '.'));
        Object lr = c.newInstance();
        c.getMethod("init").invoke(lr);
        c.getMethod("renderLevel", float.class).invoke(lr, 0f);
        c.getMethod("doEntityOutline").invoke(lr);
        expect(g.loader.loadClass(RT.replace('/', '.')).getField("blits").getInt(null) == 1,
                "without the patch the empty layer is drawn (the test is real)");
        for (String kind : new String[] {"forge", "mixin", "twice"}) {
            byte[] b = levelRenderer(V1_8, kind).get(LR);
            expect(factory.apply("quiet").transform(null, LR, null, null, b) == null, kind + ": left alone");
        }
    }

    /**
     * LevelRenderer's outline code as in 1.20 - 1.21.1. kind: null = vanilla, "forge" = renderLevel calls a loader
     * hook, "mixin" = doEntityOutline calls a mixin handler, "twice" = the effect is processed in a second place.
     */
    static Map<String, byte[]> levelRenderer(int version, String kind) {
        Map<String, byte[]> m = new HashMap<String, byte[]>();
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, RT, null, "java/lang/Object", null);
        w.visitField(ACC_PUBLIC | ACC_STATIC, "blits", "I", null, null).visitEnd();
        constructor(w);
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "blitToScreen", "(IIZ)V", null, null);
        v.visitCode();
        v.visitFieldInsn(GETSTATIC, RT, "blits", "I");
        v.visitInsn(ICONST_1);
        v.visitInsn(IADD);
        v.visitFieldInsn(PUTSTATIC, RT, "blits", "I");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        m.put(RT, w.toByteArray());

        w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, PCH, null, "java/lang/Object", null);
        constructor(w);
        v = w.visitMethod(ACC_PUBLIC, "getTempTarget", "(Ljava/lang/String;)L" + RT + ";", null, null);
        v.visitCode();
        v.visitTypeInsn(NEW, RT);
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, RT, "<init>", "()V", false);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "process", "(F)V", null, null);
        v.visitCode();
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        m.put(PCH, w.toByteArray());

        w = writer();
        w.visit(version, ACC_PUBLIC | ACC_SUPER, LR, null, "java/lang/Object", null);
        w.visitField(ACC_PRIVATE, "entityEffect", "L" + PCH + ";", null, null).visitEnd();
        w.visitField(ACC_PRIVATE, "entityTarget", "L" + RT + ";", null, null).visitEnd();
        w.visitField(ACC_PUBLIC, "glowing", "Z", null, null).visitEnd();
        constructor(w);
        v = w.visitMethod(ACC_PUBLIC, "init", "()V", null, null);
        v.visitCode();
        v.visitLdcInsn("shaders/post/entity_outline.json");
        v.visitInsn(POP);
        v.visitVarInsn(ALOAD, 0);
        v.visitTypeInsn(NEW, PCH);
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, PCH, "<init>", "()V", false);
        v.visitFieldInsn(PUTFIELD, LR, "entityEffect", "L" + PCH + ";");
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, LR, "entityEffect", "L" + PCH + ";");
        v.visitLdcInsn("final");
        v.visitMethodInsn(INVOKEVIRTUAL, PCH, "getTempTarget", "(Ljava/lang/String;)L" + RT + ";", false);
        v.visitFieldInsn(PUTFIELD, LR, "entityTarget", "L" + RT + ";");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        v = w.visitMethod(ACC_PROTECTED, "shouldShowEntityOutlines", "()Z", null, null);
        v.visitCode();
        Label no = new Label();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, LR, "entityTarget", "L" + RT + ";");
        v.visitJumpInsn(IFNULL, no);
        v.visitInsn(ICONST_1);
        v.visitInsn(IRETURN);
        v.visitLabel(no);
        v.visitInsn(ICONST_0);
        v.visitInsn(IRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        v = w.visitMethod(ACC_PUBLIC, "renderLevel", "(F)V", null, null);
        v.visitCode();
        if ("forge".equals(kind)) {
            v.visitMethodInsn(INVOKESTATIC, "net/minecraftforge/client/ForgeHooksClient", "dispatchRenderStage", "()V",
                    false);
        }
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, LR, "glowing", "Z");
        v.visitVarInsn(ISTORE, 2);
        Label skip = new Label();
        v.visitVarInsn(ILOAD, 2);
        v.visitJumpInsn(IFEQ, skip);
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, LR, "entityEffect", "L" + PCH + ";");
        v.visitVarInsn(FLOAD, 1);
        v.visitMethodInsn(INVOKEVIRTUAL, PCH, "process", "(F)V", false);
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, LR, "entityTarget", "L" + RT + ";");
        v.visitInsn(POP);
        v.visitLabel(skip);
        if ("twice".equals(kind)) {
            v.visitVarInsn(ALOAD, 0);
            v.visitFieldInsn(GETFIELD, LR, "entityEffect", "L" + PCH + ";");
            v.visitVarInsn(FLOAD, 1);
            v.visitMethodInsn(INVOKEVIRTUAL, PCH, "process", "(F)V", false);
        }
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        v = w.visitMethod(ACC_PUBLIC, "doEntityOutline", "()V", null, null);
        v.visitCode();
        Label end = new Label();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKEVIRTUAL, LR, "shouldShowEntityOutlines", "()Z", false);
        v.visitJumpInsn(IFEQ, end);
        if ("mixin".equals(kind)) {
            v.visitMethodInsn(INVOKESTATIC, LR, "handler$zzb000$glowmod$outline", "()V", false);
        }
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, LR, "entityTarget", "L" + RT + ";");
        v.visitIntInsn(SIPUSH, 854);
        v.visitIntInsn(SIPUSH, 480);
        v.visitInsn(ICONST_0);
        v.visitMethodInsn(INVOKEVIRTUAL, RT, "blitToScreen", "(IIZ)V", false);
        v.visitLabel(end);
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        if ("mixin".equals(kind)) {
            v = w.visitMethod(ACC_PRIVATE | ACC_STATIC, "handler$zzb000$glowmod$outline", "()V", null, null);
            v.visitCode();
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        w.visitEnd();
        m.put(LR, w.toByteArray());
        return m;
    }
}
