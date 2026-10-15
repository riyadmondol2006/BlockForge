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

import java.io.IOException;
import java.io.InputStreamReader;
import java.lang.reflect.Type;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;

import com.blockforge.launcher.FileSystem;
import com.blockforge.launcher.Gsons;
import com.blockforge.launcher.managers.LogManager;
import com.google.gson.reflect.TypeToken;

/**
 * Remembers facts about files that were already worked out (a hash that matched, a Java version, a clean virus scan)
 * together with the size and modified time the file had then. As long as a file hasn't changed, the fact still
 * holds, so launching doesn't have to read and hash thousands of unchanged libraries, assets and mods again.
 */
public final class FileFingerprints {
    private static volatile Store store;
    private static boolean shutdownHookAdded = false;
    private static final AtomicBoolean savePending = new AtomicBoolean(false);
    private static final ExecutorService SAVER = Executors.newSingleThreadExecutor(r -> {
        Thread t = new Thread(r, "fingerprints-save");
        t.setDaemon(true);
        return t;
    });

    private FileFingerprints() {
    }

    private static Store store() {
        Store s = store;
        if (s == null) {
            synchronized (FileFingerprints.class) {
                s = store;
                if (s == null) {
                    s = new Store(FileSystem.CACHE.resolve("file-fingerprints.json"));
                    s.load();
                    if (!shutdownHookAdded) {
                        Runtime.getRuntime()
                            .addShutdownHook(new Thread(FileFingerprints::saveNow, "fingerprints-save"));
                        shutdownHookAdded = true;
                    }
                    store = s;
                }
            }
        }
        return s;
    }

    /**
     * Gets what was remembered about the file for this kind of fact, or null if nothing is remembered or the file has
     * changed since.
     */
    public static String get(Path file, String kind) {
        return store().get(file, kind);
    }

    /** Remembers a fact about the file as it is right now. */
    public static void put(Path file, String kind, String value) {
        store().put(file, kind, value);
    }

    /**
     * Writes any new facts to disk in the background, so whoever learned them (usually Play) doesn't wait for it.
     * Free when nothing changed, and calls made while a write is pending are folded into it.
     */
    public static void save() {
        Store s = store;
        if (s == null || !s.isDirty() || !savePending.compareAndSet(false, true)) {
            return;
        }

        SAVER.execute(() -> {
            savePending.set(false);
            saveNow();
        });
    }

    /** Writes any new facts to disk right away. */
    public static void saveNow() {
        Store s = store;
        if (s != null) {
            s.save();
        }
    }

    /** Writes the facts to disk and drops them from memory; they are read again the next time they're needed. */
    public static void releaseMemory() {
        synchronized (FileFingerprints.class) {
            Store s = store;
            if (s != null) {
                s.save();
                store = null;
            }
        }
    }

    /**
     * The size and modified time of a file, read with a single call, or null if the file can't be read. Changing a
     * file almost always changes one of the two, which is the same thing build tools and rsync rely on.
     */
    static String stamp(Path file) {
        try {
            BasicFileAttributes attributes = Files.readAttributes(file, BasicFileAttributes.class);
            if (!attributes.isRegularFile()) {
                return null;
            }
            return attributes.size() + "|" + attributes.lastModifiedTime().toMillis() + "|";
        } catch (IOException | UnsupportedOperationException e) {
            return null;
        }
    }

    /** The facts and where they are kept; separate from the static helpers so it can be tested on its own. */
    static final class Store {
        private static final Type MAP_TYPE = new TypeToken<Map<String, String>>() {
        }.getType();

        private final Path storage;
        private final Map<String, String> entries = new ConcurrentHashMap<>();
        private final AtomicBoolean dirty = new AtomicBoolean(false);

        Store(Path storage) {
            this.storage = storage;
        }

        void load() {
            if (!Files.exists(storage)) {
                return;
            }

            try (InputStreamReader reader = new InputStreamReader(Files.newInputStream(storage),
                StandardCharsets.UTF_8)) {
                Map<String, String> read = Gsons.DEFAULT.fromJson(reader, MAP_TYPE);
                if (read != null) {
                    entries.putAll(read);
                }
            } catch (Exception e) {
                LogManager.debug("Couldn't read file fingerprints, starting over: " + e.getMessage());
            }
        }

        private static String key(Path file, String kind) {
            return kind + "|" + file.toAbsolutePath();
        }

        String get(Path file, String kind) {
            String entry = entries.get(key(file, kind));
            if (entry == null) {
                return null;
            }

            String stamp = stamp(file);
            return stamp != null && entry.startsWith(stamp) ? entry.substring(stamp.length()) : null;
        }

        void put(Path file, String kind, String value) {
            String stamp = stamp(file);
            if (stamp == null) {
                // the file is gone or unreadable, so there's nothing to remember
                return;
            }

            String previous = entries.put(key(file, kind), stamp + value);
            if (!(stamp + value).equals(previous)) {
                dirty.set(true);
            }
        }

        synchronized void save() {
            if (!dirty.getAndSet(false)) {
                return;
            }

            // forget files that don't exist any more, so the cache doesn't grow forever
            entries.keySet().removeIf(k -> !Files.exists(Paths.get(k.substring(k.indexOf('|') + 1))));

            Path temp = storage.resolveSibling(storage.getFileName() + ".tmp");
            try {
                Files.createDirectories(storage.getParent());
                Files.write(temp, Gsons.DEFAULT.toJson(entries, MAP_TYPE).getBytes(StandardCharsets.UTF_8));
                try {
                    Files.move(temp, storage, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
                } catch (IOException e) {
                    Files.move(temp, storage, StandardCopyOption.REPLACE_EXISTING);
                }
            } catch (IOException e) {
                dirty.set(true);
                LogManager.debug("Couldn't save file fingerprints: " + e.getMessage());
            }
        }

        int size() {
            return entries.size();
        }

        boolean isDirty() {
            return dirty.get();
        }
    }
}
