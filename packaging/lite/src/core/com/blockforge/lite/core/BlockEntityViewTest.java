package com.blockforge.lite.core;

import java.util.HashMap;
import java.util.Map;
import com.blockforge.lite.core.Tests.Game;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;

/** Self-test of block-entity-view: src/stubs plus stand-ins of the dispatcher, the renderer interface and renderers. */
final class BlockEntityViewTest extends PatchTest {
    static final String D = BlockEntityView.DISPATCHER;
    static final String R = BlockEntityView.RENDERER;
    static final String BE = BlockEntityView.BLOCK_ENTITY;
    static final String VEC3 = "net/minecraft/world/phys/Vec3";
    static final String CHEST = "net/minecraft/client/renderer/blockentity/ChestRenderer";
    static final String BEACON = "net/minecraft/client/renderer/blockentity/BeaconRenderer";
    static final String MOD = "t/ModChestRenderer";

    void testsTheGamesCompactRenderers() throws Exception {
        for (int version : new int[] {V1_8, V17}) {
            if (!runs(version)) {
                continue;
            }
            String report = reportPath("be");
            Game g = load(factory.apply("quiet,report=" + report), classes(version, ""));
            expect(g.transformed.contains(D), "dispatcher stand-in patched (" + version + ")");
            expect(line(report, "block-entity-view").contains("applied"), "reported as applied");
            Scene s = new Scene(g);
            expect(s.drawn(CHEST, 5, true), "a chest in view: drawn");
            expect(!s.drawn(CHEST, 20, true), "a chest out of view: not drawn");
            expect(s.drawn(CHEST, 10, true), "a chest whose box reaches into view: drawn");
            expect(!s.drawn(CHEST, 5, false), "too far (the renderer says no): not drawn");
            expect(s.drawn(BEACON, 20, true), "a beacon out of view: drawn (its beam may be seen)");
            expect(s.drawn(MOD, 20, true), "a mod's renderer out of view: drawn");
            s.noLevel();
            expect(s.drawn(CHEST, 20, true), "no level: drawn");
            Scene p = new Scene(load(null, classes(version, "")));
            expect(p.drawn(CHEST, 20, true), "without the patch: drawn (the test is real)");
        }
    }

    void leftAlone() throws Exception {
        for (String kind : new String[] {"mixin", "twice"}) {
            String report = reportPath("be-" + kind);
            Game g = load(factory.apply("quiet,report=" + report), classes(V1_8, kind));
            expect(!g.transformed.contains(D), kind + ": left alone");
            expect(line(report, "block-entity-view").contains("skipped"), kind + ": reported as skipped");
        }
        String report = reportPath("be-iris");
        Game g = load(factory.apply("quiet,modids=iris,report=" + report), classes(V1_8, ""));
        expect(!g.transformed.contains(D), "with Iris (its shadow pass draws block entities too): left alone");
    }

    /** Drives the stand-ins: the camera's frustum sees what starts before x = 10. */
    static final class Scene {
        final Game g;
        final Object mc;

        Scene(Game g) throws Exception {
            this.g = g;
            Class<?> m = cls("net/minecraft/client/Minecraft");
            mc = m.newInstance();
            m.getField("instance").set(null, mc);
            mc.getClass().getField("level").set(mc, cls("net/minecraft/client/multiplayer/ClientLevel").newInstance());
            Object camera = cls("net/minecraft/client/renderer/GameRenderer").getField("camera")
                    .get(mc.getClass().getField("gameRenderer").get(mc));
            Object frustum = camera.getClass().getField("cullFrustum").get(camera);
            frustum.getClass().getField("endX").setDouble(frustum, 10.0);
        }

        Class<?> cls(String n) throws Exception {
            return g.loader.loadClass(n.replace('/', '.'));
        }

        void noLevel() throws Exception {
            mc.getClass().getField("level").set(mc, null);
        }

        boolean drawn(String renderer, int x, boolean near) throws Exception {
            Object be = cls(BE).newInstance();
            be.getClass().getField("pos").set(be, cls("net/minecraft/core/BlockPos")
                    .getConstructor(int.class, int.class, int.class).newInstance(x, 64, 0));
            Object r = cls(renderer).newInstance();
            r.getClass().getField("near").setBoolean(r, near);
            Object d = cls(D).newInstance();
            d.getClass().getField("renderer").set(d, r);
            return d.getClass().getMethod("tryExtractRenderState", cls(BE), float.class, Object.class, boolean.class)
                    .invoke(d, be, 0f, null, false) != null;
        }
    }

    /** kind: "" = 26.3 shape, "mixin" = a mixin handler call, "twice" = two distance tests. */
    static Map<String, byte[]> classes(int version, String kind) throws Exception {
        Map<String, byte[]> m = HiddenEntitiesTest.classes(V1_8, "");
        m.remove(HiddenEntities.RENDERER);
        for (String s : new String[] {"net/minecraft/core/BlockPos", "net/minecraft/core/Vec3i", BE}) {
            m.put(s, Template.bytes(BlockEntityViewTest.class.getClassLoader().getResourceAsStream(s + ".class")));
        }
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, VEC3, null, "java/lang/Object", null);
        constructor(w);
        w.visitEnd();
        m.put(VEC3, w.toByteArray());
        w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_ABSTRACT | ACC_INTERFACE, R, null, "java/lang/Object", null);
        w.visitMethod(ACC_PUBLIC | ACC_ABSTRACT, "shouldRender", BlockEntityView.SHOULD, null, null).visitEnd();
        w.visitEnd();
        m.put(R, w.toByteArray());
        for (String r : new String[] {CHEST, BEACON, MOD}) {
            m.put(r, renderer(r));
        }
        m.put(D, dispatcher(version, kind));
        return m;
    }

    /** A renderer whose distance test answers its field "near". */
    private static byte[] renderer(String name) {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, name, null, "java/lang/Object", new String[] {R});
        w.visitField(ACC_PUBLIC, "near", "Z", null, null).visitEnd();
        constructor(w);
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "shouldRender", BlockEntityView.SHOULD, null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, name, "near", "Z");
        v.visitInsn(IRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        return w.toByteArray();
    }

    private static byte[] dispatcher(int version, String kind) {
        ClassWriter w = writer();
        w.visit(version, ACC_PUBLIC | ACC_SUPER, D, null, "java/lang/Object", null);
        w.visitField(ACC_PUBLIC, "renderer", "Ljava/lang/Object;", null, null).visitEnd();
        w.visitField(ACC_PRIVATE, "cameraPos", "L" + VEC3 + ";", null, null).visitEnd();
        constructor(w);
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "tryExtractRenderState",
                "(L" + BE + ";FLjava/lang/Object;Z)Ljava/lang/Object;", null, null);
        v.visitCode();
        if (kind.equals("mixin")) {
            v.visitMethodInsn(INVOKESTATIC, D, "handler$zzd000$cull$extract", "()V", false);
        }
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, D, "renderer", "Ljava/lang/Object;");
        v.visitTypeInsn(CHECKCAST, R);
        v.visitVarInsn(ASTORE, 5);
        Label yes = new Label();
        for (int i = 0; i < (kind.equals("twice") ? 2 : 1); i++) {
            v.visitVarInsn(ALOAD, 5);
            v.visitVarInsn(ALOAD, 1);
            v.visitVarInsn(ALOAD, 0);
            v.visitFieldInsn(GETFIELD, D, "cameraPos", "L" + VEC3 + ";");
            v.visitMethodInsn(INVOKEINTERFACE, R, "shouldRender", BlockEntityView.SHOULD, true);
            if (i == 0 && kind.equals("twice")) {
                v.visitInsn(POP);
            }
        }
        v.visitJumpInsn(IFNE, yes);
        v.visitInsn(ACONST_NULL);
        v.visitInsn(ARETURN);
        v.visitLabel(yes);
        v.visitLdcInsn("state");
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        if (kind.equals("mixin")) {
            v = w.visitMethod(ACC_PRIVATE | ACC_STATIC, "handler$zzd000$cull$extract", "()V", null, null);
            v.visitCode();
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        w.visitEnd();
        return w.toByteArray();
    }
}
