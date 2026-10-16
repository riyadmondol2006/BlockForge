package com.blockforge.lite.core;

import java.lang.reflect.Constructor;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.HashMap;
import java.util.Map;
import com.blockforge.lite.core.Tests.Game;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;

/** Self-test of lazy-collision-shape on a stand-in of BlockCollisions (and AbstractIterator, AABB, Shapes). */
final class LazyCollisionShapeTest extends PatchTest {
    private static final String BC = "net/minecraft/world/level/BlockCollisions";
    private static final String IT = LazyCollisionShape.ITERATOR;
    private static final String AABB = "net/minecraft/world/phys/AABB";
    private static final String VS = "net/minecraft/world/phys/shapes/VoxelShape";
    private static final String SH = "net/minecraft/world/phys/shapes/Shapes";

    void lazyShape() throws Exception {
        for (int version : new int[] {V1_8, V17}) {
            if (!runs(version)) {
                continue;
            }
            String report = reportPath("lazyshape");
            Game g = load(factory.apply("quiet,report=" + report), classes(version, "vanilla"));
            expect(g.transformed.contains(BC), "BlockCollisions stand-in patched (class version " + version + ")");
            expect(line(report, "lazy-collision-shape").contains("applied"), "reported as applied");
            Class<?> bc = g.loader.loadClass(BC.replace('/', '.'));
            Class<?> sh = g.loader.loadClass(SH.replace('/', '.'));
            expect(!Modifier.isFinal(bc.getDeclaredField("entityShape").getModifiers()), "the field is not final");
            Object box = g.loader.loadClass(AABB.replace('/', '.')).getConstructor().newInstance();
            Constructor<?> make = bc.getConstructor(Object.class, g.loader.loadClass(AABB.replace('/', '.')));
            Method next = bc.getDeclaredMethod("computeNext");
            next.setAccessible(true);
            Object it = make.newInstance("level", box);
            expect(sh.getField("made").getInt(null) == 0, "no shape made by the constructor");
            expect(next.invoke(it) == null && sh.getField("made").getInt(null) == 0, "no partial block: none made");
            bc.getField("partial").setBoolean(it, true);
            expect("partial".equals(next.invoke(it)), "a partial block is found");
            Object shape = sh.getField("joined").get(null);
            expect(shape != null && sh.getField("made").getInt(null) == 1, "a partial block: made once");
            expect(shape != null && shape.getClass().getField("box").get(shape) == box, "from the iterator's box");
            next.invoke(it);
            expect(sh.getField("joined").get(null) == shape && sh.getField("made").getInt(null) == 1,
                    "and kept: the same shape again");
            expect(bc.getField("context").get(it) != null, "the other constructor call is untouched");
        }
        Game g = load(null, classes(V1_8, "vanilla"));
        Class<?> bc = g.loader.loadClass(BC.replace('/', '.'));
        Object box = g.loader.loadClass(AABB.replace('/', '.')).getConstructor().newInstance();
        bc.getConstructor(Object.class, g.loader.loadClass(AABB.replace('/', '.'))).newInstance("level", box);
        expect(g.loader.loadClass(SH.replace('/', '.')).getField("made").getInt(null) == 1,
                "without the patch the constructor makes the shape (the test is real)");
    }

    void lazyShapeLeftAlone() throws Exception {
        for (String kind : new String[] {"other-read", "written", "mixin", "not-final-box"}) {
            String report = reportPath("lazyshape-" + kind);
            Game g = load(factory.apply("quiet,report=" + report), classes(V1_8, kind));
            expect(!g.transformed.contains(BC), kind + ": left alone");
            expect(line(report, "lazy-collision-shape").contains("skipped"), kind + ": reported as skipped");
        }
    }

    /**
     * AbstractIterator, AABB, VoxelShape, Shapes (counts the shapes it makes) and BlockCollisions as in 1.17 - 26.x.
     * kind: "vanilla"; "other-read" (a second method reads the shape); "written" (a method sets it); "mixin"
     * (computeNext calls a mixin handler); "not-final-box" (the box field is not final).
     */
    static Map<String, byte[]> classes(int version, String kind) {
        Map<String, byte[]> m = new HashMap<String, byte[]>();
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER | ACC_ABSTRACT, IT, null, "java/lang/Object", null);
        constructor(w);
        w.visitMethod(ACC_PROTECTED | ACC_ABSTRACT, "computeNext", "()Ljava/lang/Object;", null, null).visitEnd();
        MethodVisitor v = w.visitMethod(ACC_PROTECTED | ACC_FINAL, "endOfData", "()Ljava/lang/Object;", null, null);
        v.visitCode();
        v.visitInsn(ACONST_NULL);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        m.put(IT, w.toByteArray());

        w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, AABB, null, "java/lang/Object", null);
        constructor(w);
        w.visitEnd();
        m.put(AABB, w.toByteArray());

        w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, VS, null, "java/lang/Object", null);
        w.visitField(ACC_PUBLIC | ACC_FINAL, "box", "L" + AABB + ";", null, null).visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "<init>", "(L" + AABB + ";)V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 1);
        v.visitFieldInsn(PUTFIELD, VS, "box", "L" + AABB + ";");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        m.put(VS, w.toByteArray());

        w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, SH, null, "java/lang/Object", null);
        w.visitField(ACC_PUBLIC | ACC_STATIC, "made", "I", null, null).visitEnd();
        constructor(w);
        v = w.visitMethod(ACC_PUBLIC | ACC_STATIC, "create", "(L" + AABB + ";)L" + VS + ";", null, null);
        v.visitCode();
        v.visitFieldInsn(GETSTATIC, SH, "made", "I");
        v.visitInsn(ICONST_1);
        v.visitInsn(IADD);
        v.visitFieldInsn(PUTSTATIC, SH, "made", "I");
        v.visitTypeInsn(NEW, VS);
        v.visitInsn(DUP);
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, VS, "<init>", "(L" + AABB + ";)V", false);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        // Shapes.block() and joinIsNotEmpty(a, b, op), which remembers the shape it got as b
        w.visitField(ACC_PUBLIC | ACC_STATIC, "AND", "Ljava/lang/Object;", null, null).visitEnd();
        w.visitField(ACC_PUBLIC | ACC_STATIC, "joined", "L" + VS + ";", null, null).visitEnd();
        v = w.visitMethod(ACC_PUBLIC | ACC_STATIC, "block", "()L" + VS + ";", null, null);
        v.visitCode();
        v.visitInsn(ACONST_NULL);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC | ACC_STATIC, "join", "(L" + VS + ";L" + VS + ";Ljava/lang/Object;)Z", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 1);
        v.visitFieldInsn(PUTSTATIC, SH, "joined", "L" + VS + ";");
        v.visitInsn(ICONST_1);
        v.visitInsn(IRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        // like CollisionContext.of(entity): a static call into a field, from an argument that is not kept
        v = w.visitMethod(ACC_PUBLIC | ACC_STATIC, "context", "(Ljava/lang/Object;)Ljava/lang/Object;", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        m.put(SH, w.toByteArray());

        w = writer();
        w.visit(version, ACC_PUBLIC | ACC_SUPER, BC, null, IT, null);
        w.visitField(ACC_PRIVATE | (kind.equals("not-final-box") ? 0 : ACC_FINAL), "box", "L" + AABB + ";", null,
                null).visitEnd();
        w.visitField(ACC_PRIVATE | ACC_FINAL, "entityShape", "L" + VS + ";", null, null).visitEnd();
        w.visitField(ACC_PUBLIC | ACC_FINAL, "context", "Ljava/lang/Object;", null, null).visitEnd();
        w.visitField(ACC_PUBLIC, "partial", "Z", null, null).visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "<init>", "(Ljava/lang/Object;L" + AABB + ";)V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, IT, "<init>", "()V", false);
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKESTATIC, SH, "context", "(Ljava/lang/Object;)Ljava/lang/Object;", false);
        v.visitFieldInsn(PUTFIELD, BC, "context", "Ljava/lang/Object;");
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 2);
        v.visitMethodInsn(INVOKESTATIC, SH, "create", "(L" + AABB + ";)L" + VS + ";", false);
        v.visitFieldInsn(PUTFIELD, BC, "entityShape", "L" + VS + ";");
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 2);
        v.visitFieldInsn(PUTFIELD, BC, "box", "L" + AABB + ";");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        v = w.visitMethod(ACC_PROTECTED, "computeNext", "()Ljava/lang/Object;", null, null);
        v.visitCode();
        if (kind.equals("mixin")) {
            v.visitMethodInsn(INVOKESTATIC, BC, "handler$zza000$collide$next", "()V", false);
        }
        // if (Shapes.block() == ...) ...; if (partial && Shapes.join(null, this.entityShape, AND)) return "partial"
        Label end = new Label();
        v.visitMethodInsn(INVOKESTATIC, SH, "block", "()L" + VS + ";", false);
        v.visitInsn(POP);
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, BC, "partial", "Z");
        v.visitJumpInsn(IFEQ, end);
        v.visitInsn(ACONST_NULL);
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, BC, "entityShape", "L" + VS + ";");
        v.visitFieldInsn(GETSTATIC, SH, "AND", "Ljava/lang/Object;");
        v.visitMethodInsn(INVOKESTATIC, SH, "join", "(L" + VS + ";L" + VS + ";Ljava/lang/Object;)Z", false);
        v.visitJumpInsn(IFEQ, end);
        v.visitLdcInsn("partial");
        v.visitInsn(ARETURN);
        v.visitLabel(end);
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKEVIRTUAL, BC, "endOfData", "()Ljava/lang/Object;", false);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        if (kind.equals("mixin")) {
            v = w.visitMethod(ACC_PRIVATE | ACC_STATIC, "handler$zza000$collide$next", "()V", null, null);
            v.visitCode();
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        if (kind.equals("other-read")) {
            v = w.visitMethod(ACC_PUBLIC, "shape", "()L" + VS + ";", null, null);
            v.visitCode();
            v.visitVarInsn(ALOAD, 0);
            v.visitFieldInsn(GETFIELD, BC, "entityShape", "L" + VS + ";");
            v.visitInsn(ARETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        if (kind.equals("written")) {
            v = w.visitMethod(ACC_PUBLIC, "reset", "()V", null, null);
            v.visitCode();
            v.visitVarInsn(ALOAD, 0);
            v.visitInsn(ACONST_NULL);
            v.visitFieldInsn(PUTFIELD, BC, "entityShape", "L" + VS + ";");
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        w.visitEnd();
        m.put(BC, w.toByteArray());
        return m;
    }
}
