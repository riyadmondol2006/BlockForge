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
import java.nio.file.StandardCopyOption;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
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
    private static final Type MAP_TYPE = new TypeToken<Map<String, String>>() {
    }.getType();

    private static final Map<String, String> ENTRIES = new ConcurrentHashMap<>();
    private static final AtomicBoolean dirty = new AtomicBoolean(false);
    private static volatile boolean loaded = false;

    private FileFingerprints() {
    }

    private static Path storage() {
        return FileSystem.CACHE.resolve("file-fingerprints.json");
    }

    private static void ensureLoaded() {
        if (loaded) {
            return;
        }

        synchronized (ENTRIES) {
            if (loaded) {
                return;
            }

            Path storage = storage();
            if (Files.exists(storage)) {
                try (InputStreamReader reader = new InputStreamReader(Files.newInputStream(storage),
                    StandardCharsets.UTF_8)) {
                    Map<String, String> read = Gsons.DEFAULT.fromJson(reader, MAP_TYPE);
                    if (read != null) {
                        ENTRIES.putAll(read);
                    }
                } catch (Exception e) {
                    LogManager.debug("Couldn't read file fingerprints, starting over: " + e.getMessage());
                }
            }

            Runtime.getRuntime().addShutdownHook(new Thread(FileFingerprints::save, "fingerprints-save"));
            loaded = true;
        }
    }

    private static String key(Path file, String kind) {
        return kind + "|" + file.toAbsolutePath();
    }

    private static String stamp(Path file) throws IOException {
        return Files.size(file) + "|" + Files.getLastModifiedTime(file).toMillis() + "|";
    }

    /**
     * Gets what was remembered about the file for this kind of fact, or null if nothing is remembered or the file has
     * changed since.
     */
    public static String get(Path file, String kind) {
        ensureLoaded();

        String entry = ENTRIES.get(key(file, kind));
        if (entry == null) {
            return null;
        }

        try {
            String stamp = stamp(file);
            return entry.startsWith(stamp) ? entry.substring(stamp.length()) : null;
        } catch (IOException e) {
            return null;
        }
    }

    /** Remembers a fact about the file as it is right now. */
    public static void put(Path file, String kind, String value) {
        ensureLoaded();

        try {
            ENTRIES.put(key(file, kind), stamp(file) + value);
            dirty.set(true);
        } catch (IOException e) {
            // the file is gone or unreadable, so there's nothing to remember
        }
    }

    /** Writes any new facts to disk. Cheap when nothing changed. */
    public static void save() {
        if (!loaded || !dirty.getAndSet(false)) {
            return;
        }

        // forget files that don't exist any more, so the cache doesn't grow forever
        ENTRIES.keySet().removeIf(k -> !Files.exists(java.nio.file.Paths.get(k.substring(k.indexOf('|') + 1))));

        Path storage = storage();
        Path temp = storage.resolveSibling(storage.getFileName() + ".tmp");
        try {
            Files.createDirectories(storage.getParent());
            Files.write(temp, Gsons.DEFAULT.toJson(ENTRIES, MAP_TYPE).getBytes(StandardCharsets.UTF_8));
            Files.move(temp, storage, StandardCopyOption.REPLACE_EXISTING);
        } catch (IOException e) {
            dirty.set(true);
            LogManager.debug("Couldn't save file fingerprints: " + e.getMessage());
        }
    }
}
