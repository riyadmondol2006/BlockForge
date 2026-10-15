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

import java.awt.AWTEvent;
import java.awt.Toolkit;
import java.lang.management.ManagementFactory;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;

import javax.management.MBeanServer;
import javax.management.ObjectName;

import com.blockforge.launcher.managers.LogManager;
import com.sun.management.HotSpotDiagnosticMXBean;
import com.sun.management.VMOption;

/**
 * Gives memory back to the computer once the launcher has been left alone for a while.
 *
 * Java keeps the memory it grew into while busy (browsing modpacks, installing, a big search) for as long as it runs,
 * even though it no longer needs it. A launcher mostly sits in the background while the game plays, so after a minute
 * without input it tidies up once: unused heap goes back to the system, along with memory freed by native code.
 * Nothing cached is dropped here; caches that can go while the game runs on a low memory computer are released by
 * the game launch code instead.
 */
public final class IdleTrim {
    /** how long without mouse or keyboard input before tidying up */
    static final long IDLE_MS = 60_000;
    /** how often to look; a check is a couple of field reads, so this costs nothing measurable */
    static final long CHECK_SECONDS = 30;
    /** with no input at all, heap growth from background work (downloads, game logs) still gets tidied this often */
    static final long MIN_GROWTH_INTERVAL_MS = 5 * 60_000;
    /** how much the heap has to have grown since the last tidy up to count as growth */
    static final long GROWTH_BYTES = 48L * 1024 * 1024;

    private static volatile long lastActivity = System.currentTimeMillis();
    private static volatile long lastTrim;
    private static volatile long committedAfterTrim;
    private static ScheduledExecutorService checker;

    private IdleTrim() {
    }

    /**
     * Whether it's time to tidy up.
     *
     * @param now               the time now
     * @param lastActivity      the time of the last mouse or keyboard input
     * @param lastTrim          the time of the last tidy up, 0 for never
     * @param committed         heap the launcher holds now
     * @param committedAfterTrim heap it held right after the last tidy up
     */
    static boolean due(long now, long lastActivity, long lastTrim, long committed, long committedAfterTrim) {
        if (now - lastActivity < IDLE_MS) {
            return false;
        }
        if (lastActivity >= lastTrim) {
            // used since the last tidy up, and now left alone
            return true;
        }
        // nobody touched it, but background work made the heap grow
        return now - lastTrim >= MIN_GROWTH_INTERVAL_MS && committed - committedAfterTrim >= GROWTH_BYTES;
    }

    /** Starts watching for idle time. Safe to call more than once. */
    public static synchronized void start() {
        if (checker != null) {
            return;
        }

        relaxHeapIfDefault();

        try {
            Toolkit.getDefaultToolkit().addAWTEventListener(e -> lastActivity = System.currentTimeMillis(),
                AWTEvent.MOUSE_EVENT_MASK | AWTEvent.MOUSE_WHEEL_EVENT_MASK | AWTEvent.KEY_EVENT_MASK);
        } catch (Throwable t) {
            // no toolkit (headless): only background growth will trigger a tidy up
        }

        checker = Executors.newSingleThreadScheduledExecutor(r -> {
            Thread thread = new Thread(r, "idle-trim");
            thread.setDaemon(true);
            thread.setPriority(Thread.MIN_PRIORITY);
            return thread;
        });
        checker.scheduleWithFixedDelay(IdleTrim::check, CHECK_SECONDS, CHECK_SECONDS, TimeUnit.SECONDS);
    }

    private static void check() {
        try {
            long committed = Runtime.getRuntime().totalMemory();
            if (!due(System.currentTimeMillis(), lastActivity, lastTrim, committed, committedAfterTrim)) {
                return;
            }
            trim();
        } catch (Throwable t) {
            LogManager.debug("Couldn't tidy up memory: " + t.getMessage());
        }
    }

    /** Collects garbage, lets the heap shrink and hands freed native memory back to the system. */
    static void trim() {
        long before = Runtime.getRuntime().totalMemory();
        System.gc();
        trimNativeHeap();
        lastTrim = System.currentTimeMillis();
        committedAfterTrim = Runtime.getRuntime().totalMemory();
        LogManager.debug("Idle for a while, gave back " + ((before - committedAfterTrim) / (1024 * 1024))
            + " MB of heap", 1);
    }

    /**
     * Native code (fonts, images, networking) frees memory that the C library keeps for later instead of giving it
     * back. Java 17.0.9 and newer can ask it to let go; on older Javas, other systems or other C libraries this does
     * nothing.
     */
    static void trimNativeHeap() {
        if (!OS.isLinux()) {
            return;
        }
        try {
            MBeanServer server = ManagementFactory.getPlatformMBeanServer();
            ObjectName name = new ObjectName("com.sun.management:type=DiagnosticCommand");
            server.invoke(name, "systemTrimNativeHeap", new Object[] { new String[0] },
                new String[] { String[].class.getName() });
        } catch (Throwable ignored) {
            // not offered by this Java
        }
    }

    /**
     * The installers start the launcher with a heap that gives unused memory back (MaxHeapFreeRatio=30). When it's
     * started some other way and those settings were left at Java's defaults, the same is set here. Both settings can
     * be changed while running.
     */
    static void relaxHeapIfDefault() {
        try {
            HotSpotDiagnosticMXBean bean = ManagementFactory.getPlatformMXBean(HotSpotDiagnosticMXBean.class);
            if (bean == null) {
                return;
            }
            VMOption max = bean.getVMOption("MaxHeapFreeRatio");
            VMOption min = bean.getVMOption("MinHeapFreeRatio");
            if (max.getOrigin() == VMOption.Origin.DEFAULT && min.getOrigin() == VMOption.Origin.DEFAULT
                && max.isWriteable() && min.isWriteable()) {
                // lower the minimum first: the minimum may never be above the maximum
                bean.setVMOption("MinHeapFreeRatio", "10");
                bean.setVMOption("MaxHeapFreeRatio", "30");
            }
        } catch (Throwable ignored) {
            // not a HotSpot Java, or the settings can't be changed: Java's own defaults stay
        }
    }
}
