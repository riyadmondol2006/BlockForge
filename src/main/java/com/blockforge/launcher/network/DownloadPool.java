/*
 * ATLauncher - https://github.com/ATLauncher/ATLauncher
 * Copyright (C) 2013-2022 ATLauncher
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
package com.blockforge.launcher.network;

import java.io.IOException;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;
import java.util.stream.Collectors;

import java.util.concurrent.atomic.AtomicInteger;

import com.blockforge.launcher.App;
import com.blockforge.launcher.data.Settings;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.utils.FileFingerprints;
import com.blockforge.launcher.utils.FileUtils;
import com.blockforge.launcher.utils.LightMode;

public final class DownloadPool extends ArrayList<Download> {
    /** how many pools are downloading right now, so background disk work can keep out of the way */
    private static final AtomicInteger ACTIVE = new AtomicInteger();

    private final boolean wait;

    /** if any files are being downloaded right now */
    public static boolean anyActive() {
        return ACTIVE.get() > 0;
    }

    public DownloadPool(boolean wait) {
        this.wait = wait;
    }

    public DownloadPool() {
        this(true);
    }

    public void downloadAll() {
        for (Download dl : this) {
            if (!Files.isDirectory(dl.to.getParent())) {
                FileUtils.createDirectory(dl.to.getParent());
            }
        }

        if (this.isEmpty()) {
            return;
        }

        int processors = Runtime.getRuntime().availableProcessors();
        int configured = forComputer(App.settings.concurrentConnections,
            Settings.DEFAULT_CONCURRENT_CONNECTIONS, LightMode.totalRamMb(), processors);
        int threads = downloadThreads(configured, processors, this.size());
        ExecutorService executor = Executors.newFixedThreadPool(threads);
        synchronized (this) {
            for (Download dl : startOrder(this, threads)) {
                executor.execute(new Downloader(dl));
            }
        }
        executor.shutdown();
        if (this.wait) {
            ACTIVE.incrementAndGet();
            try {
                awaitTermination(executor);
            } finally {
                ACTIVE.decrementAndGet();
            }
        }
    }

    /**
     * How many downloads to run at once: what the player set, or when the setting was never changed from the
     * default, what suits this computer: 8 with two cores or 4 GB of memory or less (each finished file is also
     * hashed and written, and more at once only makes the launcher stutter there), otherwise the default.
     * {@link com.blockforge.launcher.utils.PcProfile} uses the same rule; it lives here so downloads while the
     * launcher starts don't load the PC profile.
     */
    public static int forComputer(int configured, int defaultConfigured, long totalMb, int cores) {
        if (configured != defaultConfigured) {
            return configured;
        }
        boolean small = (totalMb > 0 && totalMb <= 4608) || (cores > 0 && cores <= 2);
        return small ? Math.min(configured, 8) : configured;
    }

    /** Waits for the pool to finish without spinning a CPU core while it does. */
    private static void awaitTermination(ExecutorService executor) {
        try {
            while (!executor.awaitTermination(1, TimeUnit.MINUTES)) {
                // still working
            }
        } catch (InterruptedException e) {
            executor.shutdownNow();
            Thread.currentThread().interrupt();
        }
    }

    /**
     * How many files to download at once: the number set in the settings, but no more than there are files, and on
     * computers with few cores no more than 4 per core, since every file also has to be checked (and maybe unpacked)
     * once it arrives, and a 2 core computer gains nothing from more than 8 at a time.
     */
    static int downloadThreads(int configured, int processors, int files) {
        int threads = Math.min(Math.max(1, configured), Math.max(8, processors * 4));
        return Math.max(1, Math.min(threads, files));
    }

    /**
     * The order to start downloads in: the few biggest files first, one per thread, since a large file on a single
     * connection is what everything else would otherwise end up waiting for at the end. The rest keep their order,
     * so big and small files stay mixed and the connection is kept busy while small files wait for the server.
     */
    static List<Download> startOrder(List<Download> downloads, int threads) {
        List<Download> biggest = new ArrayList<>(downloads);
        biggest.sort(Comparator.comparingLong((Download dl) -> dl.size).reversed());
        biggest = new ArrayList<>(biggest.subList(0, Math.min(Math.max(1, threads), biggest.size())));

        List<Download> ordered = new ArrayList<>(biggest);
        for (Download dl : downloads) {
            if (!containsSame(biggest, dl)) {
                ordered.add(dl);
            }
        }
        return ordered;
    }

    private static boolean containsSame(List<Download> list, Download dl) {
        for (Download other : list) {
            if (other == dl) {
                return true;
            }
        }
        return false;
    }

    /**
     * How many files to check at once. With the fingerprints most checks only look at a file's size and date, and
     * the rest hash it, which is processor work: two per core keeps the disk and processors busy without swamping
     * small computers.
     */
    static int checkThreads(int processors, int files) {
        return Math.max(1, Math.min(files, Math.max(2, Math.min(processors * 2, 16))));
    }

    public long totalSize() {
        long size = 0;
        synchronized (this) {
            for (Download dl : this) {
                if (dl.needToDownload()) {
                    size += dl.size;
                }
            }
        }
        return size;
    }

    public DownloadPool downsize() {
        final DownloadPool pool = new DownloadPool(this.wait);
        final List<Download> downloads = this.stream().distinct().collect(Collectors.toList());

        if (downloads.isEmpty()) {
            return pool;
        }

        ExecutorService executor = Executors
            .newFixedThreadPool(checkThreads(Runtime.getRuntime().availableProcessors(), downloads.size()));
        for (final Download dl : downloads) {
            executor.submit(() -> {
                if (dl.needToDownload()) {
                    synchronized (pool) {
                        pool.add(dl);
                    }
                } else {
                    dl.copy();
                    dl.runPostProcessors();
                }
            });
        }

        executor.shutdown();
        awaitTermination(executor);

        FileFingerprints.save();

        return pool;
    }

    public boolean any() {
        synchronized (this) {
            for (Download dl : this) {
                if (dl.needToDownload()) {
                    return true;
                }
            }
        }

        return false;
    }

    private static final class Downloader implements Runnable {
        private final Download dl;

        private Downloader(Download dl) {
            this.dl = dl;
        }

        @Override
        public void run() {
            try {
                if (this.dl.needToDownload()) {
                    this.dl.downloadFile();
                } else {
                    this.dl.copy();
                }
            } catch (IOException e) {
                LogManager.logStackTrace("Error trying to download " + this.dl.to.getFileName(), e);
            }
        }
    }
}
