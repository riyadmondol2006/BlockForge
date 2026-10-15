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
package com.blockforge.launcher.update;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

import com.blockforge.launcher.Gsons;

class UpdatePlanTest {
    private static final int[] CURRENT = { 3, 4, 41, 4 };

    @Test
    void readsVersionTags() {
        assertArrayEquals(new int[] { 3, 4, 42, 0 }, UpdatePlan.parseVersion("v3.4.42.0"));
        assertArrayEquals(new int[] { 3, 4, 42, 0 }, UpdatePlan.parseVersion("3.4.42"));
        assertArrayEquals(new int[] { 3, 4, 42, 1 }, UpdatePlan.parseVersion("V3.4.42.1-rc1"));
        assertArrayEquals(new int[] { 3, 4, 41, 4 }, UpdatePlan.parseVersion("3.4.41.4.Beta"));
        assertNull(UpdatePlan.parseVersion("nightly"));
        assertNull(UpdatePlan.parseVersion("v1.2.3.4.5"));
        assertNull(UpdatePlan.parseVersion(""));
        assertNull(UpdatePlan.parseVersion(null));
    }

    @Test
    void onlyHigherVersionsAreUpdates() {
        assertTrue(UpdatePlan.isNewer("v3.4.41.5", CURRENT));
        assertTrue(UpdatePlan.isNewer("v3.4.42", CURRENT));
        assertTrue(UpdatePlan.isNewer("v3.5.0.0", CURRENT));
        assertTrue(UpdatePlan.isNewer("v4.0", CURRENT));
        // numbers compare as numbers, not text
        assertTrue(UpdatePlan.isNewer("v3.4.100.0", CURRENT));

        assertFalse(UpdatePlan.isNewer("v3.4.41.4", CURRENT), "the same version is never offered");
        assertFalse(UpdatePlan.isNewer("v3.4.41.3", CURRENT));
        assertFalse(UpdatePlan.isNewer("v3.4.9.9", CURRENT));
        assertFalse(UpdatePlan.isNewer("nightly", CURRENT));
    }

    @Test
    void detectsHowItWasInstalled() {
        String ubuntu = "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n";
        String mint = "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n";
        String fedora = "NAME=\"Fedora Linux\"\nID=fedora\n";
        String opensuse = "ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n";
        String arch = "ID=arch\n";

        assertEquals(UpdatePlan.Kind.WINDOWS_EXE, UpdatePlan.detectKind(true, "Windows 11", "C:\\x\\app.jar", false, null));
        assertEquals(UpdatePlan.Kind.MAC_DMG, UpdatePlan.detectKind(true, "Mac OS X", "/Applications/x.jar", false, null));
        assertEquals(UpdatePlan.Kind.DEB, UpdatePlan.detectKind(true, "Linux", "/opt/blockforge/lib/app/x.jar", false, ubuntu));
        assertEquals(UpdatePlan.Kind.DEB, UpdatePlan.detectKind(true, "Linux", "/opt/x.jar", false, mint));
        assertEquals(UpdatePlan.Kind.RPM, UpdatePlan.detectKind(true, "Linux", "/opt/x.jar", false, fedora));
        assertEquals(UpdatePlan.Kind.RPM, UpdatePlan.detectKind(true, "Linux", "/opt/x.jar", false, opensuse));
        assertEquals(UpdatePlan.Kind.PAGE_ONLY, UpdatePlan.detectKind(true, "Linux", "/opt/x.jar", false, arch));

        assertEquals(UpdatePlan.Kind.JAR, UpdatePlan.detectKind(false, "Linux", "/home/me/BlockForge-3.4.41.4.jar", false, fedora));
        assertEquals(UpdatePlan.Kind.JAR, UpdatePlan.detectKind(false, "Windows 10", "C:\\Games\\BlockForge.JAR", false, null));
        assertEquals(UpdatePlan.Kind.PAGE_ONLY, UpdatePlan.detectKind(false, "Linux", "/src/build/classes/java/main", false, fedora));
        assertEquals(UpdatePlan.Kind.PAGE_ONLY, UpdatePlan.detectKind(true, "Linux", "/app/x.jar", true, fedora));
    }

    @Test
    void picksTheFileForThisComputer() {
        Release release = release("BlockForge-3.4.42.0.jar", "BlockForge-3.4.42.0-windows-x64.exe",
            "BlockForge-3.4.42.0-macos-arm64.dmg", "BlockForge-3.4.42.0-macos-x64.dmg",
            "BlockForge-3.4.42.0-linux-amd64.deb", "BlockForge-3.4.42.0-linux-x86_64.rpm");

        assertEquals("BlockForge-3.4.42.0-macos-arm64.dmg", UpdatePlan.pickAsset(release, UpdatePlan.Kind.MAC_DMG, "aarch64").name);
        assertEquals("BlockForge-3.4.42.0-macos-x64.dmg", UpdatePlan.pickAsset(release, UpdatePlan.Kind.MAC_DMG, "x86_64").name);
        assertEquals("BlockForge-3.4.42.0-windows-x64.exe", UpdatePlan.pickAsset(release, UpdatePlan.Kind.WINDOWS_EXE, "amd64").name);
        assertEquals("BlockForge-3.4.42.0-linux-amd64.deb", UpdatePlan.pickAsset(release, UpdatePlan.Kind.DEB, "amd64").name);
        assertEquals("BlockForge-3.4.42.0-linux-x86_64.rpm", UpdatePlan.pickAsset(release, UpdatePlan.Kind.RPM, "amd64").name);
        assertEquals("BlockForge-3.4.42.0.jar", UpdatePlan.pickAsset(release, UpdatePlan.Kind.JAR, "aarch64").name);

        // an ARM Linux machine must never get the x86 package
        assertNull(UpdatePlan.pickAsset(release, UpdatePlan.Kind.DEB, "aarch64"));
        assertNull(UpdatePlan.pickAsset(release, UpdatePlan.Kind.PAGE_ONLY, "amd64"));
    }

    @Test
    void acceptsFilesThatDontNameACpu() {
        Release release = release("BlockForge-3.4.42.0-windows-setup.exe", "BlockForge-3.4.42.0-debian-ubuntu.deb",
            "BlockForge-3.4.42.0-sources.jar", "BlockForge-3.4.42.0.jar");

        assertEquals("BlockForge-3.4.42.0-windows-setup.exe", UpdatePlan.pickAsset(release, UpdatePlan.Kind.WINDOWS_EXE, "amd64").name);
        assertEquals("BlockForge-3.4.42.0-debian-ubuntu.deb", UpdatePlan.pickAsset(release, UpdatePlan.Kind.DEB, "amd64").name);
        assertEquals("BlockForge-3.4.42.0.jar", UpdatePlan.pickAsset(release, UpdatePlan.Kind.JAR, "amd64").name);
        assertNull(UpdatePlan.pickAsset(release, UpdatePlan.Kind.MAC_DMG, "aarch64"));
    }

    @Test
    void readsGitHubReleaseJson() {
        String json = "{\"tag_name\":\"v3.4.42.0\",\"name\":\"BlockForge 3.4.42.0\",\"draft\":false,\"prerelease\":false,"
            + "\"html_url\":\"https://github.com/o/r/releases/tag/v3.4.42.0\",\"body\":\"## Changes\\n- faster\","
            + "\"assets\":[{\"name\":\"BlockForge-3.4.42.0.jar\",\"size\":123,"
            + "\"browser_download_url\":\"https://github.com/o/r/releases/download/v3.4.42.0/BlockForge-3.4.42.0.jar\","
            + "\"digest\":\"sha256:ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789\"}]}";
        Release release = Gsons.DEFAULT.fromJson(json, Release.class);

        assertEquals("v3.4.42.0", release.tagName);
        assertFalse(release.prerelease);
        assertEquals(1, release.assets.size());
        Release.Asset asset = release.assets.get(0);
        assertEquals(123, asset.size);
        assertTrue(asset.url.endsWith("/BlockForge-3.4.42.0.jar"));
        assertEquals("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789", asset.sha256());

        asset.digest = "md5:abc";
        assertNull(asset.sha256());
        asset.digest = null;
        assertNull(asset.sha256());
    }

    @Test
    void hexMatchesKnownDigest() throws Exception {
        byte[] digest = java.security.MessageDigest.getInstance("SHA-256").digest("abc".getBytes("UTF-8"));
        assertEquals("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", LauncherUpdates.hex(digest));
    }

    private static Release release(String... names) {
        Release release = new Release();
        release.tagName = "v3.4.42.0";
        for (String name : names) {
            release.assets.add(new Release.Asset(name, "https://example.invalid/" + name, 10));
        }
        return release;
    }
}
