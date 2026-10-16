package com.blockforge.lite.core;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import com.blockforge.lite.core.Tests.Game;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;

/**
 * Self-test of hidden-entities: the stand-ins of the game's classes from src/stubs plus an EntityRenderer stand-in with
 * the game's shape of shouldRender.
 */
final class HiddenEntitiesTest extends PatchTest {
    static final String RENDERER = HiddenEntities.RENDERER;
    static final String[] STUBS = {"net/minecraft/client/Minecraft", "net/minecraft/client/Camera",
        "net/minecraft/client/renderer/GameRenderer", "net/minecraft/client/renderer/LevelRenderer",
        "net/minecraft/client/renderer/chunk/SectionRenderDispatcher",
        "net/minecraft/client/renderer/chunk/SectionRenderDispatcher$RenderSection",
        "net/minecraft/client/renderer/culling/Frustum", "net/minecraft/client/multiplayer/ClientLevel",
        "net/minecraft/world/level/Level", "net/minecraft/world/level/LevelHeightAccessor",
        "net/minecraft/world/level/chunk/ChunkAccess",
        "net/minecraft/world/level/chunk/LevelChunk", "net/minecraft/world/level/chunk/LevelChunkSection",
        "net/minecraft/core/SectionPos", "net/minecraft/world/entity/Entity", "net/minecraft/world/entity/Leashable",
        "net/minecraft/world/entity/LeashedForTest", "net/minecraft/world/entity/player/Player",
        "net/minecraft/world/phys/AABB", "it/unimi/dsi/fastutil/objects/ObjectArrayList"};

    void hidesOnlyWhatCannotBeSeen() throws Exception {
        int version = runs(V17) ? V17 : V1_8;
        for (boolean partialTicks : new boolean[] {true, false}) {
            String report = reportPath("hidden");
            String shape = partialTicks ? "" : "26.2";
            Game g = load(factory.apply("quiet,level=2,report=" + report), classes(version, shape));
            expect(g.transformed.contains(RENDERER), "EntityRenderer stand-in patched (" + partialTicks + ")");
            expect(line(report, "hidden-entities").contains("applied"), "reported as applied");
            World w = new World(g);
            w.visible(0, 4, 0);
            w.visible(1, 4, 0);
            Object cow = w.entity("net/minecraft/world/entity/Entity");
            expect(w.drawn(cow, 1, 65, 1), "in a section in view: drawn");
            expect(w.drawn(cow, 32.2, 65, 1), "half a block from a section in view: drawn");
            expect(!w.drawn(cow, 100, 65, 100), "only in a section the search did not reach: not drawn");
            expect(!w.drawn(cow, -100, 20, 40), "the same at negative positions");
            w.air(6, 4, 6);
            expect(w.drawn(cow, 100, 65, 100), "in a section without blocks: drawn");
            expect(w.drawn(cow, 100, 400, 100), "outside the world: drawn");
            expect(w.drawn(cow, 160, 65, 160, 50), "a large box: drawn");
            expect(w.drawn(cow, Double.NaN, 65, 160), "a broken box: drawn");
            for (String kind : new String[] {"player", "named", "glowing", "passenger", "vehicle", "leashed",
                "camera"}) {
                Object e = w.entity(kind.equals("player") ? "net/minecraft/world/entity/player/Player"
                        : kind.equals("leashed") ? "net/minecraft/world/entity/LeashedForTest"
                        : "net/minecraft/world/entity/Entity");
                if (kind.equals("leashed")) {
                    w.set(e, "holder", w.entity("net/minecraft/world/entity/Entity"));
                } else if (kind.equals("camera")) {
                    w.set(w.mc, "cameraEntity", e);
                } else if (!kind.equals("player")) {
                    w.set(e, kind, true);
                }
                expect(w.drawn(e, 200, 65, 200), kind + ": drawn");
            }
            // another frustum (a mod's second camera, a debug view): not touched
            expect(w.drawn(cow, 100, 65, 100, 1, w.newFrustum()), "another frustum: drawn");
            // the game's frustum test says no: still no
            Object none = w.newFrustum();
            w.set(none, "all", false);
            expect(!w.drawn(cow, 1, 65, 1, 1, none), "outside the frustum: not drawn");
            // the next frame's list
            w.nextFrame();
            w.visible(12, 4, 12);
            expect(w.drawn(cow, 200, 65, 200), "next frame, its section came into view: drawn");
            w.nextFrame();
            w.clearVisible();
            w.visible(0, 4, 0);
            expect(!w.drawn(cow, 200, 65, 200), "and out of view again: not drawn");
            w.clearVisible();
            expect(w.drawn(cow, 200, 65, 200), "nothing known (no list): drawn");
            // without the patch it would be drawn (the test is real)
            Game plain = load(null, classes(version, shape));
            World p = new World(plain);
            p.visible(0, 4, 0);
            expect(p.drawn(p.entity("net/minecraft/world/entity/Entity"), 100, 65, 100), "without the patch: drawn");
        }
    }

    void onlyOnLevel2() throws Exception {
        String report = reportPath("hidden-l1");
        Game g = load(factory.apply("quiet,report=" + report), classes(V1_8, ""));
        expect(!g.transformed.contains(RENDERER), "level 1: not applied");
        expect(line(report, "hidden-entities").contains("skipped"), "level 1: reported as skipped");
    }

    void leftAlone() throws Exception {
        for (String kind : new String[] {"mixin", "other-test", "no-section-list"}) {
            Map<String, byte[]> c = classes(V1_8, kind);
            String report = reportPath("hidden-" + kind);
            Game g = load(factory.apply("quiet,level=2,report=" + report), c);
            expect(!g.transformed.contains(RENDERER), kind + ": left alone");
            expect(line(report, "hidden-entities").contains("skipped"), kind + ": reported as skipped");
        }
        String report = reportPath("hidden-sodium");
        Game g = load(factory.apply("quiet,level=2,modids=sodium,report=" + report), classes(V1_8, ""));
        expect(!g.transformed.contains(RENDERER), "with Sodium: left alone");
        expect(line(report, "hidden-entities").contains("Sodium"), "with Sodium: reported");
    }

    /** Drives the stand-ins. */
    static final class World {
        final Game g;
        final Object mc;
        final Object level;
        final Object renderer;
        final Class<?> aabb;
        final Class<?> frustumClass;
        final Method shouldRender;
        final boolean partialTicks;

        World(Game g) throws Exception {
            this.g = g;
            Class<?> m = cls("net/minecraft/client/Minecraft");
            mc = m.newInstance();
            m.getField("instance").set(null, mc);
            level = cls("net/minecraft/client/multiplayer/ClientLevel").newInstance();
            set(mc, "level", level);
            Class<?> r = cls(RENDERER);
            renderer = r.newInstance();
            aabb = cls("net/minecraft/world/phys/AABB");
            frustumClass = cls("net/minecraft/client/renderer/culling/Frustum");
            Method found = null;
            for (Method x : r.getMethods()) {
                if (x.getName().equals("shouldRender")) {
                    found = x;
                }
            }
            shouldRender = found;
            partialTicks = found.getParameterTypes().length == 6;
        }

        Class<?> cls(String name) throws Exception {
            return g.loader.loadClass(name.replace('/', '.'));
        }

        Object entity(String cls) throws Exception {
            return cls(cls).newInstance();
        }

        void set(Object o, String field, Object v) throws Exception {
            Field f = o.getClass().getField(field);
            f.set(o, v);
        }

        Object get(Object o, String field) throws Exception {
            return o.getClass().getField(field).get(o);
        }

        Object frustum() throws Exception {
            return get(get(get(mc, "gameRenderer"), "camera"), "cullFrustum");
        }

        Object newFrustum() throws Exception {
            return frustumClass.newInstance();
        }

        void nextFrame() throws Exception {
            set(get(get(mc, "gameRenderer"), "camera"), "cullFrustum", newFrustum());
        }

        @SuppressWarnings("unchecked")
        void visible(int x, int y, int z) throws Exception {
            long node = (Long) cls("net/minecraft/core/SectionPos").getMethod("asLong", int.class, int.class, int.class)
                    .invoke(null, x, y, z);
            Object section = cls("net/minecraft/client/renderer/chunk/SectionRenderDispatcher$RenderSection")
                    .getConstructor(long.class).newInstance(node);
            ((List<Object>) get(get(mc, "levelRenderer"), "sections")).add(section);
        }

        void clearVisible() throws Exception {
            ((List<?>) get(get(mc, "levelRenderer"), "sections")).clear();
        }

        @SuppressWarnings("unchecked")
        void air(int x, int y, int z) throws Exception {
            ((java.util.Set<String>) get(level, "air")).add(x + "," + y + "," + z);
        }

        boolean drawn(Object entity, double x, double y, double z) throws Exception {
            return drawn(entity, x, y, z, 1, frustum());
        }

        boolean drawn(Object entity, double x, double y, double z, double size) throws Exception {
            return drawn(entity, x, y, z, size, frustum());
        }

        /** Asks shouldRender for an entity whose culling box is x..x+size (before the game's 0.5). */
        boolean drawn(Object entity, double x, double y, double z, double size, Object frustum) throws Exception {
            Object box = aabb.getConstructor(double.class, double.class, double.class, double.class, double.class,
                    double.class).newInstance(x, y, z, x + size, y + size, z + size);
            renderer.getClass().getField("next").set(null, box);
            Object r = partialTicks ? shouldRender.invoke(renderer, entity, frustum, 0.0, 0.0, 0.0, 0f)
                    : shouldRender.invoke(renderer, entity, frustum, 0.0, 0.0, 0.0);
            return (Boolean) r;
        }
    }

    /**
     * The stand-ins: src/stubs as they are, and EntityRenderer. kind: "" = 26.3 shape, "26.2" = without the partial
     * ticks, "mixin" = a mixin handler call in shouldRender, "other-test" = the first test is not on the inflated
     * box, "no-section-list" = a LevelRenderer without visibleSections().
     */
    static Map<String, byte[]> classes(int version, String kind) throws Exception {
        Map<String, byte[]> m = new HashMap<String, byte[]>();
        for (String s : STUBS) {
            m.put(s, Template.bytes(HiddenEntitiesTest.class.getClassLoader().getResourceAsStream(s + ".class")));
        }
        if (kind.equals("no-section-list")) {
            ClassWriter w = writer();
            w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, "net/minecraft/client/renderer/LevelRenderer", null,
                    "java/lang/Object", null);
            constructor(w);
            w.visitEnd();
            m.put("net/minecraft/client/renderer/LevelRenderer", w.toByteArray());
        }
        m.put(RENDERER, renderer(version, kind));
        return m;
    }

    private static byte[] renderer(int version, String kind) {
        String entity = "Lnet/minecraft/world/entity/Entity;";
        String frustum = "Lnet/minecraft/client/renderer/culling/Frustum;";
        String box = "Lnet/minecraft/world/phys/AABB;";
        ClassWriter w = writer();
        w.visit(version, ACC_PUBLIC | ACC_SUPER, RENDERER, null, "java/lang/Object", null);
        w.visitField(ACC_PUBLIC | ACC_STATIC, "next", box, null, null).visitEnd();
        constructor(w);
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "getBoundingBoxForCulling", "(" + entity + ")" + box, null, null);
        v.visitCode();
        v.visitFieldInsn(GETSTATIC, RENDERER, "next", box);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        boolean pt = !kind.equals("26.2");
        v = w.visitMethod(ACC_PUBLIC, "shouldRender", "(" + entity + frustum + "DDD" + (pt ? "F" : "") + ")Z", null,
                null);
        v.visitCode();
        int boxVar = pt ? 10 : 9;
        if (kind.equals("mixin")) {
            v.visitMethodInsn(INVOKESTATIC, RENDERER, "handler$zzb000$cull$shouldRender", "()V", false);
        }
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKEVIRTUAL, RENDERER, "getBoundingBoxForCulling", "(" + entity + ")" + box, false);
        v.visitLdcInsn(0.5);
        v.visitMethodInsn(INVOKEVIRTUAL, "net/minecraft/world/phys/AABB", "inflate", "(D)" + box, false);
        v.visitVarInsn(ASTORE, boxVar);
        Label no = new Label();
        v.visitVarInsn(ALOAD, 2);
        if (kind.equals("other-test")) {
            v.visitVarInsn(ALOAD, 0);
            v.visitVarInsn(ALOAD, 1);
            v.visitMethodInsn(INVOKEVIRTUAL, RENDERER, "getBoundingBoxForCulling", "(" + entity + ")" + box, false);
        } else {
            v.visitVarInsn(ALOAD, boxVar);
        }
        v.visitMethodInsn(INVOKEVIRTUAL, "net/minecraft/client/renderer/culling/Frustum", "isVisible",
                "(" + box + ")Z", false);
        v.visitJumpInsn(IFEQ, no);
        v.visitInsn(ICONST_1);
        v.visitInsn(IRETURN);
        v.visitLabel(no);
        v.visitInsn(ICONST_0);
        v.visitInsn(IRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        if (kind.equals("mixin")) {
            v = w.visitMethod(ACC_PRIVATE | ACC_STATIC, "handler$zzb000$cull$shouldRender", "()V", null, null);
            v.visitCode();
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        w.visitEnd();
        return w.toByteArray();
    }
}
