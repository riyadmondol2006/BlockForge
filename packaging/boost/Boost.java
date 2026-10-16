package com.blockforge.boost;

import java.io.File;
import java.io.IOException;
import java.lang.instrument.Instrumentation;
import java.nio.ByteBuffer;
import java.nio.channels.FileChannel;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * BlockForge's fast-start helper. It is a Java agent (-javaagent), so it works the same for vanilla, Fabric, Quilt,
 * Forge and NeoForge, on every Minecraft version, and needs no mod file.
 *
 * What it does: the game spends its first seconds loading the library classes (Guava, Netty, Log4j, Fabric Loader,
 * Mixin, ASM ...) one after the other on its main thread. A previous run wrote down which classes the application
 * class loader loaded; this run loads the same ones on spare processor cores, a little ahead of the main thread.
 * Loading a class (without starting it) changes nothing the game can see. It is only done when
 *   - the computer has at least 4 processors (on fewer, helper threads would take time from the game), and
 *   - the jar files can be read without a hard drive's slow seeks (a hard drive is slowed down by more readers;
 *     there the helper does nothing).
 * Every step is wrapped, so a failure is printed as a line starting with "[BlockForge Boost]" and the game carries
 * on as if the helper was not there.
 *
 * Options (comma separated, key=value or flag):
 *   preload=FILE     the list of class names (one per line); the same file is written again after record= seconds
 *                    of the run, for the next start
 *   record=SECONDS   when to write the list (default 30)
 *   threads=N        helper threads (default 2)
 *   quiet            no summary lines
 */
public final class Boost {
    private static final String TAG = "[BlockForge Boost] ";

    private Boost() {}

    public static void premain(String args, Instrumentation inst) {
        try {
            final Map<String, String> o = parse(args);
            final boolean quiet = o.containsKey("quiet");
            if (!quiet) {
                System.out.println(TAG + "on (" + (args == null ? "" : args) + ")");
            }
            if (Runtime.getRuntime().availableProcessors() < 4 || o.get("preload") == null) {
                // on a small computer the helper threads would only take time from the game
                if (!quiet) {
                    System.out.println(TAG + "nothing to do here");
                }
                return;
            }
            final Instrumentation instrumentation = inst;
            Thread t = new Thread(new Runnable() {
                public void run() {
                    work(o, instrumentation, quiet);
                }
            }, "BlockForge-Boost");
            t.setDaemon(true);
            t.start();
        } catch (Throwable e) {
            problem("could not start", e);
        }
    }

    public static void agentmain(String args, Instrumentation inst) {
        premain(args, inst);
    }

    private static void work(Map<String, String> o, Instrumentation inst, boolean quiet) {
        File list = new File(o.get("preload"));
        int threads = 2;
        int record = 30;
        try {
            if (o.containsKey("threads")) {
                threads = Math.max(1, Math.min(6, Integer.parseInt(o.get("threads"))));
            }
            if (o.containsKey("record")) {
                record = Integer.parseInt(o.get("record"));
            }
        } catch (Throwable ignored) {
            // keep the defaults
        }
        try {
            if (list.isFile()) {
                if (seeksAreSlow()) {
                    if (!quiet) {
                        System.out.println(TAG + "the files are on a slow disk: not loading ahead");
                    }
                } else {
                    preload(list, threads, quiet);
                }
            }
        } catch (Throwable e) {
            problem("loading classes ahead", e);
        }
        try {
            Thread.sleep(record * 1000L);
            record(list, inst, quiet);
        } catch (InterruptedException ignored) {
            // the game ended first
        } catch (Throwable e) {
            problem("writing the class list", e);
        }
    }

    /**
     * Reads a little from the middle of three jar files of the class path and times it. A read that has to wait for a
     * hard drive's head takes ten milliseconds or more; from the file cache or an SSD it takes a fraction of one.
     */
    private static boolean seeksAreSlow() {
        int probed = 0;
        ByteBuffer buf = ByteBuffer.allocate(64 * 1024);
        for (String p : System.getProperty("java.class.path", "").split(File.pathSeparator)) {
            if (!p.endsWith(".jar") || probed >= 3) {
                continue;
            }
            FileChannel ch = null;
            try {
                ch = FileChannel.open(new File(p).toPath(), StandardOpenOption.READ);
                long size = ch.size();
                if (size < 256 * 1024) {
                    continue;
                }
                buf.clear();
                long t = System.nanoTime();
                ch.read(buf, size / 2);
                if (System.nanoTime() - t > 3_000_000L) {
                    return true;
                }
                probed++;
            } catch (IOException ignored) {
                // an unreadable file says nothing about the disk
            } finally {
                if (ch != null) {
                    try {
                        ch.close();
                    } catch (IOException ignored) {
                        // closed anyway
                    }
                }
            }
        }
        return false;
    }

    private static void preload(File list, int threads, boolean quiet) throws Exception {
        long t0 = System.nanoTime();
        final List<String> names = Files.readAllLines(list.toPath(), StandardCharsets.UTF_8);
        final ClassLoader loader = ClassLoader.getSystemClassLoader();
        final AtomicInteger next = new AtomicInteger();
        final AtomicInteger done = new AtomicInteger();
        Thread[] pool = new Thread[threads];
        for (int i = 0; i < pool.length; i++) {
            pool[i] = new Thread(new Runnable() {
                public void run() {
                    int k;
                    while ((k = next.getAndIncrement()) < names.size()) {
                        String n = names.get(k).trim();
                        if (n.isEmpty()) {
                            continue;
                        }
                        try {
                            Class.forName(n, false, loader);
                            done.incrementAndGet();
                        } catch (Throwable ignored) {
                            // missing in this setup: the game does not need it either
                        }
                    }
                }
            }, "BlockForge-Boost-load");
            pool[i].setDaemon(true);
            pool[i].start();
        }
        for (Thread t : pool) {
            t.join();
        }
        if (!quiet) {
            System.out.println(TAG + "loaded " + done.get() + " of " + names.size() + " classes ahead in "
                    + (System.nanoTime() - t0) / 1_000_000 + " ms");
        }
    }

    /** Writes the names of the classes the application class loader has loaded so far, for the next start. */
    private static void record(File list, Instrumentation inst, boolean quiet) throws IOException {
        ClassLoader loader = ClassLoader.getSystemClassLoader();
        List<String> names = new ArrayList<String>();
        for (Class<?> c : inst.getInitiatedClasses(loader)) {
            String n = c.getName();
            if (c.getClassLoader() == loader && n.indexOf('/') < 0 && !n.startsWith("com.blockforge.boost.")
                    && !c.isArray()) {
                names.add(n);
            }
        }
        Collections.sort(names);
        File parent = list.getAbsoluteFile().getParentFile();
        if (parent != null) {
            parent.mkdirs();
        }
        Path tmp = new File(list.getPath() + ".tmp").toPath();
        Files.write(tmp, names, StandardCharsets.UTF_8);
        Files.move(tmp, list.toPath(), StandardCopyOption.REPLACE_EXISTING);
        if (!quiet) {
            System.out.println(TAG + "wrote " + names.size() + " class names for the next start");
        }
    }

    private static Map<String, String> parse(String args) {
        Map<String, String> m = new HashMap<String, String>();
        if (args == null) {
            return m;
        }
        for (String part : args.split(",")) {
            String p = part.trim();
            if (p.isEmpty()) {
                continue;
            }
            int eq = p.indexOf('=');
            if (eq < 0) {
                m.put(p, "");
            } else {
                m.put(p.substring(0, eq), p.substring(eq + 1));
            }
        }
        return m;
    }

    private static void problem(String what, Throwable e) {
        try {
            System.out.println(TAG + "problem: " + what + ": " + e);
        } catch (Throwable ignored) {
            // nothing more to do
        }
    }
}
