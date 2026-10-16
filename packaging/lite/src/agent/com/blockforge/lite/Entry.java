package com.blockforge.lite;

import java.lang.instrument.ClassFileTransformer;
import java.security.ProtectionDomain;
import java.util.concurrent.atomic.AtomicLongArray;

/**
 * The transformer the JVM calls for every class it loads. It looks only at the class name and hands Minecraft's own
 * classes to the patch code in the private class loader; everything else (the JDK, libraries, mod loaders, mods) is
 * passed by at once.
 */
public final class Entry implements ClassFileTransformer {
    /** [0] nanoseconds spent in here, [1] classes seen, [2] classes handed on. Read by the patch code for the report. */
    final AtomicLongArray stats = new AtomicLongArray(3);
    private final String args;
    private volatile ClassFileTransformer core;
    private volatile boolean broken;

    public Entry(String args) {
        this.args = args == null ? "" : args;
    }

    /** Loads the patch code on a helper thread, so the first Minecraft class does not wait for it. */
    void loadCoreSoon() {
        Thread t = new Thread(new Runnable() {
            public void run() {
                core();
            }
        }, "BlockForge-Lite-init");
        t.setDaemon(true);
        t.start();
    }

    @Override
    public byte[] transform(ClassLoader loader, String name, Class<?> redefined, ProtectionDomain domain,
            byte[] bytes) {
        long t0 = System.nanoTime();
        try {
            stats.incrementAndGet(1);
            // classes are only changed the first time they load, never when someone redefines them later
            if (name == null || bytes == null || redefined != null || broken || !mayBeGame(name)) {
                return null;
            }
            ClassFileTransformer c = core();
            if (c == null) {
                return null;
            }
            stats.incrementAndGet(2);
            return c.transform(loader, name, redefined, domain, bytes);
        } catch (Throwable e) {
            // the patch code catches its own problems; this is the last line of defence
            return null;
        } finally {
            stats.addAndGet(0, System.nanoTime() - t0);
        }
    }

    /**
     * Minecraft's classes: under net/minecraft/ or com/mojang/blaze3d/ (official, Fabric/Quilt intermediary and Forge
     * names), or in no package at all (the obfuscated names of a vanilla game).
     */
    static boolean mayBeGame(String name) {
        return name.startsWith("net/minecraft/") || name.startsWith("com/mojang/blaze3d/") || name.indexOf('/') < 0;
    }

    ClassFileTransformer core() {
        ClassFileTransformer c = core;
        if (c != null || broken) {
            return c;
        }
        synchronized (this) {
            if (core == null && !broken) {
                try {
                    core = CoreLoader.newEngine(args, stats);
                } catch (Throwable e) {
                    broken = true;
                    Agent.problem("could not load the patch code", e);
                }
            }
            return core;
        }
    }
}
