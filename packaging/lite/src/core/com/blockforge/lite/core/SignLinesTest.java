package com.blockforge.lite.core;

import java.util.HashMap;
import java.util.Map;
import com.blockforge.lite.core.Tests.Game;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;

/** Self-test of sign-lines: src/stubs plus a sign renderer stand-in and a collector that counts submissions. */
final class SignLinesTest extends PatchTest {
    static final String SIGNS = SignLines.SIGNS;
    static final String COLLECTOR = SignLines.COLLECTOR;
    static final String FCS = "net/minecraft/util/FormattedCharSequence";
    static final String MODE = "net/minecraft/client/gui/Font$DisplayMode";
    static final String POSE = "com/mojang/blaze3d/vertex/PoseStack";
    static final String COUNTER = "t/Counter";
    static final String TEXT = "t/Text";

    void dropsOnlyEmptyLinesWithoutBackground() throws Exception {
        for (int version : new int[] {V1_8, V17}) {
            if (!runs(version)) {
                continue;
            }
            String report = reportPath("signs");
            Game g = load(factory.apply("quiet,report=" + report), classes(version, ""));
            expect(g.transformed.contains(SIGNS), "sign renderer stand-in patched (" + version + ")");
            expect(line(report, "sign-lines").contains("applied"), "reported as applied");
            expect(submits(g, 0) == 1, "one line with text, three empty: one submission");
            expect(submits(g, 0x40000000) == 4, "with a background colour the empty lines are drawn: four");
            expect(submits(load(null, classes(version, "")), 0) == 4, "without the patch: four (the test is real)");
        }
    }

    void leftAlone() throws Exception {
        for (String kind : new String[] {"mixin", "twice"}) {
            String report = reportPath("signs-" + kind);
            Game g = load(factory.apply("quiet,report=" + report), classes(V1_8, kind));
            expect(!g.transformed.contains(SIGNS), kind + ": left alone");
            expect(line(report, "sign-lines").contains("skipped"), kind + ": reported as skipped");
        }
        Game g = load(factory.apply("quiet,modids=immediatelyfast"), classes(V1_8, ""));
        expect(!g.transformed.contains(SIGNS), "with ImmediatelyFast: left alone");
    }

    /** Lines: text, empty, empty, empty; returns how many reached the collector. */
    private static int submits(Game g, int background) throws Exception {
        Class<?> signs = g.loader.loadClass(SIGNS.replace('/', '.'));
        Class<?> counter = g.loader.loadClass(COUNTER.replace('/', '.'));
        Class<?> fcs = g.loader.loadClass(FCS.replace('/', '.'));
        Object empty = fcs.getField("EMPTY").get(null);
        Object text = g.loader.loadClass(TEXT.replace('/', '.')).newInstance();
        Object c = counter.newInstance();
        signs.getField("background").setInt(null, background);
        signs.getMethod("submitSignText", g.loader.loadClass(COLLECTOR.replace('/', '.')), Object[].class)
                .invoke(signs.newInstance(), c, new Object[] {text, empty, empty, empty});
        return counter.getField("n").getInt(c);
    }

    /** kind: "" = 26.x shape, "mixin" = a mixin handler call, "twice" = two submission calls. */
    static Map<String, byte[]> classes(int version, String kind) throws Exception {
        Map<String, byte[]> m = new HashMap<String, byte[]>();
        for (String s : new String[] {POSE, "net/minecraft/client/gui/Font", MODE, FCS, FCS + "$1", COLLECTOR}) {
            m.put(s, Template.bytes(SignLinesTest.class.getClassLoader().getResourceAsStream(s + ".class")));
        }
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, TEXT, null, "java/lang/Object", new String[] {FCS});
        constructor(w);
        w.visitEnd();
        m.put(TEXT, w.toByteArray());
        // a collector that counts what it gets
        w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, COUNTER, null, "java/lang/Object", new String[] {COLLECTOR});
        w.visitField(ACC_PUBLIC, "n", "I", null, null).visitEnd();
        constructor(w);
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "submitText", SignLines.SUBMIT, null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitInsn(DUP);
        v.visitFieldInsn(GETFIELD, COUNTER, "n", "I");
        v.visitInsn(ICONST_1);
        v.visitInsn(IADD);
        v.visitFieldInsn(PUTFIELD, COUNTER, "n", "I");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        m.put(COUNTER, w.toByteArray());
        m.put(SIGNS, signs(version, kind));
        return m;
    }

    /** void submitSignText(SubmitNodeCollector c, Object[] lines): each line, its background from a static field. */
    private static byte[] signs(int version, String kind) {
        ClassWriter w = writer();
        w.visit(version, ACC_PUBLIC | ACC_SUPER, SIGNS, null, "java/lang/Object", null);
        w.visitField(ACC_PUBLIC | ACC_STATIC, "background", "I", null, null).visitEnd();
        constructor(w);
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "submitSignText", "(L" + COLLECTOR + ";[Ljava/lang/Object;)V",
                null, null);
        v.visitCode();
        if (kind.equals("mixin")) {
            v.visitMethodInsn(INVOKESTATIC, SIGNS, "handler$zze000$signs$text", "()V", false);
        }
        v.visitInsn(ICONST_0);
        v.visitVarInsn(ISTORE, 3);
        Label loop = new Label();
        Label end = new Label();
        v.visitLabel(loop);
        v.visitVarInsn(ILOAD, 3);
        v.visitVarInsn(ALOAD, 2);
        v.visitInsn(ARRAYLENGTH);
        v.visitJumpInsn(IF_ICMPGE, end);
        for (int i = 0; i < (kind.equals("twice") ? 2 : 1); i++) {
            v.visitVarInsn(ALOAD, 1);
            v.visitTypeInsn(NEW, POSE);
            v.visitInsn(DUP);
            v.visitMethodInsn(INVOKESPECIAL, POSE, "<init>", "()V", false);
            v.visitInsn(FCONST_0);
            v.visitInsn(FCONST_0);
            v.visitVarInsn(ALOAD, 2);
            v.visitVarInsn(ILOAD, 3);
            v.visitInsn(AALOAD);
            v.visitTypeInsn(CHECKCAST, FCS);
            v.visitInsn(ICONST_0);
            v.visitFieldInsn(GETSTATIC, MODE, "POLYGON_OFFSET", "L" + MODE + ";");
            v.visitIntInsn(BIPUSH, 15);
            v.visitInsn(ICONST_M1);
            v.visitFieldInsn(GETSTATIC, SIGNS, "background", "I");
            v.visitInsn(ICONST_0);
            v.visitMethodInsn(INVOKEINTERFACE, COLLECTOR, "submitText", SignLines.SUBMIT, true);
        }
        v.visitIincInsn(3, 1);
        v.visitJumpInsn(GOTO, loop);
        v.visitLabel(end);
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        if (kind.equals("mixin")) {
            v = w.visitMethod(ACC_PRIVATE | ACC_STATIC, "handler$zze000$signs$text", "()V", null, null);
            v.visitCode();
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        w.visitEnd();
        return w.toByteArray();
    }
}
