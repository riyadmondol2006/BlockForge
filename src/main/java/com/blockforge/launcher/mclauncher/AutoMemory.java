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

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Locale;

/**
 * Picks the game's memory (-Xmx) from what the game needs and what this computer can spare. Used whenever the player
 * hasn't set the memory themselves. Pure: every input is passed in, so it is easy to test and to reason about.
 *
 * <p>
 * The idea: work out what this game wants (Minecraft version, loader, number of mods, shaders, big resource packs,
 * what the modpack asks for), then make sure that never takes memory the rest of the computer needs. More heap than
 * the game uses doesn't make it faster: the garbage collector has more to walk through, pauses get longer, and memory
 * taken from the system pushes the graphics driver, browser and the system itself into swap, which is far slower.
 */
public final class AutoMemory {
    /** 32-bit Java can rarely reserve more than about 1.4 GB in one piece, and fails to start if asked for more */
    public static final int JAVA_32_BIT_CAP_MB = 1408;
    /** below this Minecraft doesn't get to the title screen */
    public static final int FLOOR_MB = 512;
    /**
     * The most the launcher gives on its own (unless the modpack asks for more). Even the biggest packs settle well
     * under this; past 8 GB, G1 has more regions to scan and its pauses get longer, and the well known guides for big
     * packs say the same: more than 8-10 GB makes stutters worse, not better.
     */
    public static final int SOFT_CAP_MB = 8192;
    /** with one or two cores the collector has little help, so pauses grow with the heap; keep it smaller there */
    public static final int FEW_CORES_CAP_MB = 4096;
    /** what the system always keeps for itself, at the least */
    public static final int MIN_OS_RESERVE_MB = 1536;
    /** the most kept back for the system on computers with lots of memory */
    public static final int MAX_OS_RESERVE_MB = 4096;
    /**
     * Memory the game's Java uses besides the heap: loaded classes (metaspace), compiled code, threads, the graphics
     * driver's buffers and native libraries. Typically 400-800 MB for Minecraft.
     */
    public static final int NON_HEAP_MB = 512;

    private AutoMemory() {
    }

    /** The mod loader, as far as memory is concerned. */
    public enum Loader {
        NONE, FABRIC, QUILT, FORGE, NEOFORGE, OTHER;

        /** from the loader type stored with an instance ("Fabric", "NeoForge", "LegacyFabric", ...); null is none */
        public static Loader of(String type) {
            if (type == null || type.isEmpty()) {
                return NONE;
            }
            switch (type.toLowerCase(Locale.ROOT)) {
                case "fabric":
                case "legacyfabric":
                    return FABRIC;
                case "quilt":
                    return QUILT;
                case "forge":
                    return FORGE;
                case "neoforge":
                    return NEOFORGE;
                default:
                    return OTHER;
            }
        }

        boolean fabricLike() {
            return this == FABRIC || this == QUILT;
        }

        boolean forgeLike() {
            return this == FORGE || this == NEOFORGE || this == OTHER;
        }
    }

    /** The computer, at the moment Play is pressed. Unknown numbers are 0 or less. */
    public static final class Pc {
        public final long totalMb;
        public final long availableMb;
        public final int cores;
        public final boolean java32Bit;

        public Pc(long totalMb, long availableMb, int cores, boolean java32Bit) {
            this.totalMb = totalMb;
            this.availableMb = availableMb;
            this.cores = cores;
            this.java32Bit = java32Bit;
        }
    }

    /** The game being started. */
    public static final class Game {
        public final String minecraftVersion;
        public final Loader loader;
        public final int modCount;
        public final boolean shaders;
        public final long resourcePacksMb;
        public final int packRecommendedMb;
        public final int packMinimumMb;

        public Game(String minecraftVersion, Loader loader, int modCount, boolean shaders, long resourcePacksMb,
            int packRecommendedMb, int packMinimumMb) {
            this.minecraftVersion = minecraftVersion;
            this.loader = loader == null ? Loader.NONE : loader;
            this.modCount = Math.max(0, modCount);
            this.shaders = shaders;
            this.resourcePacksMb = Math.max(0, resourcePacksMb);
            this.packRecommendedMb = Math.max(0, packRecommendedMb);
            this.packMinimumMb = Math.max(0, packMinimumMb);
        }

        /** a plain game of this version: no mods, no packs */
        public static Game vanilla(String minecraftVersion) {
            return new Game(minecraftVersion, Loader.NONE, 0, false, 0, 0, 0);
        }
    }

    /** What was picked, and why it may be less than the game would like. */
    public static final class Result {
        /** the -Xmx to use */
        public final int heapMb;
        /** what the game would like, before looking at the computer */
        public final int idealMb;
        /** the most this computer can spare at all */
        public final int ceilingMb;
        /** lowered because little memory was free when Play was pressed */
        public final boolean limitedByFreeMemory;
        /** less than the modpack says it needs */
        public final boolean belowPackNeeds;
        public final List<String> warnings;

        Result(int heapMb, int idealMb, int ceilingMb, boolean limitedByFreeMemory, boolean belowPackNeeds,
            List<String> warnings) {
            this.heapMb = heapMb;
            this.idealMb = idealMb;
            this.ceilingMb = ceilingMb;
            this.limitedByFreeMemory = limitedByFreeMemory;
            this.belowPackNeeds = belowPackNeeds;
            this.warnings = Collections.unmodifiableList(warnings);
        }

        /** if the game gets noticeably less than it would like, so the player should be told */
        public boolean isShort() {
            return belowPackNeeds || heapMb < idealMb - 256;
        }
    }

    /**
     * What the game would like, before looking at what the computer can spare. Rounded to 256 MB.
     */
    public static int idealMb(Game game) {
        int minor = minorVersion(game.minecraftVersion);

        // the base game:
        // - up to 1.6: tiny worlds (128 high) and few textures; 1 GB is plenty
        // - 1.7 to 1.12: still 256-high worlds; 1 GB runs, 1.5 GB leaves room for render distances over 8
        // - 1.13 to 1.17: the "flattening" and new lighting engine roughly doubled what chunks take; Mojang's own
        // launcher gives 2 GB
        // - 1.18 and newer: worlds are 384 high (50% more chunk data) and the default view is 12 chunks plus a
        // simulation distance; 2 GB means frequent collections there, 2.5 GB doesn't
        int mb;
        if (minor <= 6) {
            mb = 1024;
        } else if (minor <= 12) {
            mb = 1536;
        } else if (minor <= 17) {
            mb = 2048;
        } else {
            mb = 2560;
        }

        // mod loaders: Fabric and Quilt load mods lazily and their mods are small, so a light performance list stays
        // around 2.5-3 GB; Forge and NeoForge load every mod's classes and registries up front, so big packs need
        // 4-8 GB, growing with the number of mods
        if (game.loader.fabricLike()) {
            mb += 256 + game.modCount * 12;
        } else if (game.loader.forgeLike()) {
            mb += 768 + game.modCount * (minor <= 12 ? 16 : 20);
        }

        // shader packs: the shader loader (Iris, Oculus, OptiFine) and the pack's own textures and buffers
        if (game.shaders) {
            mb += 512;
        }

        // high resolution resource packs: textures are unpacked into atlases at 4 bytes a pixel, so a 128x pack
        // (often 100 MB or more zipped) takes several times its size while loading
        if (game.resourcePacksMb >= 256) {
            mb += 1024;
        } else if (game.resourcePacksMb >= 64) {
            mb += 512;
        }

        mb = Math.min(mb, SOFT_CAP_MB);

        // the modpack knows itself best; what it asks for is used even past the usual cap
        mb = Math.max(mb, Math.max(game.packRecommendedMb, game.packMinimumMb));

        return roundUp(mb);
    }

    /**
     * The least the game should get when memory is tight: about three quarters of what it would like (that is
     * mostly headroom for the collector), never under what the modpack says it needs.
     */
    static int neededMb(Game game, int idealMb) {
        int minor = minorVersion(game.minecraftVersion);
        int base = minor <= 12 ? 768 : 1024;
        return Math.max(game.packMinimumMb, Math.max(base, roundDown(idealMb * 3 / 4)));
    }

    /**
     * The most the computer can spare for the game at all: everything but what the system needs, which is a quarter
     * of the memory, at least 1.5 GB (the system, the graphics driver and a browser) and at most 4 GB.
     */
    public static int ceilingMb(Pc pc) {
        int cap = Integer.MAX_VALUE;
        if (pc.totalMb > 0) {
            long reserve = Math.max(MIN_OS_RESERVE_MB, Math.min(MAX_OS_RESERVE_MB, pc.totalMb / 4));
            cap = (int) Math.max(FLOOR_MB, roundDown((int) Math.min(Integer.MAX_VALUE, pc.totalMb - reserve)));
        }
        if (pc.java32Bit) {
            cap = Math.min(cap, JAVA_32_BIT_CAP_MB);
        }
        return cap;
    }

    /** Picks the memory for this game on this computer. */
    public static Result choose(Pc pc, Game game) {
        List<String> warnings = new ArrayList<>();
        int ideal = idealMb(game);
        int ceiling = ceilingMb(pc);

        int heap = Math.min(ideal, ceiling);

        // one or two cores: only when the pack doesn't need more
        if (pc.cores > 0 && pc.cores <= 2 && heap > FEW_CORES_CAP_MB && game.packMinimumMb <= FEW_CORES_CAP_MB) {
            heap = FEW_CORES_CAP_MB;
        }

        // memory not known at all: stay modest
        if (pc.totalMb <= 0) {
            heap = Math.min(heap, Math.max(GameMemory.UNKNOWN_DEFAULT_MB, game.packMinimumMb));
        }

        // little free right now: give less, but not less than the game needs; swapping a little is better than
        // running out of memory, which crashes the game
        boolean limitedByFree = false;
        int needed = Math.min(neededMb(game, ideal), heap);
        if (pc.availableMb > 0) {
            int spare = roundDown((int) Math.min(Integer.MAX_VALUE, pc.availableMb - NON_HEAP_MB));
            if (spare < heap) {
                int lowered = Math.max(needed, spare);
                if (lowered < heap) {
                    heap = lowered;
                    limitedByFree = true;
                }
            }
        }

        // never under what the game needs to reach the title screen at all, even on a tiny computer: there it will
        // swap, but with less it would just run out of memory and crash
        int startMinimum = AutoMemory.minorVersion(game.minecraftVersion) <= 12 ? 768 : 1024;
        heap = Math.max(heap, Math.min(startMinimum, pc.java32Bit ? JAVA_32_BIT_CAP_MB : Integer.MAX_VALUE));
        heap = Math.max(FLOOR_MB, heap);

        if (limitedByFree) {
            warnings.add(String.format(Locale.ENGLISH,
                "Only %d MB of memory is free right now, so the game gets %d MB instead of the %d MB it should have. "
                    + "Closing other programs (web browsers especially) lets it have more next time.",
                pc.availableMb, heap, ideal));
        }

        boolean belowPack = game.packMinimumMb > 0 && heap < game.packMinimumMb;
        if (belowPack) {
            warnings.add(String.format(Locale.ENGLISH,
                "This modpack needs at least %d MB of memory, but this computer can only spare %d MB. It may run "
                    + "slowly or run out of memory.",
                game.packMinimumMb, heap));
        } else if (!limitedByFree && heap < ideal - 256) {
            warnings.add(String.format(Locale.ENGLISH,
                "This game would like about %d MB of memory, but this computer can only spare %d MB. It may run "
                    + "slowly; fewer mods or a lower render distance will help.",
                ideal, heap));
        }

        return new Result(heap, ideal, ceiling, limitedByFree, belowPack, warnings);
    }

    /**
     * The "1.x" part of a Minecraft version, or 100 for the year based versions (26.1 and newer), so they count as
     * newer than any 1.x. Weekly snapshots ("24w14a") are mapped to the release they led to; very old versions
     * (alpha, beta, classic) are 0.
     */
    public static int minorVersion(String version) {
        if (version == null) {
            return 100;
        }
        String v = version.trim().toLowerCase(Locale.ROOT);
        try {
            if (v.startsWith("1.")) {
                int end = 2;
                while (end < v.length() && Character.isDigit(v.charAt(end))) {
                    end++;
                }
                return end == 2 ? 0 : Integer.parseInt(v.substring(2, end));
            }
            if (v.matches("\\d\\dw\\d\\d.*")) {
                int year = Integer.parseInt(v.substring(0, 2));
                int[] byYear = { 10, 10, 10, 10, 10, 10, 12, 13, 15, 16, 17, 19, 20, 21, 21 }; // 2011 .. 2025
                int index = year - 11;
                if (index < 0) {
                    return 0;
                }
                return index < byYear.length ? byYear[index] : 100;
            }
            int dot = v.indexOf('.');
            int dash = v.indexOf('-');
            int end = dot > 0 ? dot : (dash > 0 ? dash : v.length());
            if (end > 0 && v.substring(0, end).matches("\\d+")) {
                int major = Integer.parseInt(v.substring(0, end));
                return major >= 25 ? 100 : 0;
            }
        } catch (NumberFormatException e) {
            // fall through
        }
        // alpha, beta, classic, infdev and other odd names
        return v.startsWith("a") || v.startsWith("b") || v.startsWith("c") || v.startsWith("inf")
            || v.startsWith("rd") ? 0 : 100;
    }

    /** "3 GB", "2.5 GB", "768 MB" */
    public static String describeMb(int mb) {
        if (mb < 1024) {
            return mb + " MB";
        }
        if (mb % 1024 == 0) {
            return (mb / 1024) + " GB";
        }
        // to the nearest quarter: "2.5 GB", "1.75 GB"
        String gb = String.format(Locale.ENGLISH, "%.2f", Math.round(mb / 256.0) / 4.0);
        gb = gb.replaceAll("0+$", "").replaceAll("\\.$", "");
        return gb + " GB";
    }

    /** a few words for the kind of game: "modern vanilla", "a Fabric game with 5 mods", ... */
    public static String describeGame(Game game) {
        int minor = minorVersion(game.minecraftVersion);
        String age = minor <= 12 ? "an older" : "a modern";
        String loader;
        switch (game.loader) {
            case FABRIC:
                loader = "Fabric";
                break;
            case QUILT:
                loader = "Quilt";
                break;
            case FORGE:
                loader = "Forge";
                break;
            case NEOFORGE:
                loader = "NeoForge";
                break;
            case OTHER:
                loader = "modded";
                break;
            default:
                return age + " vanilla game";
        }
        return String.format(Locale.ENGLISH, "%s %s game with %d mod%s", age, loader, game.modCount,
            game.modCount == 1 ? "" : "s");
    }

    static int roundUp(int mb) {
        return (mb + 255) / 256 * 256;
    }

    static int roundDown(int mb) {
        return Math.max(0, mb) / 256 * 256;
    }
}
