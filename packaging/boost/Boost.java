package com.blockforge.boost;

import java.io.File;
import java.io.IOException;
import java.io.OutputStream;
import java.lang.instrument.Instrumentation;
import java.nio.ByteBuffer;
import java.nio.channels.FileChannel;
import java.nio.charset.StandardCharsets;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * BlockForge's fast-start helper. It is a Java agent (-javaagent), so it works the same for vanilla, Fabric, Quilt,
 * Forge and NeoForge, on every Minecraft version, and needs no mod file.
 *
 * What it does: the game spends its first seconds loading classes one after the other on its main thread (for vanilla
 * that is Minecraft itself, for every game the libraries: Guava, Netty, Log4j, LWJGL, the loader ...). A start without
 * a list writes down which classes the application class loader loaded, in the order they came. Later starts load
 * and check (link) the same classes on spare processor cores, in that order, a little ahead of the main thread.
 * Loading and linking a class does not start it (no static code runs), so the game sees no difference; only the
 * application class loader is used, the one that loaded those classes before, never a mod loader's own loader.
 * With a mod loader that is the libraries only (Minecraft's classes are left to the mod loader's class loader); it
 * helps most without one.
 *
 * It stays out of the way:
 *   - nothing is done on a computer with one processor, and nothing is loaded ahead when the jar files are on a disk
 *     with slow seeks (a hard drive whose files aren't in the system's file cache);
 *   - the list is only used for the same Java, the same main class, the same class path (every file's size and
 *     date), the same helper and the same key from the launcher; otherwise this start writes a new one. A list is
 *     only written by a start that didn't load ahead, so it holds what the game itself loaded; one whose classes
 *     turn out to be another class loader's is removed;
 *   - the threads are daemon threads with the lowest priority and stop by themselves after a time limit;
 *   - every step is wrapped: a failure is one line starting with "[BlockForge Boost]" and the game carries on.
 *
 * Options (comma separated, key=value or flag):
 *   preload=FILE     the class list; written during a start that has no usable one
 *   record=SECONDS   write the list at the latest this long into the start (default 60); sooner once class loading
 *                    has calmed down (the title screen), but not before settle=SECONDS (default 15)
 *   threads=N        helper threads (default: none on 1 processor, 1 on 2-5, 2 on more; 0 switches it off)
 *   budget=SECONDS   the helper threads stop after this long (default 40)
 *   link=0           only load the classes, don't link them
 *   key=TEXT         the launcher's description of the setup (a list made with another key is not used)
 *   disk=ssd         the launcher knows the game's files are not on a hard drive: no need to time reads
 *   quiet            no summary lines
 */
public final class Boost implements Runnable {
    static final String TAG = "[BlockForge Boost] ";
    /** Part of every list's fingerprint: a list written by another version of the helper is not used. */
    static final String VERSION = "2";
    static final String HEADER = "# BlockForge Boost class list " + VERSION;
    static final long MAX_LIST_BYTES = 8L * 1024 * 1024;
    static final int MAX_NAMES = 100000;
    static final int MAX_NAME_LENGTH = 400;
    /** A list shorter than this comes from a start that didn't get far: write a new one. */
    static final int MIN_NAMES = 100;

    private final Map<String, String> options;
    private final Instrumentation inst;
    private final ClassLoader loader;
    private final boolean quiet;
    private final long started = System.nanoTime();

    Boost(Map<String, String> options, Instrumentation inst, ClassLoader loader) {
        this.options = options;
        this.inst = inst;
        this.loader = loader;
        this.quiet = options.containsKey("quiet");
    }

    public static void premain(String args, Instrumentation inst) {
        try {
            Map<String, String> o = parse(args);
            if (o.get("preload") == null || o.get("preload").isEmpty()) {
                if (!o.containsKey("quiet")) {
                    say("on, no class list given: nothing to do");
                }
                return;
            }
            Thread t = new Thread(new Boost(o, inst, ClassLoader.getSystemClassLoader()), "BlockForge-Boost");
            t.setDaemon(true);
            t.setPriority(Thread.MIN_PRIORITY);
            t.start();
        } catch (Throwable e) {
            problem("could not start", e);
        }
    }

    public static void agentmain(String args, Instrumentation inst) {
        premain(args, inst);
    }

    public void run() {
        try {
            work();
        } catch (OutOfMemoryError e) {
            // nothing to print it with
        } catch (Throwable e) {
            problem("unexpected", e);
        }
    }

    private void work() {
        Path list;
        try {
            list = Paths.get(options.get("preload"));
        } catch (Throwable e) {
            note("the class list's path can't be used here: " + e.getMessage());
            return;
        }
        int threads = threads(Runtime.getRuntime().availableProcessors());
        if (threads == 0) {
            // not even a list is written: watching the class loader would also cost a little
            note("off: " + Runtime.getRuntime().availableProcessors() + " processors are too few to load ahead");
            return;
        }
        String fingerprint = fingerprint(options.get("key"));
        String[] why = new String[1];
        ListFile old = read(list, fingerprint, why);
        if (old == null) {
            note(why[0] + ": this start writes a new class list");
            record(list, fingerprint);
            return;
        }
        // the launcher says "disk=ssd" when it knows; on a hard drive it reads the game's files once before the start
        if (!"ssd".equals(options.get("disk")) && seeksAreSlow()) {
            note("not loading ahead: the game's files are on a disk with slow seeks");
            return;
        }
        boolean link = !"0".equals(options.get("link"));
        long budget = seconds("budget", 40) * 1_000_000_000L;
        String main = mainClass();
        List<String> names = new ArrayList<String>(old.names.size());
        for (String n : old.names) {
            if (allowed(n, main)) {
                names.add(n);
            }
        }
        int[] counts = preload(names, loader, threads, link, started + budget);
        note("loaded " + counts[0] + " of " + names.size() + " classes ahead in "
                + (System.nanoTime() - started) / 1_000_000 + " ms (" + threads
                + (threads == 1 ? " thread" : " threads") + (counts[1] > 0 ? ", stopped at the time limit" : "") + ")");
        checkDrift(list, old);
    }

    /** Helper threads for this many processors. */
    int threads(int processors) {
        if (options.containsKey("threads")) {
            try {
                return Math.max(0, Math.min(6, Integer.parseInt(options.get("threads").trim())));
            } catch (NumberFormatException ignored) {
                // the default then
            }
        }
        // measured: one thread already takes a quarter of the class loading off the main thread on 2 and 4 processors
        if (processors < 2) {
            return 0;
        }
        return processors < 6 ? 1 : 2;
    }

    private long seconds(String key, long fallback) {
        try {
            if (options.containsKey(key)) {
                return Math.max(0, Math.min(3600, Long.parseLong(options.get(key).trim())));
            }
        } catch (NumberFormatException ignored) {
            // the default then
        }
        return fallback;
    }

    // ---- loading ahead

    /**
     * Loads (and links) the named classes with the given loader on helper threads, in list order. Returns {loaded,
     * stopped at the time limit (1/0)}. Never initialises a class.
     */
    static int[] preload(final List<String> names, final ClassLoader loader, int threads, final boolean link,
            final long deadline) {
        final AtomicInteger next = new AtomicInteger();
        final AtomicInteger loaded = new AtomicInteger();
        final AtomicInteger late = new AtomicInteger();
        Thread[] pool = new Thread[Math.max(1, threads)];
        for (int i = 0; i < pool.length; i++) {
            pool[i] = new Thread(new Runnable() {
                public void run() {
                    int k;
                    while ((k = next.getAndIncrement()) < names.size()) {
                        if (System.nanoTime() - deadline > 0) {
                            late.set(1);
                            next.set(names.size());
                            return;
                        }
                        try {
                            Class<?> c = Class.forName(names.get(k), false, loader);
                            loaded.incrementAndGet();
                            if (link) {
                                // asking for the fields links (verifies) the class; it still isn't initialised
                                c.getDeclaredFields();
                            }
                        } catch (OutOfMemoryError e) {
                            next.set(names.size());
                            return;
                        } catch (Throwable ignored) {
                            // missing or broken in this setup: the game won't need it either
                        }
                    }
                }
            }, "BlockForge-Boost-load-" + i);
            pool[i].setDaemon(true);
            pool[i].setPriority(Thread.MIN_PRIORITY);
            pool[i].start();
        }
        for (Thread t : pool) {
            try {
                t.join();
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                break;
            }
        }
        return new int[] {loaded.get(), late.get()};
    }

    /**
     * Whether the jar files of the class path are on a disk with slow seeks (a hard drive), where more readers would
     * slow the game down. Two rounds a moment apart, each reading other parts of the files: a burst of other programs'
     * disk traffic can make one round look slow on an SSD, hardly two.
     */
    static boolean seeksAreSlow() {
        if (!seeksLookSlow(2)) {
            return false;
        }
        try {
            Thread.sleep(300);
        } catch (InterruptedException e) {
            return false;
        }
        return seeksLookSlow(4);
    }

    /**
     * Reads 64 KB at size/part of up to three jar files of the class path and times it. A read that has to wait for a
     * hard drive's head takes ten milliseconds or more; from the file cache or an SSD it takes a fraction of one.
     * The same block is then read again: that read comes from the file cache, so when it is slow too, this thread was
     * only kept waiting by a busy processor, and the first read says nothing about the disk. Slow when at least one
     * read waited for the disk and none was quick.
     */
    private static boolean seeksLookSlow(int part) {
        int probed = 0;
        int slow = 0;
        int quick = 0;
        ByteBuffer buf = ByteBuffer.allocate(64 * 1024);
        for (String p : System.getProperty("java.class.path", "").split(File.pathSeparator)) {
            if (!p.endsWith(".jar") || probed >= 3) {
                continue;
            }
            FileChannel ch = null;
            try {
                ch = FileChannel.open(Paths.get(p), StandardOpenOption.READ);
                long size = ch.size();
                if (size < 256 * 1024) {
                    continue;
                }
                probed++;
                buf.clear();
                long t0 = System.nanoTime();
                ch.read(buf, size / part);
                long t1 = System.nanoTime();
                buf.clear();
                ch.read(buf, size / part);
                long t2 = System.nanoTime();
                if (t1 - t0 < 1_000_000L) {
                    quick++;
                } else if (t1 - t0 > 3_000_000L && t2 - t1 < 500_000L) {
                    slow++;
                }
            } catch (Throwable ignored) {
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
        return slow > 0 && quick == 0;
    }

    // ---- writing the list

    /**
     * Watches which classes the application class loader loads, in the order they come, and writes them once class
     * loading has calmed down (or at the record= limit). Nothing is written when the game ends first.
     */
    private void record(Path list, String fingerprint) {
        long limit = seconds("record", 60) * 1000L;
        long settle = seconds("settle", 15) * 1000L;
        Set<Class<?>> seen = Collections.newSetFromMap(new IdentityHashMap<Class<?>, Boolean>());
        List<String> order = new ArrayList<String>();
        long calmSince = -1;
        int calmCount = 0;
        while (true) {
            long now = elapsedMs();
            long pause = now < 10000 ? 100 : now < 30000 ? 250 : 1000;
            try {
                Thread.sleep(pause);
            } catch (InterruptedException e) {
                return;
            }
            List<String> fresh = new ArrayList<String>();
            for (Class<?> c : inst.getInitiatedClasses(loader)) {
                if (c.getClassLoader() == loader && !c.isArray() && seen.add(c)) {
                    String n = c.getName();
                    if (validName(n)) {
                        fresh.add(n);
                    }
                }
            }
            Collections.sort(fresh);
            order.addAll(fresh);
            now = elapsedMs();
            // calm: fewer than 5 new classes in 5 seconds, at least settle= seconds into the start
            if (order.size() - calmCount >= 5) {
                calmSince = now;
                calmCount = order.size();
            }
            if ((now >= settle && calmSince >= 0 && now - calmSince >= 5000) || now >= limit) {
                break;
            }
        }
        // a class that another loader has defined too (a mod loader's copy is the one the game uses) is left out
        Set<String> elsewhere = definedElsewhere();
        String main = mainClass();
        List<String> kept = new ArrayList<String>(order.size());
        for (String n : order) {
            if (!elsewhere.contains(n) && allowed(n, main)) {
                kept.add(n);
            }
        }
        if (kept.size() < MIN_NAMES) {
            note("only " + kept.size() + " classes were loaded: no class list written");
            return;
        }
        try {
            write(list, fingerprint, elapsedMs(), kept);
            note("wrote " + kept.size() + " class names for the next start");
        } catch (Throwable e) {
            note("could not save the class list: " + e);
        }
    }

    /** Names of the classes that a class loader other than the application loader (and Java's own) has defined. */
    private Set<String> definedElsewhere() {
        Set<String> names = new HashSet<String>();
        for (Class<?> c : inst.getAllLoadedClasses()) {
            ClassLoader l = c.getClassLoader();
            if (l != null && l != loader && !c.isArray()) {
                names.add(c.getName());
            }
        }
        return names;
    }

    /**
     * After loading ahead, the list is removed (so the next start writes a new one) when
     *   - this start loaded many classes the list doesn't have (by the time the list was written in its own start):
     *     the setup changed in a way the fingerprint doesn't see; the classes loaded while the helper threads ran
     *     don't count (linking loads a few more), or
     *   - many classes on the list were also defined by another class loader: the game uses those, and the list's
     *     copies were loaded for nothing (a list from a start that ran differently).
     */
    private void checkDrift(Path list, ListFile old) {
        Set<String> before = new HashSet<String>(old.names);
        for (Class<?> c : inst.getInitiatedClasses(loader)) {
            before.add(c.getName());
        }
        long until = Math.min(old.recordedAtMs, seconds("record", 60) * 1000L);
        long wait = until - elapsedMs();
        if (wait > 0) {
            try {
                Thread.sleep(wait);
            } catch (InterruptedException e) {
                return;
            }
        }
        int added = 0;
        for (Class<?> c : inst.getInitiatedClasses(loader)) {
            if (c.getClassLoader() == loader && !c.isArray() && !before.contains(c.getName())
                    && validName(c.getName())) {
                added++;
            }
        }
        int twice = 0;
        Set<String> elsewhere = definedElsewhere();
        for (String n : old.names) {
            if (elsewhere.contains(n)) {
                twice++;
            }
        }
        String why = null;
        if (added > Math.max(200, old.names.size() / 10)) {
            why = added + " classes were not on the class list";
        } else if (twice > Math.max(50, old.names.size() / 20)) {
            why = twice + " classes on the class list belong to another class loader";
        }
        if (why != null) {
            try {
                Files.deleteIfExists(list);
                note(why + ": the next start writes a new one");
            } catch (Throwable e) {
                // it stays; the fingerprint still protects the game
            }
        }
    }

    private long elapsedMs() {
        return (System.nanoTime() - started) / 1_000_000;
    }

    // ---- the list file

    /** A read class list. */
    static final class ListFile {
        final List<String> names;
        final long recordedAtMs;

        ListFile(List<String> names, long recordedAtMs) {
            this.names = names;
            this.recordedAtMs = recordedAtMs;
        }
    }

    /**
     * Reads a class list made for this fingerprint. Returns null when there is none, it is for another setup, or it is
     * damaged in any way; why[0] then says which.
     */
    static ListFile read(Path file, String fingerprint, String[] why) {
        try {
            if (!Files.isRegularFile(file)) {
                why[0] = "no class list yet";
                return null;
            }
            long size = Files.size(file);
            if (size > MAX_LIST_BYTES) {
                why[0] = "the class list is too big";
                return null;
            }
            byte[] bytes = Files.readAllBytes(file);
            String text = new String(bytes, StandardCharsets.UTF_8);
            String[] lines = text.split("\n", MAX_NAMES + 4);
            if (lines.length < 3 || !lines[0].trim().equals(HEADER)) {
                why[0] = "the class list is from an older helper or damaged";
                return null;
            }
            if (!lines[1].trim().equals("fingerprint " + fingerprint)) {
                why[0] = "the class list was made for another Java or other game files";
                return null;
            }
            long recordedAt = 30000;
            int first = 2;
            if (lines[2].startsWith("recorded ")) {
                try {
                    recordedAt = Long.parseLong(lines[2].substring(9).trim());
                } catch (NumberFormatException ignored) {
                    // keep the default
                }
                first = 3;
            }
            List<String> names = new ArrayList<String>(lines.length);
            Set<String> unique = new HashSet<String>();
            boolean complete = false;
            for (int i = first; i < lines.length && names.size() < MAX_NAMES; i++) {
                String n = lines[i].trim();
                if (n.equals("# end")) {
                    complete = true;
                    break;
                }
                if (validName(n) && unique.add(n)) {
                    names.add(n);
                }
            }
            if (!complete) {
                why[0] = "the class list is cut short";
                return null;
            }
            if (names.size() < MIN_NAMES) {
                why[0] = "the class list is too short";
                return null;
            }
            return new ListFile(names, recordedAt);
        } catch (Throwable e) {
            why[0] = "the class list can't be read (" + e.getClass().getSimpleName() + ")";
            return null;
        }
    }

    /**
     * Writes a class list: to a file of its own beside it first, then moved over the old one in one step, so a game
     * that reads it, or another game that writes it at the same time, never sees half a file.
     */
    static void write(Path file, String fingerprint, long recordedAtMs, List<String> names) throws IOException {
        Path abs = file.toAbsolutePath();
        Path dir = abs.getParent();
        if (dir != null) {
            Files.createDirectories(dir);
        }
        String base = abs.getFileName().toString();
        cleanLeftovers(dir, base);
        long unique = System.nanoTime() ^ ((long) System.identityHashCode(names) << 20);
        Path tmp = abs.resolveSibling(base + "." + Long.toHexString(unique) + ".tmp");
        boolean moved = false;
        try {
            OutputStream out = Files.newOutputStream(tmp);
            try {
                StringBuilder sb = new StringBuilder(names.size() * 48 + 200);
                sb.append(HEADER).append('\n');
                sb.append("fingerprint ").append(fingerprint).append('\n');
                sb.append("recorded ").append(recordedAtMs).append('\n');
                for (String n : names) {
                    sb.append(n).append('\n');
                }
                sb.append("# end\n");
                out.write(sb.toString().getBytes(StandardCharsets.UTF_8));
            } finally {
                out.close();
            }
            try {
                Files.move(tmp, abs, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
            } catch (AtomicMoveNotSupportedException e) {
                Files.move(tmp, abs, StandardCopyOption.REPLACE_EXISTING);
            }
            moved = true;
        } finally {
            if (!moved) {
                try {
                    Files.deleteIfExists(tmp);
                } catch (Throwable ignored) {
                    // a leftover is cleaned up by a later write
                }
            }
        }
    }

    /** Removes temporary files of earlier writes that were cut off (the game was killed), older than ten minutes. */
    private static void cleanLeftovers(Path dir, String base) {
        if (dir == null) {
            return;
        }
        DirectoryStream<Path> ds = null;
        try {
            ds = Files.newDirectoryStream(dir);
            long old = System.currentTimeMillis() - 10 * 60 * 1000L;
            for (Path p : ds) {
                String n = p.getFileName().toString();
                if (n.startsWith(base + ".") && n.endsWith(".tmp") && Files.getLastModifiedTime(p).toMillis() < old) {
                    Files.deleteIfExists(p);
                }
            }
        } catch (Throwable ignored) {
            // only tidying
        } finally {
            if (ds != null) {
                try {
                    ds.close();
                } catch (Throwable ignored) {
                    // closed anyway
                }
            }
        }
    }

    /**
     * The game's main class: the first word of the command (the rest can hold account details and isn't looked at),
     * or "" when this Java doesn't say.
     */
    static String mainClass() {
        String command = System.getProperty("sun.java.command", "").trim();
        int space = command.indexOf(' ');
        return space < 0 ? command : command.substring(0, space);
    }

    /**
     * Whether a class may be loaded ahead (or written down) for a game started with this main class. Minecraft's own
     * classes only when the game runs without a mod loader (vanilla's main class): with Fabric, Quilt, Forge, NeoForge
     * or LaunchWrapper the mod loader's class loader defines them, and a copy in the application loader is dead weight.
     */
    static boolean allowed(String name, String mainClass) {
        if (VANILLA_MAIN.equals(mainClass)) {
            return true;
        }
        boolean minecraft = name.startsWith("net.minecraft.") && !name.startsWith("net.minecraft.launchwrapper.");
        return !minecraft && !name.startsWith("com.mojang.blaze3d.");
    }

    static final String VANILLA_MAIN = "net.minecraft.client.main.Main";

    /** A plain binary class name: Java identifiers joined by dots, not too long, not one of the helper's own. */
    static boolean validName(String n) {
        int len = n.length();
        if (len == 0 || len > MAX_NAME_LENGTH || n.startsWith("com.blockforge.boost.")) {
            return false;
        }
        boolean start = true;
        for (int i = 0; i < len; i++) {
            char c = n.charAt(i);
            if (c == '.') {
                if (start) {
                    return false;
                }
                start = true;
            } else if ((start ? Character.isJavaIdentifierStart(c) : Character.isJavaIdentifierPart(c))
                    && !Character.isIdentifierIgnorable(c)) {
                start = false;
            } else {
                return false;
            }
        }
        return !start;
    }

    /**
     * What a list describes: this helper's list format (VERSION), the Java, the launcher's key, the main class, and
     * every class path and module path entry with its size and date. A change to any of them (a Java or library
     * update, another loader version) means the list is not used.
     */
    static String fingerprint(String key) {
        long h = 0xcbf29ce484222325L;
        h = mix(h, VERSION);
        String[] props = {"java.version", "java.vm.name", "java.vm.version", "java.home", "os.arch"};
        for (String p : props) {
            h = mix(h, System.getProperty(p, ""));
        }
        h = mix(h, key == null ? "" : key);
        // the main class: a game started through another loader with the same class path loads other classes
        h = mix(h, mainClass());
        // Java 8 lists this helper's jar in java.class.path: only its name counts, not its size and date, as two
        // launchers sharing a folder (two editions, or two versions) can write it in turns
        File own = ownJar();
        String[] paths = {"java.class.path", "jdk.module.path"};
        int entries = 0;
        for (String p : paths) {
            h = mix(h, "|" + p);
            for (String e : System.getProperty(p, "").split(File.pathSeparator)) {
                if (e.isEmpty()) {
                    continue;
                }
                h = mix(h, e);
                try {
                    File f = new File(e).getAbsoluteFile();
                    h = mix(h, f.equals(own) ? "agent" : f.length() + ":" + f.lastModified());
                } catch (Throwable ignored) {
                    h = mix(h, "?");
                }
                entries++;
            }
        }
        return Long.toHexString(h) + "-" + entries;
    }

    /** This helper's jar file, or null. */
    private static File ownJar() {
        try {
            return new File(Boost.class.getProtectionDomain().getCodeSource().getLocation().toURI()).getAbsoluteFile();
        } catch (Throwable e) {
            return null;
        }
    }

    private static long mix(long h, String s) {
        for (int i = 0; i < s.length(); i++) {
            h ^= s.charAt(i);
            h *= 0x100000001b3L;
        }
        h ^= 0xff;
        return h * 0x100000001b3L;
    }

    static Map<String, String> parse(String args) {
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
                m.put(p.substring(0, eq).trim(), p.substring(eq + 1));
            }
        }
        return m;
    }

    private void note(String what) {
        if (!quiet) {
            say(what);
        }
    }

    private static void say(String what) {
        try {
            System.out.println(TAG + what);
        } catch (Throwable ignored) {
            // nothing more to do
        }
    }

    static void problem(String what, Throwable e) {
        say("problem: " + what + ": " + e);
    }
}
