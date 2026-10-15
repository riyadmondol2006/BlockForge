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

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.attribute.FileTime;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

public class FileFingerprintsTest {
    @TempDir
    Path dir;

    private Path file(String name, String content) throws Exception {
        Path file = dir.resolve(name);
        Files.write(file, content.getBytes(StandardCharsets.UTF_8));
        Files.setLastModifiedTime(file, FileTime.fromMillis(1_600_000_000_000L));
        return file;
    }

    @Test
    public void testThatARememberedFactIsReturnedWhileTheFileIsUnchanged() throws Exception {
        FileFingerprints.Store store = new FileFingerprints.Store(dir.resolve("fp.json"));
        Path lib = file("lib.jar", "hello");

        store.put(lib, "hash", "abc");

        assertEquals("abc", store.get(lib, "hash"));
        assertNull(store.get(lib, "url"), "facts of another kind are separate");
    }

    @Test
    public void testThatChangingTheContentsForgetsTheFact() throws Exception {
        FileFingerprints.Store store = new FileFingerprints.Store(dir.resolve("fp.json"));
        Path lib = file("lib.jar", "hello");
        store.put(lib, "hash", "abc");

        // same modified time, different size
        file("lib.jar", "hello world");

        assertNull(store.get(lib, "hash"));
    }

    @Test
    public void testThatTouchingTheFileForgetsTheFact() throws Exception {
        FileFingerprints.Store store = new FileFingerprints.Store(dir.resolve("fp.json"));
        Path lib = file("lib.jar", "hello");
        store.put(lib, "hash", "abc");

        // same size, different modified time
        Files.setLastModifiedTime(lib, FileTime.fromMillis(1_700_000_000_000L));

        assertNull(store.get(lib, "hash"));
    }

    @Test
    public void testThatDeletedFilesHaveNoFacts() throws Exception {
        FileFingerprints.Store store = new FileFingerprints.Store(dir.resolve("fp.json"));
        Path lib = file("lib.jar", "hello");
        store.put(lib, "hash", "abc");

        Files.delete(lib);

        assertNull(store.get(lib, "hash"));
        store.put(lib, "hash", "def");
        assertNull(store.get(lib, "hash"));
    }

    @Test
    public void testThatFactsSurviveSavingAndLoading() throws Exception {
        Path storage = dir.resolve("cache/fp.json");
        Path kept = file("kept.jar", "one");
        Path gone = file("gone.jar", "two");

        FileFingerprints.Store store = new FileFingerprints.Store(storage);
        store.put(kept, "hash", "111");
        store.put(gone, "hash", "222");
        Files.delete(gone);
        store.save();

        FileFingerprints.Store reloaded = new FileFingerprints.Store(storage);
        reloaded.load();

        assertEquals("111", reloaded.get(kept, "hash"));
        assertEquals(1, reloaded.size(), "files that no longer exist are dropped when saving");
    }

    @Test
    public void testThatDirectoriesHaveNoStamp() {
        assertNull(FileFingerprints.stamp(dir));
        assertNull(FileFingerprints.stamp(dir.resolve("missing")));
    }
}
