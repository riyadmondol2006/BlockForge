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

import java.lang.management.ManagementFactory;

import javax.swing.UIManager;

import com.blockforge.launcher.App;

/**
 * Light mode keeps the launcher easy on slower computers: fewer redraws, no smooth-scroll animation and a smaller
 * picture cache. It turns itself on for computers with little memory or few processor cores, and can be switched
 * either way in Settings.
 */
public final class LightMode {
    /** 4 GB machines report a little less than 4096 MB, and shared graphics memory takes some too */
    static final long LOW_RAM_MB = 4608;
    static final int LOW_CORES = 2;

    private static volatile Boolean detected;

    private LightMode() {
    }

    /** True when this computer counts as low end. */
    public static boolean isLowEndComputer(long totalRamMb, int cores) {
        return (totalRamMb > 0 && totalRamMb <= LOW_RAM_MB) || (cores > 0 && cores <= LOW_CORES);
    }

    /** Whether light mode is on: the player's choice, or the automatic choice for this computer. */
    public static boolean isOn() {
        Boolean chosen = App.settings == null ? null : App.settings.lightMode;
        return chosen != null ? chosen : detectedLowEnd();
    }

    public static boolean detectedLowEnd() {
        if (detected == null) {
            detected = isLowEndComputer(totalRamMb(), Runtime.getRuntime().availableProcessors());
        }
        return detected;
    }

    /** Total physical memory in MB, or -1 when the Java in use can't tell. */
    @SuppressWarnings("deprecation") // the newer name doesn't exist on Java 8
    public static long totalRamMb() {
        try {
            java.lang.management.OperatingSystemMXBean bean = ManagementFactory.getOperatingSystemMXBean();
            if (bean instanceof com.sun.management.OperatingSystemMXBean) {
                return ((com.sun.management.OperatingSystemMXBean) bean).getTotalPhysicalMemorySize() / (1024 * 1024);
            }
        } catch (Throwable ignored) {
            // some Javas don't offer it; fall through
        }
        return -1;
    }

    /** Applies the Swing settings that depend on light mode. Call after the look and feel is set. */
    public static void applyToSwing() {
        // smooth scrolling animates every wheel turn with a timer; on a slow machine plain scrolling feels faster
        UIManager.put("ScrollPane.smoothScrolling", !isOn());
    }

    /** Milliseconds between frames for small animations such as the loading spinner. */
    public static int frameDelayMs() {
        return isOn() ? 50 : 16;
    }

    /** How many bytes of decoded pictures the image cache may keep. */
    public static long imageCacheBytes() {
        return (isOn() ? 16L : 48L) * 1024 * 1024;
    }

    /** How many pictures load at the same time. */
    public static int imageLoaderThreads() {
        return isOn() ? 2 : 3;
    }
}
