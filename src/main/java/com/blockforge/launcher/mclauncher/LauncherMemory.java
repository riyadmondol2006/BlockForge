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

import java.util.List;
import java.util.concurrent.CopyOnWriteArrayList;

import com.blockforge.launcher.Network;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.utils.FileFingerprints;
import com.blockforge.launcher.utils.OS;

/**
 * Gives memory back while Minecraft runs on computers that have little of it, when the launcher stays open. Anything
 * that keeps a cache that is cheap to rebuild (images, lists) can register a way to drop it.
 */
public final class LauncherMemory {
    private static final List<Runnable> RELEASERS = new CopyOnWriteArrayList<>();

    private LauncherMemory() {
    }

    /** Adds something to run to drop a cache while the game runs on a computer with little memory. */
    public static void register(Runnable releaser) {
        RELEASERS.add(releaser);
    }

    /** Called once the game is running. Does nothing on computers with plenty of memory. */
    public static void gameStarted() {
        int physicalMb = OS.getSystemRam();
        if (!GameMemory.isLowMemoryComputer(physicalMb)) {
            return;
        }

        Thread thread = new Thread(() -> {
            LogManager.info("This computer has " + physicalMb
                + " MB of memory, so the launcher is freeing what it can while Minecraft runs");

            for (Runnable releaser : RELEASERS) {
                try {
                    releaser.run();
                } catch (Throwable t) {
                    LogManager.debug("Couldn't free a cache: " + t.getMessage());
                }
            }

            // written to disk first, then read again when next needed
            FileFingerprints.releaseMemory();
            // idle connections each hold buffers and a socket
            Network.CLIENT.connectionPool().evictAll();

            // a full collection lets Java hand unused heap back to the system, which it otherwise keeps; the window
            // is hidden or idle now, so the pause isn't noticed
            System.gc();
        }, "free-launcher-memory");
        thread.setDaemon(true);
        thread.setPriority(Thread.MIN_PRIORITY);
        thread.start();
    }
}
