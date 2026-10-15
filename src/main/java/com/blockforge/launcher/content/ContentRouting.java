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
package com.blockforge.launcher.content;

import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Enumeration;
import java.util.List;
import java.util.Locale;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

/**
 * Works out which folder of an instance a file belongs in: mods, resource packs or shader packs. The file is looked
 * at by {@link DroppedFileDetector}; this only maps what it found to a folder.
 */
public final class ContentRouting {
    /** at most this many names are read from a zip; enough to recognise any pack */
    static final int MAX_ENTRIES = 4000;

    private ContentRouting() {
    }

    /**
     * Where a file goes.
     *
     * @param fileName   the file's name
     * @param zipEntries the names inside the file if it is a zip that could be read, otherwise null
     * @param current    the tab the player dropped it on, used when a zip can't be told apart
     * @return the kind, or null when the file doesn't belong in any of them (a data pack, a modpack, a text file)
     */
    public static ContentKind route(String fileName, List<String> zipEntries, ContentKind current) {
        if (fileName == null) {
            return null;
        }
        String name = fileName.toLowerCase(Locale.ROOT);
        if (name.endsWith(".jar") || name.endsWith(".litemod")) {
            return ContentKind.MODS;
        }
        if (!name.endsWith(".zip")) {
            return null;
        }
        if (zipEntries != null) {
            DroppedFile found = DroppedFileDetector.classify(null, fileName, zipEntries, null);
            if (found.kind != DroppedFile.Kind.UNKNOWN) {
                return found.kind.contentKind();
            }
        }
        // an unrecognised zip only makes sense in a pack folder, and only if that's where it was dropped
        return current == ContentKind.MODS ? null : current;
    }

    /** The folder for what the detector found, or null if it doesn't go into one of these folders. */
    public static ContentKind route(DroppedFile file) {
        return file == null ? null : file.kind.contentKind();
    }

    /** The names inside a zip file, or null if it isn't a zip that can be read. */
    public static List<String> zipEntries(Path file) {
        try (ZipFile zip = new ZipFile(file.toFile())) {
            List<String> names = new ArrayList<>();
            Enumeration<? extends ZipEntry> entries = zip.entries();
            while (entries.hasMoreElements() && names.size() < MAX_ENTRIES) {
                names.add(entries.nextElement().getName());
            }
            return Collections.unmodifiableList(names);
        } catch (IOException | RuntimeException e) {
            return null;
        }
    }
}
