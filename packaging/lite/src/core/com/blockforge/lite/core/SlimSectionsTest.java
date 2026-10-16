package com.blockforge.lite.core;

import java.io.File;
import java.util.HashMap;
import java.util.Map;
import com.blockforge.lite.core.Tests.Game;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;

/** Self-test of slim-sections on a stand-in of PalettedContainer and its ThreadingDetector. */
final class SlimSectionsTest extends PatchTest {
    private static final String PC = "net/minecraft/world/level/chunk/PalettedContainer";
    private static final String TD = "net/minecraft/util/ThreadingDetector";

    void slimSections() throws Exception {
        for (int version : new int[] {V1_8, V21}) {
            if (version == V21 && !runs(V21)) {
                continue;
            }
            String report = reportPath("slim");
            Game g = load(factory.apply("quiet,report=" + report), containers(version, "vanilla"));
            expect(g.transformed.contains(PC), "PalettedContainer stand-in patched (class version " + version + ")");
            expect(line(report, "slim-sections").contains("applied"), "reported as applied");
            Class<?> pc = g.loader.loadClass(PC.replace('/', '.'));
            Class<?> td = g.loader.loadClass(TD.replace('/', '.'));
            Object a = pc.getConstructor().newInstance();
            pc.getConstructor(int.class).newInstance(3);
            pc.getMethod("acquire").invoke(a);
            pc.getMethod("release").invoke(a);
            pc.getMethod("work").invoke(a);
            expect(td.getField("made").getInt(null) == 0, "no guard objects made");
            expect(td.getField("locks").getInt(null) == 0, "acquire/release don't touch a guard");
        }
        Game g = load(null, containers(V1_8, "vanilla"));
        Class<?> pc = g.loader.loadClass(PC.replace('/', '.'));
        pc.getMethod("work").invoke(pc.getConstructor().newInstance());
        expect(g.loader.loadClass(TD.replace('/', '.')).getField("made").getInt(null) == 1
                && g.loader.loadClass(TD.replace('/', '.')).getField("locks").getInt(null) == 2,
                "without the patch the stand-in makes and locks a guard (the test is real)");

        String report = reportPath("slim2");
        g = load(factory.apply("quiet,report=" + report), containers(V1_8, "other-use"));
        expect(!g.transformed.contains(PC), "a third method using the guard: left alone");
        expect(line(report, "slim-sections").contains("skipped"), "reported as skipped");
        g = load(factory.apply("quiet"), containers(V1_8, "no-locking"));
        expect(g.transformed.contains(PC), "acquire/release already empty (Lithium): the guard is still dropped");
        pc = g.loader.loadClass(PC.replace('/', '.'));
        pc.getMethod("work").invoke(pc.getConstructor().newInstance());
        expect(g.loader.loadClass(TD.replace('/', '.')).getField("made").getInt(null) == 0, "and not made");
        File list = new File(tmp, "mods2.txt");
        write(list, "ferritecore-7.0.0-fabric.jar\n");
        g = load(factory.apply("quiet,mods=" + list), containers(V1_8, "vanilla"));
        expect(!g.transformed.contains(PC), "FerriteCore installed: left alone");
    }

    /**
     * A PalettedContainer with a ThreadingDetector guard like 1.18 - 26.x, in three ways: "vanilla", "other-use" (a
     * third method reads the guard), "no-locking" (acquire/release already do nothing).
     */
    static Map<String, byte[]> containers(int version, String kind) {
        Map<String, byte[]> m = new HashMap<String, byte[]>();
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, TD, null, "java/lang/Object", null);
        w.visitField(ACC_PUBLIC | ACC_STATIC, "made", "I", null, null).visitEnd();
        w.visitField(ACC_PUBLIC | ACC_STATIC, "locks", "I", null, null).visitEnd();
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "<init>", "(Ljava/lang/String;)V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        v.visitFieldInsn(GETSTATIC, TD, "made", "I");
        v.visitInsn(ICONST_1);
        v.visitInsn(IADD);
        v.visitFieldInsn(PUTSTATIC, TD, "made", "I");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        for (String name : new String[] {"checkAndLock", "checkAndUnlock"}) {
            v = w.visitMethod(ACC_PUBLIC, name, "()V", null, null);
            v.visitCode();
            v.visitFieldInsn(GETSTATIC, TD, "locks", "I");
            v.visitInsn(ICONST_1);
            v.visitInsn(IADD);
            v.visitFieldInsn(PUTSTATIC, TD, "locks", "I");
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        w.visitEnd();
        m.put(TD, w.toByteArray());

        w = writer();
        w.visit(version, ACC_PUBLIC | ACC_SUPER, PC, null, "java/lang/Object", null);
        w.visitField(ACC_PRIVATE | ACC_FINAL, "threadingDetector", "L" + TD + ";", null, null).visitEnd();
        w.visitField(ACC_PRIVATE, "size", "I", null, null).visitEnd();
        for (String desc : new String[] {"()V", "(I)V"}) {
            v = w.visitMethod(ACC_PUBLIC, "<init>", desc, null, null);
            v.visitCode();
            v.visitVarInsn(ALOAD, 0);
            v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
            v.visitVarInsn(ALOAD, 0);
            v.visitTypeInsn(NEW, TD);
            v.visitInsn(DUP);
            v.visitLdcInsn("PalettedContainer");
            v.visitMethodInsn(INVOKESPECIAL, TD, "<init>", "(Ljava/lang/String;)V", false);
            v.visitFieldInsn(PUTFIELD, PC, "threadingDetector", "L" + TD + ";");
            if (desc.equals("(I)V")) {
                // a branch after the guard, so the constructor has a frame
                Label done = new Label();
                v.visitVarInsn(ILOAD, 1);
                v.visitJumpInsn(IFLE, done);
                v.visitVarInsn(ALOAD, 0);
                v.visitVarInsn(ILOAD, 1);
                v.visitFieldInsn(PUTFIELD, PC, "size", "I");
                v.visitLabel(done);
            }
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        String[][] guards = {{"acquire", "checkAndLock"}, {"release", "checkAndUnlock"}};
        for (String[] g : guards) {
            v = w.visitMethod(ACC_PUBLIC, g[0], "()V", null, null);
            v.visitCode();
            if (!kind.equals("no-locking")) {
                v.visitVarInsn(ALOAD, 0);
                v.visitFieldInsn(GETFIELD, PC, "threadingDetector", "L" + TD + ";");
                v.visitMethodInsn(INVOKEVIRTUAL, TD, g[1], "()V", false);
            }
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        v = w.visitMethod(ACC_PUBLIC, "work", "()V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKEVIRTUAL, PC, "acquire", "()V", false);
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKEVIRTUAL, PC, "release", "()V", false);
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        if (kind.equals("other-use")) {
            v = w.visitMethod(ACC_PUBLIC, "guard", "()Ljava/lang/Object;", null, null);
            v.visitCode();
            v.visitVarInsn(ALOAD, 0);
            v.visitFieldInsn(GETFIELD, PC, "threadingDetector", "L" + TD + ";");
            v.visitInsn(ARETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        w.visitEnd();
        m.put(PC, w.toByteArray());
        return m;
    }
}
