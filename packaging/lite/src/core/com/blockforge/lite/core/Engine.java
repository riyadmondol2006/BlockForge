package com.blockforge.lite.core;

import java.io.File;
import java.io.FileOutputStream;
import java.io.OutputStreamWriter;
import java.io.Writer;
import java.lang.instrument.ClassFileTransformer;
import java.security.ProtectionDomain;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicLongArray;
import org.objectweb.asm.tree.MethodNode;

/**
 * Runs the patches. For each class the agent hands over it
 *   1. checks the constant pool for the patches' marker texts (most classes stop here),
 *   2. skips patches that a mod installed in this game already does,
 *   3. lets each matching patch change the class (read with ASM, written back without computing frames),
 *   4. reads the result again and runs ASM's verifier over the changed methods.
 * Any surprise or error in a step leaves that patch out; the class is then loaded as it was (or with the patches
 * that did work). Nothing here can stop the game from starting.
 */
public final class Engine implements ClassFileTransformer {
    static final String TAG = "[BlockForge Lite] ";
    /** When the summary is printed if some patch never found its target (it may come later, or never). */
    static final long SUMMARY_AFTER_MS = 60000L;

    /** All patches, in the order they are applied to a class. */
    static List<Patch> all() {
        return Arrays.<Patch>asList(new IdleFps(), new NoWorldGc(), new SlimSections(), new ShortFade(),
                new OutlineSkip(), new WorkerPriority());
    }

    final Options options;
    final Mods mods;
    private final AtomicLongArray entryStats;
    private final Map<String, State> states = new LinkedHashMap<String, State>();
    /** The patches still looking for their target; replaced (never changed) so the class loaders can read it freely. */
    private volatile Active active;
    private long nanos;
    private long parsed;
    private boolean summaryDone;
    /** Whether the game was started through a mod loader (Fabric/Quilt Knot, ModLauncher, LaunchWrapper). */
    private final boolean modLoaderMain = modLoaderMain();
    private final ClassLoader appLoader = ClassLoader.getSystemClassLoader();

    private static boolean modLoaderMain() {
        try {
            String cmd = System.getProperty("sun.java.command", "");
            String main = cmd.split(" ")[0];
            return main.contains("knot.Knot") || main.startsWith("cpw.mods.") || main.startsWith("net.neoforged.")
                    || main.startsWith("net.minecraftforge.") || main.equals("net.minecraft.launchwrapper.Launch")
                    || main.startsWith("org.quiltmc.");
        } catch (Throwable e) {
            return false;
        }
    }

    /** From targets=FILE: patch id -> the runtime names of its target class, as the launcher worked them out. */
    private final Map<String, java.util.Set<String>> targets;

    /** Whether a class of this name may be the patch's target. */
    private boolean mayBe(Patch p, String name) {
        java.util.Set<String> known = targets.get(p.id);
        if (known != null) {
            return known.contains(name);
        }
        if (options.strict) {
            // only where the runtime class name is Mojang's (NeoForge, Forge 1.17+, 26.x): no obfuscated or
            // intermediary names
            return p.names != null && p.names.contains(name) && !name.startsWith("net/minecraft/class_");
        }
        return p.mayBe(name);
    }

    private static Map<String, java.util.Set<String>> readTargets(String file) {
        Map<String, java.util.Set<String>> m = new java.util.HashMap<String, java.util.Set<String>>();
        if (file == null) {
            return m;
        }
        try {
            for (String line : new String(java.nio.file.Files.readAllBytes(new File(file).toPath()), "UTF-8")
                    .split("\n")) {
                String[] f = line.trim().split("\\s+");
                if (f.length == 2 && !f[0].startsWith("#")) {
                    java.util.Set<String> s = m.get(f[0]);
                    if (s == null) {
                        s = new java.util.HashSet<String>();
                        m.put(f[0], s);
                    }
                    s.add(f[1].replace('.', '/'));
                }
            }
        } catch (Throwable e) {
            problem("could not read the targets " + file + ": " + e);
        }
        return m;
    }

    public Engine(String args, AtomicLongArray entryStats) {
        this(args, entryStats, all());
    }

    Engine(String args, AtomicLongArray entryStats, List<Patch> patches) {
        this.options = new Options(args);
        this.mods = new Mods(options.mods, options.modIds);
        this.entryStats = entryStats;
        this.targets = readTargets(options.targets);
        List<Patch> on = new ArrayList<Patch>();
        for (Patch p : patches) {
            State s = new State(p);
            states.put(p.id, s);
            String why = options.whyOff(p);
            if (why == null) {
                on.add(p);
            } else {
                s.skip("-", why);
            }
        }
        setActive(on);
        for (Patch p : on) {
            if (p instanceof Tweak) {
                startTweak((Tweak) p);
            }
        }
        writeReport();
        if (!options.quiet || options.report != null) {
            Thread t = new Thread(new Runnable() {
                public void run() {
                    try {
                        // the report again now and then, for the time spent so far
                        long slept = 0;
                        for (long at : new long[] {15000L, 30000L, SUMMARY_AFTER_MS, 120000L, 300000L}) {
                            Thread.sleep(at - slept);
                            slept = at;
                            writeReport();
                            if (at == SUMMARY_AFTER_MS) {
                                summary();
                            }
                        }
                    } catch (Throwable ignored) {
                        // the game ended first
                    }
                }
            }, "BlockForge-Lite-summary");
            t.setDaemon(true);
            t.start();
        }
    }

    /** Tweaks that already run in this JVM (the agent's engine and the self-test's engines share the JVM). */
    private static final java.util.Set<String> STARTED = new java.util.HashSet<String>();

    private void startTweak(Tweak p) {
        String why;
        try {
            synchronized (STARTED) {
                why = STARTED.contains(p.id) ? null : p.start();
                if (why == null) {
                    STARTED.add(p.id);
                }
            }
        } catch (Throwable e) {
            why = null;
            decide(p, states.get(p.id), false, "-", "error: " + e, true);
            return;
        }
        decide(p, states.get(p.id), why == null, "-", why, false);
    }

    private void setActive(List<Patch> on) {
        active = new Active(on);
    }

    /** The patches still looking, with the filter for all their marker texts. */
    private static final class Active {
        final Patch[] patches;
        final String[] texts;
        final ConstantPool filter;

        Active(List<Patch> on) {
            List<String> t = new ArrayList<String>();
            for (Patch p : on) {
                for (String[] set : p.markers) {
                    for (String m : set) {
                        if (!t.contains(m)) {
                            t.add(m);
                        }
                    }
                }
            }
            patches = on.toArray(new Patch[0]);
            texts = t.toArray(new String[0]);
            filter = new ConstantPool(texts);
        }
    }

    @Override
    public byte[] transform(ClassLoader loader, String name, Class<?> redefined, ProtectionDomain domain,
            byte[] bytes) {
        long t0 = System.nanoTime();
        try {
            Active now = active;
            if (now.patches.length == 0 || bytes == null || name == null) {
                return null;
            }
            if (modLoaderMain && loader == appLoader) {
                // with a mod loader the game lives in its own class loader; a copy in the application class loader
                // (Fast start's preloading can make one) is never used, so it is not worth a decision
                return null;
            }
            boolean any = false;
            for (Patch p : now.patches) {
                any |= mayBe(p, name);
            }
            if (!any) {
                return null;
            }
            if (options.strict && bytes.length > 8 && (((bytes[6] & 0xFF) << 8) | (bytes[7] & 0xFF)) < 60) {
                // strict: Minecraft 1.17 and newer only (Java 16+ class files)
                return null;
            }
            String[] texts = now.texts;
            boolean[] found = new boolean[texts.length];
            if (!now.filter.scan(bytes, found)) {
                return null;
            }
            List<Patch> candidates = null;
            for (Patch p : now.patches) {
                if (mayBe(p, name) && hasAll(p, texts, found)) {
                    if (candidates == null) {
                        candidates = new ArrayList<Patch>(2);
                    }
                    candidates.add(p);
                }
            }
            if (candidates == null) {
                return null;
            }
            return run(loader, name, bytes, candidates);
        } catch (Throwable e) {
            problem("while looking at " + name + ": " + e);
            return null;
        } finally {
            synchronized (this) {
                nanos += System.nanoTime() - t0;
            }
        }
    }

    /** Whether the class has all the texts of one of the patch's marker sets. */
    private static boolean hasAll(Patch p, String[] texts, boolean[] found) {
        for (String[] set : p.markers) {
            boolean all = true;
            for (String m : set) {
                for (int i = 0; i < texts.length && all; i++) {
                    if (texts[i].equals(m) && !found[i]) {
                        all = false;
                    }
                }
            }
            if (all) {
                return true;
            }
        }
        return false;
    }

    private byte[] run(ClassLoader loader, String name, byte[] original, List<Patch> candidates) {
        synchronized (this) {
            parsed++;
        }
        Patch.Ctx ctx = new Patch.Ctx(loader, mods, options.level);
        byte[] current = original;
        boolean changed = false;
        for (Patch p : candidates) {
            State s = states.get(p.id);
            if (p.conflictsWith.length > 0) {
                String mod = mods.installed(p.conflictsWith, loader);
                if (mod != null) {
                    decide(p, s, false, name, mod + " is installed and does the same", false);
                    continue;
                }
            }
            byte[] out;
            List<MethodNode> touched;
            try {
                Target t = new Target(current);
                p.apply(t, ctx);
                touched = t.touched();
                if (touched.isEmpty()) {
                    continue;
                }
                out = t.write();
            } catch (Patch.Skip skip) {
                if (!skip.notTarget) {
                    decide(p, s, false, name, skip.getMessage(), false);
                }
                continue;
            } catch (IllegalArgumentException e) {
                // ASM refuses class files of a newer Java than it knows
                decide(p, s, false, name, "can't read this class file: " + e.getMessage(), false);
                continue;
            } catch (Throwable e) {
                decide(p, s, false, name, "error: " + e, true);
                continue;
            }
            try {
                Target.check(out, touched);
            } catch (Throwable e) {
                decide(p, s, false, name, "the changed code did not pass the check: " + e.getMessage(), true);
                continue;
            }
            current = out;
            changed = true;
            decide(p, s, true, name, null, false);
        }
        dump(name, original, changed ? current : null);
        return changed ? current : null;
    }

    /**
     * For people writing patches: with -Dblockforge.lite.dump=DIR, every class a patch looked at is written there as
     * NAME.before.class (as the agent got it, after the mod loader's changes) and, when changed, NAME.after.class.
     */
    private static void dump(String name, byte[] before, byte[] after) {
        String dir = System.getProperty("blockforge.lite.dump");
        if (dir == null || dir.isEmpty()) {
            return;
        }
        try {
            File base = new File(dir, name.replace('/', '.'));
            base.getParentFile().mkdirs();
            java.nio.file.Files.write(new File(base.getPath() + ".before.class").toPath(), before);
            if (after != null) {
                java.nio.file.Files.write(new File(base.getPath() + ".after.class").toPath(), after);
            }
        } catch (Throwable e) {
            problem("could not write " + name + " to " + dir + ": " + e);
        }
    }

    private void decide(Patch p, State s, boolean applied, String cls, String reason, boolean failed) {
        boolean printSummary = false;
        synchronized (this) {
            if (applied) {
                s.apply(cls);
            } else if (failed) {
                s.fail(cls, reason);
            } else {
                s.skip(cls, reason);
            }
            // a patch with one target stops looking once it found it (applied or not)
            if (p.once() && (applied || s.status == State.SKIPPED || s.status == State.FAILED)) {
                List<Patch> on = new ArrayList<Patch>(Arrays.asList(active.patches));
                if (on.remove(p)) {
                    setActive(on);
                }
                printSummary = active.patches.length == 0;
            }
        }
        if (failed) {
            problem(p.id + " was not applied to " + cls + ": " + reason);
        } else if (options.verbose) {
            System.out.println(TAG + p.id + (applied ? " applied to " + cls : " skipped (" + cls + "): " + reason));
        }
        writeReport();
        if (printSummary) {
            summary();
        }
    }

    /** What happened to each patch so far (a copy). */
    synchronized List<State> states() {
        return new ArrayList<State>(states.values());
    }

    /** One line: what is on. Printed once, when every patch has decided or a while after the start. */
    void summary() {
        String line;
        synchronized (this) {
            if (summaryDone || options.quiet) {
                return;
            }
            summaryDone = true;
            List<String> ids = new ArrayList<String>();
            int skipped = 0;
            for (State s : states.values()) {
                if (s.status == State.APPLIED) {
                    ids.add(s.patch.id);
                } else {
                    skipped++;
                }
            }
            line = TAG + "on: " + ids.size() + " patches applied" + (ids.isEmpty() ? "" : " (" + join(ids) + ")")
                    + ", " + skipped + " skipped";
        }
        System.out.println(line);
    }

    private static String join(List<String> l) {
        StringBuilder b = new StringBuilder();
        for (String s : l) {
            if (b.length() > 0) {
                b.append(' ');
            }
            b.append(s);
        }
        return b.toString();
    }

    /**
     * The report file: one line per patch, tab separated: id, applied/skipped/failed, class, reason, and what the patch
     * does in plain words (the launcher shows that column; older launchers read only the first four).
     */
    void writeReport() {
        if (options.report == null) {
            return;
        }
        StringBuilder b = new StringBuilder();
        synchronized (this) {
            b.append("# BlockForge Lite report. Columns: id, applied/skipped/failed, class, reason, what it does\n");
            long entryMs = entryStats == null ? 0 : entryStats.get(0) / 1000000L;
            long seen = entryStats == null ? 0 : entryStats.get(1);
            long handed = entryStats == null ? 0 : entryStats.get(2);
            b.append("# time: ").append(entryMs).append(" ms in the agent for ").append(seen).append(" classes (")
                    .append(handed).append(" looked at, ").append(parsed).append(" read with ASM; ")
                    .append(nanos / 1000000L).append(" ms of it in the patch code)\n");
            for (State s : states.values()) {
                b.append(s.patch.id).append('\t').append(s.statusName()).append('\t').append(s.cls).append('\t')
                        .append(s.reason == null ? "" : s.reason.replace('\t', ' ').replace('\n', ' ')).append('\t')
                        .append(s.patch.what == null ? "" : s.patch.what.replace('\t', ' ').replace('\n', ' '))
                        .append('\n');
            }
        }
        try {
            File f = new File(options.report);
            File tmp = new File(f.getPath() + ".tmp");
            Writer w = new OutputStreamWriter(new FileOutputStream(tmp), "UTF-8");
            try {
                w.write(b.toString());
            } finally {
                w.close();
            }
            if (!tmp.renameTo(f)) {
                f.delete();
                if (!tmp.renameTo(f)) {
                    tmp.delete();
                }
            }
        } catch (Throwable e) {
            // a report that can't be written is not worth stopping for; say it once
            if (!reportProblem) {
                reportProblem = true;
                problem("could not write the report " + options.report + ": " + e);
            }
        }
    }

    private boolean reportProblem;

    static void problem(String what) {
        try {
            System.out.println(TAG + "problem: " + what);
        } catch (Throwable ignored) {
            // nothing more to do
        }
    }

    /** What happened to one patch so far. */
    static final class State {
        static final int PENDING = 0;
        static final int APPLIED = 1;
        static final int SKIPPED = 2;
        static final int FAILED = 3;
        final Patch patch;
        int status = PENDING;
        String cls = "-";
        String reason = "no matching code was loaded";
        int count;

        State(Patch patch) {
            this.patch = patch;
        }

        void apply(String c) {
            count++;
            status = APPLIED;
            cls = count == 1 ? c : cls + " " + c;
            reason = patch.what;
        }

        void skip(String c, String why) {
            if (status == APPLIED) {
                return;
            }
            status = SKIPPED;
            cls = c;
            reason = why;
        }

        void fail(String c, String why) {
            if (status == APPLIED) {
                return;
            }
            status = FAILED;
            cls = c;
            reason = why;
        }

        String statusName() {
            return status == APPLIED ? "applied" : status == FAILED ? "failed" : "skipped";
        }
    }
}
