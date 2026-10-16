package com.blockforge.lite;

import java.io.File;
import java.lang.instrument.Instrumentation;

/**
 * BlockForge Lite: a Java agent (-javaagent) that makes Minecraft run smoother on slow computers. It is not a mod, so
 * it works the same for vanilla, Fabric, Quilt, Forge and NeoForge.
 *
 * This class and {@link Entry} are the only ones the JVM loads from the agent jar on the game's class path. The patch
 * code and the ASM library it uses are kept inside the jar as a nested file and loaded by a private class loader
 * ({@link CoreLoader}), so the game and its mod loader never see a second copy of ASM.
 *
 * Off switches: {@code -Dblockforge.lite=off}, or {@code -Dblockforge.lite.off=FILE} when that file exists.
 */
public final class Agent {
    static final String TAG = "[BlockForge Lite] ";

    private Agent() {}

    public static void premain(String args, Instrumentation inst) {
        start(args, inst);
    }

    public static void agentmain(String args, Instrumentation inst) {
        // started late, after the game's classes were loaded: the patches only see classes loaded from now on
        start(args, inst);
    }

    private static void start(String args, Instrumentation inst) {
        try {
            if (switchedOff()) {
                if (!hasFlag(args, "quiet")) {
                    System.out.println(TAG + "off");
                }
                return;
            }
            Entry entry = new Entry(args);
            inst.addTransformer(entry);
            entry.loadCoreSoon();
        } catch (Throwable e) {
            problem("could not start", e);
        }
    }

    /** Whether the player or the launcher switched the agent off. */
    static boolean switchedOff() {
        try {
            if ("off".equalsIgnoreCase(System.getProperty("blockforge.lite", "").trim())) {
                return true;
            }
            String file = System.getProperty("blockforge.lite.off");
            return file != null && !file.isEmpty() && new File(file).exists();
        } catch (Throwable e) {
            // a security manager that hides properties or files: stay on
            return false;
        }
    }

    static boolean hasFlag(String args, String flag) {
        if (args == null) {
            return false;
        }
        for (String part : args.split(",")) {
            if (part.trim().equals(flag)) {
                return true;
            }
        }
        return false;
    }

    static void problem(String what, Throwable e) {
        try {
            System.out.println(TAG + "problem: " + what + ": " + e);
        } catch (Throwable ignored) {
            // nothing more to do
        }
    }
}
