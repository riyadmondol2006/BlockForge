package com.blockforge.lite.core;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.util.Enumeration;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

/**
 * The patches against a game jar, without starting the game (started by com.blockforge.lite.Check). Every class of the
 * jars goes through the agent's transformer exactly as at a game start (including the check of the changed code), and
 * the result is printed: one line per patch, "id applied|skipped|failed class reason". For CI: run it on the newest
 * release and snapshot jars to see which patches would stop applying.
 */
public final class Matcher {
    private Matcher() {}

    /** Returns 0 when nothing failed, 1 when a patch failed (wrote code that did not pass the check, or threw). */
    public static int run(String args, String[] jars) throws Exception {
        Engine e = new Engine(args == null || args.isEmpty() ? "quiet" : "quiet," + args, null);
        long t0 = System.nanoTime();
        // stands in for the game's class loader where a patch looks up other classes' files (it loads nothing; plain
        // zip reads, as a URLClassLoader would first check the jar's signature)
        final java.util.List<ZipFile> zips = new java.util.ArrayList<ZipFile>();
        for (String jar : jars) {
            zips.add(new ZipFile(jar));
        }
        ClassLoader files = new ClassLoader(null) {
            @Override
            public InputStream getResourceAsStream(String name) {
                for (ZipFile z : zips) {
                    ZipEntry entry = z.getEntry(name);
                    if (entry != null) {
                        try {
                            return z.getInputStream(entry);
                        } catch (java.io.IOException ex) {
                            return null;
                        }
                    }
                }
                return null;
            }
        };
        try {
            scan(e, files, jars);
        } finally {
            for (ZipFile z : zips) {
                z.close();
            }
        }
        System.out.println("# " + scanned + " classes in " + (System.nanoTime() - t0) / 1000000 + " ms");
        int failed = 0;
        for (Engine.State s : e.states()) {
            System.out.println(s.patch.id + "\t" + s.statusName() + "\t" + s.cls + "\t" + (s.reason == null ? "" : s.reason));
            failed += s.status == Engine.State.FAILED ? 1 : 0;
        }
        return failed == 0 ? 0 : 1;
    }

    private static int scanned;

    private static void scan(Engine e, ClassLoader files, String[] jars) throws Exception {
        int classes = 0;
        for (String jar : jars) {
            ZipFile z = new ZipFile(jar);
            try {
                for (Enumeration<? extends ZipEntry> en = z.entries(); en.hasMoreElements(); ) {
                    ZipEntry entry = en.nextElement();
                    String n = entry.getName();
                    if (!n.endsWith(".class") || n.startsWith("META-INF/") || n.endsWith("module-info.class")) {
                        continue;
                    }
                    String name = n.substring(0, n.length() - 6);
                    if (!(name.startsWith("net/minecraft/") || name.startsWith("com/mojang/blaze3d/")
                            || name.indexOf('/') < 0)) {
                        continue;
                    }
                    InputStream in = z.getInputStream(entry);
                    ByteArrayOutputStream out = new ByteArrayOutputStream();
                    byte[] buf = new byte[16384];
                    for (int r; (r = in.read(buf)) > 0; ) {
                        out.write(buf, 0, r);
                    }
                    in.close();
                    e.transform(files, name, null, null, out.toByteArray());
                    classes++;
                }
            } finally {
                z.close();
            }
        }
        scanned = classes;
    }
}
