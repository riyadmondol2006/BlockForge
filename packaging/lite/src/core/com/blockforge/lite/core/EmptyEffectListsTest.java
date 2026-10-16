package com.blockforge.lite.core;

import java.util.Arrays;
import java.util.ConcurrentModificationException;
import java.util.HashMap;
import java.util.Iterator;
import java.util.List;
import java.util.Map;
import com.blockforge.lite.core.Tests.Game;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodVisitor;

/** Self-test of empty-effect-lists on a stand-in of InsideBlockEffectApplier.StepBasedCollector. */
final class EmptyEffectListsTest extends PatchTest {
    private static final String SC = "net/minecraft/world/entity/InsideBlockEffectApplier$StepBasedCollector";

    @SuppressWarnings("unchecked")
    void emptyEffectLists() throws Exception {
        for (int version : new int[] {V1_8, V21}) {
            if (!runs(version)) {
                continue;
            }
            String report = reportPath("effects");
            Game g = load(factory.apply("quiet,report=" + report), collector(version, "vanilla"));
            expect(g.transformed.contains(SC), "StepBasedCollector stand-in patched (class version " + version + ")");
            expect(line(report, "empty-effect-lists").contains("applied"), "reported as applied");
            Class<?> c = g.loader.loadClass(SC.replace('/', '.'));
            Object o = c.getConstructor().newInstance();
            List<Object> before = (List<Object>) c.getField("before").get(o);
            List<Object> after = (List<Object>) c.getField("after").get(o);
            List<Object> finals = (List<Object>) c.getMethod("finals").invoke(o);
            before.addAll(Arrays.<Object>asList("a", "b"));
            after.add("c");
            c.getMethod("flushStep").invoke(o);
            expect(finals.equals(Arrays.<Object>asList("a", "b", "c")), "the effects in order: " + finals);
            expect(before.isEmpty() && after.isEmpty(), "the step lists are cleared");
            Iterator<Object> it = finals.iterator();
            c.getMethod("flushStep").invoke(o);
            boolean same;
            try {
                same = "a".equals(it.next());
            } catch (ConcurrentModificationException e) {
                same = false;
            }
            expect(same, "empty lists leave the final list untouched");
            expect(finals.size() == 3, "nothing added");
        }
        Game g = load(null, collector(V1_8, "vanilla"));
        Class<?> c = g.loader.loadClass(SC.replace('/', '.'));
        Object o = c.getConstructor().newInstance();
        Iterator<?> it = ((List<?>) c.getMethod("finals").invoke(o)).iterator();
        c.getMethod("flushStep").invoke(o);
        boolean touched = false;
        try {
            it.next();
        } catch (ConcurrentModificationException e) {
            touched = true;
        } catch (java.util.NoSuchElementException e) {
            touched = false;
        }
        expect(touched, "without the patch, empty lists are still added (the test is real)");

        for (String kind : new String[] {"mixin", "third", "linked"}) {
            String report = reportPath("effects-" + kind);
            g = load(factory.apply("quiet,report=" + report), collector(V1_8, kind));
            expect(!g.transformed.contains(SC), kind + ": left alone");
            expect(line(report, "empty-effect-lists").contains("skipped"), kind + ": reported as skipped");
        }
        java.io.File list = new java.io.File(tmp, "mods-lithium.txt");
        write(list, "lithium-fabric-0.15.0.jar\n");
        g = load(factory.apply("quiet,mods=" + list), collector(V1_8, "vanilla"));
        expect(!g.transformed.contains(SC), "Lithium installed: left alone");
    }

    /**
     * StepBasedCollector's flushStep as in 1.21.2 - 26.x, with the two step lists as fields. kind: "vanilla";
     * "mixin" (flushStep calls a mixin handler); "third" (a third addAll); "linked" (the final list is a LinkedList).
     */
    static Map<String, byte[]> collector(int version, String kind) {
        Map<String, byte[]> m = new HashMap<String, byte[]>();
        ClassWriter w = writer();
        w.visit(version, ACC_PUBLIC | ACC_SUPER, SC, null, "java/lang/Object", null);
        w.visitField(ACC_PRIVATE | ACC_FINAL, "finalEffects", "Ljava/util/List;", null, null).visitEnd();
        w.visitField(ACC_PRIVATE | ACC_FINAL, "effectsInStep", "Ljava/util/Set;", null, null).visitEnd();
        w.visitField(ACC_PUBLIC | ACC_FINAL, "before", "Ljava/util/List;", null, null).visitEnd();
        w.visitField(ACC_PUBLIC | ACC_FINAL, "after", "Ljava/util/List;", null, null).visitEnd();
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "<init>", "()V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        v.visitVarInsn(ALOAD, 0);
        v.visitLdcInsn(org.objectweb.asm.Type.getType("Ljava/util/concurrent/TimeUnit;"));
        v.visitMethodInsn(INVOKESTATIC, "java/util/EnumSet", "noneOf", "(Ljava/lang/Class;)Ljava/util/EnumSet;", false);
        v.visitFieldInsn(PUTFIELD, SC, "effectsInStep", "Ljava/util/Set;");
        String[][] lists = {{"finalEffects", kind.equals("linked") ? "java/util/LinkedList" : "java/util/ArrayList"},
            {"before", "java/util/ArrayList"}, {"after", "java/util/ArrayList"}};
        for (String[] l : lists) {
            v.visitVarInsn(ALOAD, 0);
            v.visitTypeInsn(NEW, l[1]);
            v.visitInsn(DUP);
            v.visitMethodInsn(INVOKESPECIAL, l[1], "<init>", "()V", false);
            v.visitFieldInsn(PUTFIELD, SC, l[0], "Ljava/util/List;");
        }
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        v = w.visitMethod(ACC_PUBLIC, "flushStep", "()V", null, null);
        v.visitCode();
        if (kind.equals("mixin")) {
            v.visitMethodInsn(INVOKESTATIC, SC, "handler$zza000$effects$flush", "()V", false);
        }
        String[] from = kind.equals("third") ? new String[] {"before", "after", "before"} : new String[] {"before",
            "after"};
        for (int i = 0; i < from.length; i++) {
            v.visitVarInsn(ALOAD, 0);
            v.visitFieldInsn(GETFIELD, SC, from[i], "Ljava/util/List;");
            v.visitVarInsn(ASTORE, 1 + i);
            v.visitVarInsn(ALOAD, 0);
            v.visitFieldInsn(GETFIELD, SC, "finalEffects", "Ljava/util/List;");
            v.visitVarInsn(ALOAD, 1 + i);
            v.visitMethodInsn(INVOKEINTERFACE, "java/util/List", "addAll", "(Ljava/util/Collection;)Z", true);
            v.visitInsn(POP);
            v.visitVarInsn(ALOAD, 1 + i);
            v.visitMethodInsn(INVOKEINTERFACE, "java/util/List", "clear", "()V", true);
        }
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        if (kind.equals("mixin")) {
            v = w.visitMethod(ACC_PRIVATE | ACC_STATIC, "handler$zza000$effects$flush", "()V", null, null);
            v.visitCode();
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        v = w.visitMethod(ACC_PUBLIC, "finals", "()Ljava/util/List;", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, SC, "finalEffects", "Ljava/util/List;");
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        m.put(SC, w.toByteArray());
        return m;
    }
}
