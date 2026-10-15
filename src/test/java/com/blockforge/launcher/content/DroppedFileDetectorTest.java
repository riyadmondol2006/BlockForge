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

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import com.blockforge.launcher.content.DroppedFile.Kind;

/** Every kind of file a player might drop, built as a real zip or jar and read back. */
public class DroppedFileDetectorTest {
    @TempDir
    Path temp;

    /** Writes a zip with the given entries; an entry ending in / is a folder. */
    private Path zip(String name, Map<String, String> entries) throws IOException {
        Path file = temp.resolve(name);
        try (OutputStream out = Files.newOutputStream(file); ZipOutputStream z = new ZipOutputStream(out)) {
            for (Map.Entry<String, String> e : entries.entrySet()) {
                z.putNextEntry(new ZipEntry(e.getKey()));
                if (e.getValue() != null) {
                    z.write(e.getValue().getBytes(StandardCharsets.UTF_8));
                }
                z.closeEntry();
            }
        }
        return file;
    }

    private static Map<String, String> entries(String... pairs) {
        Map<String, String> map = new LinkedHashMap<>();
        for (int i = 0; i < pairs.length; i += 2) {
            map.put(pairs[i], pairs[i + 1]);
        }
        return map;
    }

    @Test
    public void fabricModWithNameVersionAndMinecraftRange() throws IOException {
        DroppedFile f = DroppedFileDetector.detect(zip("sodium.jar", entries("fabric.mod.json",
            "{\"schemaVersion\":1,\"id\":\"sodium\",\"name\":\"Sodium\",\"version\":\"0.9.2\","
                + "\"depends\":{\"minecraft\":\">=26.3 <26.4\",\"fabricloader\":\">=0.16\"}}",
            "net/caffeinemc/Sodium.class", "")));
        assertEquals(Kind.MOD, f.kind);
        assertEquals(LoaderSupport.FABRIC, f.loader);
        assertEquals("Sodium", f.name);
        assertEquals("0.9.2", f.version);
        assertEquals(Collections.singletonList(">=26.3 <26.4"), f.minecraftVersions);
        assertNull(f.problemFor(LoaderSupport.FABRIC, "26.3"));
        assertNull(f.problemFor(LoaderSupport.QUILT, "26.3"));
        assertTrue(f.problemFor(LoaderSupport.FABRIC, "26.2").contains("Minecraft"));
        assertTrue(f.problemFor(LoaderSupport.NEOFORGE, "26.3").contains("Fabric"));
        assertTrue(f.problemFor(null, "26.3").contains("loader"));
    }

    @Test
    public void fabricVersionListsMeanAnyOfThem() throws IOException {
        DroppedFile f = DroppedFileDetector.detect(zip("multi.jar", entries("fabric.mod.json",
            "{\"name\":\"Multi\",\"version\":\"${version}\",\"depends\":{\"minecraft\":[\"1.20.1\",\"1.20.4\"]}}")));
        assertNull(f.version);
        assertEquals(Arrays.asList("1.20.1", "1.20.4"), f.minecraftVersions);
        assertNull(f.problemFor(LoaderSupport.FABRIC, "1.20.4"));
        assertTrue(f.problemFor(LoaderSupport.FABRIC, "1.21").contains("1.20.1 or 1.20.4"));
    }

    @Test
    public void quiltMod() throws IOException {
        DroppedFile f = DroppedFileDetector.detect(zip("q.jar", entries("quilt.mod.json",
            "{\"quilt_loader\":{\"id\":\"q\",\"version\":\"2.0\",\"metadata\":{\"name\":\"Quilty\"},"
                + "\"depends\":[\"quilt_loader\",{\"id\":\"minecraft\",\"versions\":\">=1.20\"}]}}")));
        assertEquals(Kind.MOD, f.kind);
        assertEquals(LoaderSupport.QUILT, f.loader);
        assertEquals("Quilty", f.name);
        assertEquals("2.0", f.version);
        assertEquals(Collections.singletonList(">=1.20"), f.minecraftVersions);
        assertTrue(f.problemFor(LoaderSupport.FABRIC, "1.21.1").contains("Quilt"));
    }

    @Test
    public void neoForgeModFromItsOwnToml() throws IOException {
        DroppedFile f = DroppedFileDetector.detect(zip("neo.jar", entries("META-INF/neoforge.mods.toml",
            "modLoader=\"javafml\"\nloaderVersion=\"[4,)\"\n[[mods]]\nmodId=\"neo\"\nversion=\"1.4.0\" # comment\n"
                + "displayName=\"Neo Thing\"\n[[dependencies.neo]]\nmodId=\"neoforge\"\ntype=\"required\"\n"
                + "versionRange=\"[21.1,)\"\n[[dependencies.neo]]\nmodId=\"minecraft\"\ntype=\"required\"\n"
                + "versionRange=\"[1.21.1,1.22)\"\n")));
        assertEquals(LoaderSupport.NEOFORGE, f.loader);
        assertEquals("Neo Thing", f.name);
        assertEquals("1.4.0", f.version);
        assertEquals(Collections.singletonList("[1.21.1,1.22)"), f.minecraftVersions);
        assertNull(f.problemFor(LoaderSupport.NEOFORGE, "1.21.1"));
        assertTrue(f.problemFor(LoaderSupport.NEOFORGE, "1.20.1").contains("Minecraft"));
    }

    @Test
    public void forgeModsTomlAndNeoForgeOnesThatStillUseIt() throws IOException {
        String forgeToml = "[[mods]]\nmodId='jei'\nversion='${file.jarVersion}'\ndisplayName='Just Enough Items'\n"
            + "[[dependencies.jei]]\nmodId='forge'\nversionRange='[47,)'\n"
            + "[[dependencies.jei]]\nmodId='minecraft'\nversionRange='[1.20.1]'\n";
        DroppedFile forge = DroppedFileDetector.detect(zip("jei.jar", entries("META-INF/mods.toml", forgeToml)));
        assertEquals(LoaderSupport.FORGE, forge.loader);
        assertEquals("Just Enough Items", forge.name);
        assertNull(forge.version);
        assertNull(forge.problemFor(LoaderSupport.FORGE, "1.20.1"));
        // NeoForge ran Forge mods on 1.20.1
        assertNull(forge.problemFor(LoaderSupport.NEOFORGE, "1.20.1"));
        assertTrue(forge.problemFor(LoaderSupport.FABRIC, "1.20.1").contains("Forge"));

        DroppedFile oldNeo = DroppedFileDetector.detect(zip("oldneo.jar", entries("META-INF/mods.toml",
            "[[mods]]\nmodId=\"x\"\n[[dependencies.x]]\nmodId=\"neoforge\"\nversionRange=\"[20.4,)\"\n")));
        assertEquals(LoaderSupport.NEOFORGE, oldNeo.loader);
    }

    @Test
    public void oldForgeModWithMcmodInfo() throws IOException {
        DroppedFile f = DroppedFileDetector.detect(zip("old.jar", entries("mcmod.info",
            "[{\"modid\":\"old\",\"name\":\"Old Mod\",\"version\":\"1.0\",\"mcversion\":\"1.7.10\"}]")));
        assertEquals(Kind.MOD, f.kind);
        assertEquals(LoaderSupport.FORGE, f.loader);
        assertEquals("Old Mod", f.name);
        assertEquals(Collections.singletonList("1.7.10"), f.minecraftVersions);
        assertTrue(f.problemFor(LoaderSupport.FORGE, "1.12.2").contains("1.7.10"));
    }

    @Test
    public void liteLoaderAndPlainJars() throws IOException {
        DroppedFile lite = DroppedFileDetector.detect(zip("voxel.litemod", entries("litemod.json",
            "{\"name\":\"VoxelMap\",\"version\":\"1.7\",\"mcversion\":\"1.12.2\"}")));
        assertEquals(Kind.MOD, lite.kind);
        assertEquals(DroppedFile.LITELOADER, lite.loader);
        assertTrue(lite.problemFor(LoaderSupport.FORGE, "1.12.2").contains("LiteLoader"));

        DroppedFile library = DroppedFileDetector.detect(zip("library.jar", entries("a/B.class", "")));
        assertEquals(Kind.MOD, library.kind);
        assertNull(library.loader);
        assertNull(library.problemFor(LoaderSupport.FABRIC, "26.3"));
    }

    @Test
    public void resourceDataAndShaderPacks() throws IOException {
        DroppedFile rp = DroppedFileDetector.detect(zip("Faithful.zip", entries("pack.mcmeta",
            "{\"pack\":{\"pack_format\":46,\"description\":\"Faithful 32x\"}}", "assets/minecraft/textures/a.png", "")));
        assertEquals(Kind.RESOURCE_PACK, rp.kind);
        assertEquals(ContentKind.RESOURCE_PACKS, rp.kind.contentKind());
        assertEquals("Faithful 32x", rp.version);

        DroppedFile dp = DroppedFileDetector.detect(zip("Terralith.zip", entries("pack.mcmeta", "{}",
            "data/terralith/worldgen/x.json", "{}")));
        assertEquals(Kind.DATA_PACK, dp.kind);
        assertNull(dp.kind.contentKind());
        assertTrue(dp.kind.goesIntoInstance());

        DroppedFile flat = DroppedFileDetector.detect(zip("BSL.zip", entries("shaders/", null,
            "shaders/composite.fsh", "void main(){}")));
        assertEquals(Kind.SHADER_PACK, flat.kind);
        DroppedFile wrapped = DroppedFileDetector.detect(zip("Complementary.zip", entries("Complementary/", null,
            "Complementary/shaders/final.fsh", "")));
        assertEquals(Kind.SHADER_PACK, wrapped.kind);
        assertEquals(ContentKind.SHADERS, wrapped.kind.contentKind());
        assertNull(wrapped.problemFor(LoaderSupport.FABRIC, "26.3"));
        assertTrue(wrapped.problemFor(null, "26.3").contains("Iris"));
    }

    @Test
    public void modpacksOfEveryFormat() throws IOException {
        DroppedFile mrpack = DroppedFileDetector.detect(zip("Fabulously Optimized.mrpack", entries(
            "modrinth.index.json", "{\"formatVersion\":1,\"name\":\"Fabulously Optimized\",\"versionId\":\"6.0\"}")));
        assertEquals(Kind.MODRINTH_MODPACK, mrpack.kind);
        assertEquals("Fabulously Optimized", mrpack.name);
        assertTrue(mrpack.kind.isModpack());
        assertFalse(mrpack.kind.goesIntoInstance());

        DroppedFile mrzip = DroppedFileDetector.detect(zip("pack.zip", entries("modrinth.index.json", "{}")));
        assertEquals(Kind.MODRINTH_MODPACK, mrzip.kind);

        DroppedFile curse = DroppedFileDetector.detect(zip("All the Mods.zip", entries("manifest.json",
            "{\"minecraft\":{\"version\":\"1.20.1\",\"modLoaders\":[{\"id\":\"forge-47.2.0\"}]},"
                + "\"name\":\"All the Mods 9\",\"version\":\"0.2\",\"files\":[]}",
            "overrides/config/x.toml", "")));
        assertEquals(Kind.CURSEFORGE_MODPACK, curse.kind);
        assertEquals("All the Mods 9", curse.name);

        DroppedFile prism = DroppedFileDetector.detect(zip("Exported.zip", entries("My Pack/", null,
            "My Pack/mmc-pack.json", "{}", "My Pack/instance.cfg", "name=My Pack")));
        assertEquals(Kind.PRISM_EXPORT, prism.kind);
        DroppedFile multimc = DroppedFileDetector.detect(zip("Old.zip", entries("instance.cfg", "name=x")));
        assertEquals(Kind.PRISM_EXPORT, multimc.kind);

        // a manifest.json that isn't CurseForge's is not a modpack
        DroppedFile other = DroppedFileDetector.detect(zip("other.zip", entries("manifest.json", "{\"a\":1}")));
        assertEquals(Kind.UNKNOWN, other.kind);
    }

    @Test
    public void unrecognisedFilesAreUnknown() throws IOException {
        Path text = temp.resolve("notes.txt");
        Files.write(text, "hi".getBytes(StandardCharsets.UTF_8));
        assertEquals(Kind.UNKNOWN, DroppedFileDetector.detect(text).kind);

        Path broken = temp.resolve("broken.jar");
        Files.write(broken, new byte[] { 1, 2, 3 });
        assertEquals(Kind.UNKNOWN, DroppedFileDetector.detect(broken).kind);

        assertEquals(Kind.UNKNOWN,
            DroppedFileDetector.detect(zip("random.zip", entries("photos/cat.jpg", "x"))).kind);
    }

    @Test
    public void badMetadataStillCountsAsAMod() throws IOException {
        DroppedFile f = DroppedFileDetector.detect(zip("broken-meta.jar", entries("fabric.mod.json", "{ not json")));
        assertEquals(Kind.MOD, f.kind);
        assertEquals(LoaderSupport.FABRIC, f.loader);
        assertNull(f.name);
        assertEquals("broken-meta.jar", f.displayName());
    }

    @Test
    public void keepBothNamesCountUp() {
        assertEquals("sodium (2).jar", ContentService.uniqueName("sodium.jar", n -> false));
        assertEquals("sodium (3).jar", ContentService.uniqueName("sodium.jar", n -> n.equals("sodium (2).jar")));
        assertEquals("README (2)", ContentService.uniqueName("README", n -> false));
    }
}
