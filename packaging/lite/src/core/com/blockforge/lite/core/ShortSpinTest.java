package com.blockforge.lite.core;

import java.util.HashMap;
import java.util.Map;
import com.blockforge.lite.core.Tests.Game;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.tree.MethodNode;

/** Self-test of short-spin on a stand-in of 26.x's FramerateLimiter. */
final class ShortSpinTest extends PatchTest {
    private static final String FL = "net/minecraft/client/FramerateLimiter";

    void shortSpin() throws Exception {
        if (!runs(V9)) {
            return; // Thread.onSpinWait is Java 9+
        }
        String report = reportPath("spin");
        Game g = load(factory.apply("quiet,level=2,report=" + report), limiter("vanilla"));
        expect(g.transformed.contains(FL), "FramerateLimiter stand-in patched at level 2");
        expect(line(report, "short-spin").contains("applied"), "reported as applied");
        Class<?> c = g.loader.loadClass(FL.replace('/', '.'));
        long t0 = System.nanoTime();
        for (int i = 0; i < 20; i++) {
            c.getMethod("limitDisplayFPS", int.class).invoke(null, 1000);
        }
        expect(System.nanoTime() - t0 < 2000000000L, "and it still limits and returns");
        Target after = new Target(limiterBytes(g));
        MethodNode m = after.method("limitDisplayFPS", "(I)V");
        expect(Code.ldcs(m, ShortSpin.NEW_SPIN).size() == 2 && Code.ldcs(m, ShortSpin.SPIN).isEmpty(),
                "0.1 ms margin in both places");
        expect(Code.ldcs(m, ShortSpin.NEW_CAP).size() == 1 && Code.ldcs(m, ShortSpin.CAP).isEmpty(), "1 ms cap");

        g = load(factory.apply("quiet"), limiter("vanilla"));
        expect(!g.transformed.contains(FL), "level 1: not patched");
        for (String kind : new String[] {"mixin", "other"}) {
            report = reportPath("spin-" + kind);
            g = load(factory.apply("quiet,level=2,report=" + report), limiter(kind));
            expect(!g.transformed.contains(FL), kind + ": left alone");
            expect(line(report, "short-spin").contains("skipped"), kind + ": reported as skipped");
        }
    }

    /** The bytes the stand-in loader got for the limiter, patched. */
    private byte[] limiterBytes(Game g) throws Exception {
        return factory.apply("quiet,level=2").transform(null, FL, null, null, limiter("vanilla").get(FL));
    }

    /**
     * limitDisplayFPS(int) like 26.1 - 26.4: park until margin + overshoot before the deadline, then spin. kind:
     * "vanilla"; "mixin" (calls a mixin handler); "other" (a third margin constant).
     */
    static Map<String, byte[]> limiter(String kind) {
        Map<String, byte[]> m = new HashMap<String, byte[]>();
        ClassWriter w = writer();
        w.visit(V9, ACC_PUBLIC | ACC_SUPER, FL, null, "java/lang/Object", null);
        w.visitField(ACC_PRIVATE | ACC_STATIC, "last", "J", null, null).visitEnd();
        w.visitField(ACC_PRIVATE | ACC_STATIC, "over", "J", null, null).visitEnd();
        constructor(w);
        MethodVisitor v = w.visitMethod(ACC_PUBLIC | ACC_STATIC, "limitDisplayFPS", "(I)V", null, null);
        v.visitCode();
        if (kind.equals("mixin")) {
            v.visitMethodInsn(INVOKESTATIC, FL, "handler$zza000$fps$limit", "()V", false);
        }
        // target = last + 1e9 / limit (local 1)
        v.visitFieldInsn(GETSTATIC, FL, "last", "J");
        v.visitLdcInsn(1000000000L);
        v.visitVarInsn(ILOAD, 0);
        v.visitInsn(I2L);
        v.visitInsn(LDIV);
        v.visitInsn(LADD);
        v.visitVarInsn(LSTORE, 1);
        Label loop = new Label();
        Label spin = new Label();
        Label end = new Label();
        v.visitLabel(loop);
        // remaining (local 3)
        v.visitVarInsn(LLOAD, 1);
        v.visitMethodInsn(INVOKESTATIC, "java/lang/System", "nanoTime", "()J", false);
        v.visitInsn(LSUB);
        v.visitInsn(DUP2);
        v.visitVarInsn(LSTORE, 3);
        v.visitInsn(LCONST_0);
        v.visitInsn(LCMP);
        v.visitJumpInsn(IFLE, end);
        v.visitVarInsn(LLOAD, 3);
        v.visitFieldInsn(GETSTATIC, FL, "over", "J");
        v.visitLdcInsn(ShortSpin.SPIN);
        v.visitInsn(LADD);
        v.visitInsn(LCMP);
        v.visitJumpInsn(IFLE, spin);
        v.visitVarInsn(LLOAD, 3);
        v.visitFieldInsn(GETSTATIC, FL, "over", "J");
        v.visitInsn(LSUB);
        v.visitLdcInsn(ShortSpin.SPIN);
        v.visitInsn(LSUB);
        v.visitMethodInsn(INVOKESTATIC, "java/util/concurrent/locks/LockSupport", "parkNanos", "(J)V", false);
        v.visitFieldInsn(GETSTATIC, FL, "over", "J");
        v.visitLdcInsn(ShortSpin.CAP);
        v.visitMethodInsn(INVOKESTATIC, "java/lang/Math", "min", "(JJ)J", false);
        v.visitFieldInsn(PUTSTATIC, FL, "over", "J");
        if (kind.equals("other")) {
            v.visitLdcInsn(ShortSpin.SPIN);
            v.visitInsn(POP2);
        }
        v.visitJumpInsn(GOTO, loop);
        v.visitLabel(spin);
        v.visitMethodInsn(INVOKESTATIC, "java/lang/Thread", "onSpinWait", "()V", false);
        v.visitJumpInsn(GOTO, loop);
        v.visitLabel(end);
        v.visitMethodInsn(INVOKESTATIC, "java/lang/System", "nanoTime", "()J", false);
        v.visitFieldInsn(PUTSTATIC, FL, "last", "J");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        if (kind.equals("mixin")) {
            v = w.visitMethod(ACC_PRIVATE | ACC_STATIC, "handler$zza000$fps$limit", "()V", null, null);
            v.visitCode();
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        w.visitEnd();
        m.put(FL, w.toByteArray());
        return m;
    }
}
