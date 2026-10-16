package com.blockforge.lite.core;

import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import com.blockforge.lite.core.Tests.Game;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;

/**
 * Self-test of shared-state-tables on a stand-in of StateHolder (1.20 - 1.21.1) and small stand-ins of Guava's
 * Table, HashBasedTable and ArrayTable (the real ones are not on the self-test's class path). The ArrayTable
 * stand-in keeps the parts that matter: index maps per table, the copy made from another ArrayTable shares them,
 * put() refuses keys outside the index.
 */
final class SharedStateTablesTest extends PatchTest {
    private static final String G = "com/google/common/collect/";
    private static final String TABLE = G + "Table";
    private static final String HBT = G + "HashBasedTable";
    private static final String AT = G + "ArrayTable";
    private static final String SH = "net/minecraft/world/level/block/state/StateHolder";
    private static final String PROP = "net/minecraft/world/level/block/state/properties/Property";
    private static final String OBJ = "Ljava/lang/Object;";

    void sharedStateTables() throws Exception {
        for (int version : new int[] {V1_8, V17}) {
            if (!runs(version)) {
                continue;
            }
            String report = reportPath("tables");
            Game g = load(factory.apply("quiet,report=" + report), classes(version, "vanilla"));
            expect(g.transformed.contains(SH), "StateHolder stand-in patched (class version " + version + ")");
            expect(line(report, "shared-state-tables").contains("applied"), "reported as applied");
            List<Object> states = block(g);
            checkNeighbours(g, states, "patched, class version " + version);
            Object first = table(states.get(0));
            boolean shared = true;
            for (int i = 1; i < 6; i++) {
                Object t = table(states.get(i));
                shared &= t.getClass().getName().endsWith("ArrayTable") && t != first
                        && t.getClass().getField("rowIndex").get(t) == first.getClass().getField("rowIndex").get(first)
                        && t.getClass().getField("colIndex").get(t) == first.getClass().getField("colIndex").get(first);
            }
            expect(shared, "every state has its own table, all with the same index maps");
            // another block with the same properties (the game shares property objects) shares them too
            List<Object> other = block(g.loader.loadClass(SH.replace('/', '.')), states.get(7), states.get(8),
                    states.get(9));
            expect(table(other.get(0)).getClass().getField("rowIndex").get(table(other.get(0)))
                    == first.getClass().getField("rowIndex").get(first), "another block with the same properties too");
        }
        Game g = load(null, classes(V1_8, "vanilla"));
        List<Object> states = block(g);
        checkNeighbours(g, states, "unpatched");
        Object a = table(states.get(0));
        Object b = table(states.get(1));
        expect(a.getClass().getField("rowIndex").get(a) != b.getClass().getField("rowIndex").get(b),
                "without the patch every table has its own index maps (the test is real)");

        for (String kind : new String[] {"mixin", "reader"}) {
            String report = reportPath("tables-" + kind);
            g = load(factory.apply("quiet,report=" + report), classes(V1_8, kind));
            expect(!g.transformed.contains(SH), kind + ": left alone");
            expect(line(report, "shared-state-tables").contains("skipped"), kind + ": reported as skipped");
        }
        java.io.File list = new java.io.File(tmp, "mods-ferrite.txt");
        write(list, "ferritecore-6.0.1-forge.jar\n");
        g = load(factory.apply("quiet,mods=" + list), classes(V1_8, "vanilla"));
        expect(!g.transformed.contains(SH), "FerriteCore installed: left alone");
    }

    /** A mod's property whose equals() makes two rows the same: the game's own table, no error. */
    void sharedStateTablesOddProperty() throws Exception {
        Game g = load(factory.apply("quiet"), classes(V1_8, "vanilla"));
        Class<?> prop = g.loader.loadClass(PROP.replace('/', '.'));
        Class<?> sh = g.loader.loadClass(SH.replace('/', '.'));
        Object p1 = prop.getConstructor(List.class, boolean.class).newInstance(Arrays.<Object>asList(1, 2), true);
        Object p2 = prop.getConstructor(List.class, boolean.class).newInstance(Arrays.<Object>asList(3, 4), true);
        Map<Object, Object> byValues = new HashMap<Object, Object>();
        List<Object> states = new ArrayList<Object>();
        for (Object x : Arrays.asList(1, 2)) {
            for (Object y : Arrays.asList(3, 4)) {
                Map<Object, Object> values = new LinkedHashMap<Object, Object>();
                values.put(p1, x);
                values.put(p2, y);
                Object s = sh.getConstructor(Map.class).newInstance(values);
                byValues.put(values, s);
                states.add(s);
            }
        }
        boolean ok = true;
        try {
            for (Object s : states) {
                sh.getMethod("populateNeighbours", Map.class).invoke(s, byValues);
            }
        } catch (Exception e) {
            ok = false;
        }
        expect(ok, "no error with a property whose equals() is always true");
    }

    private static Object table(Object state) throws Exception {
        java.lang.reflect.Field f = state.getClass().getDeclaredField("neighbours");
        f.setAccessible(true);
        return f.get(state);
    }

    /** The six states of a block with A = 1|2|3, B = true|false and C = "x" (one value: no row), populated. */
    private static List<Object> block(Game g) throws Exception {
        Class<?> prop = g.loader.loadClass(PROP.replace('/', '.'));
        Class<?> sh = g.loader.loadClass(SH.replace('/', '.'));
        Object pa = prop.getConstructor(List.class, boolean.class).newInstance(Arrays.<Object>asList(1, 2, 3), false);
        Object pb = prop.getConstructor(List.class, boolean.class).newInstance(Arrays.<Object>asList(true, false),
                false);
        Object pc = prop.getConstructor(List.class, boolean.class).newInstance(Arrays.<Object>asList("x"), false);
        return block(sh, pa, pb, pc);
    }

    private static List<Object> block(Class<?> sh, Object pa, Object pb, Object pc) throws Exception {
        Map<Object, Object> byValues = new HashMap<Object, Object>();
        List<Object> states = new ArrayList<Object>();
        for (Object a : Arrays.asList(1, 2, 3)) {
            for (Object b : Arrays.asList(true, false)) {
                Map<Object, Object> values = new LinkedHashMap<Object, Object>();
                values.put(pa, a);
                values.put(pb, b);
                values.put(pc, "x");
                Object s = sh.getConstructor(Map.class).newInstance(values);
                byValues.put(values, s);
                states.add(s);
            }
        }
        for (Object s : states) {
            sh.getMethod("populateNeighbours", Map.class).invoke(s, byValues);
        }
        List<Object> out = new ArrayList<Object>(states);
        out.add(byValues);
        out.add(pa);
        out.add(pb);
        out.add(pc);
        return out;
    }

    /** setValue gives the right state for every property and value, and refuses values that don't exist. */
    @SuppressWarnings("unchecked")
    private void checkNeighbours(Game g, List<Object> all, String what) throws Exception {
        Class<?> sh = g.loader.loadClass(SH.replace('/', '.'));
        Method set = sh.getMethod("setValue", Object.class, Object.class);
        Map<Object, Object> byValues = (Map<Object, Object>) all.get(6);
        Object[] props = {all.get(7), all.get(8), all.get(9)};
        Object[][] values = {{1, 2, 3}, {true, false}, {"x"}};
        boolean ok = true;
        for (int i = 0; i < 6; i++) {
            Object s = all.get(i);
            Map<Object, Object> mine = (Map<Object, Object>) sh.getField("values").get(s);
            for (int p = 0; p < props.length; p++) {
                for (Object v : values[p]) {
                    Map<Object, Object> want = new LinkedHashMap<Object, Object>(mine);
                    want.put(props[p], v);
                    ok &= set.invoke(s, props[p], v) == byValues.get(want);
                }
                boolean refused;
                try {
                    set.invoke(s, props[p], 99);
                    refused = false;
                } catch (java.lang.reflect.InvocationTargetException e) {
                    refused = e.getCause() instanceof IllegalArgumentException;
                }
                ok &= refused;
            }
        }
        expect(ok, what + ": every neighbour is right, unknown values are refused");
    }

    /**
     * Guava stand-ins, Property and StateHolder as in 1.20 - 1.21.1. kind: "vanilla"; "mixin" (populateNeighbours
     * calls a mixin handler); "reader" (another method hands out the table).
     */
    static Map<String, byte[]> classes(int version, String kind) {
        Map<String, byte[]> m = new HashMap<String, byte[]>();
        m.put(TABLE, table());
        m.put(HBT, hashBased());
        m.put(AT, arrayTable());
        m.put(PROP, property());
        m.put(SH, stateHolder(version, kind));
        return m;
    }

    private static byte[] table() {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_INTERFACE | ACC_ABSTRACT, TABLE, null, "java/lang/Object", null);
        String[][] ms = {{"get", "(" + OBJ + OBJ + ")" + OBJ}, {"put", "(" + OBJ + OBJ + OBJ + ")" + OBJ},
            {"isEmpty", "()Z"}, {"rowKeySet", "()Ljava/util/Set;"}, {"columnKeySet", "()Ljava/util/Set;"},
            {"cells", "()Ljava/util/Map;"}};
        for (String[] x : ms) {
            w.visitMethod(ACC_PUBLIC | ACC_ABSTRACT, x[0], x[1], null, null).visitEnd();
        }
        w.visitEnd();
        return w.toByteArray();
    }

    /** cells.get/put with the key Arrays.asList(r, c). */
    private static void key(MethodVisitor v, int r, int c) {
        v.visitInsn(ICONST_2);
        v.visitTypeInsn(ANEWARRAY, "java/lang/Object");
        v.visitInsn(DUP);
        v.visitInsn(ICONST_0);
        v.visitVarInsn(ALOAD, r);
        v.visitInsn(AASTORE);
        v.visitInsn(DUP);
        v.visitInsn(ICONST_1);
        v.visitVarInsn(ALOAD, c);
        v.visitInsn(AASTORE);
        v.visitMethodInsn(INVOKESTATIC, "java/util/Arrays", "asList", "([Ljava/lang/Object;)Ljava/util/List;", false);
    }

    private static void getter(ClassWriter w, String owner, String name, String desc, String field, String fdesc) {
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, name, desc, null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, owner, field, fdesc);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
    }

    private static void cellGet(ClassWriter w, String owner) {
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "get", "(" + OBJ + OBJ + ")" + OBJ, null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, owner, "cells", "Ljava/util/Map;");
        key(v, 1, 2);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", "get", "(" + OBJ + ")" + OBJ, true);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "isEmpty", "()Z", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, owner, "cells", "Ljava/util/Map;");
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", "isEmpty", "()Z", true);
        v.visitInsn(IRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        getter(w, owner, "cells", "()Ljava/util/Map;", "cells", "Ljava/util/Map;");
    }

    private static byte[] hashBased() {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, HBT, null, "java/lang/Object", new String[] {TABLE});
        for (String f : new String[] {"rows:Ljava/util/Set;", "cols:Ljava/util/Set;", "cells:Ljava/util/Map;"}) {
            w.visitField(ACC_PUBLIC, f.split(":")[0], f.split(":")[1], null, null).visitEnd();
        }
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "<init>", "()V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        for (String[] f : new String[][] {{"rows", "java/util/LinkedHashSet", "Ljava/util/Set;"},
            {"cols", "java/util/LinkedHashSet", "Ljava/util/Set;"}, {"cells", "java/util/LinkedHashMap",
                "Ljava/util/Map;"}}) {
            v.visitVarInsn(ALOAD, 0);
            v.visitTypeInsn(NEW, f[1]);
            v.visitInsn(DUP);
            v.visitMethodInsn(INVOKESPECIAL, f[1], "<init>", "()V", false);
            v.visitFieldInsn(PUTFIELD, HBT, f[0], f[2]);
        }
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC | ACC_STATIC, "create", "()L" + HBT + ";", null, null);
        v.visitCode();
        v.visitTypeInsn(NEW, HBT);
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, HBT, "<init>", "()V", false);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "put", "(" + OBJ + OBJ + OBJ + ")" + OBJ, null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, HBT, "rows", "Ljava/util/Set;");
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Set", "add", "(" + OBJ + ")Z", true);
        v.visitInsn(POP);
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, HBT, "cols", "Ljava/util/Set;");
        v.visitVarInsn(ALOAD, 2);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Set", "add", "(" + OBJ + ")Z", true);
        v.visitInsn(POP);
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, HBT, "cells", "Ljava/util/Map;");
        key(v, 1, 2);
        v.visitVarInsn(ALOAD, 3);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", "put", "(" + OBJ + OBJ + ")" + OBJ, true);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        cellGet(w, HBT);
        getter(w, HBT, "rowKeySet", "()Ljava/util/Set;", "rows", "Ljava/util/Set;");
        getter(w, HBT, "columnKeySet", "()Ljava/util/Set;", "cols", "Ljava/util/Set;");
        w.visitEnd();
        return w.toByteArray();
    }

    private static byte[] arrayTable() {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, AT, null, "java/lang/Object", new String[] {TABLE});
        for (String f : new String[] {"rowIndex", "colIndex", "cells"}) {
            w.visitField(ACC_PUBLIC, f, "Ljava/util/Map;", null, null).visitEnd();
        }
        MethodVisitor v = w.visitMethod(ACC_PRIVATE, "<init>", "()V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        // static Map index(Iterable keys): key -> position; a key twice is an error (as in Guava)
        v = w.visitMethod(ACC_PRIVATE | ACC_STATIC, "index", "(Ljava/lang/Iterable;)Ljava/util/Map;", null, null);
        v.visitCode();
        v.visitTypeInsn(NEW, "java/util/HashMap");
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, "java/util/HashMap", "<init>", "()V", false);
        v.visitVarInsn(ASTORE, 1);
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKEINTERFACE, "java/lang/Iterable", "iterator", "()Ljava/util/Iterator;", true);
        v.visitVarInsn(ASTORE, 2);
        Label loop = new Label();
        Label done = new Label();
        Label fresh = new Label();
        v.visitLabel(loop);
        v.visitVarInsn(ALOAD, 2);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Iterator", "hasNext", "()Z", true);
        v.visitJumpInsn(IFEQ, done);
        v.visitVarInsn(ALOAD, 2);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Iterator", "next", "()" + OBJ, true);
        v.visitVarInsn(ASTORE, 3);
        v.visitVarInsn(ALOAD, 1);
        v.visitVarInsn(ALOAD, 3);
        v.visitMethodInsn(INVOKEVIRTUAL, "java/util/HashMap", "containsKey", "(" + OBJ + ")Z", false);
        v.visitJumpInsn(IFEQ, fresh);
        v.visitTypeInsn(NEW, "java/lang/IllegalArgumentException");
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/IllegalArgumentException", "<init>", "()V", false);
        v.visitInsn(ATHROW);
        v.visitLabel(fresh);
        v.visitVarInsn(ALOAD, 1);
        v.visitVarInsn(ALOAD, 3);
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKEVIRTUAL, "java/util/HashMap", "size", "()I", false);
        v.visitMethodInsn(INVOKESTATIC, "java/lang/Integer", "valueOf", "(I)Ljava/lang/Integer;", false);
        v.visitMethodInsn(INVOKEVIRTUAL, "java/util/HashMap", "put", "(" + OBJ + OBJ + ")" + OBJ, false);
        v.visitInsn(POP);
        v.visitJumpInsn(GOTO, loop);
        v.visitLabel(done);
        v.visitVarInsn(ALOAD, 1);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        // static ArrayTable create(Iterable rows, Iterable cols)
        v = w.visitMethod(ACC_PUBLIC | ACC_STATIC, "create", "(Ljava/lang/Iterable;Ljava/lang/Iterable;)L" + AT + ";",
                null, null);
        v.visitCode();
        v.visitTypeInsn(NEW, AT);
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, AT, "<init>", "()V", false);
        v.visitVarInsn(ASTORE, 2);
        v.visitVarInsn(ALOAD, 2);
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESTATIC, AT, "index", "(Ljava/lang/Iterable;)Ljava/util/Map;", false);
        v.visitFieldInsn(PUTFIELD, AT, "rowIndex", "Ljava/util/Map;");
        v.visitVarInsn(ALOAD, 2);
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKESTATIC, AT, "index", "(Ljava/lang/Iterable;)Ljava/util/Map;", false);
        v.visitFieldInsn(PUTFIELD, AT, "colIndex", "Ljava/util/Map;");
        v.visitVarInsn(ALOAD, 2);
        v.visitTypeInsn(NEW, "java/util/LinkedHashMap");
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, "java/util/LinkedHashMap", "<init>", "()V", false);
        v.visitFieldInsn(PUTFIELD, AT, "cells", "Ljava/util/Map;");
        v.visitVarInsn(ALOAD, 2);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        // static ArrayTable create(Table t): a copy of an ArrayTable shares its index maps
        v = w.visitMethod(ACC_PUBLIC | ACC_STATIC, "create", "(L" + TABLE + ";)L" + AT + ";", null, null);
        v.visitCode();
        Label other = new Label();
        v.visitVarInsn(ALOAD, 0);
        v.visitTypeInsn(INSTANCEOF, AT);
        v.visitJumpInsn(IFEQ, other);
        v.visitTypeInsn(NEW, AT);
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, AT, "<init>", "()V", false);
        v.visitVarInsn(ASTORE, 1);
        for (String f : new String[] {"rowIndex", "colIndex"}) {
            v.visitVarInsn(ALOAD, 1);
            v.visitVarInsn(ALOAD, 0);
            v.visitTypeInsn(CHECKCAST, AT);
            v.visitFieldInsn(GETFIELD, AT, f, "Ljava/util/Map;");
            v.visitFieldInsn(PUTFIELD, AT, f, "Ljava/util/Map;");
        }
        v.visitVarInsn(ALOAD, 1);
        v.visitTypeInsn(NEW, "java/util/LinkedHashMap");
        v.visitInsn(DUP);
        v.visitVarInsn(ALOAD, 0);
        v.visitTypeInsn(CHECKCAST, AT);
        v.visitFieldInsn(GETFIELD, AT, "cells", "Ljava/util/Map;");
        v.visitMethodInsn(INVOKESPECIAL, "java/util/LinkedHashMap", "<init>", "(Ljava/util/Map;)V", false);
        v.visitFieldInsn(PUTFIELD, AT, "cells", "Ljava/util/Map;");
        v.visitVarInsn(ALOAD, 1);
        v.visitInsn(ARETURN);
        v.visitLabel(other);
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKEINTERFACE, TABLE, "rowKeySet", "()Ljava/util/Set;", true);
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKEINTERFACE, TABLE, "columnKeySet", "()Ljava/util/Set;", true);
        v.visitMethodInsn(INVOKESTATIC, AT, "create", "(Ljava/lang/Iterable;Ljava/lang/Iterable;)L" + AT + ";", false);
        v.visitInsn(DUP);
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKEVIRTUAL, AT, "putAll", "(L" + TABLE + ";)V", false);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        // put(r, c, v): only keys of the index
        v = w.visitMethod(ACC_PUBLIC, "put", "(" + OBJ + OBJ + OBJ + ")" + OBJ, null, null);
        v.visitCode();
        Label bad = new Label();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, AT, "rowIndex", "Ljava/util/Map;");
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", "containsKey", "(" + OBJ + ")Z", true);
        v.visitJumpInsn(IFEQ, bad);
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, AT, "colIndex", "Ljava/util/Map;");
        v.visitVarInsn(ALOAD, 2);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", "containsKey", "(" + OBJ + ")Z", true);
        v.visitJumpInsn(IFEQ, bad);
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, AT, "cells", "Ljava/util/Map;");
        key(v, 1, 2);
        v.visitVarInsn(ALOAD, 3);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", "put", "(" + OBJ + OBJ + ")" + OBJ, true);
        v.visitInsn(ARETURN);
        v.visitLabel(bad);
        v.visitTypeInsn(NEW, "java/lang/IllegalArgumentException");
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/IllegalArgumentException", "<init>", "()V", false);
        v.visitInsn(ATHROW);
        v.visitMaxs(0, 0);
        v.visitEnd();

        // putAll(Table t): every cell of t
        v = w.visitMethod(ACC_PUBLIC, "putAll", "(L" + TABLE + ";)V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKEINTERFACE, TABLE, "cells", "()Ljava/util/Map;", true);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", "entrySet", "()Ljava/util/Set;", true);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Set", "iterator", "()Ljava/util/Iterator;", true);
        v.visitVarInsn(ASTORE, 2);
        Label more = new Label();
        Label end = new Label();
        v.visitLabel(more);
        v.visitVarInsn(ALOAD, 2);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Iterator", "hasNext", "()Z", true);
        v.visitJumpInsn(IFEQ, end);
        v.visitVarInsn(ALOAD, 2);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Iterator", "next", "()" + OBJ, true);
        v.visitTypeInsn(CHECKCAST, "java/util/Map$Entry");
        v.visitVarInsn(ASTORE, 3);
        v.visitVarInsn(ALOAD, 3);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map$Entry", "getKey", "()" + OBJ, true);
        v.visitTypeInsn(CHECKCAST, "java/util/List");
        v.visitVarInsn(ASTORE, 4);
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 4);
        v.visitInsn(ICONST_0);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/List", "get", "(I)" + OBJ, true);
        v.visitVarInsn(ALOAD, 4);
        v.visitInsn(ICONST_1);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/List", "get", "(I)" + OBJ, true);
        v.visitVarInsn(ALOAD, 3);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map$Entry", "getValue", "()" + OBJ, true);
        v.visitMethodInsn(INVOKEVIRTUAL, AT, "put", "(" + OBJ + OBJ + OBJ + ")" + OBJ, false);
        v.visitInsn(POP);
        v.visitJumpInsn(GOTO, more);
        v.visitLabel(end);
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        cellGet(w, AT);
        v = w.visitMethod(ACC_PUBLIC, "rowKeySet", "()Ljava/util/Set;", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, AT, "rowIndex", "Ljava/util/Map;");
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", "keySet", "()Ljava/util/Set;", true);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "columnKeySet", "()Ljava/util/Set;", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, AT, "colIndex", "Ljava/util/Map;");
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", "keySet", "()Ljava/util/Set;", true);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        return w.toByteArray();
    }

    /** Property(List values, boolean odd): getPossibleValues(); odd: equals() is always true, hashCode() 0. */
    private static byte[] property() {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, PROP, null, "java/lang/Object", null);
        w.visitField(ACC_PUBLIC | ACC_FINAL, "values", "Ljava/util/List;", null, null).visitEnd();
        w.visitField(ACC_PUBLIC | ACC_FINAL, "odd", "Z", null, null).visitEnd();
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "<init>", "(Ljava/util/List;Z)V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 1);
        v.visitFieldInsn(PUTFIELD, PROP, "values", "Ljava/util/List;");
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ILOAD, 2);
        v.visitFieldInsn(PUTFIELD, PROP, "odd", "Z");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "getPossibleValues", "()Ljava/util/Collection;", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, PROP, "values", "Ljava/util/List;");
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "equals", "(" + OBJ + ")Z", null, null);
        v.visitCode();
        Label normal = new Label();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, PROP, "odd", "Z");
        v.visitJumpInsn(IFEQ, normal);
        v.visitInsn(ICONST_1);
        v.visitInsn(IRETURN);
        v.visitLabel(normal);
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 1);
        Label same = new Label();
        v.visitJumpInsn(IF_ACMPEQ, same);
        v.visitInsn(ICONST_0);
        v.visitInsn(IRETURN);
        v.visitLabel(same);
        v.visitInsn(ICONST_1);
        v.visitInsn(IRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "hashCode", "()I", null, null);
        v.visitCode();
        Label normalHash = new Label();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, PROP, "odd", "Z");
        v.visitJumpInsn(IFEQ, normalHash);
        v.visitInsn(ICONST_0);
        v.visitInsn(IRETURN);
        v.visitLabel(normalHash);
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESTATIC, "java/lang/System", "identityHashCode", "(" + OBJ + ")I", false);
        v.visitInsn(IRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        return w.toByteArray();
    }

    private static byte[] stateHolder(int version, String kind) {
        ClassWriter w = writer();
        w.visit(version, ACC_PUBLIC | ACC_SUPER, SH, null, "java/lang/Object", null);
        w.visitField(ACC_PUBLIC | ACC_FINAL, "values", "Ljava/util/Map;", null, null).visitEnd();
        w.visitField(ACC_PRIVATE, "neighbours", "L" + TABLE + ";", null, null).visitEnd();
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "<init>", "(Ljava/util/Map;)V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 1);
        v.visitFieldInsn(PUTFIELD, SH, "values", "Ljava/util/Map;");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        // populateNeighbours(Map statesByValues), the shape of 1.20.1 / 1.21.1
        v = w.visitMethod(ACC_PUBLIC, "populateNeighbours", "(Ljava/util/Map;)V", null, null);
        v.visitCode();
        if (kind.equals("mixin")) {
            v.visitMethodInsn(INVOKESTATIC, SH, "handler$zza000$tables$populate", "()V", false);
        }
        Label fresh = new Label();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, SH, "neighbours", "L" + TABLE + ";");
        v.visitJumpInsn(IFNULL, fresh);
        v.visitTypeInsn(NEW, "java/lang/IllegalStateException");
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/IllegalStateException", "<init>", "()V", false);
        v.visitInsn(ATHROW);
        v.visitLabel(fresh);
        v.visitMethodInsn(INVOKESTATIC, HBT, "create", "()L" + HBT + ";", false);
        v.visitVarInsn(ASTORE, 2);
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, SH, "values", "Ljava/util/Map;");
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", "entrySet", "()Ljava/util/Set;", true);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Set", "iterator", "()Ljava/util/Iterator;", true);
        v.visitVarInsn(ASTORE, 3);
        Label outer = new Label();
        Label inner = new Label();
        Label next = new Label();
        Label done = new Label();
        v.visitLabel(outer);
        v.visitVarInsn(ALOAD, 3);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Iterator", "hasNext", "()Z", true);
        v.visitJumpInsn(IFEQ, done);
        v.visitVarInsn(ALOAD, 3);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Iterator", "next", "()" + OBJ, true);
        v.visitTypeInsn(CHECKCAST, "java/util/Map$Entry");
        v.visitVarInsn(ASTORE, 4);
        v.visitVarInsn(ALOAD, 4);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map$Entry", "getKey", "()" + OBJ, true);
        v.visitTypeInsn(CHECKCAST, PROP);
        v.visitVarInsn(ASTORE, 5);
        v.visitVarInsn(ALOAD, 5);
        v.visitMethodInsn(INVOKEVIRTUAL, PROP, "getPossibleValues", "()Ljava/util/Collection;", false);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Collection", "iterator", "()Ljava/util/Iterator;", true);
        v.visitVarInsn(ASTORE, 6);
        v.visitLabel(inner);
        v.visitVarInsn(ALOAD, 6);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Iterator", "hasNext", "()Z", true);
        v.visitJumpInsn(IFEQ, outer);
        v.visitVarInsn(ALOAD, 6);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Iterator", "next", "()" + OBJ, true);
        v.visitVarInsn(ASTORE, 7);
        v.visitVarInsn(ALOAD, 7);
        v.visitVarInsn(ALOAD, 4);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map$Entry", "getValue", "()" + OBJ, true);
        v.visitMethodInsn(INVOKEVIRTUAL, "java/lang/Object", "equals", "(" + OBJ + ")Z", false);
        v.visitJumpInsn(IFNE, next);
        v.visitVarInsn(ALOAD, 2);
        v.visitVarInsn(ALOAD, 5);
        v.visitVarInsn(ALOAD, 7);
        v.visitVarInsn(ALOAD, 1);
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 5);
        v.visitVarInsn(ALOAD, 7);
        v.visitMethodInsn(INVOKEVIRTUAL, SH, "neighbourValues", "(L" + PROP + ";" + OBJ + ")Ljava/util/Map;", false);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", "get", "(" + OBJ + ")" + OBJ, true);
        v.visitMethodInsn(INVOKEINTERFACE, TABLE, "put", "(" + OBJ + OBJ + OBJ + ")" + OBJ, true);
        v.visitInsn(POP);
        v.visitLabel(next);
        v.visitJumpInsn(GOTO, inner);
        v.visitLabel(done);
        Label array = new Label();
        Label store = new Label();
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 2);
        v.visitMethodInsn(INVOKEINTERFACE, TABLE, "isEmpty", "()Z", true);
        v.visitJumpInsn(IFEQ, array);
        v.visitVarInsn(ALOAD, 2);
        v.visitJumpInsn(GOTO, store);
        v.visitLabel(array);
        v.visitVarInsn(ALOAD, 2);
        v.visitMethodInsn(INVOKESTATIC, AT, "create", "(L" + TABLE + ";)L" + AT + ";", false);
        v.visitLabel(store);
        v.visitFieldInsn(PUTFIELD, SH, "neighbours", "L" + TABLE + ";");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        if (kind.equals("mixin")) {
            v = w.visitMethod(ACC_PRIVATE | ACC_STATIC, "handler$zza000$tables$populate", "()V", null, null);
            v.visitCode();
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }

        v = w.visitMethod(ACC_PRIVATE, "neighbourValues", "(L" + PROP + ";" + OBJ + ")Ljava/util/Map;", null, null);
        v.visitCode();
        v.visitTypeInsn(NEW, "java/util/LinkedHashMap");
        v.visitInsn(DUP);
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, SH, "values", "Ljava/util/Map;");
        v.visitMethodInsn(INVOKESPECIAL, "java/util/LinkedHashMap", "<init>", "(Ljava/util/Map;)V", false);
        v.visitVarInsn(ASTORE, 3);
        v.visitVarInsn(ALOAD, 3);
        v.visitVarInsn(ALOAD, 1);
        v.visitVarInsn(ALOAD, 2);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", "put", "(" + OBJ + OBJ + ")" + OBJ, true);
        v.visitInsn(POP);
        v.visitVarInsn(ALOAD, 3);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        // setValue(property, value): this, the neighbour, or IllegalArgumentException
        v = w.visitMethod(ACC_PUBLIC, "setValue", "(" + OBJ + OBJ + ")" + OBJ, null, null);
        v.visitCode();
        Label known = new Label();
        Label other = new Label();
        Label found = new Label();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, SH, "values", "Ljava/util/Map;");
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Map", "get", "(" + OBJ + ")" + OBJ, true);
        v.visitVarInsn(ASTORE, 3);
        v.visitVarInsn(ALOAD, 3);
        v.visitJumpInsn(IFNONNULL, known);
        v.visitTypeInsn(NEW, "java/lang/IllegalArgumentException");
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/IllegalArgumentException", "<init>", "()V", false);
        v.visitInsn(ATHROW);
        v.visitLabel(known);
        v.visitVarInsn(ALOAD, 3);
        v.visitVarInsn(ALOAD, 2);
        v.visitMethodInsn(INVOKEVIRTUAL, "java/lang/Object", "equals", "(" + OBJ + ")Z", false);
        v.visitJumpInsn(IFEQ, other);
        v.visitVarInsn(ALOAD, 0);
        v.visitInsn(ARETURN);
        v.visitLabel(other);
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, SH, "neighbours", "L" + TABLE + ";");
        v.visitVarInsn(ALOAD, 1);
        v.visitVarInsn(ALOAD, 2);
        v.visitMethodInsn(INVOKEINTERFACE, TABLE, "get", "(" + OBJ + OBJ + ")" + OBJ, true);
        v.visitVarInsn(ASTORE, 4);
        v.visitVarInsn(ALOAD, 4);
        v.visitJumpInsn(IFNONNULL, found);
        v.visitTypeInsn(NEW, "java/lang/IllegalArgumentException");
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/IllegalArgumentException", "<init>", "()V", false);
        v.visitInsn(ATHROW);
        v.visitLabel(found);
        v.visitVarInsn(ALOAD, 4);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        if (kind.equals("reader")) {
            v = w.visitMethod(ACC_PUBLIC, "table", "()L" + TABLE + ";", null, null);
            v.visitCode();
            v.visitVarInsn(ALOAD, 0);
            v.visitFieldInsn(GETFIELD, SH, "neighbours", "L" + TABLE + ";");
            v.visitInsn(ARETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        w.visitEnd();
        return w.toByteArray();
    }
}
