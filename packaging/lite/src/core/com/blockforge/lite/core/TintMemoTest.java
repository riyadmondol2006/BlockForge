package com.blockforge.lite.core;

import java.lang.reflect.Method;
import java.util.HashMap;
import java.util.Map;
import com.blockforge.lite.core.Tests.Game;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;

/** Self-test of tint-memo on a stand-in of ClientLevel's biome blend (and BlockPos, Cursor3D, the colour cache). */
final class TintMemoTest extends PatchTest {
    private static final String LEVEL = "net/minecraft/client/multiplayer/ClientLevel";
    private static final String POS = "t/Pos";
    private static final String MPOS = "t/MPos";
    private static final String CURSOR = "t/Cursor";
    private static final String HOLDER = "t/Holder";
    private static final String HOLDER_IMPL = "t/HolderImpl";
    private static final String BIOME = "t/Biome";
    private static final String RESOLVER = "t/ColorResolver";
    private static final String RESOLVER_IMPL = "t/Resolver";
    private static final String CACHE = "t/TintCache";

    void sameColours() throws Exception {
        for (int version : new int[] {V1_8, V17}) {
            if (!runs(version)) {
                continue;
            }
            String report = reportPath("tint");
            Game patched = load(factory.apply("quiet,report=" + report), classes(version, null));
            expect(patched.transformed.contains(LEVEL), "ClientLevel stand-in patched (class version " + version + ")");
            expect(line(report, "tint-memo").contains("applied"), "reported as applied");
            Game plain = load(null, classes(version, null));
            Level a = new Level(patched);
            Level b = new Level(plain);
            boolean same = true;
            for (int radius : new int[] {2, 1, 0}) {
                a.radius(radius);
                b.radius(radius);
                for (int y = -70; y < -40; y += 7) {
                    for (int z = -20; z < 20; z++) {
                        for (int x = -20; x < 20; x++) {
                            same &= a.tint(x, y, z) == b.tint(x, y, z);
                        }
                    }
                }
                // positions far apart that share the memo slot
                same &= a.tint(32 * 1000 + 3, 5, 8 * 7 + 1) == b.tint(32 * 1000 + 3, 5, 8 * 7 + 1);
                same &= a.tint(3, 5 + 16 * 9, 1) == b.tint(3, 5 + 16 * 9, 1);
                same &= a.tint(-30000000, 2031, 29999999) == b.tint(-30000000, 2031, 29999999);
            }
            expect(same, "the same colours as without the patch");
        }
    }

    void fewerLookups() throws Exception {
        Game g = load(factory.apply("quiet"), classes(V1_8, null));
        Level a = new Level(g);
        a.radius(2);
        a.lookups(0);
        a.tint(0, 64, 0);
        expect(a.lookups() == 25, "the first block looks up 25 biomes (" + a.lookups() + ")");
        a.tint(1, 64, 0);
        expect(a.lookups() == 30, "its neighbour only 5 more (" + a.lookups() + ")");
        a.tint(0, 64, 0);
        expect(a.lookups() == 30, "the same block again: none");
        Game plain = load(null, classes(V1_8, null));
        Level b = new Level(plain);
        b.radius(2);
        b.lookups(0);
        b.tint(0, 64, 0);
        b.tint(1, 64, 0);
        expect(b.lookups() == 50, "without the patch 50 (the test is real)");
    }

    /** Two threads making the memo at once: the one whose ThreadLocal was replaced must not fail when it stores. */
    void raceAtTheStart() throws Exception {
        Game g = load(factory.apply("quiet"), classes(V1_8, null));
        Class<?> c = g.loader.loadClass(LEVEL.replace('/', '.'));
        java.lang.reflect.Method put = c.getDeclaredMethod(Code.PREFIX + "tintPut", int.class, int.class, int.class,
                Object.class);
        put.setAccessible(true);
        boolean ok = true;
        try {
            put.invoke(null, 1, 2, 3, "biome");
            java.lang.reflect.Field f = c.getDeclaredField(Code.PREFIX + "tintMemo");
            f.setAccessible(true);
            f.set(null, new ThreadLocal<Object>());
            put.invoke(null, 1, 2, 3, "biome");
        } catch (Exception e) {
            ok = false;
        }
        expect(ok, "storing without a table of this thread: nothing happens");
        Level a = new Level(g);
        a.radius(2);
        Level b = new Level(load(null, classes(V1_8, null)));
        b.radius(2);
        expect(a.tint(0, 64, 0) == b.tint(0, 64, 0) && a.tint(1, 64, 0) == b.tint(1, 64, 0), "and it still works");
    }

    void clearedWithTheGamesCache() throws Exception {
        Game g = load(factory.apply("quiet"), classes(V1_8, null));
        Game plain = load(null, classes(V1_8, null));
        Level a = new Level(g);
        Level b = new Level(plain);
        a.radius(2);
        b.radius(2);
        int before = a.tint(5, 64, 5);
        a.data(1);
        b.data(1);
        // the biomes changed, but the game did not clear its colour cache yet: vanilla's cache would answer the old
        // colour too; after the clear (a chunk arrived) the new biomes count
        a.call("onChunkLoaded", 0, 0);
        int after = a.tint(5, 64, 5);
        expect(after == b.tint(5, 64, 5), "after a chunk arrived: the new biomes");
        expect(after != before, "the stand-in's biomes did change (the test is real)");
        a.lookups(0);
        a.tint(5, 64, 5);
        expect(a.lookups() == 0, "remembered again");
        a.call("onChunkLoaded", 100, -3);
        a.call("onChunkLoaded", -2, 0);
        a.call("onChunkLoaded", 1, 1);
        a.tint(5, 64, 5);
        expect(a.lookups() == 0, "chunks far away arrived: still remembered (" + a.lookups() + ")");
        a.call("onChunkLoaded", 0, -1);
        a.tint(5, 64, 5);
        expect(a.lookups() == 25, "a chunk next to it arrived: looked up again (" + a.lookups() + ")");
        for (int i = 0; i < 129; i++) {
            a.call("onChunkLoaded", 1000, i);
        }
        a.lookups(0);
        a.tint(5, 64, 5);
        expect(a.lookups() == 25, "more chunks arrived than the memo follows: looked up again");
        a.lookups(0);
        a.call("clearTintCaches");
        a.tint(5, 64, 5);
        expect(a.lookups() == 25, "after a resource reload: looked up again");
        // another level instance (another dimension) starts over
        Level other = new Level(g);
        other.radius(2);
        a.lookups(0);
        other.tint(5, 64, 5);
        expect(a.lookups() == 25, "another level: looked up again (" + a.lookups() + ")");
    }

    void leftAlone() throws Exception {
        for (String kind : new String[] {"mixin", "three", "clear-twice", "no-cache"}) {
            byte[] b = classes(V1_8, kind).get(LEVEL);
            String report = reportPath("tint-" + kind);
            expect(factory.apply("quiet,report=" + report).transform(null, LEVEL, null, null, b) == null,
                    kind + ": left alone");
            if (!kind.equals("three")) {
                expect(line(report, "tint-memo").contains("skipped"), kind + ": reported as skipped");
            }
        }
        String report = reportPath("tint-optifine");
        Map<String, byte[]> c = classes(V1_8, null);
        Game g = load(factory.apply("quiet,modids=optifine,report=" + report), c);
        expect(!g.transformed.contains(LEVEL), "with OptiFine: left alone");
    }

    /** Drives a loaded stand-in level. */
    static final class Level {
        final Class<?> c;
        final Object level;
        final Object resolver;
        final Class<?> pos;

        Level(Game g) throws Exception {
            c = g.loader.loadClass(LEVEL.replace('/', '.'));
            level = c.newInstance();
            resolver = g.loader.loadClass(RESOLVER_IMPL.replace('/', '.')).newInstance();
            pos = g.loader.loadClass(POS.replace('/', '.'));
        }

        int tint(int x, int y, int z) throws Exception {
            Object p = pos.getConstructor(int.class, int.class, int.class).newInstance(x, y, z);
            Method m = c.getMethod("calculateBlockTint", pos, resolver.getClass().getInterfaces()[0]);
            return (Integer) m.invoke(level, p, resolver);
        }

        void radius(int r) throws Exception {
            c.getField("radius").setInt(null, r);
        }

        void data(int d) throws Exception {
            c.getField("data").setInt(null, d);
        }

        int lookups() throws Exception {
            return c.getField("lookups").getInt(null);
        }

        void lookups(int n) throws Exception {
            c.getField("lookups").setInt(null, n);
        }

        void call(String name, int... args) throws Exception {
            if (args.length == 0) {
                c.getMethod(name).invoke(level);
            } else {
                c.getMethod(name, int.class, int.class).invoke(level, args[0], args[1]);
            }
        }
    }

    /**
     * The stand-ins. kind: null = vanilla's shape, "mixin" = the blend calls a mixin handler, "three" = a third biome
     * lookup, "clear-twice" = the colour cache is cleared in two places, "no-cache" = no getBlockTint.
     */
    static Map<String, byte[]> classes(int version, String kind) {
        Map<String, byte[]> m = new HashMap<String, byte[]>();
        m.put(POS, pos());
        m.put(MPOS, mutablePos());
        m.put(CURSOR, cursor());
        m.put(BIOME, biome());
        m.put(HOLDER, iface(HOLDER, "value", "()Ljava/lang/Object;"));
        m.put(HOLDER_IMPL, holderImpl());
        m.put(RESOLVER, iface(RESOLVER, "getColor", "(L" + BIOME + ";DD)I"));
        m.put(RESOLVER_IMPL, resolverImpl());
        m.put(CACHE, cache());
        m.put(LEVEL, level(version, kind));
        return m;
    }

    private static byte[] pos() {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, POS, null, "java/lang/Object", null);
        for (String f : new String[] {"x", "y", "z"}) {
            w.visitField(ACC_PUBLIC, f, "I", null, null).visitEnd();
        }
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "<init>", "(III)V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        String[] f = {"x", "y", "z"};
        for (int i = 0; i < 3; i++) {
            v.visitVarInsn(ALOAD, 0);
            v.visitVarInsn(ILOAD, i + 1);
            v.visitFieldInsn(PUTFIELD, POS, f[i], "I");
        }
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        String[] getters = {"getX", "getY", "getZ"};
        for (int i = 0; i < 3; i++) {
            v = w.visitMethod(ACC_PUBLIC, getters[i], "()I", null, null);
            v.visitCode();
            v.visitVarInsn(ALOAD, 0);
            v.visitFieldInsn(GETFIELD, POS, f[i], "I");
            v.visitInsn(IRETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        w.visitEnd();
        return w.toByteArray();
    }

    private static byte[] mutablePos() {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, MPOS, null, POS, null);
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "<init>", "()V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitInsn(ICONST_0);
        v.visitInsn(ICONST_0);
        v.visitInsn(ICONST_0);
        v.visitMethodInsn(INVOKESPECIAL, POS, "<init>", "(III)V", false);
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "set", "(III)L" + MPOS + ";", null, null);
        v.visitCode();
        String[] f = {"x", "y", "z"};
        for (int i = 0; i < 3; i++) {
            v.visitVarInsn(ALOAD, 0);
            v.visitVarInsn(ILOAD, i + 1);
            v.visitFieldInsn(PUTFIELD, POS, f[i], "I");
        }
        v.visitVarInsn(ALOAD, 0);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        return w.toByteArray();
    }

    /** Walks x0..x1, z0..z1 at y0 (x fastest). */
    private static byte[] cursor() {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, CURSOR, null, "java/lang/Object", null);
        for (String f : new String[] {"x0", "y0", "z0", "w", "n", "i", "x", "z"}) {
            w.visitField(ACC_PUBLIC, f, "I", null, null).visitEnd();
        }
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "<init>", "(IIIIII)V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        put(v, "x0", 1);
        put(v, "y0", 2);
        put(v, "z0", 3);
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ILOAD, 4);
        v.visitVarInsn(ILOAD, 1);
        v.visitInsn(ISUB);
        v.visitInsn(ICONST_1);
        v.visitInsn(IADD);
        v.visitFieldInsn(PUTFIELD, CURSOR, "w", "I");
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, CURSOR, "w", "I");
        v.visitVarInsn(ILOAD, 6);
        v.visitVarInsn(ILOAD, 3);
        v.visitInsn(ISUB);
        v.visitInsn(ICONST_1);
        v.visitInsn(IADD);
        v.visitInsn(IMUL);
        v.visitFieldInsn(PUTFIELD, CURSOR, "n", "I");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        // boolean advance(): if (i >= n) return false; x = x0 + i % w; z = z0 + i / w; i++; return true
        v = w.visitMethod(ACC_PUBLIC, "advance", "()Z", null, null);
        v.visitCode();
        Label more = new Label();
        get(v, "i");
        get(v, "n");
        v.visitJumpInsn(IF_ICMPLT, more);
        v.visitInsn(ICONST_0);
        v.visitInsn(IRETURN);
        v.visitLabel(more);
        v.visitVarInsn(ALOAD, 0);
        get(v, "x0");
        get(v, "i");
        get(v, "w");
        v.visitInsn(IREM);
        v.visitInsn(IADD);
        v.visitFieldInsn(PUTFIELD, CURSOR, "x", "I");
        v.visitVarInsn(ALOAD, 0);
        get(v, "z0");
        get(v, "i");
        get(v, "w");
        v.visitInsn(IDIV);
        v.visitInsn(IADD);
        v.visitFieldInsn(PUTFIELD, CURSOR, "z", "I");
        v.visitVarInsn(ALOAD, 0);
        get(v, "i");
        v.visitInsn(ICONST_1);
        v.visitInsn(IADD);
        v.visitFieldInsn(PUTFIELD, CURSOR, "i", "I");
        v.visitInsn(ICONST_1);
        v.visitInsn(IRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        String[][] nexts = {{"nextX", "x"}, {"nextY", "y0"}, {"nextZ", "z"}};
        for (String[] n : nexts) {
            v = w.visitMethod(ACC_PUBLIC, n[0], "()I", null, null);
            v.visitCode();
            get(v, n[1]);
            v.visitInsn(IRETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        w.visitEnd();
        return w.toByteArray();
    }

    private static void put(MethodVisitor v, String field, int var) {
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ILOAD, var);
        v.visitFieldInsn(PUTFIELD, CURSOR, field, "I");
    }

    private static void get(MethodVisitor v, String field) {
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, CURSOR, field, "I");
    }

    private static byte[] biome() {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, BIOME, null, "java/lang/Object", null);
        w.visitField(ACC_PUBLIC, "color", "I", null, null).visitEnd();
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "<init>", "(I)V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ILOAD, 1);
        v.visitFieldInsn(PUTFIELD, BIOME, "color", "I");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        return w.toByteArray();
    }

    private static byte[] iface(String name, String method, String desc) {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_ABSTRACT | ACC_INTERFACE, name, null, "java/lang/Object", null);
        w.visitMethod(ACC_PUBLIC | ACC_ABSTRACT, method, desc, null, null).visitEnd();
        w.visitEnd();
        return w.toByteArray();
    }

    private static byte[] holderImpl() {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, HOLDER_IMPL, null, "java/lang/Object", new String[] {HOLDER});
        w.visitField(ACC_PUBLIC, "biome", "L" + BIOME + ";", null, null).visitEnd();
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "<init>", "(I)V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        v.visitVarInsn(ALOAD, 0);
        v.visitTypeInsn(NEW, BIOME);
        v.visitInsn(DUP);
        v.visitVarInsn(ILOAD, 1);
        v.visitMethodInsn(INVOKESPECIAL, BIOME, "<init>", "(I)V", false);
        v.visitFieldInsn(PUTFIELD, HOLDER_IMPL, "biome", "L" + BIOME + ";");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "value", "()Ljava/lang/Object;", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, HOLDER_IMPL, "biome", "L" + BIOME + ";");
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        return w.toByteArray();
    }

    /** getColor(biome, x, z) = biome.color * 3 + ((int) x &amp; 1) (like the swamp grass noise, x matters too). */
    private static byte[] resolverImpl() {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, RESOLVER_IMPL, null, "java/lang/Object", new String[] {RESOLVER});
        constructor(w);
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "getColor", "(L" + BIOME + ";DD)I", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 1);
        v.visitFieldInsn(GETFIELD, BIOME, "color", "I");
        v.visitInsn(ICONST_3);
        v.visitInsn(IMUL);
        v.visitVarInsn(DLOAD, 2);
        v.visitInsn(D2I);
        v.visitInsn(ICONST_1);
        v.visitInsn(IAND);
        v.visitInsn(IADD);
        v.visitInsn(IRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        return w.toByteArray();
    }

    private static byte[] cache() {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, CACHE, null, "java/lang/Object", null);
        constructor(w);
        String[][] ms = {{"getColor", "(L" + POS + ";)I"}, {"invalidateForChunk", "(II)V"}, {"invalidateAll", "()V"}};
        for (String[] md : ms) {
            MethodVisitor v = w.visitMethod(ACC_PUBLIC, md[0], md[1], null, null);
            v.visitCode();
            if (md[1].endsWith("I")) {
                v.visitInsn(ICONST_0);
                v.visitInsn(IRETURN);
            } else {
                v.visitInsn(RETURN);
            }
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        w.visitEnd();
        return w.toByteArray();
    }

    private static byte[] level(int version, String kind) {
        ClassWriter w = writer();
        w.visit(version, ACC_PUBLIC | ACC_SUPER, LEVEL, null, "java/lang/Object", null);
        w.visitField(ACC_PUBLIC | ACC_STATIC, "radius", "I", null, null).visitEnd();
        w.visitField(ACC_PUBLIC | ACC_STATIC, "data", "I", null, null).visitEnd();
        w.visitField(ACC_PUBLIC | ACC_STATIC, "lookups", "I", null, null).visitEnd();
        w.visitField(ACC_PUBLIC, "cache", "Ljava/lang/Object;", null, null).visitEnd();
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "<init>", "()V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        v.visitVarInsn(ALOAD, 0);
        v.visitTypeInsn(NEW, CACHE);
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, CACHE, "<init>", "()V", false);
        v.visitFieldInsn(PUTFIELD, LEVEL, "cache", "Ljava/lang/Object;");
        v.visitLdcInsn("ClientLevel");
        v.visitInsn(POP);
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        // Holder getBiome(Pos p): lookups++; return new HolderImpl(((x * 7 + z * 13 + y) >> 2) + data &amp; 15)
        v = w.visitMethod(ACC_PUBLIC, "getBiome", "(L" + POS + ";)L" + HOLDER + ";", null, null);
        v.visitCode();
        v.visitFieldInsn(GETSTATIC, LEVEL, "lookups", "I");
        v.visitInsn(ICONST_1);
        v.visitInsn(IADD);
        v.visitFieldInsn(PUTSTATIC, LEVEL, "lookups", "I");
        v.visitTypeInsn(NEW, HOLDER_IMPL);
        v.visitInsn(DUP);
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKEVIRTUAL, POS, "getX", "()I", false);
        v.visitIntInsn(BIPUSH, 7);
        v.visitInsn(IMUL);
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKEVIRTUAL, POS, "getZ", "()I", false);
        v.visitIntInsn(BIPUSH, 13);
        v.visitInsn(IMUL);
        v.visitInsn(IADD);
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKEVIRTUAL, POS, "getY", "()I", false);
        v.visitInsn(IADD);
        v.visitInsn(ICONST_2);
        v.visitInsn(ISHR);
        v.visitFieldInsn(GETSTATIC, LEVEL, "data", "I");
        v.visitInsn(IADD);
        v.visitIntInsn(BIPUSH, 15);
        v.visitInsn(IAND);
        v.visitMethodInsn(INVOKESPECIAL, HOLDER_IMPL, "<init>", "(I)V", false);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        blend(w, kind);

        if (!"no-cache".equals(kind)) {
            // int getBlockTint(Pos p, ColorResolver r) { return ((TintCache) this.cache).getColor(p); }
            v = w.visitMethod(ACC_PUBLIC, "getBlockTint", "(L" + POS + ";L" + RESOLVER + ";)I", null, null);
            v.visitCode();
            v.visitVarInsn(ALOAD, 0);
            v.visitFieldInsn(GETFIELD, LEVEL, "cache", "Ljava/lang/Object;");
            v.visitTypeInsn(CHECKCAST, CACHE);
            v.visitVarInsn(ASTORE, 3);
            v.visitVarInsn(ALOAD, 3);
            v.visitVarInsn(ALOAD, 1);
            v.visitMethodInsn(INVOKEVIRTUAL, CACHE, "getColor", "(L" + POS + ";)I", false);
            v.visitInsn(IRETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        v = w.visitMethod(ACC_PUBLIC, "onChunkLoaded", "(II)V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, LEVEL, "cache", "Ljava/lang/Object;");
        v.visitTypeInsn(CHECKCAST, CACHE);
        v.visitVarInsn(ILOAD, 1);
        v.visitVarInsn(ILOAD, 2);
        v.visitMethodInsn(INVOKEVIRTUAL, CACHE, "invalidateForChunk", "(II)V", false);
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "clearTintCaches", "()V", null, null);
        v.visitCode();
        for (int i = 0; i < ("clear-twice".equals(kind) ? 2 : 1); i++) {
            v.visitVarInsn(ALOAD, 0);
            v.visitFieldInsn(GETFIELD, LEVEL, "cache", "Ljava/lang/Object;");
            v.visitTypeInsn(CHECKCAST, CACHE);
            v.visitMethodInsn(INVOKEVIRTUAL, CACHE, "invalidateAll", "()V", false);
        }
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        if ("mixin".equals(kind)) {
            v = w.visitMethod(ACC_PRIVATE | ACC_STATIC, "handler$zza000$tints$blend", "()V", null, null);
            v.visitCode();
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        w.visitEnd();
        return w.toByteArray();
    }

    /** calculateBlockTint as in 1.21.1 (the sum of all three colour parts is one sum here). */
    private static void blend(ClassWriter w, String kind) {
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "calculateBlockTint", "(L" + POS + ";L" + RESOLVER + ";)I", null,
                null);
        v.visitCode();
        if ("mixin".equals(kind)) {
            v.visitMethodInsn(INVOKESTATIC, LEVEL, "handler$zza000$tints$blend", "()V", false);
        }
        v.visitFieldInsn(GETSTATIC, LEVEL, "radius", "I");
        v.visitVarInsn(ISTORE, 3);
        Label blendIt = new Label();
        v.visitVarInsn(ILOAD, 3);
        v.visitJumpInsn(IFNE, blendIt);
        lookup(v, 1, POS);
        v.visitInsn(IRETURN);
        v.visitLabel(blendIt);
        // n = (2r+1)^2
        v.visitVarInsn(ILOAD, 3);
        v.visitInsn(ICONST_2);
        v.visitInsn(IMUL);
        v.visitInsn(ICONST_1);
        v.visitInsn(IADD);
        v.visitVarInsn(ILOAD, 3);
        v.visitInsn(ICONST_2);
        v.visitInsn(IMUL);
        v.visitInsn(ICONST_1);
        v.visitInsn(IADD);
        v.visitInsn(IMUL);
        v.visitVarInsn(ISTORE, 4);
        v.visitInsn(ICONST_0);
        v.visitVarInsn(ISTORE, 5);
        v.visitTypeInsn(NEW, CURSOR);
        v.visitInsn(DUP);
        coord(v, "getX", ISUB);
        coord(v, "getY", -1);
        coord(v, "getZ", ISUB);
        coord(v, "getX", IADD);
        coord(v, "getY", -1);
        coord(v, "getZ", IADD);
        v.visitMethodInsn(INVOKESPECIAL, CURSOR, "<init>", "(IIIIII)V", false);
        v.visitVarInsn(ASTORE, 6);
        v.visitTypeInsn(NEW, MPOS);
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, MPOS, "<init>", "()V", false);
        v.visitVarInsn(ASTORE, 7);
        Label loop = new Label();
        Label done = new Label();
        v.visitLabel(loop);
        v.visitVarInsn(ALOAD, 6);
        v.visitMethodInsn(INVOKEVIRTUAL, CURSOR, "advance", "()Z", false);
        v.visitJumpInsn(IFEQ, done);
        v.visitVarInsn(ALOAD, 7);
        v.visitVarInsn(ALOAD, 6);
        v.visitMethodInsn(INVOKEVIRTUAL, CURSOR, "nextX", "()I", false);
        v.visitVarInsn(ALOAD, 6);
        v.visitMethodInsn(INVOKEVIRTUAL, CURSOR, "nextY", "()I", false);
        v.visitVarInsn(ALOAD, 6);
        v.visitMethodInsn(INVOKEVIRTUAL, CURSOR, "nextZ", "()I", false);
        v.visitMethodInsn(INVOKEVIRTUAL, MPOS, "set", "(III)L" + MPOS + ";", false);
        v.visitInsn(POP);
        lookup(v, 7, MPOS);
        v.visitVarInsn(ILOAD, 5);
        v.visitInsn(IADD);
        v.visitVarInsn(ISTORE, 5);
        if ("three".equals(kind)) {
            v.visitVarInsn(ALOAD, 0);
            v.visitVarInsn(ALOAD, 7);
            v.visitMethodInsn(INVOKEVIRTUAL, LEVEL, "getBiome", "(L" + POS + ";)L" + HOLDER + ";", false);
            v.visitInsn(POP);
        }
        v.visitJumpInsn(GOTO, loop);
        v.visitLabel(done);
        v.visitVarInsn(ILOAD, 5);
        v.visitVarInsn(ILOAD, 4);
        v.visitInsn(IDIV);
        v.visitInsn(IRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
    }

    /** resolver.getColor((Biome) this.getBiome(p).value(), p.getX(), p.getZ()) for the position in local `var`. */
    private static void lookup(MethodVisitor v, int var, String type) {
        v.visitVarInsn(ALOAD, 2);
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, var);
        v.visitMethodInsn(INVOKEVIRTUAL, LEVEL, "getBiome", "(L" + POS + ";)L" + HOLDER + ";", false);
        v.visitMethodInsn(INVOKEINTERFACE, HOLDER, "value", "()Ljava/lang/Object;", true);
        v.visitTypeInsn(CHECKCAST, BIOME);
        v.visitVarInsn(ALOAD, var);
        v.visitMethodInsn(INVOKEVIRTUAL, type, "getX", "()I", false);
        v.visitInsn(I2D);
        v.visitVarInsn(ALOAD, var);
        v.visitMethodInsn(INVOKEVIRTUAL, type, "getZ", "()I", false);
        v.visitInsn(I2D);
        v.visitMethodInsn(INVOKEINTERFACE, RESOLVER, "getColor", "(L" + BIOME + ";DD)I", true);
    }

    /** pos.getX() - r (op ISUB), + r (IADD), or alone (-1). */
    private static void coord(MethodVisitor v, String getter, int op) {
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKEVIRTUAL, POS, getter, "()I", false);
        if (op >= 0) {
            v.visitVarInsn(ILOAD, 3);
            v.visitInsn(op);
        }
    }
}
