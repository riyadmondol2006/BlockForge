package com.blockforge.lite.core;

import java.lang.reflect.Field;
import java.util.HashMap;
import java.util.Map;
import com.blockforge.lite.core.Tests.Game;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodVisitor;

/** Self-test of section-lookup on a stand-in of UberGpuBuffer (26.3). */
final class SectionLookupTest extends PatchTest {
    private static final String UB = "com/mojang/blaze3d/vertex/UberGpuBuffer";
    private static final String FAST = "it/unimi/dsi/fastutil/objects/Object2ObjectOpenHashMap";

    void sectionLookup() throws Exception {
        for (int version : new int[] {V1_8, V21}) {
            if (!runs(version)) {
                continue;
            }
            String report = reportPath("section");
            Game g = load(factory.apply("quiet,report=" + report), buffer(version, "vanilla"));
            expect(g.transformed.contains(UB), "UberGpuBuffer stand-in patched (class version " + version + ")");
            expect(line(report, "section-lookup").contains("applied"), "reported as applied");
            Class<?> c = g.loader.loadClass(UB.replace('/', '.'));
            Object o = c.getConstructor().newInstance();
            expect(mapKind(o).equals(FAST.replace('/', '.')), "the allocation map is the fastutil one: " + mapKind(o));
            exercise(c, o);
        }
        Game g = load(null, buffer(V1_8, "vanilla"));
        Class<?> c = g.loader.loadClass(UB.replace('/', '.'));
        Object o = c.getConstructor().newInstance();
        expect(mapKind(o).equals("java.util.HashMap"), "without the patch it is a HashMap (the test is real)");
        exercise(c, o);

        for (String kind : new String[] {"mixin", "iterates", "other-map"}) {
            String report = reportPath("section-" + kind);
            g = load(factory.apply("quiet,report=" + report), buffer(V1_8, kind));
            expect(!g.transformed.contains(UB), kind + ": left alone");
            expect(line(report, "section-lookup").contains("skipped"), kind + ": reported as skipped");
        }
        String report = reportPath("section-sodium");
        g = load(factory.apply("quiet,modids=sodium,report=" + report), buffer(V1_8, "vanilla"));
        expect(!g.transformed.contains(UB), "with Sodium: left alone");
        expect(line(report, "section-lookup").contains("Sodium"), "with Sodium: reported");
        report = reportPath("section-iris");
        g = load(factory.apply("quiet,modids=iris,report=" + report), buffer(V1_8, "vanilla"));
        expect(!g.transformed.contains(UB), "with Iris: left alone");
        expect(line(report, "section-lookup").contains("Iris"), "with Iris: reported");
    }

    /** The Map contract the game relies on, whoever implements the map. */
    private void exercise(Class<?> c, Object o) throws Exception {
        put(c, o, new String("mesh"), "alloc");
        expect("alloc".equals(get(c, o, new String("mesh"))), "an equal key finds the allocation");
        expect(get(c, o, "absent") == null, "an absent key finds nothing");
        expect(get(c, o, null) == null, "a null key finds nothing");
        put(c, o, null, "nullalloc");
        expect("nullalloc".equals(get(c, o, null)), "a null key can be stored");
        expect("nullalloc".equals(remove(c, o, null)), "removing the null key gives its value back");
        expect("alloc".equals(remove(c, o, new String("mesh"))), "removing gives the value back");
        expect(get(c, o, "mesh") == null, "and then it is gone");
        put(c, o, "mesh", "alloc2");
        c.getMethod("clearAll").invoke(o);
        expect(get(c, o, "mesh") == null, "clear empties the map");
    }

    private static String mapKind(Object o) throws Exception {
        Field f = o.getClass().getDeclaredField("allocationMap");
        f.setAccessible(true);
        return f.get(o).getClass().getName();
    }

    private static Object get(Class<?> c, Object o, Object key) throws Exception {
        return c.getMethod("getAllocation", Object.class).invoke(o, key);
    }

    private static void put(Class<?> c, Object o, Object key, Object value) throws Exception {
        c.getMethod("putAllocation", Object.class, Object.class).invoke(o, key, value);
    }

    private static Object remove(Class<?> c, Object o, Object key) throws Exception {
        return c.getMethod("removeAllocation", Object.class).invoke(o, key);
    }

    /**
     * UberGpuBuffer's shape in 26.3, cut down to the allocation map. kind: "vanilla"; "mixin" (a mixin handler
     * call in the constructor); "iterates" (the map is also read with size()); "other-map" (it starts as a
     * LinkedHashMap).
     */
    static Map<String, byte[]> buffer(int version, String kind) throws Exception {
        Map<String, byte[]> m = new HashMap<String, byte[]>();
        m.put(FAST, Template.bytes(SectionLookupTest.class.getClassLoader().getResourceAsStream(FAST + ".class")));
        ClassWriter w = writer();
        w.visit(version, ACC_PUBLIC | ACC_SUPER, UB, null, "java/lang/Object", null);
        w.visitField(ACC_PRIVATE | ACC_FINAL, "stagedAllocations", "L" + FAST + ";", null, null).visitEnd();
        w.visitField(ACC_PRIVATE | ACC_FINAL, "allocationMap", "Ljava/util/Map;", null, null).visitEnd();
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "<init>", "()V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        if (kind.equals("mixin")) {
            v.visitMethodInsn(INVOKESTATIC, UB, "handler$zza000$uber$init", "()V", false);
        }
        v.visitVarInsn(ALOAD, 0);
        v.visitTypeInsn(NEW, FAST);
        v.visitInsn(DUP);
        v.visitIntInsn(BIPUSH, 32);
        v.visitMethodInsn(INVOKESPECIAL, FAST, "<init>", "(I)V", false);
        v.visitFieldInsn(PUTFIELD, UB, "stagedAllocations", "L" + FAST + ";");
        String start = kind.equals("other-map") ? "java/util/LinkedHashMap" : "java/util/HashMap";
        v.visitVarInsn(ALOAD, 0);
        v.visitTypeInsn(NEW, start);
        v.visitInsn(DUP);
        if (kind.equals("other-map")) {
            v.visitMethodInsn(INVOKESPECIAL, start, "<init>", "()V", false);
        } else {
            v.visitIntInsn(SIPUSH, 256);
            v.visitMethodInsn(INVOKESPECIAL, start, "<init>", "(I)V", false);
        }
        v.visitFieldInsn(PUTFIELD, UB, "allocationMap", "Ljava/util/Map;");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        if (kind.equals("mixin")) {
            v = w.visitMethod(ACC_PRIVATE | ACC_STATIC, "handler$zza000$uber$init", "()V", null, null);
            v.visitCode();
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        mapMethod(w, "getAllocation", "get", true);
        mapMethod(w, "putAllocation", "put", true);
        mapMethod(w, "removeAllocation", "remove", true);
        v = w.visitMethod(ACC_PUBLIC, "clearAll", "()V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, UB, "allocationMap", "Ljava/util/Map;");
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", "clear", "()V", true);
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        if (kind.equals("iterates")) {
            v = w.visitMethod(ACC_PUBLIC, "countAllocations", "()I", null, null);
            v.visitCode();
            v.visitVarInsn(ALOAD, 0);
            v.visitFieldInsn(GETFIELD, UB, "allocationMap", "Ljava/util/Map;");
            v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", "size", "()I", true);
            v.visitInsn(IRETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        w.visitEnd();
        m.put(UB, w.toByteArray());
        return m;
    }

    private static void mapMethod(ClassWriter w, String name, String call, boolean two) {
        boolean isPut = call.equals("put");
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, name, isPut ? "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;"
                : "(Ljava/lang/Object;)Ljava/lang/Object;", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, UB, "allocationMap", "Ljava/util/Map;");
        v.visitVarInsn(ALOAD, 1);
        if (isPut) {
            v.visitVarInsn(ALOAD, 2);
            v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", call, "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;", true);
        } else {
            v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", call, "(Ljava/lang/Object;)Ljava/lang/Object;", true);
        }
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
    }
}
