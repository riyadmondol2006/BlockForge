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
package com.blockforge.launcher.downloads;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

class RestrictedFileMatcherTest {
    private static Path file(Path dir, String name, int size) throws IOException {
        Path p = dir.resolve(name);
        Files.write(p, new byte[size]);
        return p;
    }

    @Test
    void findsTheFileUnderItsOwnNameOrABrowsersRename(@TempDir Path temp) throws IOException {
        file(temp, "create-1.20.1.jar", 100);
        file(temp, "jei (1).jar", 50);
        file(temp, "sodium+fabric.jar", 70);
        List<Path> files = RestrictedFileMatcher.finishedFiles(temp);

        assertEquals("create-1.20.1.jar", RestrictedFileMatcher.find(files, "create-1.20.1.jar", 100, p -> true).get().getFileName().toString());
        assertEquals("jei (1).jar", RestrictedFileMatcher.find(files, "jei.jar", 50, p -> true).get().getFileName().toString());
        assertEquals("sodium+fabric.jar", RestrictedFileMatcher.find(files, "sodium fabric.jar", 70, p -> true).get().getFileName().toString());
    }

    @Test
    void ignoresUnfinishedDownloadsAndWrongSizes(@TempDir Path temp) throws IOException {
        file(temp, "mod.jar.crdownload", 100);
        file(temp, "mod.jar.part", 100);
        file(temp, "other.jar", 99);
        List<Path> files = RestrictedFileMatcher.finishedFiles(temp);

        assertEquals(1, files.size());
        assertFalse(RestrictedFileMatcher.find(files, "mod.jar", 100, p -> true).isPresent());
        assertFalse(RestrictedFileMatcher.find(files, "other.jar", 100, p -> true).isPresent());
    }

    @Test
    void findsARenamedFileOnlyIfItsContentsCheckOut(@TempDir Path temp) throws IOException {
        file(temp, "download.jar", 100);
        List<Path> files = RestrictedFileMatcher.find(RestrictedFileMatcher.finishedFiles(temp), "real-name.jar", 100,
            p -> false).map(List::of).orElse(List.of());
        assertTrue(files.isEmpty());

        assertEquals("download.jar", RestrictedFileMatcher.find(RestrictedFileMatcher.finishedFiles(temp),
            "real-name.jar", 100, p -> true).get().getFileName().toString());
        // a different kind of file is never taken for it
        assertFalse(RestrictedFileMatcher.find(RestrictedFileMatcher.finishedFiles(temp), "real-name.zip", 100, p -> true)
            .isPresent());
    }

    @Test
    void namesMatchLikeBrowsersSaveThem() {
        assertTrue(RestrictedFileMatcher.sameName("Mod.JAR", "mod.jar"));
        assertTrue(RestrictedFileMatcher.sameName("mod (2).jar", "mod.jar"));
        assertTrue(RestrictedFileMatcher.sameName("mod-1.jar", "mod.jar"));
        assertFalse(RestrictedFileMatcher.sameName("mod2.jar", "mod.jar"));
        assertFalse(RestrictedFileMatcher.sameName("modx.jar", "mod.jar"));
    }
}
