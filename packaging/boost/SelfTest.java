package com.blockforge.boost;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.DirectoryStream;
import java.nio.file.FileVisitResult;
import java.nio.file.Files;
import java.nio.file.InvalidPathException;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.SimpleFileVisitor;
import java.nio.file.attribute.BasicFileAttributes;
import java.nio.file.attribute.FileTime;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.jar.JarEntry;
import java.util.jar.JarOutputStream;

/**
 * Checks the fast-start helper on this computer and this Java:
 *   java -cp blockforge-boost.jar com.blockforge.boost.SelfTest
 * Prints PASS or FAIL per check and exits with 1 when any failed. It uses only java.nio.file and starts further Javas
 * (the same one) with the helper attached to a small made-up game, so it runs the same on Windows, macOS and Linux.
 */
public final class SelfTest {
    private static int passed;
    private static int failed;
    private static int skipped;
    private static final AtomicInteger INITIALISED = new AtomicInteger();

    /** Called from the static initialiser of every made-up game class: counts the classes that were started. */
    public static void initialised() {
        INITIALISED.incrementAndGet();
    }

    public static void main(String[] args) throws Exception {
        if (args.length > 0 && args[0].equals("fake-game")) {
            int code = FakeGame.run(args);
            if (code != 0) {
                System.exit(code);
            }
            // a normal end: main returns, and only the helper's threads could keep this Java running
            return;
        }
        Path tmp = Files.createTempDirectory("bf-boost-selftest");
        System.out.println("BlockForge Boost self-test on Java " + System.getProperty("java.version") + " ("
                + System.getProperty("java.vendor") + "), " + System.getProperty("os.name") + " "
                + System.getProperty("os.arch") + ", " + Runtime.getRuntime().availableProcessors() + " processors");
        try {
            options();
            names();
            fingerprints(tmp);
            listFiles(tmp);
            badFiles(tmp);
            writing(tmp);
            preloading(tmp);
            withTheAgent(tmp);
        } catch (Throwable e) {
            fail("the self-test itself", e.toString());
            e.printStackTrace(System.out);
        } finally {
            delete(tmp);
        }
        System.out.println((failed == 0 ? "PASS" : "FAIL") + ": " + passed + " passed, " + failed + " failed, "
                + skipped + " skipped");
        System.exit(failed == 0 ? 0 : 1);
    }

    // ---- checks that need no extra Java

    private static void options() {
        Map<String, String> o = Boost.parse("preload=C:\\Users\\Jo Ann\\AppData\\Roaming\\BlockForge\\cache\\boost\\a b"
                + ".classes,record=5,quiet");
        check("options: Windows path with a drive letter, backslashes and spaces",
                "C:\\Users\\Jo Ann\\AppData\\Roaming\\BlockForge\\cache\\boost\\a b.classes".equals(o.get("preload"))
                        && "5".equals(o.get("record")) && o.containsKey("quiet"),
                o.toString());
        o = Boost.parse("preload=\\\\server\\share\\Block Forge\\x.classes");
        check("options: UNC path", "\\\\server\\share\\Block Forge\\x.classes".equals(o.get("preload")), o.toString());
        o = Boost.parse("preload=/home/ä ö/測試/x=y.classes,key=abc");
        check("options: non-ASCII path with '='", "/home/ä ö/測試/x=y.classes".equals(o.get("preload"))
                && "abc".equals(o.get("key")), o.toString());
        check("options: nothing", Boost.parse(null).isEmpty() && Boost.parse("").isEmpty()
                && Boost.parse(",, ,").isEmpty(), "");
        Boost b = new Boost(Boost.parse("threads=abc"), null, null);
        check("options: bad thread count falls back to the default", b.threads(1) == 0 && b.threads(2) == 1
                && b.threads(4) == 1
                && b.threads(16) == 2, "");
        b = new Boost(Boost.parse("threads=99"), null, null);
        check("options: thread count is capped", b.threads(16) == 6, "" + b.threads(16));
        b = new Boost(Boost.parse("threads=-3"), null, null);
        check("options: negative thread count means none", b.threads(16) == 0, "" + b.threads(16));
    }

    private static void names() {
        String[] good = {"a", "net.minecraft.client.Minecraft", "com.google.common.collect.ImmutableList$Builder",
            "a.b$1", "org.lwjgl.system.MemoryUtil$MemoryAllocator", "日本.クラス", "_x.$y"};
        String[] bad = {"", ".a", "a.", "a..b", "a/b", "[Ljava.lang.String;", "a b", "1a", "a.1b",
            "com.blockforge.boost.Boost", "x\u0000y", "Foo$$Lambda$12/0x0000000800c0b000", repeat('a', 401)};
        boolean ok = true;
        StringBuilder why = new StringBuilder();
        for (String n : good) {
            if (!Boost.validName(n)) {
                ok = false;
                why.append(" rejected ").append(n);
            }
        }
        for (String n : bad) {
            if (Boost.validName(n)) {
                ok = false;
                why.append(" accepted ").append(n.length() > 40 ? n.substring(0, 40) : n);
            }
        }
        check("class names are checked", ok, why.toString());

        String knot = "net.fabricmc.loader.impl.launch.knot.KnotClient";
        String wrapper = "net.minecraft.launchwrapper.Launch";
        String modLauncher = "cpw.mods.bootstraplauncher.BootstrapLauncher";
        check("Minecraft's classes only through the application loader when no mod loader runs",
                Boost.allowed("net.minecraft.client.Minecraft", Boost.VANILLA_MAIN)
                        && Boost.allowed("com.mojang.blaze3d.systems.RenderSystem", Boost.VANILLA_MAIN)
                        && !Boost.allowed("net.minecraft.client.Minecraft", knot)
                        && !Boost.allowed("com.mojang.blaze3d.systems.RenderSystem", knot)
                        && !Boost.allowed("net.minecraft.client.Minecraft", modLauncher)
                        && !Boost.allowed("net.minecraft.client.Minecraft", "")
                        && !Boost.allowed("net.minecraft.client.Minecraft", wrapper)
                        && Boost.allowed("net.minecraft.launchwrapper.LaunchClassLoader", wrapper)
                        && Boost.allowed("net.minecraftforge.fml.relauncher.CoreModManager", wrapper)
                        && Boost.allowed("com.google.common.collect.ImmutableList", knot)
                        && Boost.allowed("com.mojang.authlib.GameProfile", knot), "");
    }

    private static void fingerprints(Path tmp) throws Exception {
        String saved = System.getProperty("java.class.path");
        try {
            Path a = tmp.resolve("lib a.jar");
            Path b = tmp.resolve("lib-b.jar");
            Files.write(a, new byte[100]);
            Files.write(b, new byte[200]);
            System.setProperty("java.class.path", a + File.pathSeparator + b);
            String f1 = Boost.fingerprint("k");
            check("fingerprint: the same setup gives the same one", f1.equals(Boost.fingerprint("k")), f1);
            check("fingerprint: another launcher key gives another one", !f1.equals(Boost.fingerprint("k2")), f1);
            Files.write(b, new byte[201]);
            String f2 = Boost.fingerprint("k");
            check("fingerprint: a library of another size gives another one", !f1.equals(f2), f2);
            Files.setLastModifiedTime(b, FileTime.fromMillis(Files.getLastModifiedTime(b).toMillis() - 86400000L));
            check("fingerprint: a library with another date gives another one", !f2.equals(Boost.fingerprint("k")),
                    "");
            System.setProperty("java.class.path", b + File.pathSeparator + a);
            check("fingerprint: another class path order gives another one", !f2.equals(Boost.fingerprint("k")), "");
            // the helper's own jar in the class path (Java 8): written again by another launcher, same list
            Path own = ownJar();
            if (own != null) {
                System.setProperty("java.class.path", own + File.pathSeparator + a);
                String g = Boost.fingerprint("k");
                FileTime was = Files.getLastModifiedTime(own);
                try {
                    Files.setLastModifiedTime(own, FileTime.fromMillis(was.toMillis() - 3600000L));
                    check("fingerprint: the helper's own jar written again gives the same one",
                            g.equals(Boost.fingerprint("k")), g);
                } catch (IOException e) {
                    skip("fingerprint: the helper's own jar written again", "can't change its date: " + e);
                } finally {
                    try {
                        Files.setLastModifiedTime(own, was);
                    } catch (IOException ignored) {
                        // only its date
                    }
                }
            }
            // a very long class path, much of it missing
            StringBuilder cp = new StringBuilder();
            for (int i = 0; i < 3000; i++) {
                cp.append(tmp.resolve("missing-library-number-" + i + ".jar")).append(File.pathSeparator);
            }
            System.setProperty("java.class.path", cp.toString());
            long t = System.nanoTime();
            String f3 = Boost.fingerprint(null);
            long ms = (System.nanoTime() - t) / 1000000;
            check("fingerprint: 3000 class path entries in " + ms + " ms", f3.endsWith("-3000") && ms < 5000, f3);
        } finally {
            System.setProperty("java.class.path", saved);
        }
    }

    private static List<String> someNames(int n, String prefix) {
        List<String> l = new ArrayList<String>();
        for (int i = 0; i < n; i++) {
            l.add(prefix + ".C" + i);
        }
        return l;
    }

    private static void listFiles(Path tmp) throws Exception {
        Path dir = tmp.resolve("lists");
        Path f = dir.resolve("inst.classes");
        List<String> names = someNames(500, "game.pkg");
        Boost.write(f, "fp1", 12345, names);
        String[] why = new String[1];
        Boost.ListFile l = Boost.read(f, "fp1", why);
        check("list: written and read back in order", l != null && l.names.equals(names) && l.recordedAtMs == 12345,
                why[0]);
        check("list: made for another setup is not used", Boost.read(f, "fp2", why) == null
                && why[0].contains("another"), why[0]);
        check("list: a missing one is reported as missing", Boost.read(dir.resolve("none.classes"), "fp1", why) == null
                && why[0].contains("no class list"), why[0]);
        check("list: no temporary files are left", listing(dir).equals(Collections.singletonList("inst.classes")),
                listing(dir).toString());
        // Windows line ends, blank lines, duplicates and bad names are tolerated
        StringBuilder sb = new StringBuilder(Boost.HEADER + "\r\nfingerprint fp1\r\nrecorded 9\r\n");
        for (String n : names) {
            sb.append(n).append("\r\n\r\n").append(n).append("\r\n").append("bad name/").append("\r\n");
        }
        sb.append("# end\r\n");
        Files.write(f, sb.toString().getBytes(StandardCharsets.UTF_8));
        l = Boost.read(f, "fp1", why);
        check("list: Windows line ends, duplicates and bad lines", l != null && l.names.equals(names), why[0]);
        // non-ASCII folder names and spaces
        Path odd;
        try {
            odd = tmp.resolve("Blöck Forge 測試 данные").resolve("cache dir");
        } catch (InvalidPathException e) {
            skip("list: in a folder with a non-ASCII name", "this system can't name such a folder: " + e.getMessage());
            odd = null;
        }
        if (odd != null) {
            try {
                Path g = odd.resolve("ïnst ä.classes");
                Boost.write(g, "fp1", 1, names);
                l = Boost.read(g, "fp1", why);
                check("list: in a folder with a non-ASCII name and spaces", l != null && l.names.equals(names), why[0]);
            } catch (IOException e) {
                skip("list: in a folder with a non-ASCII name", "the file system refused it: " + e);
            }
        }
        // a long path
        Path deep = tmp;
        for (int i = 0; i < 12; i++) {
            deep = deep.resolve("a-rather-long-folder-name-" + i);
        }
        try {
            Path g = deep.resolve("inst.classes");
            Boost.write(g, "fp1", 1, names);
            l = Boost.read(g, "fp1", why);
            check("list: under a path of " + g.toString().length() + " characters", l != null, why[0]);
        } catch (IOException e) {
            // allowed: the agent then just doesn't save; what matters is that nothing else is thrown
            pass("list: a very long path is refused with an IOException (" + e.getClass().getSimpleName() + ")");
        }
    }

    private static void badFiles(Path tmp) throws Exception {
        Path dir = tmp.resolve("bad");
        Files.createDirectories(dir);
        String[] why = new String[1];
        Path f = dir.resolve("x.classes");

        Files.write(f, new byte[0]);
        check("bad list: empty", Boost.read(f, "fp", why) == null, why[0]);

        Files.write(f, (Boost.HEADER + "\n").getBytes(StandardCharsets.UTF_8));
        check("bad list: only the first line", Boost.read(f, "fp", why) == null, why[0]);

        byte[] junk = new byte[200000];
        new Random(42).nextBytes(junk);
        Files.write(f, junk);
        check("bad list: random bytes", Boost.read(f, "fp", why) == null, why[0]);

        Boost.write(f, "fp", 1, someNames(1000, "a.b"));
        byte[] whole = Files.readAllBytes(f);
        Files.write(f, Arrays.copyOf(whole, whole.length / 2));
        check("bad list: cut short (half written)", Boost.read(f, "fp", why) == null && why[0].contains("short"),
                why[0]);

        Boost.write(f, "fp", 1, someNames(20, "a.b"));
        check("bad list: too few names", Boost.read(f, "fp", why) == null, why[0]);

        Boost.write(f, "fp", 1, someNames(500, "a.b"));
        whole = Files.readAllBytes(f);
        for (int i = 200; i < whole.length - 100; i += 37) {
            whole[i] = (byte) 0xC3; // a broken UTF-8 sequence here and there
        }
        Files.write(f, whole);
        check("bad list: broken UTF-8 never throws", readNeverThrows(f), why[0]);

        String older = "# BlockForge Boost class list 1\nfingerprint fp\n" + join(someNames(500, "a.b")) + "# end\n";
        Files.write(f, older.getBytes(StandardCharsets.UTF_8));
        check("bad list: from another helper version", Boost.read(f, "fp", why) == null, why[0]);

        // the plain list of the first helper version (no header)
        Files.write(f, join(someNames(500, "a.b")).getBytes(StandardCharsets.UTF_8));
        check("bad list: the first version's plain list", Boost.read(f, "fp", why) == null, why[0]);

        // a huge file is not read at all
        OutputStream out = Files.newOutputStream(f);
        try {
            byte[] mb = new byte[1024 * 1024];
            Arrays.fill(mb, (byte) 'a');
            for (int i = 0; i < 9; i++) {
                out.write(mb);
            }
        } finally {
            out.close();
        }
        long t = System.nanoTime();
        boolean refused = Boost.read(f, "fp", why) == null && why[0].contains("too big");
        check("bad list: 9 MB is refused unread (" + (System.nanoTime() - t) / 1000000 + " ms)", refused, why[0]);
        Files.delete(f);

        Path d = dir.resolve("a-folder.classes");
        Files.createDirectories(d);
        check("bad list: a folder in its place", Boost.read(d, "fp", why) == null, why[0]);

        Boost.write(f, "fp", 1, someNames(500, "a.b"));
        File file = f.toFile();
        if (file.setReadable(false, false) && !Files.isReadable(f)) {
            check("bad list: not readable", Boost.read(f, "fp", why) == null, why[0]);
            file.setReadable(true, false);
        } else {
            skip("bad list: not readable", "this system can't take read access away (Windows, or running as root)");
        }
    }

    private static boolean readNeverThrows(Path f) {
        try {
            Boost.read(f, "fp", new String[1]);
            return true;
        } catch (Throwable e) {
            return false;
        }
    }

    private static void writing(Path tmp) throws Exception {
        String[] why = new String[1];
        // several games writing the same list at once: the result is always one whole list
        final Path f = tmp.resolve("race").resolve("same.classes");
        final List<List<String>> variants = new ArrayList<List<String>>();
        for (int i = 0; i < 6; i++) {
            variants.add(someNames(300 + i * 50, "race" + i));
        }
        final AtomicInteger errors = new AtomicInteger();
        final List<String> errorText = Collections.synchronizedList(new ArrayList<String>());
        Thread[] writers = new Thread[6];
        for (int i = 0; i < writers.length; i++) {
            final int k = i;
            writers[i] = new Thread(new Runnable() {
                public void run() {
                    for (int j = 0; j < 25; j++) {
                        try {
                            Boost.write(f, "fp", j, variants.get(k));
                        } catch (IOException e) {
                            // Windows can refuse a replace while another writer replaces: the agent then skips saving
                            if (!(e instanceof java.nio.file.AccessDeniedException)
                                    && !(e instanceof java.nio.file.FileSystemException)) {
                                errors.incrementAndGet();
                                errorText.add(e.toString());
                            }
                        } catch (Throwable e) {
                            errors.incrementAndGet();
                            errorText.add(e.toString());
                        }
                    }
                }
            });
            writers[i].start();
        }
        final AtomicInteger badReads = new AtomicInteger();
        final AtomicInteger goodReads = new AtomicInteger();
        Thread reader = new Thread(new Runnable() {
            public void run() {
                for (int j = 0; j < 200; j++) {
                    String[] w = new String[1];
                    Boost.ListFile l = Boost.read(f, "fp", w);
                    if (l != null) {
                        goodReads.incrementAndGet();
                        if (!variants.contains(l.names)) {
                            badReads.incrementAndGet();
                        }
                    } else if (w[0] != null && !w[0].contains("no class list") && !w[0].contains("can't be read")) {
                        badReads.incrementAndGet();
                    }
                }
            }
        });
        reader.start();
        for (Thread t : writers) {
            t.join();
        }
        reader.join();
        Boost.ListFile l = Boost.read(f, "fp", why);
        check("writing: six games at once leave one whole list", l != null && variants.contains(l.names)
                && errors.get() == 0, errorText.toString());
        check("writing: a reader meanwhile never sees half a list (" + goodReads.get() + " reads)",
                badReads.get() == 0, "" + badReads.get());
        check("writing: no temporary files are left", listing(f.getParent()).equals(
                Collections.singletonList("same.classes")), listing(f.getParent()).toString());

        // the folder can't be made: a file is in the way
        Path blocker = tmp.resolve("blocker");
        Files.write(blocker, new byte[1]);
        check("writing: a file where the folder should be gives an IOException", writeFails(
                blocker.resolve("x.classes")), "");

        // a read-only folder
        Path ro = tmp.resolve("read-only");
        Files.createDirectories(ro);
        File rof = ro.toFile();
        if (rof.setWritable(false, false) && !canCreate(ro)) {
            check("writing: a read-only folder gives an IOException and leaves nothing", writeFails(
                    ro.resolve("x.classes")) && listing(ro).isEmpty(), listing(ro).toString());
        } else {
            skip("writing: a read-only folder", "this system can't make a folder read-only (Windows, or root)");
        }
        rof.setWritable(true, false);

        // leftovers of a killed game are tidied up, fresh ones of a game still writing are not
        Path dir = tmp.resolve("tidy");
        Files.createDirectories(dir);
        Path old = dir.resolve("x.classes.1234.tmp");
        Path young = dir.resolve("x.classes.5678.tmp");
        Path other = dir.resolve("y.classes.1234.tmp");
        Files.write(old, new byte[1]);
        Files.write(young, new byte[1]);
        Files.write(other, new byte[1]);
        Files.setLastModifiedTime(old, FileTime.fromMillis(System.currentTimeMillis() - 3600000L));
        Files.setLastModifiedTime(other, FileTime.fromMillis(System.currentTimeMillis() - 3600000L));
        Boost.write(dir.resolve("x.classes"), "fp", 1, someNames(200, "t"));
        check("writing: old leftovers are removed, others kept", !Files.exists(old) && Files.exists(young)
                && Files.exists(other), listing(dir).toString());
    }

    private static boolean canCreate(Path dir) {
        try {
            Path p = Files.createTempFile(dir, "probe", ".tmp");
            Files.delete(p);
            return true;
        } catch (IOException e) {
            return false;
        }
    }

    private static boolean writeFails(Path f) {
        try {
            Boost.write(f, "fp", 1, someNames(200, "x"));
            return false;
        } catch (IOException e) {
            return true;
        } catch (Throwable e) {
            return false;
        }
    }

    // ---- loading ahead, in this Java

    private static void preloading(Path tmp) throws Exception {
        Path jar = tmp.resolve("game classes.jar");
        List<String> names = makeGameJar(jar, 300, GAME);
        INITIALISED.set(0);
        ChildFirstLoader loader = new ChildFirstLoader(jar);
        List<String> wanted = new ArrayList<String>(names);
        wanted.add("not.there.Missing");
        wanted.add("java.lang.String");
        long deadline = System.nanoTime() + 60000000000L;
        int[] r = Boost.preload(wanted, loader, 2, true, deadline);
        check("preload: a custom (not parallel-capable) class loader's classes are loaded and linked", r[0] == 301
                && r[1] == 0, Arrays.toString(r));
        check("preload: no static initialiser ran", INITIALISED.get() == 0, "" + INITIALISED.get());
        Class<?> c = Class.forName(names.get(5), false, loader);
        check("preload: the classes belong to the loader that was asked", c.getClassLoader() == loader, "");
        Class.forName(names.get(5), true, loader);
        check("preload: the game starts a class itself, once", INITIALISED.get() == 1, "" + INITIALISED.get());

        // the main thread and the helpers loading the same classes at the same time
        final ChildFirstLoader l2 = new ChildFirstLoader(jar);
        final List<String> all = names;
        final Class<?>[] mine = new Class<?>[all.size()];
        Thread main = new Thread(new Runnable() {
            public void run() {
                for (int i = 0; i < all.size(); i++) {
                    try {
                        mine[i] = Class.forName(all.get(i), false, l2);
                    } catch (Throwable e) {
                        mine[i] = null;
                    }
                }
            }
        });
        main.start();
        Boost.preload(all, l2, 3, true, deadline);
        main.join();
        boolean same = true;
        for (int i = 0; i < all.size(); i++) {
            if (mine[i] == null || mine[i] != Class.forName(all.get(i), false, l2)) {
                same = false;
            }
        }
        check("preload: at the same time as the game, every class is defined once", same, "");

        r = Boost.preload(names, new ChildFirstLoader(jar), 2, true, System.nanoTime() - 1);
        check("preload: stops at the time limit", r[1] == 1 && r[0] < names.size(), Arrays.toString(r));
    }

    // ---- the agent attached to a made-up game in another Java

    private static void withTheAgent(Path tmp) throws Exception {
        Path agent = ownJar();
        if (agent == null) {
            skip("agent runs", "not started from the jar");
            return;
        }
        Path game = tmp.resolve("game dir");
        Files.createDirectories(game);
        Path gameJar = game.resolve("fake game.jar");
        makeGameJar(gameJar, 400, GAME);
        makeGameJar(game.resolve(MODS_JAR), 150, MODS);
        Path list = tmp.resolve("cache dir").resolve("boost").resolve("fake.classes");
        // settle=1: the list is written once class loading has been calm for 5 seconds, not 15 seconds in
        String opts = "preload=" + list + ",settle=1,record=60,threads=2,key=k1";
        String rec = list.toString();

        // 1. no list: the game runs normally and the list is written in the order of loading
        Result r = runGame(agent, gameJar, opts, "record", rec);
        String[] why = new String[1];
        // the fingerprint is the other Java's (its class path has the game jar): take it from the file
        String fp1 = fingerprintIn(list);
        Boost.ListFile l = Boost.read(list, fp1, why);
        check("agent: a first start runs normally", r.code == 0 && r.out.contains("no class list yet")
                && r.out.contains("fake game: ok"), r.out);
        check("agent: it writes the classes the application loader loaded, in order", l != null
                && l.names.size() == 400 && l.names.get(0).equals(gameName(0, GAME).replace('/', '.'))
                && !containsOwn(l.names), why[0] + "\n" + r.out);

        Result r2 = runGame(agent, gameJar, opts, "preload");
        check("agent: a second start loads the classes ahead", r2.code == 0 && r2.out.contains("loaded 400 of 400"),
                r2.out);
        check("agent: the game's own class loader and static initialisers are untouched", r2.out.contains(
                "fake game: ok"), r2.out);
        check("agent: at most two lines from the helper", count(r2.out, Boost.TAG) <= 2 && count(r.out, Boost.TAG)
                <= 2, r.out + r2.out);
        Result r2b = runGame(agent, gameJar, opts + ",link=0,disk=ssd", "preload");
        check("agent: loading without linking", r2b.code == 0 && r2b.out.contains("loaded 400 of 400")
                && r2b.out.contains("fake game: ok"), r2b.out);

        // 2. another key: the list is for another setup and is replaced
        // the game only has to stay a moment: the helper says at its start that the list is for another setup
        Result r3 = runGame(agent, gameJar, opts.replace("key=k1", "key=k2"), "sleep", "3");
        check("agent: a list for another setup is not used", r3.code == 0 && r3.out.contains(
                "another Java or other game files"), r3.out);
        Files.delete(list);
        Result r3b = runGame(agent, gameJar, opts.replace("key=k1", "key=k2"), "record", rec);
        check("agent: ... and a new one is written", r3b.code == 0 && r3b.out.contains("wrote 400"), r3b.out);

        // 2b. the game's own loader defines the listed classes too (a list from a start that ran differently): the
        // helper's copies were for nothing, and the list is removed. The helper looks at the classes when the list's
        // own start had written it, and the fake game makes its copies only after waiting for the helper's threads (up
        // to 10 seconds when it never sees them run); a slow machine could have the helper look first, find nothing
        // and keep the list while the game waited for it to go. The list says it was written 20 seconds in, so the
        // helper looks after the copies are made however slow the machine is.
        String fp2 = fingerprintIn(list);
        Boost.ListFile before = Boost.read(list, fp2, why);
        if (before != null) {
            Boost.write(list, fp2, 20000, before.names);
        }
        Result rd = runGame(agent, gameJar, opts.replace("key=k1", "key=k2"), "dupes", rec);
        check("agent: a list whose classes another loader defines is removed", rd.code == 0
                && rd.out.contains("belong to another class loader") && rd.out.contains("fake game: list removed"),
                rd.out);

        // 3. a damaged list
        byte[] junk = new byte[5000];
        new Random(7).nextBytes(junk);
        Files.write(list, junk);
        Result r4 = runGame(agent, gameJar, opts, "quick");
        check("agent: a damaged list is ignored", r4.code == 0 && r4.out.contains("damaged")
                && r4.out.contains("fake game: ok"), r4.out);

        // 4. the helper never keeps the game from ending
        // (a list that makes the helper wait ten minutes before it looks at the classes again)
        Boost.write(list, fp1 == null ? "none" : fp1, 600000, gameNames(gameJar));
        long t = System.nanoTime();
        Result r5 = runGame(agent, gameJar, "preload=" + list + ",record=600,threads=2,key=k1", "quick");
        long ms = (System.nanoTime() - t) / 1000000;
        check("agent: the game ends when it is done (" + ms + " ms)", r5.code == 0 && ms < 20000, r5.out);

        // 5. nonsense options and a path that can't be used
        Path aFile = tmp.resolve("a file");
        Files.write(aFile, new byte[1]);
        Result r6 = runGame(agent, gameJar, "threads=x,record=-5,budget=y,link=maybe,settle=z,preload="
                + aFile.resolve("x.classes"), "sleep", "2");
        check("agent: nonsense options and a list path under a file don't stop the game", r6.code == 0
                && !r6.out.contains("problem") && r6.out.contains("fake game: ok"), r6.out);
        Result r7 = runGame(agent, gameJar, "", "quick");
        check("agent: no options at all", r7.code == 0 && !r7.out.contains("problem"), r7.out);

        // on too few processors (here: none allowed) it does nothing at all, not even write a list
        Path offList = tmp.resolve("off").resolve("x.classes");
        Result r7b = runGame(agent, gameJar, "preload=" + offList + ",settle=1,threads=0", "sleep", "7");
        check("agent: off on too few processors, and no list is written", r7b.code == 0 && r7b.out.contains("off:")
                && !Files.exists(offList), r7b.out);

        // 6. quiet
        Result r8 = runGame(agent, gameJar, opts + ",quiet", "quick");
        check("agent: quiet prints nothing", r8.code == 0 && !r8.out.contains(Boost.TAG), r8.out);

        // 7. the list's folder is read-only
        Path roDir = tmp.resolve("ro cache");
        Files.createDirectories(roDir);
        if (roDir.toFile().setWritable(false, false) && !canCreate(roDir)) {
            Result r9 = runGame(agent, gameJar, "preload=" + roDir.resolve("x.classes")
                    + ",settle=1,threads=2,key=k1", "sleep", "9");
            check("agent: a read-only folder doesn't stop the game", r9.code == 0 && r9.out.contains("could not save")
                    && !r9.out.contains("problem"), r9.out);
            roDir.toFile().setWritable(true, false);
        } else {
            skip("agent: a read-only folder", "this system can't make a folder read-only");
        }

        // 8. a folder with a non-ASCII name
        try {
            Path odd = tmp.resolve("Blöck Forge 測試").resolve("x.classes");
            Result a = runGame(agent, gameJar, "preload=" + odd + ",settle=1,threads=2,key=k1", "sleep", "8");
            // whether the other Java can name the file depends on the system's code page; it must run either way
            check("agent: a list in a folder with a non-ASCII name doesn't stop the game", a.code == 0
                    && !a.out.contains("problem") && a.out.contains("fake game: ok"), a.out);
        } catch (InvalidPathException e) {
            skip("agent: non-ASCII folder", "this system can't name such a folder");
        }
    }

    private static boolean containsOwn(List<String> names) {
        for (String n : names) {
            if (n.startsWith("com.blockforge.boost.")) {
                return true;
            }
        }
        return false;
    }

    private static int count(String text, String what) {
        int n = 0;
        for (int i = text.indexOf(what); i >= 0; i = text.indexOf(what, i + 1)) {
            n++;
        }
        return n;
    }

    private static final class Result {
        int code;
        String out;
    }

    /** The fingerprint written in a list file, or null. */
    private static String fingerprintIn(Path list) {
        try {
            String[] lines = new String(Files.readAllBytes(list), StandardCharsets.UTF_8).split("\n", 3);
            return lines.length > 1 && lines[1].startsWith("fingerprint ") ? lines[1].substring(12).trim() : null;
        } catch (IOException e) {
            return null;
        }
    }

    private static Result runGame(Path agent, Path gameJar, String opts, String... gameArgs) throws Exception {
        String java = Paths.get(System.getProperty("java.home"), "bin", File.separatorChar == '\\' ? "java.exe"
                : "java").toString();
        List<String> cmd = new ArrayList<String>();
        cmd.add(java);
        cmd.add("-javaagent:" + agent + (opts.isEmpty() ? "" : "=" + opts));
        cmd.add("-cp");
        cmd.add(agent + File.pathSeparator + gameJar);
        cmd.add(SelfTest.class.getName());
        cmd.add("fake-game");
        cmd.add(gameJar.toString());
        cmd.addAll(Arrays.asList(gameArgs));
        ProcessBuilder pb = new ProcessBuilder(cmd);
        pb.redirectErrorStream(true);
        final Process p = pb.start();
        final ByteArrayOutputStream buf = new ByteArrayOutputStream();
        Thread pump = new Thread(new Runnable() {
            public void run() {
                try {
                    InputStream in = p.getInputStream();
                    byte[] b = new byte[4096];
                    int n;
                    while ((n = in.read(b)) > 0) {
                        synchronized (buf) {
                            buf.write(b, 0, n);
                        }
                    }
                } catch (IOException ignored) {
                    // ended
                }
            }
        });
        pump.start();
        Result r = new Result();
        if (!p.waitFor(90, TimeUnit.SECONDS)) {
            p.destroyForcibly();
            r.code = -1;
        } else {
            r.code = p.exitValue();
        }
        pump.join(5000);
        synchronized (buf) {
            r.out = new String(buf.toByteArray(), StandardCharsets.UTF_8);
        }
        return r;
    }

    /**
     * The made-up game: loads its classes through the application loader, and its "mods" (another jar, not on the class
     * path) through a loader of its own. Arguments: the game jar, then "quick" (just run), "preload" (first let the
     * helper's threads finish, so the check that they started nothing means something), "record FILE" (afterwards
     * wait until the helper wrote FILE), "sleep SECONDS", or "dupes FILE" (as preload, then its own loader defines
     * copies of all game classes too, as a mod loader does; then wait until the helper removed FILE).
     */
    static final class FakeGame {
        static int run(String[] args) throws Exception {
            Path jar = Paths.get(args[1]);
            String mode = args[2];
            List<String> names = gameNames(jar);
            Path modsJar = jar.resolveSibling(MODS_JAR);
            List<String> mods = gameNames(modsJar);
            if (mode.equals("preload") || mode.equals("dupes")) {
                waitForHelpers();
            }
            if (INITIALISED.get() != 0) {
                System.out.println("fake game: a static initialiser ran before the game asked");
                return 3;
            }
            ClassLoader app = ClassLoader.getSystemClassLoader();
            for (int i = 0; i < names.size(); i++) {
                Class<?> a = Class.forName(names.get(i), i % 4 == 0, app);
                if (a.getClassLoader() != app) {
                    System.out.println("fake game: wrong loader for " + names.get(i));
                    return 4;
                }
                if (i % 50 == 0) {
                    Thread.sleep(20);
                }
            }
            ChildFirstLoader own = new ChildFirstLoader(modsJar);
            for (int i = 0; i < mods.size(); i++) {
                Class<?> b = Class.forName(mods.get(i), i % 2 == 0, own);
                if (b.getClassLoader() != own) {
                    System.out.println("fake game: its own loader got another loader's class");
                    return 5;
                }
            }
            try {
                Class.forName(mods.get(0), false, app);
                System.out.println("fake game: the application loader found a mod class");
                return 7;
            } catch (ClassNotFoundException expected) {
                // right: the helper put nothing of the mods where the application loader looks
            }
            int expected = (names.size() + 3) / 4 + (mods.size() + 1) / 2;
            if (INITIALISED.get() != expected) {
                System.out.println("fake game: " + INITIALISED.get() + " static initialisers ran, not " + expected);
                return 6;
            }
            System.out.println("fake game: ok");
            if (mode.equals("record")) {
                Path list = Paths.get(args[3]);
                for (int i = 0; i < 600 && !Files.exists(list); i++) {
                    Thread.sleep(100);
                }
                // the line about it is printed right after the file is in place
                Thread.sleep(300);
            } else if (mode.equals("sleep")) {
                Thread.sleep(Long.parseLong(args[3]) * 1000);
            } else if (mode.equals("dupes")) {
                ChildFirstLoader copies = new ChildFirstLoader(jar);
                for (String n : names) {
                    Class.forName(n, false, copies);
                }
                Path list = Paths.get(args[3]);
                for (int i = 0; i < 600 && Files.exists(list); i++) {
                    Thread.sleep(100);
                }
                Thread.sleep(300);
                System.out.println("fake game: list " + (Files.exists(list) ? "kept" : "removed"));
            }
            return 0;
        }

        /** Waits until the helper's loading threads have come and gone (at most 30 seconds). */
        private static void waitForHelpers() throws InterruptedException {
            boolean seen = false;
            long end = System.nanoTime() + 30000000000L;
            while (System.nanoTime() < end) {
                boolean running = false;
                for (Thread t : Thread.getAllStackTraces().keySet()) {
                    if (t.getName().startsWith("BlockForge-Boost-load") && t.isAlive()) {
                        running = true;
                    }
                }
                if (running) {
                    seen = true;
                } else if (seen || System.nanoTime() > end - 20000000000L) {
                    return;
                }
                Thread.sleep(20);
            }
        }
    }

    /** A class loader that looks in its own jar first and is not parallel-capable, like many a game's loader. */
    static final class ChildFirstLoader extends ClassLoader {
        private final Map<String, byte[]> classes;

        ChildFirstLoader(Path jar) throws IOException {
            super(ChildFirstLoader.class.getClassLoader());
            classes = readJar(jar);
        }

        @Override
        protected synchronized Class<?> loadClass(String name, boolean resolve) throws ClassNotFoundException {
            Class<?> c = findLoadedClass(name);
            if (c == null) {
                byte[] b = classes.get(name.replace('.', '/') + ".class");
                c = b != null ? defineClass(name, b, 0, b.length) : super.loadClass(name, false);
            }
            if (resolve) {
                resolveClass(c);
            }
            return c;
        }
    }

    // ---- made-up game classes: copies of SelfTestTemplate under other names

    private static final String TEMPLATE = "com/blockforge/boost/SelfTestTemplate";
    /** Name prefixes of the made-up classes: as long as each other, so the class files only change in place. */
    private static final String GAME = "bftest/fakegame/C";
    private static final String MODS = "bftest/fakemods/C";
    private static final String MODS_JAR = "fake mods.jar";

    private static String gameName(int i, String prefix) {
        String num = Integer.toString(i);
        StringBuilder sb = new StringBuilder(prefix);
        for (int k = prefix.length() + num.length(); k < TEMPLATE.length(); k++) {
            sb.append('0');
        }
        return sb.append(num).toString();
    }

    private static List<String> makeGameJar(Path jar, int count, String prefix) throws IOException {
        byte[] template = readResource("/" + TEMPLATE + ".class");
        byte[] from = TEMPLATE.getBytes(StandardCharsets.UTF_8);
        List<String> names = new ArrayList<String>();
        JarOutputStream out = new JarOutputStream(Files.newOutputStream(jar));
        try {
            for (int i = 0; i < count; i++) {
                String n = gameName(i, prefix);
                out.putNextEntry(new JarEntry(n + ".class"));
                out.write(replace(template, from, n.getBytes(StandardCharsets.UTF_8)));
                out.closeEntry();
                names.add(n.replace('/', '.'));
            }
        } finally {
            out.close();
        }
        return names;
    }

    private static List<String> gameNames(Path jar) throws IOException {
        List<String> names = new ArrayList<String>(readJar(jar).keySet());
        for (int i = 0; i < names.size(); i++) {
            String n = names.get(i);
            names.set(i, n.substring(0, n.length() - 6).replace('/', '.'));
        }
        Collections.sort(names);
        return names;
    }

    private static Map<String, byte[]> readJar(Path jar) throws IOException {
        Map<String, byte[]> m = new java.util.HashMap<String, byte[]>();
        java.util.jar.JarInputStream in = new java.util.jar.JarInputStream(Files.newInputStream(jar));
        try {
            java.util.jar.JarEntry e;
            while ((e = in.getNextJarEntry()) != null) {
                if (e.getName().endsWith(".class")) {
                    m.put(e.getName(), readAll(in));
                }
            }
        } finally {
            in.close();
        }
        return m;
    }

    private static byte[] replace(byte[] data, byte[] from, byte[] to) {
        byte[] out = data.clone();
        for (int i = 0; i + from.length <= out.length; i++) {
            boolean match = true;
            for (int k = 0; k < from.length && match; k++) {
                match = out[i + k] == from[k];
            }
            if (match) {
                System.arraycopy(to, 0, out, i, to.length);
            }
        }
        return out;
    }

    private static byte[] readResource(String name) throws IOException {
        InputStream in = SelfTest.class.getResourceAsStream(name);
        if (in == null) {
            throw new IOException("missing " + name);
        }
        try {
            return readAll(in);
        } finally {
            in.close();
        }
    }

    private static byte[] readAll(InputStream in) throws IOException {
        ByteArrayOutputStream b = new ByteArrayOutputStream();
        byte[] buf = new byte[8192];
        int n;
        while ((n = in.read(buf)) > 0) {
            b.write(buf, 0, n);
        }
        return b.toByteArray();
    }

    private static Path ownJar() {
        try {
            Path p = Paths.get(SelfTest.class.getProtectionDomain().getCodeSource().getLocation().toURI());
            return Files.isRegularFile(p) ? p : null;
        } catch (Throwable e) {
            return null;
        }
    }

    // ---- small helpers

    private static List<String> listing(Path dir) {
        List<String> l = new ArrayList<String>();
        try {
            DirectoryStream<Path> ds = Files.newDirectoryStream(dir);
            try {
                for (Path p : ds) {
                    l.add(p.getFileName().toString());
                }
            } finally {
                ds.close();
            }
        } catch (IOException e) {
            l.add("(" + e + ")");
        }
        Collections.sort(l);
        return l;
    }

    private static String join(List<String> l) {
        StringBuilder sb = new StringBuilder();
        for (String s : l) {
            sb.append(s).append('\n');
        }
        return sb.toString();
    }

    private static String repeat(char c, int n) {
        char[] a = new char[n];
        Arrays.fill(a, c);
        return new String(a);
    }

    private static void delete(Path root) {
        try {
            Files.walkFileTree(root, new SimpleFileVisitor<Path>() {
                @Override
                public FileVisitResult visitFile(Path file, BasicFileAttributes attrs) throws IOException {
                    Files.deleteIfExists(file);
                    return FileVisitResult.CONTINUE;
                }

                @Override
                public FileVisitResult postVisitDirectory(Path dir, IOException e) throws IOException {
                    dir.toFile().setWritable(true, false);
                    Files.deleteIfExists(dir);
                    return FileVisitResult.CONTINUE;
                }
            });
        } catch (Throwable ignored) {
            // a temporary folder; the system clears it eventually
        }
    }

    private static void check(String what, boolean ok, String detail) {
        if (ok) {
            pass(what);
        } else {
            fail(what, detail);
        }
    }

    private static void pass(String what) {
        passed++;
        System.out.println("PASS " + what);
    }

    private static void fail(String what, String detail) {
        failed++;
        String d = detail == null ? "" : detail.trim();
        System.out.println("FAIL " + what + (d.isEmpty() ? "" : "\n     " + d.replace("\n", "\n     ")));
    }

    private static void skip(String what, String why) {
        skipped++;
        System.out.println("SKIP " + what + ": " + why);
    }
}

/** The pattern for the made-up game classes: its static initialiser reports that it ran. */
final class SelfTestTemplate {
    static {
        SelfTest.initialised();
    }

    private SelfTestTemplate() {}
}
