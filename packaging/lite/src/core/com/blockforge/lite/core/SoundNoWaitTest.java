package com.blockforge.lite.core;

import java.lang.reflect.Method;
import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ThreadFactory;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import com.blockforge.lite.core.Tests.Game;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Handle;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Type;

/**
 * Self-test of sound-no-wait on stand-ins of ChannelAccess, its ChannelHandle, Library and Channel, with a real sound
 * thread (a single-thread executor) that is kept busy on purpose: the handle must come back at once, and the tasks
 * queued after it must see its channel.
 */
final class SoundNoWaitTest extends PatchTest {
    private static final String CH = "com/mojang/blaze3d/audio/Channel";
    private static final String LIB = "com/mojang/blaze3d/audio/Library";
    private static final String POOL = "com/mojang/blaze3d/audio/Library$Pool";
    private static final String CA = "net/minecraft/client/sounds/ChannelAccess";
    private static final String HD = "net/minecraft/client/sounds/ChannelAccess$ChannelHandle";
    private static final String CF = "java/util/concurrent/CompletableFuture";

    void soundNoWait() throws Exception {
        if (!runs(V11)) {
            return;
        }
        String report = reportPath("sound");
        Game g = load(factory.apply("quiet,level=2,report=" + report), classes(V17, "vanilla"));
        expect(g.transformed.contains(CA), "ChannelAccess stand-in patched at level 2");
        expect(line(report, "sound-no-wait").contains("applied"), "reported as applied");
        ExecutorService soundThread = Executors.newSingleThreadExecutor(daemon());
        try {
            Sound s = new Sound(g, soundThread);
            final CountDownLatch gate = new CountDownLatch(1);
            soundThread.execute(new Runnable() {
                public void run() {
                    try {
                        gate.await(); // the sound thread is busy (decoding music, say)
                    } catch (InterruptedException e) {
                        Thread.currentThread().interrupt();
                    }
                }
            });
            CompletableFuture<?> f = s.create();
            expect(f.isDone(), "the handle is there at once, while the sound thread is busy");
            final Object h = f.getNow(null);
            expect(h != null && s.channel(h) == null && !s.stopped(h), "a handle without its channel yet");
            final AtomicReference<Object> seen = new AtomicReference<Object>();
            final Sound fs = s;
            soundThread.execute(new Runnable() {
                public void run() {
                    try {
                        seen.set(fs.channel(h)); // like handle.execute(channel -> ...), queued after the request
                    } catch (Exception e) {
                        seen.set(e);
                    }
                }
            });
            CompletableFuture<?> f2 = s.create(); // the only free channel goes to the first request
            expect(f2.isDone(), "a second handle at once too");
            Object h2 = f2.getNow(null);
            gate.countDown();
            soundThread.submit(new Runnable() {
                public void run() {}
            }).get(10, TimeUnit.SECONDS);
            expect(s.channel(h) != null, "the sound thread gave the handle its channel");
            expect(seen.get() == s.channel(h), "a task queued after the request sees the channel");
            expect(s.count() == 1 && s.lib("acquired") == 1, "the handle is in the channel list, one channel taken");
            expect(f.join() == h, "the future still answers the same handle");
            expect(h2 != null && s.stopped(h2) && s.channel(h2) == null, "no channel free: the handle is stopped");
            expect(s.lib("free") == 0, "and nothing was given back to the pool by mistake");
        } finally {
            soundThread.shutdownNow();
        }

        // vanilla: the future is only done when the sound thread got to it (play() waits for that)
        g = load(null, classes(V17, "vanilla"));
        soundThread = Executors.newSingleThreadExecutor(daemon());
        try {
            Sound s = new Sound(g, soundThread);
            final CountDownLatch gate = new CountDownLatch(1);
            soundThread.execute(new Runnable() {
                public void run() {
                    try {
                        gate.await();
                    } catch (InterruptedException e) {
                        Thread.currentThread().interrupt();
                    }
                }
            });
            CompletableFuture<?> f = s.create();
            CompletableFuture<?> f2 = s.create();
            expect(!f.isDone(), "without the patch the handle waits for the sound thread (the test is real)");
            gate.countDown();
            Object h = f.get(10, TimeUnit.SECONDS);
            expect(h != null && s.channel(h) != null && f2.get(10, TimeUnit.SECONDS) == null,
                    "without the patch: a handle with a channel, then none");
        } finally {
            soundThread.shutdownNow();
        }

        g = load(factory.apply("quiet"), classes(V17, "vanilla"));
        expect(!g.transformed.contains(CA), "level 1: not patched");
        for (String kind : new String[] {"mixin", "extra", "old"}) {
            report = reportPath("sound-" + kind);
            g = load(factory.apply("quiet,level=2,report=" + report), classes(kind.equals("old") ? V1_8 : V17, kind));
            expect(!g.transformed.contains(CA), kind + ": left alone");
            expect(line(report, "sound-no-wait").contains("skipped"), kind + ": reported as skipped");
        }
        java.io.File list = new java.io.File(tmp, "mods-sound.txt");
        write(list, "sound-physics-remastered-fabric-1.21.1-1.4.5.jar\n");
        g = load(factory.apply("quiet,level=2,mods=" + list), classes(V17, "vanilla"));
        expect(!g.transformed.contains(CA), "Sound Physics Remastered installed: left alone");
    }

    private static ThreadFactory daemon() {
        return new ThreadFactory() {
            public Thread newThread(Runnable r) {
                Thread t = new Thread(r, "Sound engine (test)");
                t.setDaemon(true);
                return t;
            }
        };
    }

    /** Drives the loaded stand-ins: one ChannelAccess with a Library that has one free channel. */
    private static final class Sound {
        final Game g;
        final Object lib;
        final Object access;
        final Object pool;

        Sound(Game g, java.util.concurrent.Executor e) throws Exception {
            this.g = g;
            Class<?> l = g.loader.loadClass(LIB.replace('/', '.'));
            lib = l.getConstructor().newInstance();
            l.getField("free").setInt(lib, 1);
            access = g.loader.loadClass(CA.replace('/', '.')).getConstructor(l, java.util.concurrent.Executor.class)
                    .newInstance(lib, e);
            pool = g.loader.loadClass(POOL.replace('/', '.')).getConstructor().newInstance();
        }

        CompletableFuture<?> create() throws Exception {
            Method m = access.getClass().getMethod("createHandle", pool.getClass());
            return (CompletableFuture<?>) m.invoke(access, pool);
        }

        Object channel(Object handle) throws Exception {
            return handle.getClass().getMethod("get").invoke(handle);
        }

        boolean stopped(Object handle) throws Exception {
            return (Boolean) handle.getClass().getMethod("isStopped").invoke(handle);
        }

        int count() throws Exception {
            return (Integer) access.getClass().getMethod("count").invoke(access);
        }

        int lib(String field) throws Exception {
            return lib.getClass().getField(field).getInt(lib);
        }
    }

    /**
     * Channel, Library (acquireChannel / releaseChannel), Library$Pool, ChannelAccess and its ChannelHandle as in
     * 1.20 - 26.x. kind: "vanilla"; "mixin" (createHandle calls a mixin handler); "extra" (the channel task does one
     * more thing); "old" (Java 8 class files).
     */
    static Map<String, byte[]> classes(int version, String kind) {
        Map<String, byte[]> m = new HashMap<String, byte[]>();
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, CH, null, "java/lang/Object", null);
        constructor(w);
        w.visitEnd();
        m.put(CH, w.toByteArray());
        w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, POOL, null, "java/lang/Object", null);
        constructor(w);
        w.visitEnd();
        m.put(POOL, w.toByteArray());
        m.put(LIB, library());
        m.put(HD, handle(version));
        m.put(CA, access(version, kind));
        return m;
    }

    private static byte[] library() {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, LIB, null, "java/lang/Object", null);
        w.visitField(ACC_PUBLIC, "free", "I", null, null).visitEnd();
        w.visitField(ACC_PUBLIC, "acquired", "I", null, null).visitEnd();
        constructor(w);
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "acquireChannel", "(L" + POOL + ";)L" + CH + ";", null, null);
        v.visitCode();
        Label some = new Label();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, LIB, "free", "I");
        v.visitJumpInsn(IFGT, some);
        v.visitInsn(ACONST_NULL);
        v.visitInsn(ARETURN);
        v.visitLabel(some);
        for (String f : new String[] {"free", "acquired"}) {
            v.visitVarInsn(ALOAD, 0);
            v.visitInsn(DUP);
            v.visitFieldInsn(GETFIELD, LIB, f, "I");
            v.visitInsn(f.equals("free") ? ICONST_M1 : ICONST_1);
            v.visitInsn(IADD);
            v.visitFieldInsn(PUTFIELD, LIB, f, "I");
        }
        v.visitTypeInsn(NEW, CH);
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, CH, "<init>", "()V", false);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        // releaseChannel(c): an unknown channel (null) is an error, as in the game
        v = w.visitMethod(ACC_PUBLIC, "releaseChannel", "(L" + CH + ";)V", null, null);
        v.visitCode();
        Label known = new Label();
        v.visitVarInsn(ALOAD, 1);
        v.visitJumpInsn(IFNONNULL, known);
        v.visitTypeInsn(NEW, "java/lang/IllegalStateException");
        v.visitInsn(DUP);
        v.visitLdcInsn("Tried to release unknown channel");
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/IllegalStateException", "<init>", "(Ljava/lang/String;)V", false);
        v.visitInsn(ATHROW);
        v.visitLabel(known);
        v.visitVarInsn(ALOAD, 0);
        v.visitInsn(DUP);
        v.visitFieldInsn(GETFIELD, LIB, "free", "I");
        v.visitInsn(ICONST_1);
        v.visitInsn(IADD);
        v.visitFieldInsn(PUTFIELD, LIB, "free", "I");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        return w.toByteArray();
    }

    /** The handle: private channel and stopped flag (set by ChannelAccess through the nest), release(). */
    private static byte[] handle(int version) {
        ClassWriter w = writer();
        w.visit(version, ACC_PUBLIC | ACC_SUPER, HD, null, "java/lang/Object", null);
        w.visitNestHost(CA);
        w.visitInnerClass(HD, CA, "ChannelHandle", ACC_PUBLIC);
        w.visitField(ACC_PRIVATE, "channel", "L" + CH + ";", null, null).visitEnd();
        w.visitField(ACC_PRIVATE, "stopped", "Z", null, null).visitEnd();
        w.visitField(ACC_FINAL | ACC_SYNTHETIC, "this$0", "L" + CA + ";", null, null).visitEnd();
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "<init>", "(L" + CA + ";L" + CH + ";)V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 1);
        v.visitFieldInsn(PUTFIELD, HD, "this$0", "L" + CA + ";");
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 2);
        v.visitFieldInsn(PUTFIELD, HD, "channel", "L" + CH + ";");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "isStopped", "()Z", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, HD, "stopped", "Z");
        v.visitInsn(IRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "get", "()L" + CH + ";", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, HD, "channel", "L" + CH + ";");
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        v = w.visitMethod(ACC_PUBLIC, "release", "()V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitInsn(ICONST_1);
        v.visitFieldInsn(PUTFIELD, HD, "stopped", "Z");
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, HD, "this$0", "L" + CA + ";");
        v.visitFieldInsn(GETFIELD, CA, "library", "L" + LIB + ";");
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, HD, "channel", "L" + CH + ";");
        v.visitMethodInsn(INVOKEVIRTUAL, LIB, "releaseChannel", "(L" + CH + ";)V", false);
        v.visitVarInsn(ALOAD, 0);
        v.visitInsn(ACONST_NULL);
        v.visitFieldInsn(PUTFIELD, HD, "channel", "L" + CH + ";");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        return w.toByteArray();
    }

    private static byte[] access(int version, String kind) {
        ClassWriter w = writer();
        w.visit(version, ACC_PUBLIC | ACC_SUPER, CA, null, "java/lang/Object", null);
        w.visitNestMember(HD);
        w.visitInnerClass(HD, CA, "ChannelHandle", ACC_PUBLIC);
        w.visitField(ACC_PRIVATE | ACC_FINAL, "channels", "Ljava/util/Set;", null, null).visitEnd();
        w.visitField(ACC_FINAL, "library", "L" + LIB + ";", null, null).visitEnd();
        w.visitField(ACC_FINAL, "executor", "Ljava/util/concurrent/Executor;", null, null).visitEnd();
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "<init>", "(L" + LIB + ";Ljava/util/concurrent/Executor;)V", null,
                null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        v.visitVarInsn(ALOAD, 0);
        v.visitTypeInsn(NEW, "java/util/HashSet");
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, "java/util/HashSet", "<init>", "()V", false);
        v.visitFieldInsn(PUTFIELD, CA, "channels", "Ljava/util/Set;");
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 1);
        v.visitFieldInsn(PUTFIELD, CA, "library", "L" + LIB + ";");
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 2);
        v.visitFieldInsn(PUTFIELD, CA, "executor", "Ljava/util/concurrent/Executor;");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        String lambdaDesc = "(L" + POOL + ";L" + CF + ";)V";
        v = w.visitMethod(ACC_PUBLIC, "createHandle", "(L" + POOL + ";)L" + CF + ";", null, null);
        v.visitCode();
        if (kind.equals("mixin")) {
            v.visitMethodInsn(INVOKESTATIC, CA, "handler$zza000$sounds$create", "()V", false);
        }
        v.visitTypeInsn(NEW, CF);
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, CF, "<init>", "()V", false);
        v.visitVarInsn(ASTORE, 2);
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, CA, "executor", "Ljava/util/concurrent/Executor;");
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 1);
        v.visitVarInsn(ALOAD, 2);
        Handle meta = new Handle(H_INVOKESTATIC, "java/lang/invoke/LambdaMetafactory", "metafactory",
                "(Ljava/lang/invoke/MethodHandles$Lookup;Ljava/lang/String;Ljava/lang/invoke/MethodType;"
                        + "Ljava/lang/invoke/MethodType;Ljava/lang/invoke/MethodHandle;Ljava/lang/invoke/MethodType;)"
                        + "Ljava/lang/invoke/CallSite;", false);
        v.visitInvokeDynamicInsn("run", "(L" + CA + ";L" + POOL + ";L" + CF + ";)Ljava/lang/Runnable;", meta,
                Type.getType("()V"), new Handle(H_INVOKESPECIAL, CA, "lambda$createHandle$0", lambdaDesc, false),
                Type.getType("()V"));
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/concurrent/Executor", "execute", "(Ljava/lang/Runnable;)V", true);
        v.visitVarInsn(ALOAD, 2);
        v.visitInsn(ARETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        if (kind.equals("mixin")) {
            v = w.visitMethod(ACC_PRIVATE | ACC_STATIC, "handler$zza000$sounds$create", "()V", null, null);
            v.visitCode();
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }

        // the channel task, exactly as the game has it
        v = w.visitMethod(ACC_PRIVATE | ACC_SYNTHETIC, "lambda$createHandle$0", lambdaDesc, null, null);
        v.visitCode();
        Label none = new Label();
        Label end = new Label();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, CA, "library", "L" + LIB + ";");
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKEVIRTUAL, LIB, "acquireChannel", "(L" + POOL + ";)L" + CH + ";", false);
        v.visitVarInsn(ASTORE, 3);
        v.visitVarInsn(ALOAD, 3);
        v.visitJumpInsn(IFNULL, none);
        v.visitTypeInsn(NEW, HD);
        v.visitInsn(DUP);
        v.visitVarInsn(ALOAD, 0);
        v.visitVarInsn(ALOAD, 3);
        v.visitMethodInsn(INVOKESPECIAL, HD, "<init>", "(L" + CA + ";L" + CH + ";)V", false);
        v.visitVarInsn(ASTORE, 4);
        if (kind.equals("extra")) {
            v.visitVarInsn(ALOAD, 4);
            v.visitInsn(POP);
        }
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, CA, "channels", "Ljava/util/Set;");
        v.visitVarInsn(ALOAD, 4);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Set", "add", "(Ljava/lang/Object;)Z", true);
        v.visitInsn(POP);
        v.visitVarInsn(ALOAD, 2);
        v.visitVarInsn(ALOAD, 4);
        v.visitMethodInsn(INVOKEVIRTUAL, CF, "complete", "(Ljava/lang/Object;)Z", false);
        v.visitInsn(POP);
        v.visitJumpInsn(GOTO, end);
        v.visitLabel(none);
        v.visitVarInsn(ALOAD, 2);
        v.visitInsn(ACONST_NULL);
        v.visitMethodInsn(INVOKEVIRTUAL, CF, "complete", "(Ljava/lang/Object;)Z", false);
        v.visitInsn(POP);
        v.visitLabel(end);
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        // like scheduleTick's task: a finished channel is released and dropped
        v = w.visitMethod(ACC_PUBLIC, "sweep", "()V", null, null);
        v.visitCode();
        Label loop = new Label();
        Label done = new Label();
        Label next = new Label();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, CA, "channels", "Ljava/util/Set;");
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Set", "iterator", "()Ljava/util/Iterator;", true);
        v.visitVarInsn(ASTORE, 1);
        v.visitLabel(loop);
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Iterator", "hasNext", "()Z", true);
        v.visitJumpInsn(IFEQ, done);
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Iterator", "next", "()Ljava/lang/Object;", true);
        v.visitTypeInsn(CHECKCAST, HD);
        v.visitVarInsn(ASTORE, 2);
        v.visitVarInsn(ALOAD, 2);
        v.visitFieldInsn(GETFIELD, HD, "channel", "L" + CH + ";");
        v.visitJumpInsn(IFNULL, next);
        v.visitVarInsn(ALOAD, 2);
        v.visitMethodInsn(INVOKEVIRTUAL, HD, "release", "()V", false);
        v.visitVarInsn(ALOAD, 1);
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Iterator", "remove", "()V", true);
        v.visitLabel(next);
        v.visitJumpInsn(GOTO, loop);
        v.visitLabel(done);
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        v = w.visitMethod(ACC_PUBLIC, "count", "()I", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, CA, "channels", "Ljava/util/Set;");
        v.visitMethodInsn(INVOKEINTERFACE, "java/util/Set", "size", "()I", true);
        v.visitInsn(IRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        return w.toByteArray();
    }
}
