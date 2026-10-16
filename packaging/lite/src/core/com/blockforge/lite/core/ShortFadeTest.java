package com.blockforge.lite.core;

import java.util.HashMap;
import java.util.Map;
import com.blockforge.lite.core.Tests.Game;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;

/** Self-test of short-fade on a stand-in of LoadingOverlay's fade arithmetic. */
final class ShortFadeTest extends PatchTest {
    private static final String LO = "net/minecraft/client/gui/screens/LoadingOverlay";

    void shortFade() throws Exception {
        for (boolean newer : new boolean[] {false, true}) {
            String report = reportPath("fade");
            Map<String, byte[]> classes = new HashMap<String, byte[]>();
            classes.put(LO, overlay(newer, false));
            Game g = load(factory.apply("quiet,report=" + report), classes);
            expect(g.transformed.contains(LO), "LoadingOverlay stand-in patched (with isReadyToFadeOut: " + newer + ")");
            expect(line(report, "short-fade").contains("applied"), "reported as applied");
            Class<?> c = g.loader.loadClass(LO.replace('/', '.'));
            Object o = c.newInstance();
            c.getField("now").setLong(o, 10500L);
            c.getField("fadeOutStart").setLong(o, 10000L);
            c.getField("fadeInStart").setLong(o, 10000L);
            c.getMethod("render", Object.class, int.class, int.class, float.class).invoke(o, null, 0, 0, 0f);
            expect(c.getField("out").getFloat(o) == 2.0F, "fade-out done after 500 ms (was 2000 ms)");
            expect(c.getField("in").getFloat(o) == 4.0F, "fade-in four times faster");
            if (newer) {
                c.getField("now").setLong(o, 10300L);
                expect((Boolean) c.getMethod("ready").invoke(o), "ready to fade out after 250 ms (was 1000 ms)");
                c.getField("now").setLong(o, 10200L);
                expect(!(Boolean) c.getMethod("ready").invoke(o), "not before");
            }
        }
        Map<String, byte[]> classes = new HashMap<String, byte[]>();
        classes.put(LO, overlay(false, true));
        Game g = load(factory.apply("quiet"), classes);
        expect(!g.transformed.contains(LO), "a render method changed by a mod: left alone");
    }

    /** LoadingOverlay's fade arithmetic; newer adds isReadyToFadeOut (1.21.2+); mixin adds a handler call. */
    static byte[] overlay(boolean newer, boolean mixin) {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, LO, null, "java/lang/Object", null);
        for (String f : new String[] {"now", "fadeOutStart", "fadeInStart"}) {
            w.visitField(ACC_PUBLIC, f, "J", null, null).visitEnd();
        }
        w.visitField(ACC_PUBLIC, "out", "F", null, null).visitEnd();
        w.visitField(ACC_PUBLIC, "in", "F", null, null).visitEnd();
        constructor(w);
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "render", "(Ljava/lang/Object;IIF)V", null, null);
        v.visitCode();
        if (mixin) {
            v.visitMethodInsn(INVOKESTATIC, LO, "handler$zza000$fancymenu$render", "()V", false);
        }
        v.visitLdcInsn("textures/gui/title/mojangstudios.png");
        v.visitInsn(POP);
        String[][] fades = {{"fadeOutStart", "out"}, {"fadeInStart", "in"}};
        for (String[] f : fades) {
            v.visitVarInsn(ALOAD, 0);
            v.visitVarInsn(ALOAD, 0);
            v.visitFieldInsn(GETFIELD, LO, "now", "J");
            v.visitVarInsn(ALOAD, 0);
            v.visitFieldInsn(GETFIELD, LO, f[0], "J");
            v.visitInsn(LSUB);
            v.visitInsn(L2F);
            v.visitLdcInsn(f[1].equals("out") ? 1000.0F : 500.0F);
            v.visitInsn(FDIV);
            v.visitFieldInsn(PUTFIELD, LO, f[1], "F");
        }
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        if (mixin) {
            v = w.visitMethod(ACC_PRIVATE | ACC_STATIC, "handler$zza000$fancymenu$render", "()V", null, null);
            v.visitCode();
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        if (newer) {
            v = w.visitMethod(ACC_PUBLIC, "ready", "()Z", null, null);
            v.visitCode();
            Label no = new Label();
            v.visitVarInsn(ALOAD, 0);
            v.visitFieldInsn(GETFIELD, LO, "now", "J");
            v.visitVarInsn(ALOAD, 0);
            v.visitFieldInsn(GETFIELD, LO, "fadeInStart", "J");
            v.visitInsn(LSUB);
            v.visitLdcInsn(1000L);
            v.visitInsn(LCMP);
            v.visitJumpInsn(IFLT, no);
            v.visitInsn(ICONST_1);
            v.visitInsn(IRETURN);
            v.visitLabel(no);
            v.visitInsn(ICONST_0);
            v.visitInsn(IRETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        w.visitEnd();
        return w.toByteArray();
    }
}
