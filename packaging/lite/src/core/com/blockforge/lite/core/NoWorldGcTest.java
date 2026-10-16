package com.blockforge.lite.core;

import java.util.HashMap;
import java.util.Map;
import com.blockforge.lite.core.Tests.Game;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;

/** Self-test of no-world-gc on a stand-in of 1.8 - 1.12's world loading. */
final class NoWorldGcTest extends PatchTest {
    void noWorldGc() throws Exception {
        for (int version : new int[] {V1_6, V1_8}) {
            String report = reportPath("gc");
            Map<String, byte[]> classes = new HashMap<String, byte[]>();
            classes.put(Tests.MC, legacyWorlds(version, true));
            int before = gcCalls(classes.get(Tests.MC));
            Game g = load(factory.apply("quiet,report=" + report), classes);
            expect(g.transformed.contains(Tests.MC), "patched (class version " + version + ")");
            expect(line(report, "no-world-gc").contains("applied"), "reported as applied");
            Class<?> c = g.loader.loadClass(Tests.MC.replace('/', '.'));
            Object mc = c.newInstance();
            c.getField("systemTime").setLong(mc, 5L);
            c.getMethod("launchIntegratedServer", String.class).invoke(mc, "x");
            expect(c.getField("loads").getInt(mc) == 1 && c.getField("systemTime").getLong(mc) == 0L,
                    "the world is still loaded the same way");
            byte[] after = factory.apply("quiet").transform(null, Tests.MC, null, null, classes.get(Tests.MC));
            expect(before == 4 && after != null && gcCalls(after) == 2,
                    "the two world System.gc() calls are gone, freeMemory's two stay");
        }
        Map<String, byte[]> classes = new HashMap<String, byte[]>();
        classes.put(Tests.MC, legacyWorlds(V1_8, false));
        String report = reportPath("gc2");
        Game g = load(factory.apply("quiet,report=" + report), classes);
        expect(!g.transformed.contains(Tests.MC), "System.gc() in other places: left alone");
        expect(line(report, "no-world-gc").contains("skipped"), "reported as skipped");
    }

    private static int gcCalls(byte[] b) {
        return new Target(b).calls(INVOKESTATIC, "java/lang/System", "gc", "()V").size();
    }

    /**
     * The world loading of a 1.8 - 1.12 Minecraft: launchIntegratedServer and loadWorld with a System.gc() each (or,
     * when matching is false, somewhere else), and freeMemory with one that must stay.
     */
    static byte[] legacyWorlds(int version, boolean matching) {
        ClassWriter w = writer();
        w.visit(version, ACC_PUBLIC | ACC_SUPER, Tests.MC, null, "java/lang/Object", null);
        w.visitField(ACC_PUBLIC, "systemTime", "J", null, null).visitEnd();
        w.visitField(ACC_PUBLIC, "world", "Ljava/lang/Object;", null, null).visitEnd();
        w.visitField(ACC_PUBLIC, "loads", "I", null, null).visitEnd();
        constructor(w);
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "launchIntegratedServer", "(Ljava/lang/String;)V", null, null);
        v.visitCode();
        if (!matching) {
            v.visitMethodInsn(INVOKESTATIC, "java/lang/System", "gc", "()V", false);
        }
        v.visitVarInsn(ALOAD, 0);
        v.visitInsn(ACONST_NULL);
        v.visitMethodInsn(INVOKEVIRTUAL, Tests.MC, "loadWorld", "(Ljava/lang/Object;)V", false);
        if (matching) {
            v.visitMethodInsn(INVOKESTATIC, "java/lang/System", "gc", "()V", false);
        }
        v.visitLdcInsn("fpslimit_wait");
        v.visitInsn(POP);
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "loadWorld", "(Ljava/lang/Object;)V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 1);
        v.visitLdcInsn("");
        v.visitMethodInsn(INVOKEVIRTUAL, Tests.MC, "loadWorld", "(Ljava/lang/Object;Ljava/lang/String;)V", false);
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "loadWorld", "(Ljava/lang/Object;Ljava/lang/String;)V", null, null);
        v.visitCode();
        Label join = new Label();
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 1);
        v.visitFieldInsn(PUTFIELD, Tests.MC, "world", "Ljava/lang/Object;");
        v.visitVarInsn(ALOAD, 1);
        v.visitJumpInsn(IFNONNULL, join);
        v.visitVarInsn(ALOAD, 0);
        v.visitInsn(DUP);
        v.visitFieldInsn(GETFIELD, Tests.MC, "loads", "I");
        v.visitInsn(ICONST_1);
        v.visitInsn(IADD);
        v.visitFieldInsn(PUTFIELD, Tests.MC, "loads", "I");
        // a branch target right at the call, as in the real game
        v.visitLabel(join);
        v.visitMethodInsn(INVOKESTATIC, "java/lang/System", "gc", "()V", false);
        v.visitVarInsn(ALOAD, 0);
        v.visitInsn(LCONST_0);
        v.visitFieldInsn(PUTFIELD, Tests.MC, "systemTime", "J");
        if (!matching) {
            v.visitInsn(NOP);
        }
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        // freeMemory (out of memory): try { System.gc(); this.loadWorld(null); } catch (Throwable t) {} System.gc();
        v = w.visitMethod(ACC_PUBLIC, "freeMemory", "()V", null, null);
        v.visitCode();
        Label start = new Label();
        Label end = new Label();
        Label handler = new Label();
        Label after = new Label();
        v.visitTryCatchBlock(start, end, handler, "java/lang/Throwable");
        v.visitLabel(start);
        v.visitMethodInsn(INVOKESTATIC, "java/lang/System", "gc", "()V", false);
        v.visitVarInsn(ALOAD, 0);
        v.visitInsn(ACONST_NULL);
        v.visitMethodInsn(INVOKEVIRTUAL, Tests.MC, "loadWorld", "(Ljava/lang/Object;)V", false);
        v.visitLabel(end);
        v.visitJumpInsn(GOTO, after);
        v.visitLabel(handler);
        v.visitVarInsn(ASTORE, 1);
        v.visitLabel(after);
        v.visitMethodInsn(INVOKESTATIC, "java/lang/System", "gc", "()V", false);
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        return w.toByteArray();
    }

    /** Forge 1.12.2's launchIntegratedServer: its own call first, and a cast null. */
    void noWorldGcForgeShape() throws Exception {
        byte[] b = legacyWorlds(V1_8, true);
        org.objectweb.asm.tree.ClassNode c = new org.objectweb.asm.tree.ClassNode();
        new org.objectweb.asm.ClassReader(b).accept(c, 0);
        for (org.objectweb.asm.tree.MethodNode m : c.methods) {
            if (m.name.equals("launchIntegratedServer")) {
                org.objectweb.asm.tree.InsnList head = new org.objectweb.asm.tree.InsnList();
                head.add(new org.objectweb.asm.tree.VarInsnNode(ALOAD, 1));
                head.add(new org.objectweb.asm.tree.InsnNode(POP));
                m.instructions.insert(head);
                for (org.objectweb.asm.tree.AbstractInsnNode n : m.instructions.toArray()) {
                    if (n.getOpcode() == ACONST_NULL) {
                        m.instructions.insert(n, new org.objectweb.asm.tree.TypeInsnNode(CHECKCAST, "java/lang/Object"));
                    }
                }
            }
        }
        ClassWriter w = writer();
        c.accept(w);
        byte[] forge = w.toByteArray();
        byte[] out = factory.apply("quiet").transform(null, Tests.MC, null, null, forge);
        expect(gcCalls(forge) == 4 && out != null && gcCalls(out) == 2,
                "Forge's shape: the two world calls go, the two in freeMemory stay");
        Map<String, byte[]> classes = new HashMap<String, byte[]>();
        classes.put(Tests.MC, out);
        Game g = load(null, classes);
        Class<?> k = g.loader.loadClass(Tests.MC.replace('/', '.'));
        Object mc = k.newInstance();
        k.getMethod("launchIntegratedServer", String.class).invoke(mc, "x");
        k.getMethod("freeMemory").invoke(mc);
        expect(k.getField("loads").getInt(mc) == 2, "and the code still runs");
    }
}
