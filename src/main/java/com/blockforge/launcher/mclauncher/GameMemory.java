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
import java.util.List;

/**
 * Decides how much memory the game gets, so it never takes more than the computer can spare. Giving Minecraft more
 * than about half of the memory leaves too little for the system, the graphics driver and the launcher, and the
 * whole computer starts swapping, which is far slower than a smaller game heap.
 */
public final class GameMemory {
    /** the most the launcher picks on its own; Minecraft and most modpacks run well with 4 GB */
    public static final int DEFAULT_CAP_MB = 4096;
    /** what 32-bit Java can use at most, reliably */
    public static final int MAX_32_BIT_MB = 1024;
    /** below this the game doesn't start properly at all */
    public static final int MINIMUM_MB = 512;
    /** what to use when the computer's memory can't be found out */
    public static final int UNKNOWN_DEFAULT_MB = 2048;

    private GameMemory() {
    }

    /**
     * The default amount of memory for the game on a computer with this much memory: half of it, rounded down to 256
     * MB and at most 4 GB, so 2 GB gives 1 GB, 4 GB gives 2 GB and 8 GB or more gives 4 GB.
     */
    public static int defaultMaximumMb(int physicalMb, boolean is64Bit) {
        if (physicalMb <= 0) {
            return is64Bit ? UNKNOWN_DEFAULT_MB : MAX_32_BIT_MB;
        }

        return Math.min(safeMaximumMb(physicalMb, is64Bit), DEFAULT_CAP_MB);
    }

    /**
     * The most memory the game can have without starving the rest of the computer: half of it, rounded down to 256
     * MB (1 GB at most for 32-bit Java), or no limit if the amount of memory isn't known.
     */
    public static int safeMaximumMb(int physicalMb, boolean is64Bit) {
        if (physicalMb <= 0) {
            return is64Bit ? Integer.MAX_VALUE : MAX_32_BIT_MB;
        }

        int half = Math.max(MINIMUM_MB, (physicalMb / 2) / 256 * 256);
        return is64Bit ? half : Math.min(half, MAX_32_BIT_MB);
    }

    /** How much memory the game gets and anything the person should know about it. */
    public static final class Plan {
        public final int heapMb;
        public final List<String> warnings = new ArrayList<>();

        Plan(int heapMb) {
            this.heapMb = heapMb;
        }
    }

    /**
     * Works out the game's -Xmx.
     *
     * @param configuredMb  the memory set in the instance or in the settings
     * @param recommendedMb what the modpack says it needs, or 0 if it doesn't say
     * @param physicalMb    the computer's memory, or 0 if unknown
     */
    public static Plan plan(int configuredMb, int recommendedMb, int physicalMb, boolean is64Bit) {
        int safe = safeMaximumMb(physicalMb, is64Bit);
        int heap = configuredMb > 0 ? configuredMb : defaultMaximumMb(physicalMb, is64Bit);
        List<String> warnings = new ArrayList<>();

        // a modpack that needs more than set gets more, as far as the computer can spare it
        if (recommendedMb > heap) {
            int raised = Math.max(heap, Math.min(recommendedMb, safe));
            if (recommendedMb > safe) {
                warnings.add(String.format(
                    "This modpack recommends %d MB of memory, but this computer (%d MB) can only spare about %d MB, "
                        + "so the game gets %d MB. It may run slowly or run out of memory.",
                    recommendedMb, physicalMb, safe, raised));
            }
            heap = raised;
        }

        // what the person set themselves is respected, but they should know when it's more than is good
        if (physicalMb > 0 && heap > safe) {
            warnings.add(String.format(
                "The game is set to use %d MB of memory, which is more than half of this computer's %d MB. That leaves "
                    + "little for everything else and can make the whole computer slow; about %d MB is recommended.",
                heap, physicalMb, safe));
        }

        Plan plan = new Plan(heap);
        plan.warnings.addAll(warnings);
        return plan;
    }

    /** a warning when the game is given more memory than is free right now, or null */
    public static String availabilityWarning(int heapMb, long availableMb) {
        if (availableMb <= 0 || heapMb <= availableMb) {
            return null;
        }

        return String.format("The game may use up to %d MB of memory, but only %d MB is free right now. Closing other "
            + "programs (web browsers especially) will help it run smoothly.", heapMb, availableMb);
    }

    /** if the computer is low on memory, so the launcher should give back what it can while the game runs */
    public static boolean isLowMemoryComputer(int physicalMb) {
        return physicalMb > 0 && physicalMb <= 4096 + 512;
    }
}
