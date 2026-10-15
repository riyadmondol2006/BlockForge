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
import static org.junit.jupiter.api.Assertions.assertNull;

import java.io.IOException;
import java.io.OutputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import com.blockforge.launcher.data.Type;
import com.blockforge.launcher.data.modrinth.ModrinthProjectType;

public class ContentRoutingTest {
    @TempDir
    Path temp;

    @Test
    public void jarsAreAlwaysMods() {
        assertEquals(ContentKind.MODS, ContentRouting.route("sodium-0.6.jar", null, ContentKind.SHADERS));
        assertEquals(ContentKind.MODS, ContentRouting.route("Old.LITEMOD", null, ContentKind.RESOURCE_PACKS));
        assertEquals(ContentKind.MODS, ContentRouting.route("Upper.JAR", null, ContentKind.MODS));
    }

    @Test
    public void shaderPacksAreRecognisedEvenInsideTheirOwnFolder() {
        List<String> flat = Arrays.asList("shaders/", "shaders/composite.fsh", "shaders/lang/en_us.lang");
        List<String> wrapped = Arrays.asList("Complementary/", "Complementary/shaders/final.fsh");
        assertEquals(ContentKind.SHADERS, ContentRouting.route("BSL.zip", flat, ContentKind.MODS));
        assertEquals(ContentKind.SHADERS, ContentRouting.route("Comp.zip", wrapped, ContentKind.RESOURCE_PACKS));
    }

    @Test
    public void resourcePacksAreRecognisedFromTheirMetadata() {
        List<String> pack = Arrays.asList("pack.mcmeta", "pack.png", "assets/minecraft/textures/block/stone.png");
        assertEquals(ContentKind.RESOURCE_PACKS, ContentRouting.route("Faithful.zip", pack, ContentKind.MODS));
        // a resource pack that also ships core shaders is still a resource pack
        List<String> withShaders = Arrays.asList("pack.mcmeta", "assets/minecraft/shaders/core/x.json");
        assertEquals(ContentKind.RESOURCE_PACKS, ContentRouting.route("Fancy.zip", withShaders, ContentKind.SHADERS));
    }

    @Test
    public void dataPacksAndOtherFilesAreRejected() {
        List<String> dataPack = Arrays.asList("pack.mcmeta", "data/minecraft/tags/blocks/x.json");
        assertNull(ContentRouting.route("Terralith.zip", dataPack, ContentKind.RESOURCE_PACKS));
        assertNull(ContentRouting.route("readme.txt", null, ContentKind.MODS));
        assertNull(ContentRouting.route(null, null, ContentKind.MODS));
    }

    @Test
    public void unknownZipsFollowThePackTabTheyWereDroppedOn() {
        List<String> unknown = Collections.singletonList("something/else.txt");
        assertNull(ContentRouting.route("mystery.zip", unknown, ContentKind.MODS));
        assertEquals(ContentKind.SHADERS, ContentRouting.route("mystery.zip", unknown, ContentKind.SHADERS));
        assertEquals(ContentKind.RESOURCE_PACKS, ContentRouting.route("broken.zip", null, ContentKind.RESOURCE_PACKS));
        assertNull(ContentRouting.route("broken.zip", null, ContentKind.MODS));
    }

    @Test
    public void zipModsAreMods() {
        assertEquals(ContentKind.MODS,
            ContentRouting.route("old-forge-mod.zip", Arrays.asList("mcmod.info", "a/B.class"), ContentKind.SHADERS));
    }

    @Test
    public void readsTheNamesInsideARealZip() throws IOException {
        Path zip = temp.resolve("pack.zip");
        try (OutputStream out = Files.newOutputStream(zip); ZipOutputStream z = new ZipOutputStream(out)) {
            z.putNextEntry(new ZipEntry("pack.mcmeta"));
            z.write("{}".getBytes("UTF-8"));
            z.closeEntry();
            z.putNextEntry(new ZipEntry("assets/minecraft/lang/en_us.json"));
            z.closeEntry();
        }
        List<String> entries = ContentRouting.zipEntries(zip);
        assertEquals(Arrays.asList("pack.mcmeta", "assets/minecraft/lang/en_us.json"), entries);
        assertEquals(ContentKind.RESOURCE_PACKS, ContentRouting.route("pack.zip", entries, ContentKind.MODS));

        Path notZip = temp.resolve("fake.zip");
        Files.write(notZip, new byte[] { 1, 2, 3 });
        assertNull(ContentRouting.zipEntries(notZip));
    }

    @Test
    public void everyKindHasItsFolderAndTypes() {
        assertEquals("mods", ContentKind.MODS.folder);
        assertEquals("resourcepacks", ContentKind.RESOURCE_PACKS.folder);
        assertEquals("shaderpacks", ContentKind.SHADERS.folder);
        assertEquals(ContentKind.SHADERS, ContentKind.of(Type.shaderpack));
        assertEquals(ContentKind.RESOURCE_PACKS, ContentKind.of(ModrinthProjectType.RESOURCEPACK));
        assertNull(ContentKind.of(Type.datapack));
        assertNull(ContentKind.of(ModrinthProjectType.MODPACK));
    }
}
