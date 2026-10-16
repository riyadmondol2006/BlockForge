package com.blockforge.lite.core;

import java.lang.reflect.Method;
import java.util.HashMap;
import java.util.Map;
import com.blockforge.lite.core.Tests.Game;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;

/** Self-test of quiet-music on a stand-in of SoundEngine.play's volume check (1.21.2 - 26.x and 1.21.1). */
final class QuietMusicTest extends PatchTest {
    private static final String SE = "net/minecraft/client/sounds/SoundEngine";
    private static final String SRC = "net/minecraft/sounds/SoundSource";
    private static final String SI = "net/minecraft/client/resources/sounds/SoundInstance";
    private static final String SND = "net/minecraft/client/resources/sounds/TestSound";

    void quietMusic() throws Exception {
        String report = reportPath("music");
        Game g = load(factory.apply("quiet,level=2,report=" + report), engine("vanilla"));
        expect(g.transformed.contains(SE), "SoundEngine stand-in patched at level 2");
        expect(line(report, "quiet-music").contains("applied"), "reported as applied");
        expect(play(g, "MUSIC", 0f, false) == 0, "muted music: not started");
        expect(play(g, "MUSIC", 1f, false) == 1, "music with volume: started");
        expect(play(g, "OTHER", 0f, false) == 0, "other muted sounds: not started, as before");
        expect(play(g, "MUSIC", 0f, true) == 2, "a sound that may start silently still does");
        g = load(null, engine("vanilla"));
        expect(play(g, "MUSIC", 0f, false) == 2, "without the patch muted music starts silently (the test is real)");
        g = load(factory.apply("quiet"), engine("vanilla"));
        expect(!g.transformed.contains(SE), "level 1: not patched");
        for (String kind : new String[] {"old", "mixin"}) {
            report = reportPath("music-" + kind);
            g = load(factory.apply("quiet,level=2,report=" + report), engine(kind));
            expect(!g.transformed.contains(SE), kind + ": left alone");
            expect(line(report, "quiet-music").contains("skipped"), kind + ": reported as skipped");
        }
    }

    /** 0 = not started, 1 = started, 2 = started silently. */
    private static int play(Game g, String source, float volume, boolean silentOk) throws Exception {
        Class<?> src = g.loader.loadClass(SRC.replace('/', '.'));
        Class<?> snd = g.loader.loadClass(SND.replace('/', '.'));
        Object s = snd.getConstructor(src, boolean.class).newInstance(src.getField(source).get(null), silentOk);
        Class<?> se = g.loader.loadClass(SE.replace('/', '.'));
        Method play = se.getMethod("play", g.loader.loadClass(SI.replace('/', '.')), float.class);
        return (Integer) play.invoke(se.getConstructor().newInstance(), s, volume);
    }

    /** kind: "vanilla" (1.21.2+: MUSIC may start silently), "old" (1.21.1: it may not), "mixin". */
    static Map<String, byte[]> engine(String kind) {
        Map<String, byte[]> m = new HashMap<String, byte[]>();
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, SRC, null, "java/lang/Object", null);
        for (String f : new String[] {"MUSIC", "OTHER"}) {
            w.visitField(ACC_PUBLIC | ACC_STATIC | ACC_FINAL, f, "L" + SRC + ";", null, null).visitEnd();
        }
        constructor(w);
        MethodVisitor v = w.visitMethod(ACC_STATIC, "<clinit>", "()V", null, null);
        v.visitCode();
        for (String f : new String[] {"MUSIC", "OTHER"}) {
            v.visitTypeInsn(NEW, SRC);
            v.visitInsn(DUP);
            v.visitMethodInsn(INVOKESPECIAL, SRC, "<init>", "()V", false);
            v.visitFieldInsn(PUTSTATIC, SRC, f, "L" + SRC + ";");
        }
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        m.put(SRC, w.toByteArray());

        w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_INTERFACE | ACC_ABSTRACT, SI, null, "java/lang/Object", null);
        w.visitMethod(ACC_PUBLIC | ACC_ABSTRACT, "canStartSilent", "()Z", null, null).visitEnd();
        w.visitMethod(ACC_PUBLIC | ACC_ABSTRACT, "getSource", "()L" + SRC + ";", null, null).visitEnd();
        w.visitEnd();
        m.put(SI, w.toByteArray());

        w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, SND, null, "java/lang/Object", new String[] {SI});
        w.visitField(ACC_PRIVATE, "source", "L" + SRC + ";", null, null).visitEnd();
        w.visitField(ACC_PRIVATE, "silent", "Z", null, null).visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "<init>", "(L" + SRC + ";Z)V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 1);
        v.visitFieldInsn(PUTFIELD, SND, "source", "L" + SRC + ";");
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ILOAD, 2);
        v.visitFieldInsn(PUTFIELD, SND, "silent", "Z");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "canStartSilent", "()Z", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, SND, "silent", "Z");
        v.visitInsn(IRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "getSource", "()L" + SRC + ";", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, SND, "source", "L" + SRC + ";");
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        m.put(SND, w.toByteArray());

        w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, SE, null, "java/lang/Object", null);
        constructor(w);
        v = w.visitMethod(ACC_PUBLIC, "play", "(L" + SI + ";F)I", null, null);
        v.visitCode();
        if (kind.equals("mixin")) {
            v.visitMethodInsn(INVOKESTATIC, SE, "handler$zza000$sound$play", "()V", false);
        }
        Label started = new Label();
        Label silent = new Label();
        Label skip = new Label();
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKEINTERFACE, SI, "getSource", "()L" + SRC + ";", true);
        v.visitVarInsn(ASTORE, 3);
        v.visitVarInsn(FLOAD, 2);
        v.visitInsn(FCONST_0);
        v.visitInsn(FCMPL);
        v.visitJumpInsn(IFNE, started);
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKEINTERFACE, SI, "canStartSilent", "()Z", true);
        if (kind.equals("old")) {
            v.visitJumpInsn(IFEQ, skip);
        } else {
            v.visitJumpInsn(IFNE, silent);
            v.visitVarInsn(ALOAD, 3);
            v.visitFieldInsn(GETSTATIC, SRC, "MUSIC", "L" + SRC + ";");
            v.visitJumpInsn(IF_ACMPNE, skip);
        }
        v.visitLabel(silent);
        v.visitInsn(ICONST_2);
        v.visitInsn(IRETURN);
        v.visitLabel(skip);
        v.visitFieldInsn(GETSTATIC, "java/lang/System", "out", "Ljava/io/PrintStream;");
        v.visitLdcInsn(QuietMusic.SKIPPED);
        v.visitInsn(POP2);
        v.visitInsn(ICONST_0);
        v.visitInsn(IRETURN);
        v.visitLabel(started);
        v.visitInsn(ICONST_1);
        v.visitInsn(IRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        if (kind.equals("mixin")) {
            v = w.visitMethod(ACC_PRIVATE | ACC_STATIC, "handler$zza000$sound$play", "()V", null, null);
            v.visitCode();
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        w.visitEnd();
        m.put(SE, w.toByteArray());
        return m;
    }
}
