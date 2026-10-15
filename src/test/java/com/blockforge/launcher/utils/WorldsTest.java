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
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.attribute.FileTime;
import java.util.Optional;
import java.util.zip.GZIPOutputStream;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

public class WorldsTest {
    /** a level.dat like the game writes: a root compound with a Data compound holding a few tags */
    static byte[] levelDat(String levelName) throws Exception {
        ByteArrayOutputStream raw = new ByteArrayOutputStream();
        DataOutputStream nbt = new DataOutputStream(raw);
        nbt.writeByte(10); // root compound
        nbt.writeUTF("");
        nbt.writeByte(10); // Data
        nbt.writeUTF("Data");
        nbt.writeByte(3); // an int before the name, as in real files
        nbt.writeUTF("GameType");
        nbt.writeInt(0);
        nbt.writeByte(8); // a string that isn't the name
        nbt.writeUTF("generatorName");
        nbt.writeUTF("default");
        if (levelName != null) {
            nbt.writeByte(8);
            nbt.writeUTF("LevelName");
            nbt.writeUTF(levelName);
        }
        nbt.writeByte(4);
        nbt.writeUTF("LastPlayed");
        nbt.writeLong(1760000000000L);
        nbt.writeByte(0); // end of Data
        nbt.writeByte(0); // end of root
        nbt.flush();

        ByteArrayOutputStream zipped = new ByteArrayOutputStream();
        try (OutputStream gz = new GZIPOutputStream(zipped)) {
            gz.write(raw.toByteArray());
        }
        return zipped.toByteArray();
    }

    static void world(Path saves, String folder, String name, long modifiedMillis) throws Exception {
        Path dir = saves.resolve(folder);
        Files.createDirectories(dir);
        Path dat = dir.resolve("level.dat");
        Files.write(dat, levelDat(name));
        Files.setLastModifiedTime(dat, FileTime.fromMillis(modifiedMillis));
    }

    @Test
    public void testFindsTheWorldPlayedLast(@TempDir Path dir) throws Exception {
        Path saves = dir.resolve("saves");
        world(saves, "New World", "My first world", 1_000_000L);
        world(saves, "New World (1)", "Castle build ✓", 3_000_000L);
        world(saves, "Old", "Old", 2_000_000L);
        // a folder without level.dat (a world being copied, a backup folder) is no world
        Files.createDirectories(saves.resolve("not a world"));
        Files.setLastModifiedTime(saves.resolve("not a world"), FileTime.fromMillis(9_000_000L));

        Optional<Worlds.World> latest = Worlds.latest(saves);
        assertTrue(latest.isPresent());
        assertEquals("New World (1)", latest.get().folder);
        assertEquals("Castle build ✓", latest.get().name);
    }

    @Test
    public void testUsesTheFolderNameWhenTheNameCantBeRead(@TempDir Path dir) throws Exception {
        Path saves = dir.resolve("saves");
        world(saves, "Unnamed", null, 1_000_000L);
        assertEquals("Unnamed", Worlds.latest(saves).get().name);

        Path broken = saves.resolve("Broken");
        Files.createDirectories(broken);
        Files.write(broken.resolve("level.dat"), "not gzip".getBytes(StandardCharsets.UTF_8));
        Files.setLastModifiedTime(broken.resolve("level.dat"), FileTime.fromMillis(5_000_000L));
        Worlds.World w = Worlds.latest(saves).get();
        assertEquals("Broken", w.folder);
        assertEquals("Broken", w.name);
    }

    @Test
    public void testNoWorlds(@TempDir Path dir) throws Exception {
        assertFalse(Worlds.latest(dir.resolve("saves")).isPresent());
        Files.createDirectories(dir.resolve("saves"));
        assertFalse(Worlds.latest(dir.resolve("saves")).isPresent());
        assertFalse(Worlds.latest(null).isPresent());
    }

    @Test
    public void testFindStringTag() {
        byte[] data = { 1, 8, 0, 4, 'N', 'a', 'm', 'e', 0, 2, 'h', 'i', 0 };
        assertEquals("hi", Worlds.findStringTag(data, "Name"));
        assertNull(Worlds.findStringTag(data, "LevelName"));
        // a length running past the end
        byte[] cut = { 8, 0, 4, 'N', 'a', 'm', 'e', 0, 9, 'h' };
        assertNull(Worlds.findStringTag(cut, "Name"));
    }
}
