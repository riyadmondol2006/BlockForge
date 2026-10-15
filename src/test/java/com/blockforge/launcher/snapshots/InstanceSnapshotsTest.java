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
package com.blockforge.launcher.snapshots;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

class InstanceSnapshotsTest {
    private static void write(Path file, String content) throws IOException {
        Files.createDirectories(file.getParent());
        Files.write(file, content.getBytes(StandardCharsets.UTF_8));
    }

    private static String read(Path file) throws IOException {
        return new String(Files.readAllBytes(file), StandardCharsets.UTF_8);
    }

    private Path instance(Path temp) throws IOException {
        Path root = temp.resolve("instance");
        write(root.resolve("mods/a.jar"), "A1");
        write(root.resolve("config/a.toml"), "setting=1");
        write(root.resolve("instance.json"), "{\"v\":1}");
        write(root.resolve("saves/World/level.dat"), "my world");
        write(root.resolve("options.txt"), "fov:90");
        return root;
    }

    @Test
    void restoresWhatAnUpdateChangedWithoutTouchingWorlds(@TempDir Path temp) throws IOException {
        Path root = instance(temp);
        Path dir = temp.resolve("snapshots");

        InstanceSnapshots.Snapshot snapshot = InstanceSnapshots.create(root, dir, "before-update");

        // an "update" changes, adds and removes things, and the player keeps playing
        write(root.resolve("mods/a.jar"), "A2");
        write(root.resolve("mods/new.jar"), "NEW");
        write(root.resolve("config/a.toml"), "setting=2");
        write(root.resolve("saves/World/level.dat"), "my world, played more");
        Files.delete(root.resolve("instance.json"));

        InstanceSnapshots.restore(snapshot, root, dir);

        assertEquals("A1", read(root.resolve("mods/a.jar")));
        assertFalse(Files.exists(root.resolve("mods/new.jar")));
        assertEquals("setting=1", read(root.resolve("config/a.toml")));
        assertEquals("{\"v\":1}", read(root.resolve("instance.json")));
        assertEquals("my world, played more", read(root.resolve("saves/World/level.dat")));
        assertEquals("fov:90", read(root.resolve("options.txt")));
    }

    @Test
    void aRestoreCanItselfBeUndone(@TempDir Path temp) throws IOException {
        Path root = instance(temp);
        Path dir = temp.resolve("snapshots");
        InstanceSnapshots.Snapshot original = InstanceSnapshots.create(root, dir, "first");

        write(root.resolve("mods/a.jar"), "A2");
        InstanceSnapshots.restore(original, root, dir);

        List<InstanceSnapshots.Snapshot> all = InstanceSnapshots.list(dir);
        assertTrue(all.get(0).label().contains("before-restore"));

        InstanceSnapshots.restore(all.get(0), root, dir);
        assertEquals("A2", read(root.resolve("mods/a.jar")));
    }

    @Test
    void keepsOnlyTheNewestSnapshots(@TempDir Path temp) throws IOException {
        Path root = instance(temp);
        Path dir = temp.resolve("snapshots");

        for (int i = 0; i < InstanceSnapshots.KEEP + 3; i++) {
            InstanceSnapshots.create(root, dir, "s" + i);
        }

        List<InstanceSnapshots.Snapshot> all = InstanceSnapshots.list(dir);
        assertEquals(InstanceSnapshots.KEEP, all.size());
        assertTrue(all.get(0).label().equals("s" + (InstanceSnapshots.KEEP + 2)));
    }

    @Test
    void refusesSnapshotsThatWriteOutsideTheInstance(@TempDir Path temp) throws IOException {
        Path root = instance(temp);
        Path dir = temp.resolve("snapshots");
        Files.createDirectories(dir);
        Path evil = dir.resolve("20000101-000000-000-evil.zip");
        try (ZipOutputStream z = new ZipOutputStream(Files.newOutputStream(evil))) {
            z.putNextEntry(new ZipEntry("../escaped.txt"));
            z.write("gotcha".getBytes(StandardCharsets.UTF_8));
            z.closeEntry();
        }

        InstanceSnapshots.Snapshot snapshot = InstanceSnapshots.list(dir).get(0);
        assertThrows(IOException.class, () -> InstanceSnapshots.restore(snapshot, root, dir));

        assertFalse(Files.exists(temp.resolve("escaped.txt")));
        assertEquals("A1", read(root.resolve("mods/a.jar")));
    }

    @Test
    void restoringRemovesPackFoldersThatDidNotExistYet(@TempDir Path temp) throws IOException {
        Path root = instance(temp);
        Files.createDirectories(root.resolve("defaultconfigs"));
        Path dir = temp.resolve("snapshots");
        InstanceSnapshots.Snapshot snapshot = InstanceSnapshots.create(root, dir, "before-update");

        // the update brings a scripts folder and fills the empty defaultconfigs folder
        write(root.resolve("kubejs/startup.js"), "new script");
        write(root.resolve("defaultconfigs/new.toml"), "x=1");

        InstanceSnapshots.restore(snapshot, root, dir);

        assertFalse(Files.exists(root.resolve("kubejs")));
        assertTrue(Files.isDirectory(root.resolve("defaultconfigs")));
        assertFalse(Files.exists(root.resolve("defaultconfigs/new.toml")));
        assertEquals("A1", read(root.resolve("mods/a.jar")));
        assertEquals("my world", read(root.resolve("saves/World/level.dat")));
    }

    @Test
    void theOldestKeptSnapshotCanBeRestored(@TempDir Path temp) throws IOException {
        Path root = instance(temp);
        Path dir = temp.resolve("snapshots");

        InstanceSnapshots.Snapshot oldest = InstanceSnapshots.create(root, dir, "oldest");
        write(root.resolve("mods/a.jar"), "A2");
        for (int i = 1; i < InstanceSnapshots.KEEP; i++) {
            InstanceSnapshots.create(root, dir, "s" + i);
        }
        assertEquals(InstanceSnapshots.KEEP, InstanceSnapshots.list(dir).size());

        // saving the state before the restore pushes the oldest snapshot out of the kept ones
        InstanceSnapshots.restore(oldest, root, dir);

        assertEquals("A1", read(root.resolve("mods/a.jar")));
    }

    @Test
    void leavesNoTemporaryFoldersBehind(@TempDir Path temp) throws IOException {
        Path root = instance(temp);
        Path dir = temp.resolve("snapshots");
        InstanceSnapshots.restore(InstanceSnapshots.create(root, dir, "x"), root, dir);

        try (java.util.stream.Stream<Path> left = Files.list(temp)) {
            assertTrue(left.noneMatch(p -> p.getFileName().toString().startsWith(".")));
        }
    }
}
