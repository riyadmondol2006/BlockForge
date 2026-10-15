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
package com.blockforge.launcher.data.minecraft.loaders;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.Reader;
import java.nio.charset.StandardCharsets;
import java.util.regex.Pattern;

import org.junit.jupiter.api.Test;

import com.blockforge.launcher.Gsons;
import com.blockforge.launcher.data.minecraft.Library;
import com.blockforge.launcher.data.minecraft.loaders.fabric.FabricMetaProfile;
import com.blockforge.launcher.data.minecraft.loaders.forge.ForgePromotions;
import com.blockforge.launcher.utils.Utils;

/**
 * What the launcher relies on from loader metadata, checked as rules instead of exact values, so a harmless change
 * upstream doesn't fail them but a change that would break installs does.
 */
class LoaderContractTest {
    private static final Pattern HTTPS_URL = Pattern.compile("^https://[^\\s]+\\.jar$");

    private static <T> T load(String resource, Class<T> type) throws IOException {
        try (InputStream in = LoaderContractTest.class.getResourceAsStream("/loaders/" + resource)) {
            assertNotNull(in, "missing fixture " + resource);
            try (Reader reader = new InputStreamReader(in, StandardCharsets.UTF_8)) {
                return Gsons.DEFAULT.fromJson(reader, type);
            }
        }
    }

    @Test
    void fabricProfileHasEverythingAnInstallNeeds() throws IOException {
        FabricMetaProfile profile = load("fabric-profile.json", FabricMetaProfile.class);

        assertFalse(profile.mainClass == null || profile.mainClass.isEmpty());
        assertFalse(profile.inheritsFrom == null || profile.inheritsFrom.isEmpty());
        assertFalse(profile.libraries.isEmpty());
        assertTrue(profile.libraries.stream().anyMatch(l -> l.name.startsWith("net.fabricmc:fabric-loader:")));
    }

    @Test
    void everyLibraryResolvesToADownloadableUrlThatMatchesItsName() throws IOException {
        FabricMetaProfile profile = load("fabric-profile.json", FabricMetaProfile.class);

        for (Library library : profile.libraries) {
            assertNotNull(library.downloads.artifact, library.name);
            String url = library.downloads.artifact.url;
            String path = library.downloads.artifact.path;

            assertTrue(HTTPS_URL.matcher(url).matches(), "not a usable URL for " + library.name + ": " + url);
            assertTrue(url.endsWith(path), url + " doesn't end with " + path);

            // group/artifact/version/artifact-version.jar, built from the name and never guessed
            String[] parts = library.name.split(":");
            String group = parts[0].replace('.', '/');
            assertTrue(path.startsWith(group + "/" + parts[1] + "/" + parts[2] + "/"), path);
            assertTrue(path.endsWith(parts[1] + "-" + parts[2] + ".jar"), path);
        }
    }

    @Test
    void unknownFieldsFromUpstreamAreIgnoredAndOptionalOnesMayBeMissing() throws IOException {
        FabricMetaProfile profile = Gsons.DEFAULT.fromJson("{\"id\":\"x\",\"mainClass\":\"a.B\",\"libraries\":[],\"brandNew\":{\"a\":1}}",
            FabricMetaProfile.class);

        assertEquals("a.B", profile.mainClass);
        assertTrue(profile.libraries.isEmpty());
    }

    @Test
    void mavenNamesBecomeRepositoryPaths() {
        assertEquals("net/fabricmc/fabric-loader/0.15.7/fabric-loader-0.15.7.jar",
            Utils.convertMavenIdentifierToPath("net.fabricmc:fabric-loader:0.15.7"));
        assertEquals("org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-linux.jar",
            Utils.convertMavenIdentifierToPath("org.lwjgl:lwjgl:3.3.1:natives-linux"));
        assertEquals("net/neoforged/neoforge/21.1.1/neoforge-21.1.1-installer.jar",
            Utils.convertMavenIdentifierToPath("net.neoforged:neoforge:21.1.1:installer"));
        assertEquals("com/example/lib/1.0/lib-1.0.zip", Utils.convertMavenIdentifierToPath("com.example:lib:1.0@zip"));
    }

    @Test
    void forgePromotionsAreLookedUpByMinecraftVersionAndKind() throws IOException {
        ForgePromotions promotions = load("forge-promotions.json", ForgePromotions.class);

        assertTrue(promotions.hasPromo("1.20.1-recommended"));
        assertEquals("47.2.0", promotions.getPromo("1.20.1-recommended"));
        assertEquals("47.2.20", promotions.getPromo("1.20.1-latest"));
        assertFalse(promotions.hasPromo("1.99-latest"));
        // old Minecraft versions keep four-part Forge versions
        assertEquals("10.13.4.1614", promotions.getPromo("1.7.10-latest"));
    }

    @Test
    void newMinecraftVersionNamesCompareCorrectly() {
        // Minecraft moved from 1.x to year based names such as 26.1
        assertTrue(Utils.compareVersions("26.1", "1.21.11") == 1);
        assertTrue(Utils.compareVersions("26.1.2", "26.1") == 1);
        assertTrue(Utils.compareVersions("1.20.1", "1.20.1") == 0);
        assertTrue(Utils.compareVersions("1.20", "1.20.1") == -1);
    }
}
