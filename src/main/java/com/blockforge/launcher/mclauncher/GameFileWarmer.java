/*
 * BlockForge - a fork of ATLauncher (https://github.com/ATLauncher/ATLauncher)
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program. If not, see <http://www.gnu.org/licenses/>.
 */
package com.blockforge.launcher.mclauncher;

import java.io.IOException;
import java.io.Reader;
import java.nio.ByteBuffer;
import java.nio.channels.FileChannel;
import java.nio.charset.StandardCharsets;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.function.BooleanSupplier;

import com.blockforge.launcher.App;
import com.blockforge.launcher.FileSystem;
import com.blockforge.launcher.Gsons;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.data.minecraft.AssetIndex;
import com.blockforge.launcher.data.minecraft.AssetObject;
import com.blockforge.launcher.data.minecraft.Library;
import com.blockforge.launcher.managers.InstanceManager;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.network.DownloadPool;
import com.blockforge.launcher.utils.LightMode;
import com.blockforge.launcher.utils.PcProfile;
import com.blockforge.launcher.workers.InstanceInstaller;

/**
 * Reads a game's files once while the launcher sits idle, so they are in the system's file cache when Play is
 * pressed. A game's start is mostly reading: the Java runtime, the libraries, the game jar and the mods, a few hundred
 * MB that a hard drive or a slow SSD needs seconds for after a reboot.
 *
 * <p>
 * It stays out of the way: one thread at the lowest priority, reading one file after another, a limit on how much it
 * reads, nothing at all while a game runs, an install runs or files download, and it stops the moment Play is
 * pressed. The files are only read, never changed.
 */
public final class GameFileWarmer {
    /** the same instance isn't read again within this time; the system keeps its cache for much longer */
    static final long REWARM_AFTER_MS = 10 * 60 * 1000L;
    private static final int CHUNK = 1024 * 1024;

    /** bumped to abandon whatever is being read */
    private static final AtomicInteger GENERATION = new AtomicInteger();
    private static final Map<Path, Long> WARMED_AT = new ConcurrentHashMap<>();
    private static volatile ExecutorService executor;

    private GameFileWarmer() {
    }

    // ---- when -----------------------------------------------------------------------------------------------------

    /** Called a few seconds after the launcher's window shows: reads the most recently played game(s). */
    public static void launcherIdle() {
        if (!enabled()) {
            return;
        }
        List<Instance> instances = new ArrayList<>(InstanceManager.getInstances());
        instances.sort(Comparator.comparing(Instance::getLastPlayedOrEpoch).reversed());
        int count = instancesToWarm(LightMode.isOn(), LightMode.totalRamMb());
        for (int i = 0; i < Math.min(count, instances.size()); i++) {
            if (instances.get(i).getLastPlayedOrEpoch().toEpochMilli() > 0) {
                warmSoon(instances.get(i));
            }
        }
    }

    /** The player is pointing at or opening an instance: read its files if that hasn't been done lately. */
    public static void warmSoon(Instance instance) {
        if (instance == null || !enabled() || busy()) {
            return;
        }
        // on a small computer only the game played last is kept warm, as the cache is small too
        if (smallComputer()) {
            Instance latest = InstanceManager.getInstances().stream()
                .max(Comparator.comparing(Instance::getLastPlayedOrEpoch)).orElse(null);
            if (latest != instance) {
                return;
            }
        }
        Path root = instance.getRoot();
        Long last = WARMED_AT.get(root);
        long now = System.currentTimeMillis();
        if (last != null && now - last < REWARM_AFTER_MS) {
            return;
        }
        WARMED_AT.put(root, now);

        int generation = GENERATION.get();
        executor().execute(() -> {
            if (GENERATION.get() != generation) {
                return;
            }
            try {
                long read = warm(instance, () -> GENERATION.get() != generation || busy());
                LogManager.debug(String.format(Locale.ENGLISH, "Read %d MB of %s's files ahead of Play",
                    read / (1024 * 1024), instance.getName()));
            } catch (Throwable t) {
                LogManager.debug("Reading game files ahead failed: " + t);
            }
        });
    }

    /** Play was pressed: stop reading now, the game itself is about to. */
    public static void stopForLaunch() {
        GENERATION.incrementAndGet();
    }

    /** For measuring: reads an instance's files right away and returns how many bytes were read. */
    public static long warmNowForTesting(Instance instance) throws IOException {
        return warm(instance, () -> false);
    }

    static boolean enabled() {
        return App.settings != null && App.settings.warmUpGameFiles;
    }

    /** a game, an install or downloads are running: the disk is theirs */
    static boolean busy() {
        return (App.launcher != null && App.launcher.minecraftLaunched) || DownloadPool.anyActive()
            || InstanceInstaller.anyRunning();
    }

    static boolean smallComputer() {
        return LightMode.isOn() || (LightMode.totalRamMb() > 0 && LightMode.totalRamMb() <= 4096 + 512);
    }

    /** how many recently played games to read when the launcher has started */
    static int instancesToWarm(boolean lightMode, long totalMb) {
        if (lightMode || (totalMb > 0 && totalMb <= 4608)) {
            return 1;
        }
        return totalMb >= 15360 ? 2 : 1;
    }

    /**
     * How much to read at most: a quarter of the memory free right now (the cache shouldn't push out what other
     * programs use), at most 1 GB, or 256 MB on a small computer. 0 when there is too little free to bother.
     */
    static long budgetBytes(long availableMb, boolean small) {
        if (availableMb > 0 && availableMb < 1024) {
            return 0;
        }
        long cap = small ? 256 : 1024;
        long mb = availableMb > 0 ? Math.min(cap, availableMb / 4) : cap / 2;
        return mb * 1024 * 1024;
    }

    private static ExecutorService executor() {
        ExecutorService e = executor;
        if (e == null) {
            synchronized (GameFileWarmer.class) {
                e = executor;
                if (e == null) {
                    // one thread, gone when idle, and at most a few waiting jobs (older hovers are dropped)
                    ThreadPoolExecutor pool = new ThreadPoolExecutor(1, 1, 30, TimeUnit.SECONDS,
                        new LinkedBlockingQueue<>(4), r -> {
                            Thread t = new Thread(r, "game-file-warmer");
                            t.setDaemon(true);
                            t.setPriority(Thread.MIN_PRIORITY);
                            return t;
                        }, new ThreadPoolExecutor.DiscardOldestPolicy());
                    pool.allowCoreThreadTimeOut(true);
                    executor = e = pool;
                }
            }
        }
        return e;
    }

    // ---- what -----------------------------------------------------------------------------------------------------

    /**
     * The files a start of this game reads, roughly in the order it reads them: the Java runtime, its class cache,
     * the libraries, the game jar, the mods and the asset index with the few assets read at start.
     */
    static List<Path> filesFor(Instance instance) {
        Set<Path> files = new LinkedHashSet<>();

        String javaPath = instance.getJavaPathQuietly();
        if (javaPath != null) {
            Path java = Paths.get(javaPath);
            files.add(java.resolve("lib/modules"));
            files.add(java.resolve("lib/server/libjvm.so"));
            files.add(java.resolve("lib/server/classes.jsa"));
            files.add(java.resolve("bin/server/jvm.dll"));
            files.add(java.resolve("lib/server/libjvm.dylib"));
            int major = javaMajor(java);
            if (major > 0) {
                files.add(ClassDataCache.archiveFor(instance.getSafeName(), major));
            }
        }

        if (instance.libraries != null) {
            for (Library library : instance.libraries) {
                try {
                    if (!library.shouldInstall()) {
                        continue;
                    }
                    if (library.hasNativeForOS()) {
                        files.add(FileSystem.LIBRARIES.resolve(library.getNativeDownloadForOS().path));
                    } else if (library.downloads != null && library.downloads.artifact != null
                        && library.downloads.artifact.path != null) {
                        files.add(FileSystem.LIBRARIES.resolve(library.downloads.artifact.path));
                    }
                } catch (RuntimeException e) {
                    // an odd library entry; skip it
                }
            }
        }

        files.add(instance.getMinecraftJar().toPath());

        Path mods = instance.getRoot().resolve("mods");
        if (Files.isDirectory(mods)) {
            List<Path> modFiles = new ArrayList<>();
            try (DirectoryStream<Path> stream = Files.newDirectoryStream(mods, "*.jar")) {
                stream.forEach(modFiles::add);
            } catch (IOException e) {
                // what was found
            }
            modFiles.sort(null);
            files.addAll(modFiles);
        }

        files.addAll(assetsReadAtStart(instance));
        return new ArrayList<>(files);
    }

    private static int javaMajor(Path java) {
        try {
            return com.blockforge.launcher.utils.Java
                .parseJavaVersionNumber(com.blockforge.launcher.utils.Java.getVersionForJavaPath(java.toFile()));
        } catch (RuntimeException e) {
            return -1;
        }
    }

    /** The asset index and the assets every start reads: the sound list and the language files of the language. */
    static List<Path> assetsReadAtStart(Instance instance) {
        List<Path> files = new ArrayList<>();
        if (instance.assetIndex == null || instance.assetIndex.id == null) {
            return files;
        }
        Path index = FileSystem.RESOURCES_INDEXES.resolve(instance.assetIndex.id + ".json");
        if (!Files.isRegularFile(index)) {
            return files;
        }
        files.add(index);
        try (Reader reader = Files.newBufferedReader(index, StandardCharsets.UTF_8)) {
            AssetIndex assets = Gsons.DEFAULT_SLIM.fromJson(reader, AssetIndex.class);
            if (assets != null && assets.objects != null) {
                String language = languageOf(instance.getRoot());
                for (Map.Entry<String, AssetObject> e : assets.objects.entrySet()) {
                    String key = e.getKey();
                    if (key.equals("minecraft/sounds.json") || key.equals("pack.mcmeta")
                        || key.equals("minecraft/lang/" + language + ".json")
                        || key.startsWith("minecraft/sounds/ui/")) {
                        String hash = e.getValue().hash;
                        files.add(FileSystem.RESOURCES_OBJECTS.resolve(hash.substring(0, 2)).resolve(hash));
                    }
                }
            }
        } catch (Exception e) {
            // only a nicety
        }
        return files;
    }

    /** the game's language from options.txt, "en_us" when not set */
    static String languageOf(Path gameDirectory) {
        Path options = gameDirectory.resolve("options.txt");
        if (Files.isRegularFile(options)) {
            try {
                for (String line : Files.readAllLines(options, StandardCharsets.UTF_8)) {
                    if (line.startsWith("lang:")) {
                        return line.substring(5).trim().toLowerCase(Locale.ROOT);
                    }
                }
            } catch (IOException | RuntimeException e) {
                // default
            }
        }
        return "en_us";
    }

    // ---- how ------------------------------------------------------------------------------------------------------

    private static long warm(Instance instance, BooleanSupplier stop) throws IOException {
        long budget = budgetBytes(PcProfile.availableMb(), smallComputer());
        return readAll(filesFor(instance), budget, stop);
    }

    /**
     * Reads the files one after another, up to the budget, stopping as soon as it's told to.
     *
     * @return the bytes read
     */
    static long readAll(List<Path> files, long budget, BooleanSupplier stop) {
        long read = 0;
        ByteBuffer buffer = ByteBuffer.allocateDirect(CHUNK);
        for (Path file : files) {
            if (read >= budget || stop.getAsBoolean()) {
                break;
            }
            if (!Files.isRegularFile(file)) {
                continue;
            }
            try (FileChannel channel = FileChannel.open(file, StandardOpenOption.READ)) {
                while (read < budget) {
                    buffer.clear();
                    int n = channel.read(buffer);
                    if (n < 0) {
                        break;
                    }
                    read += n;
                    if (stop.getAsBoolean()) {
                        return read;
                    }
                }
            } catch (IOException e) {
                // a file in use or gone; the next one
            }
        }
        return read;
    }
}
