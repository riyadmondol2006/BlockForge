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

import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import java.util.stream.Collectors;

import org.junit.jupiter.api.Test;

import com.blockforge.launcher.data.modrinth.ModrinthChannel;
import com.blockforge.launcher.data.modrinth.ModrinthVersion;

public class VersionPickerTest {
    private static ModrinthVersion version(String id, String date, ModrinthChannel channel, List<String> loaders,
        String... games) {
        ModrinthVersion v = new ModrinthVersion();
        v.id = id;
        v.datePublished = date;
        v.versionType = channel;
        v.loaders = loaders;
        v.gameVersions = Arrays.asList(games);
        return v;
    }

    private static final List<String> FABRIC = Collections.singletonList("fabric");
    private static final List<String> NEOFORGE = Collections.singletonList("neoforge");

    @Test
    public void picksTheNewestReleaseForTheExactVersionAndLoader() {
        List<ModrinthVersion> versions = Arrays.asList(
            version("old", "2026-01-01T00:00:00Z", ModrinthChannel.RELEASE, FABRIC, "26.3"),
            version("new", "2026-06-01T00:00:00Z", ModrinthChannel.RELEASE, FABRIC, "26.3"),
            version("neo", "2026-07-01T00:00:00Z", ModrinthChannel.RELEASE, NEOFORGE, "26.3"),
            version("other-mc", "2026-08-01T00:00:00Z", ModrinthChannel.RELEASE, FABRIC, "26.2"));
        assertEquals("new", VersionPicker.pick(versions, FABRIC, "26.3", null).id);
        assertEquals("neo", VersionPicker.pick(versions, NEOFORGE, "26.3", null).id);
    }

    @Test
    public void prefersAReleaseOverANewerBeta() {
        List<ModrinthVersion> versions = Arrays.asList(
            version("beta", "2026-09-01T00:00:00Z", ModrinthChannel.BETA, FABRIC, "26.3"),
            version("release", "2026-05-01T00:00:00Z", ModrinthChannel.RELEASE, FABRIC, "26.3"));
        assertEquals("release", VersionPicker.pick(versions, FABRIC, "26.3", null).id);

        List<ModrinthVersion> onlyBeta = Collections.singletonList(versions.get(0));
        assertEquals("beta", VersionPicker.pick(onlyBeta, FABRIC, "26.3", null).id);
    }

    @Test
    public void quiltAcceptsFabricBuilds() {
        List<String> quilt = LoaderSupport.acceptedLoaders(LoaderSupport.QUILT, "26.3", null);
        List<ModrinthVersion> versions = Collections
            .singletonList(version("f", "2026-05-01T00:00:00Z", ModrinthChannel.RELEASE, FABRIC, "26.3"));
        assertEquals("f", VersionPicker.pick(versions, quilt, "26.3", null).id);
        assertNull(VersionPicker.pick(versions, Collections.singletonList("forge"), "26.3", null));
    }

    @Test
    public void nothingForTheExactVersionMeansNothingUnlessNearbyVersionsAreAllowed() {
        List<ModrinthVersion> versions = Collections.singletonList(
            version("near", "2026-05-01T00:00:00Z", ModrinthChannel.RELEASE, FABRIC, "26.2"));
        assertNull(VersionPicker.pick(versions, FABRIC, "26.2.1", Collections.<String>emptyList()));
        assertEquals("near", VersionPicker.pick(versions, FABRIC, "26.2.1", Arrays.asList("26.2", "26.2.1")).id);
    }

    @Test
    public void packsIgnoreLoaders() {
        List<ModrinthVersion> versions = Collections.singletonList(version("shader", "2026-05-01T00:00:00Z",
            ModrinthChannel.RELEASE, Arrays.asList("iris", "optifine"), "26.3"));
        assertEquals("shader", VersionPicker.pick(versions, Collections.<String>emptyList(), "26.3", null).id);
        assertEquals("shader", VersionPicker.pick(versions, null, "26.3", null).id);
    }

    @Test
    public void compatibleListsBestFirstWithoutRepeats() {
        List<ModrinthVersion> versions = Arrays.asList(
            version("beta-new", "2026-09-01T00:00:00Z", ModrinthChannel.BETA, FABRIC, "26.3"),
            version("rel-old", "2026-01-01T00:00:00Z", ModrinthChannel.RELEASE, FABRIC, "26.3"),
            version("rel-new", "2026-06-01T00:00:00Z", ModrinthChannel.RELEASE, FABRIC, "26.3"));
        List<String> ids = VersionPicker.compatible(versions, FABRIC, "26.3", null).stream().map(v -> v.id)
            .collect(Collectors.toList());
        assertEquals(Arrays.asList("rel-new", "rel-old", "beta-new"), ids);
        assertTrue(VersionPicker.compatible(null, FABRIC, "26.3", null).isEmpty());
    }

    @Test
    public void tellsWhenAVersionIsNewer() {
        ModrinthVersion installed = version("a", "2026-01-01T00:00:00Z", ModrinthChannel.RELEASE, FABRIC, "26.3");
        ModrinthVersion newer = version("b", "2026-02-01T00:00:00Z", ModrinthChannel.RELEASE, FABRIC, "26.3");
        ModrinthVersion same = version("a", "2026-01-01T00:00:00Z", ModrinthChannel.RELEASE, FABRIC, "26.3");
        assertTrue(VersionPicker.isNewer(newer, installed));
        assertFalse(VersionPicker.isNewer(installed, newer));
        assertFalse(VersionPicker.isNewer(same, installed));
        assertFalse(VersionPicker.isNewer(null, installed));
        assertTrue(VersionPicker.isNewer(newer, null));
    }
}
