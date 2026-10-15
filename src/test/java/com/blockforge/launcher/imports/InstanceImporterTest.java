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
package com.blockforge.launcher.imports;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Collections;
import java.util.List;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

class InstanceImporterTest {
    private static void write(Path file, String content) throws IOException {
        Files.createDirectories(file.getParent());
        Files.write(file, content.getBytes(StandardCharsets.UTF_8));
    }

    private Path prism(Path root) throws IOException {
        Path dir = root.resolve("My Pack");
        write(dir.resolve("instance.cfg"), "name=My Pack\nOverrideMemory=true\nMaxMemAlloc=6144\nOverrideJavaArgs=true\nJvmArgs=-Dfoo=bar\n");
        write(dir.resolve("mmc-pack.json"), "{\"components\":[{\"uid\":\"net.minecraft\",\"version\":\"1.20.1\"},"
            + "{\"uid\":\"net.fabricmc.fabric-loader\",\"version\":\"0.15.7\"}],\"formatVersion\":1}");
        write(dir.resolve(".minecraft/mods/sodium.jar"), "jar");
        write(dir.resolve(".minecraft/mods/lithium.jar"), "jar");
        write(dir.resolve(".minecraft/saves/World1/level.dat"), "world");
        write(dir.resolve(".minecraft/config/sodium.json"), "{}");
        write(dir.resolve(".minecraft/options.txt"), "fov:90");
        write(dir.resolve(".minecraft/logs/latest.log"), "old log");
        write(dir.resolve(".minecraft/crash-reports/crash.txt"), "crash");
        return dir;
    }

    @Test
    void readsPrismInstances(@TempDir Path temp) throws IOException {
        ImportedInstance i = InstanceImporter.read(prism(temp));

        assertEquals("My Pack", i.name);
        assertEquals("1.20.1", i.minecraftVersion);
        assertEquals("fabric", i.loaderType);
        assertEquals("0.15.7", i.loaderVersion);
        assertEquals(Integer.valueOf(6144), i.maximumMemoryMb);
        assertEquals("-Dfoo=bar", i.javaArguments);
        assertEquals("Fabric 0.15.7", i.loaderLabel());
    }

    @Test
    void readsCurseForgeInstances(@TempDir Path temp) throws IOException {
        Path dir = temp.resolve("ATM9");
        write(dir.resolve("minecraftinstance.json"), "{\"name\":\"ATM9\",\"gameVersion\":\"1.20.1\","
            + "\"baseModLoader\":{\"name\":\"forge-47.2.0\",\"minecraftVersion\":\"1.20.1\"},\"allocatedMemory\":8192}");
        write(dir.resolve("mods/a.jar"), "x");

        ImportedInstance i = InstanceImporter.read(dir);
        assertEquals("CurseForge", i.source);
        assertEquals("forge", i.loaderType);
        assertEquals("47.2.0", i.loaderVersion);
        assertEquals(Integer.valueOf(8192), i.maximumMemoryMb);
    }

    @Test
    void curseForgeFabricVersionsLeaveOutTheMinecraftVersion(@TempDir Path temp) throws IOException {
        // CurseForge names Fabric and Quilt loaders "fabric-<loader>-<minecraft>"
        Path named = temp.resolve("Named");
        write(named.resolve("minecraftinstance.json"), "{\"name\":\"Named\",\"gameVersion\":\"1.20.1\","
            + "\"baseModLoader\":{\"name\":\"fabric-0.14.21-1.20.1\",\"minecraftVersion\":\"1.20.1\"}}");
        ImportedInstance fromName = InstanceImporter.read(named);
        assertEquals("fabric", fromName.loaderType);
        assertEquals("0.14.21", fromName.loaderVersion);

        // and when it says the version on its own, that is used
        Path stored = temp.resolve("Stored");
        write(stored.resolve("minecraftinstance.json"), "{\"name\":\"Stored\",\"gameVersion\":\"1.21.1\","
            + "\"baseModLoader\":{\"name\":\"quilt-0.26.4-1.21.1\",\"forgeVersion\":\"0.26.4\"}}");
        ImportedInstance fromField = InstanceImporter.read(stored);
        assertEquals("quilt", fromField.loaderType);
        assertEquals("0.26.4", fromField.loaderVersion);
    }

    @Test
    void picksANameThatIsNotTakenYet() {
        java.util.Set<String> taken = new java.util.HashSet<>(java.util.Arrays.asList("My Pack", "My Pack (2)"));

        assertEquals("Other", InstanceImporter.uniqueName("Other", taken::contains));
        assertEquals("My Pack (3)", InstanceImporter.uniqueName("My Pack", taken::contains));
        assertEquals("Imported instance", InstanceImporter.uniqueName("★★★", taken::contains));
        assertEquals("Imported instance", InstanceImporter.uniqueName(null, taken::contains));
    }

    @Test
    void readsAtlauncherAndGdLauncherInstances(@TempDir Path temp) throws IOException {
        Path atl = temp.resolve("atl");
        write(atl.resolve("instance.json"), "{\"id\":\"1.19.2\",\"launcher\":{\"name\":\"Old Pack\",\"loaderVersion\":{\"type\":\"Forge\",\"version\":\"43.2.0\"}}}");
        ImportedInstance a = InstanceImporter.read(atl);
        assertEquals("ATLauncher", a.source);
        assertEquals("forge", a.loaderType);

        Path gd = temp.resolve("gd");
        write(gd.resolve("config.json"), "{\"name\":\"GD Pack\",\"game_configuration\":{\"version\":{\"release\":\"1.18.2\",\"modloaders\":[{\"type\":\"quilt\",\"version\":\"0.20\"}]}}}");
        Files.createDirectories(gd.resolve("instance"));
        ImportedInstance g = InstanceImporter.read(gd);
        assertEquals("GDLauncher", g.source);
        assertEquals("quilt", g.loaderType);
        assertEquals(gd.resolve("instance"), g.gameDir);
    }

    @Test
    void vanillaInstancesHaveNoLoader(@TempDir Path temp) throws IOException {
        Path dir = temp.resolve("v");
        write(dir.resolve("instance.cfg"), "name=Vanilla\n");
        write(dir.resolve("mmc-pack.json"), "{\"components\":[{\"uid\":\"net.minecraft\",\"version\":\"1.21\"}]}");
        Files.createDirectories(dir.resolve(".minecraft"));

        ImportedInstance i = InstanceImporter.read(dir);
        assertNull(i.loaderType);
        assertEquals("Vanilla", i.loaderLabel());
    }

    @Test
    void rejectsBrokenOrUnknownFolders(@TempDir Path temp) throws IOException {
        assertThrows(IOException.class, () -> InstanceImporter.read(temp));

        Path broken = temp.resolve("broken");
        write(broken.resolve("instance.cfg"), "name=x");
        write(broken.resolve("mmc-pack.json"), "{ not json");
        assertThrows(IOException.class, () -> InstanceImporter.read(broken));

        Path noVersion = temp.resolve("noversion");
        write(noVersion.resolve("instance.cfg"), "name=x");
        write(noVersion.resolve("mmc-pack.json"), "{\"components\":[]}");
        assertThrows(IOException.class, () -> InstanceImporter.read(noVersion));
    }

    @Test
    void previewCountsWithoutChangingAnything(@TempDir Path temp) throws IOException {
        ImportedInstance i = InstanceImporter.read(prism(temp));
        InstanceImporter.Preview p = InstanceImporter.preview(i);

        assertEquals(2, p.mods);
        assertEquals(1, p.worlds);
        assertTrue(p.topLevel.contains("mods"));
        assertFalse(p.topLevel.contains("logs"));
        assertFalse(p.topLevel.contains("crash-reports"));
        assertEquals(5, p.files);
        assertTrue(Files.exists(i.gameDir.resolve("logs/latest.log")));
    }

    @Test
    void copiesGameFilesAndSkipsHistory(@TempDir Path temp) throws IOException {
        ImportedInstance i = InstanceImporter.read(prism(temp.resolve("src")));
        Path target = temp.resolve("instance");
        Files.createDirectories(target.resolve("mods"));
        write(target.resolve("options.txt"), "fov:70");

        InstanceImporter.copyInto(i, target);

        assertTrue(Files.exists(target.resolve("mods/sodium.jar")));
        assertTrue(Files.exists(target.resolve("saves/World1/level.dat")));
        assertEquals("fov:90", new String(Files.readAllBytes(target.resolve("options.txt")), StandardCharsets.UTF_8));
        assertFalse(Files.exists(target.resolve("logs")));
        assertFalse(Files.exists(target.resolve("crash-reports")));
        try (java.util.stream.Stream<Path> left = Files.list(temp)) {
            assertTrue(left.noneMatch(p -> p.getFileName().toString().startsWith(".import-")));
        }
    }

    @Test
    void skipsSymbolicLinksInsteadOfFollowingThem(@TempDir Path temp) throws IOException {
        Path src = prism(temp.resolve("src"));
        Path secret = temp.resolve("secret.txt");
        write(secret, "do not copy");
        try {
            Files.createSymbolicLink(src.resolve(".minecraft/mods/link.jar"), secret);
        } catch (UnsupportedOperationException | IOException e) {
            return; // symlinks aren't available here
        }

        Path target = temp.resolve("instance");
        Files.createDirectories(target);
        InstanceImporter.copyInto(InstanceImporter.read(src), target);

        assertFalse(Files.exists(target.resolve("mods/link.jar")));
        assertTrue(Files.exists(target.resolve("mods/sodium.jar")));
    }

    @Test
    void scanFindsInstancesInLauncherFolders(@TempDir Path temp) throws IOException {
        prism(temp.resolve("prism"));
        List<ImportedInstance> found = InstanceImporter.scan(java.util.Arrays.asList(temp.resolve("prism"),
            temp.resolve("missing")));
        assertEquals(1, found.size());
        assertEquals("My Pack", found.get(0).name);
        assertTrue(InstanceImporter.scan(Collections.emptyList()).isEmpty());
    }
}
