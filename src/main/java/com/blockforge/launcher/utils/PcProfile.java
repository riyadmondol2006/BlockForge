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
package com.blockforge.launcher.utils;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.Reader;
import java.io.Writer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.time.Duration;
import java.time.Instant;
import java.util.List;
import java.util.Locale;

import com.blockforge.launcher.FileSystem;
import com.blockforge.launcher.Gsons;
import com.blockforge.launcher.managers.LogManager;

/**
 * What kind of computer this is, for the choices BlockForge makes on its own: how much memory the game gets, how
 * many files download at once and which graphics settings a new game starts with. All of it is decided here so the
 * choices agree with each other.
 *
 * <p>
 * Finding the graphics card is slow (oshi starts native code and asks the system, often 100 ms or more), so it is
 * never done while the launcher starts: only when something first needs it, off the Swing thread, and the answer is
 * kept in the cache folder so it isn't asked again until the memory or processor count changes or a month passes.
 * Memory and core counts come from Java itself, which is instant.
 */
public final class PcProfile {
    /** How capable the computer is for Minecraft. */
    public enum Tier {
        LOW, MEDIUM, HIGH
    }

    /** What kind of graphics the computer has. */
    public enum Gpu {
        INTEGRATED, DEDICATED, UNKNOWN
    }

    /** the graphics card is looked up again after this long, in case it changed */
    static final Duration REFRESH_AFTER = Duration.ofDays(30);

    public final long totalMb;
    public final int cores;
    public final Gpu gpu;
    public final String gpuName;
    public final Tier tier;

    PcProfile(long totalMb, int cores, Gpu gpu, String gpuName) {
        this.totalMb = totalMb;
        this.cores = cores;
        this.gpu = gpu;
        this.gpuName = gpuName;
        this.tier = tierFor(totalMb, cores, gpu);
    }

    // ---- pure decisions ------------------------------------------------------------------------------------------

    /**
     * Sorts a graphics card into integrated or dedicated from its vendor and name, as the system reports them.
     * Integrated: Intel's UHD, HD, Iris and Arc-less mobile parts, AMD's APUs (which call themselves just "Radeon
     * Graphics" or "Radeon Vega"), Apple's M chips and the virtual adapters of virtual machines. Dedicated: NVIDIA,
     * AMD Radeon RX/Pro and Intel Arc.
     */
    public static Gpu classifyGpu(String vendor, String name) {
        String v = vendor == null ? "" : vendor.toLowerCase(Locale.ROOT);
        String n = name == null ? "" : name.toLowerCase(Locale.ROOT);
        String all = v + " " + n;

        if (all.trim().isEmpty()) {
            return Gpu.UNKNOWN;
        }

        // virtual machines and remote desktops: software rendering or a thin layer over it
        if (all.contains("vmware") || all.contains("virtualbox") || all.contains("llvmpipe")
            || all.contains("microsoft basic") || all.contains("virtio") || all.contains("qxl")
            || all.contains("cirrus") || all.contains("hyper-v") || all.contains("parallels")) {
            return Gpu.INTEGRATED;
        }

        if (all.contains("nvidia") || all.contains("geforce") || all.contains("quadro") || all.contains("rtx")
            || all.contains("gtx")) {
            return Gpu.DEDICATED;
        }

        if (all.contains("intel")) {
            // Intel Arc A/B series cards are dedicated; "Arc Graphics" with no model number is the integrated GPU
            // of Core Ultra processors
            if (n.matches(".*\\barc\\b.*\\b[ab]\\d{3}.*")) {
                return Gpu.DEDICATED;
            }
            return Gpu.INTEGRATED;
        }

        if (all.contains("apple") || n.matches(".*\\bm\\d+\\b.*")) {
            return Gpu.INTEGRATED;
        }

        if (all.contains("amd") || all.contains("ati ") || all.contains("advanced micro devices")
            || all.contains("radeon")) {
            // the APUs report "AMD Radeon(TM) Graphics", "Radeon Vega 8 Graphics" or a 6xxM/7xxM/8xxM model
            if (n.contains("radeon(tm) graphics") || n.equals("amd radeon graphics") || n.contains("radeon graphics")
                || n.contains("vega") && !n.contains("vega 56") && !n.contains("vega 64")
                || n.matches(".*\\b(6[0-9]0|7[0-9]0|8[0-9]0)m\\b.*") || n.contains("renoir")
                || n.contains("cezanne") || n.contains("rembrandt") || n.contains("phoenix") || n.contains("raphael")
                || n.contains("barcelo") || n.contains("lucienne") || n.contains("picasso") || n.contains("raven")
                || n.contains("van gogh") || n.contains("mendocino") || n.contains("strix")) {
                return Gpu.INTEGRATED;
            }
            return Gpu.DEDICATED;
        }

        return Gpu.UNKNOWN;
    }

    /** Picks the dedicated card if there is one, as that's what games run on. */
    public static Gpu bestOf(List<Gpu> cards) {
        if (cards == null || cards.isEmpty()) {
            return Gpu.UNKNOWN;
        }
        if (cards.contains(Gpu.DEDICATED)) {
            return Gpu.DEDICATED;
        }
        if (cards.contains(Gpu.INTEGRATED)) {
            return Gpu.INTEGRATED;
        }
        return Gpu.UNKNOWN;
    }

    /**
     * How capable the computer is.
     * <ul>
     * <li>LOW: 4 GB of memory or less, two cores or fewer, or integrated graphics with 8 GB or less and four cores or
     * fewer (a typical budget laptop). Minecraft's defaults (12 chunks, fancy) run poorly on all of these.</li>
     * <li>HIGH: a dedicated graphics card with 16 GB or more and at least six cores.</li>
     * <li>MEDIUM: everything in between, including integrated graphics on otherwise good computers and any computer
     * whose graphics couldn't be identified.</li>
     * </ul>
     */
    public static Tier tierFor(long totalMb, int cores, Gpu gpu) {
        boolean lowMemory = totalMb > 0 && totalMb <= 4608;
        boolean fewCores = cores > 0 && cores <= 2;
        if (lowMemory || fewCores) {
            return Tier.LOW;
        }
        if (gpu == Gpu.INTEGRATED && totalMb > 0 && totalMb <= 8704 && cores <= 4) {
            return Tier.LOW;
        }
        if (gpu == Gpu.DEDICATED && totalMb >= 15360 && cores >= 6) {
            return Tier.HIGH;
        }
        return Tier.MEDIUM;
    }

    /**
     * How many files to download at once by default. Downloads are many small files, so the number in flight
     * matters more than bandwidth and 16 roughly halves a first install compared to 8. But each finished file is
     * also hashed and written, and on a computer with two cores or 4 GB of memory 16 at once only makes the launcher
     * stutter, so those get 8.
     */
    public static int downloadThreads(long totalMb, int cores) {
        if ((totalMb > 0 && totalMb <= 4608) || (cores > 0 && cores <= 2)) {
            return 8;
        }
        return 16;
    }

    /**
     * How many downloads to run at once: what the player set, or when the setting was never changed from the
     * default, what suits this computer.
     */
    public static int downloadThreads(int configured, int defaultConfigured, long totalMb, int cores) {
        return com.blockforge.launcher.network.DownloadPool.forComputer(configured, defaultConfigured, totalMb, cores);
    }

    // ---- this computer -------------------------------------------------------------------------------------------

    private static volatile PcProfile current;

    /**
     * This computer's profile. May look up the graphics card the first time (slow), so call it off the Swing
     * thread; {@link #peek()} never blocks.
     */
    public static PcProfile get() {
        PcProfile profile = current;
        if (profile == null) {
            synchronized (PcProfile.class) {
                profile = current;
                if (profile == null) {
                    profile = load();
                    current = profile;
                }
            }
        }
        return profile;
    }

    /** The profile if it has been worked out already, or one from memory and cores alone (graphics unknown). */
    public static PcProfile peek() {
        PcProfile profile = current;
        return profile != null ? profile
            : new PcProfile(LightMode.totalRamMb(), Runtime.getRuntime().availableProcessors(), Gpu.UNKNOWN, null);
    }

    /** Works the profile out in the background, if that hasn't been done, and then runs the callback with it. */
    public static void whenReady(java.util.function.Consumer<PcProfile> callback) {
        PcProfile profile = current;
        if (profile != null) {
            callback.accept(profile);
            return;
        }
        Thread t = new Thread(() -> callback.accept(get()), "pc-profile");
        t.setDaemon(true);
        t.setPriority(Thread.MIN_PRIORITY);
        t.start();
    }

    /** What is kept in the cache folder. */
    static final class Saved {
        long totalMb;
        int cores;
        String gpu;
        String gpuName;
        String checkedAt;
    }

    static Path cacheFile() {
        return FileSystem.CACHE.resolve("pc-profile.json");
    }

    private static PcProfile load() {
        long totalMb = LightMode.totalRamMb();
        int cores = Runtime.getRuntime().availableProcessors();

        Saved saved = readSaved(cacheFile());
        if (saved != null && isStillValid(saved, totalMb, cores, Instant.now())) {
            return new PcProfile(totalMb, cores, parseGpu(saved.gpu), saved.gpuName);
        }

        Gpu gpu = Gpu.UNKNOWN;
        String gpuName = null;
        long started = System.nanoTime();
        try {
            List<oshi.hardware.GraphicsCard> cards = OS.getSystemInfo().getHardware().getGraphicsCards();
            java.util.List<Gpu> kinds = new java.util.ArrayList<>();
            for (oshi.hardware.GraphicsCard card : cards) {
                Gpu kind = classifyGpu(card.getVendor(), card.getName());
                kinds.add(kind);
                if (gpuName == null || kind == Gpu.DEDICATED) {
                    gpuName = card.getName();
                }
            }
            gpu = bestOf(kinds);
        } catch (Throwable t) {
            LogManager.debug("Couldn't find the graphics card: " + t);
        }
        LogManager.info(String.format(Locale.ENGLISH, "This computer: %d MB, %d cores, %s graphics (%s), %s tier "
            + "(looked up in %d ms)", totalMb, cores, gpu.name().toLowerCase(Locale.ROOT), gpuName,
            tierFor(totalMb, cores, gpu).name().toLowerCase(Locale.ROOT), (System.nanoTime() - started) / 1_000_000));

        Saved fresh = new Saved();
        fresh.totalMb = totalMb;
        fresh.cores = cores;
        fresh.gpu = gpu.name();
        fresh.gpuName = gpuName;
        fresh.checkedAt = Instant.now().toString();
        writeSaved(cacheFile(), fresh);

        return new PcProfile(totalMb, cores, gpu, gpuName);
    }

    /** if what was saved still describes this computer */
    static boolean isStillValid(Saved saved, long totalMb, int cores, Instant now) {
        if (saved.gpu == null || saved.checkedAt == null) {
            return false;
        }
        // a memory upgrade or a different processor: look again (allow a little wobble in the reported memory)
        if (Math.abs(saved.totalMb - totalMb) > 256 || saved.cores != cores) {
            return false;
        }
        try {
            return Duration.between(Instant.parse(saved.checkedAt), now).compareTo(REFRESH_AFTER) < 0;
        } catch (RuntimeException e) {
            return false;
        }
    }

    static Gpu parseGpu(String gpu) {
        try {
            return Gpu.valueOf(gpu);
        } catch (RuntimeException e) {
            return Gpu.UNKNOWN;
        }
    }

    static Saved readSaved(Path file) {
        if (!Files.isRegularFile(file)) {
            return null;
        }
        try (Reader reader = Files.newBufferedReader(file, StandardCharsets.UTF_8)) {
            return Gsons.DEFAULT_SLIM.fromJson(new BufferedReader(reader), Saved.class);
        } catch (Exception e) {
            return null;
        }
    }

    static void writeSaved(Path file, Saved saved) {
        try {
            Files.createDirectories(file.getParent());
            try (Writer writer = Files.newBufferedWriter(file, StandardCharsets.UTF_8)) {
                Gsons.DEFAULT_SLIM.toJson(saved, writer);
            }
        } catch (IOException e) {
            // only a cache
        }
    }

    /**
     * Memory free for new programs right now, in MB, or -1 if unknown. Counts memory the system would give up from
     * its file cache, which is what matters for starting a game.
     */
    public static long availableMb() {
        // Linux says it directly; reading one small file is quicker than anything else
        Path meminfo = Paths.get("/proc/meminfo");
        if (Files.isReadable(meminfo)) {
            try (BufferedReader reader = Files.newBufferedReader(meminfo, StandardCharsets.US_ASCII)) {
                String line;
                while ((line = reader.readLine()) != null) {
                    if (line.startsWith("MemAvailable:")) {
                        String[] parts = line.trim().split("\\s+");
                        return Long.parseLong(parts[1]) / 1024;
                    }
                }
            } catch (Exception e) {
                // fall through
            }
        }
        try {
            return OS.getSystemInfo().getHardware().getMemory().getAvailable() / (1024 * 1024);
        } catch (Throwable t) {
            return -1;
        }
    }
}
