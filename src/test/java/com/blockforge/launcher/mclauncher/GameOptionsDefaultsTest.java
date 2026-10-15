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
package com.blockforge.launcher.mclauncher;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Map;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import com.blockforge.launcher.utils.PcProfile.Tier;

public class GameOptionsDefaultsTest {
    static Path jarWithDataVersion(Path dir, int dataVersion) throws Exception {
        Path jar = dir.resolve("client.jar");
        try (OutputStream out = Files.newOutputStream(jar); ZipOutputStream zip = new ZipOutputStream(out)) {
            zip.putNextEntry(new ZipEntry("version.json"));
            zip.write(("{\"id\":\"1.21.11\",\"world_version\":" + dataVersion + ",\"java_version\":21}")
                .getBytes(StandardCharsets.UTF_8));
            zip.closeEntry();
        }
        return jar;
    }

    @Test
    public void testALowEndComputerGetsLightSettings() {
        Map<String, String> o = GameOptionsDefaults.forTier(Tier.LOW, "1.21.11");
        assertEquals("6", o.get("renderDistance"));
        assertEquals("5", o.get("simulationDistance"));
        assertEquals("0", o.get("graphicsMode"), "fast");
        assertEquals("1", o.get("particles"), "decreased");
        assertEquals("60", o.get("maxFps"));
        assertEquals("false", o.get("entityShadows"));
    }

    @Test
    public void testMediumAndHigh() {
        Map<String, String> medium = GameOptionsDefaults.forTier(Tier.MEDIUM, "26.3");
        assertEquals("10", medium.get("renderDistance"));
        assertEquals("8", medium.get("simulationDistance"));
        assertEquals("1", medium.get("graphicsMode"), "fancy");
        assertEquals("0", medium.get("particles"));

        Map<String, String> high = GameOptionsDefaults.forTier(Tier.HIGH, "1.21.11");
        assertEquals("14", high.get("renderDistance"));
        assertEquals("1", high.get("graphicsMode"), "fancy, not fabulous");
        assertEquals("260", high.get("maxFps"), "unlimited");
    }

    @Test
    public void testTheOptionNamesFollowTheVersion() {
        // before 1.16 graphics were a true/false "fancyGraphics"; simulation distance only exists since 1.18
        Map<String, String> v1122 = GameOptionsDefaults.forTier(Tier.LOW, "1.12.2");
        assertEquals("false", v1122.get("fancyGraphics"));
        assertFalse(v1122.containsKey("graphicsMode"));
        assertFalse(v1122.containsKey("simulationDistance"));

        Map<String, String> v1165 = GameOptionsDefaults.forTier(Tier.LOW, "1.16.5");
        assertEquals("0", v1165.get("graphicsMode"));
        assertFalse(v1165.containsKey("fancyGraphics"));
        assertFalse(v1165.containsKey("simulationDistance"));

        Map<String, String> v1182 = GameOptionsDefaults.forTier(Tier.LOW, "1.18.2");
        assertTrue(v1182.containsKey("simulationDistance"));

        // 1.7 has no maxFps or entityShadows yet; older ones aren't touched at all
        Map<String, String> v1710 = GameOptionsDefaults.forTier(Tier.LOW, "1.7.10");
        assertTrue(v1710.containsKey("renderDistance"));
        assertFalse(v1710.containsKey("maxFps"));
        assertTrue(GameOptionsDefaults.forTier(Tier.LOW, "1.6.4").isEmpty());
        assertTrue(GameOptionsDefaults.forTier(Tier.LOW, "b1.7.3").isEmpty());
    }

    @Test
    public void testGamesWithGraphicsPresetsGetAPreset() {
        // 1.21.11 and 26.x apply the preset when they start, overriding the single settings unless it is "custom"
        Map<String, String> low = GameOptionsDefaults.forTier(Tier.LOW, "26.3", true);
        assertEquals("\"fast\"", low.get("graphicsPreset"));
        assertEquals("60", low.get("maxFps"));
        assertFalse(low.containsKey("graphicsMode"));

        Map<String, String> medium = GameOptionsDefaults.forTier(Tier.MEDIUM, "26.3", true);
        assertEquals("\"custom\"", medium.get("graphicsPreset"));
        assertEquals("10", medium.get("renderDistance"));
        assertEquals("8", medium.get("simulationDistance"));

        assertEquals("\"fancy\"", GameOptionsDefaults.forTier(Tier.HIGH, "1.21.11", true).get("graphicsPreset"));
    }

    @Test
    public void testPresetsAreFoundInTheGamesTexts(@TempDir Path dir) throws Exception {
        Path with = dir.resolve("with.jar");
        try (OutputStream out = Files.newOutputStream(with); ZipOutputStream zip = new ZipOutputStream(out)) {
            zip.putNextEntry(new ZipEntry("version.json"));
            zip.write("{\"world_version\":5023}".getBytes(StandardCharsets.UTF_8));
            zip.closeEntry();
            zip.putNextEntry(new ZipEntry("assets/minecraft/lang/en_us.json"));
            zip.write("{\"options.graphics.preset\":\"Preset\"}".getBytes(StandardCharsets.UTF_8));
            zip.closeEntry();
        }
        assertTrue(GameOptionsDefaults.hasGraphicsPresets(with));
        assertFalse(GameOptionsDefaults.hasGraphicsPresets(jarWithDataVersion(dir, 3465)));

        Path game = dir.resolve("game");
        Map<String, String> written = GameOptionsDefaults.writeIfMissing(game, Tier.LOW, "26.3", with);
        assertEquals("\"fast\"", written.get("graphicsPreset"));
        String text = new String(Files.readAllBytes(game.resolve("options.txt")), StandardCharsets.UTF_8);
        assertEquals("version:5023\ngraphicsPreset:\"fast\"\nmaxFps:60\n", text);
    }

    @Test
    public void testTheFileStartsWithTheDataVersion() {
        String text = GameOptionsDefaults.toText(GameOptionsDefaults.forTier(Tier.LOW, "1.21.11"), 4671);
        assertTrue(text.startsWith("version:4671\n"));
        assertTrue(text.contains("renderDistance:6\n"));
        assertTrue(text.contains("graphicsMode:0\n"));
    }

    @Test
    public void testWritesOptionsForANewGame(@TempDir Path dir) throws Exception {
        Path game = dir.resolve("game");
        Path jar = jarWithDataVersion(dir, 4671);

        assertNotNull(GameOptionsDefaults.writeIfMissing(game, Tier.LOW, "1.21.11", jar));
        String written = new String(Files.readAllBytes(game.resolve("options.txt")), StandardCharsets.UTF_8);
        assertTrue(written.startsWith("version:4671\n"));
        assertTrue(written.contains("renderDistance:6"));
        assertTrue(written.contains("simulationDistance:5"));
    }

    @Test
    public void testNeverOverwritesThePlayersSettings(@TempDir Path dir) throws Exception {
        Path game = dir.resolve("game");
        Files.createDirectories(game);
        Path options = game.resolve("options.txt");
        Files.write(options, "version:4671\nrenderDistance:32\n".getBytes(StandardCharsets.UTF_8));

        assertNull(GameOptionsDefaults.writeIfMissing(game, Tier.LOW, "1.21.11", jarWithDataVersion(dir, 4671)));
        assertEquals("version:4671\nrenderDistance:32\n",
            new String(Files.readAllBytes(options), StandardCharsets.UTF_8));
    }

    @Test
    public void testLeavesModpackDefaultsToTheirMod(@TempDir Path dir) throws Exception {
        Path game = dir.resolve("game");
        Files.createDirectories(game.resolve("config/yosbr"));
        Files.write(game.resolve("config/yosbr/options.txt"), "renderDistance:8\n".getBytes(StandardCharsets.UTF_8));

        assertNull(GameOptionsDefaults.writeIfMissing(game, Tier.LOW, "1.21.11", jarWithDataVersion(dir, 4671)));
        assertFalse(Files.exists(game.resolve("options.txt")));
    }

    @Test
    public void testWithoutADataVersionModernGamesAreLeftAlone(@TempDir Path dir) throws Exception {
        Path game = dir.resolve("game");
        // no jar to read the data version from: writing would get the file "upgraded" wrongly, so don't
        assertNull(GameOptionsDefaults.writeIfMissing(game, Tier.LOW, "1.21.11", dir.resolve("missing.jar")));
        assertFalse(Files.exists(game.resolve("options.txt")));

        // 1.12 has no data versions at all
        assertNotNull(GameOptionsDefaults.writeIfMissing(game, Tier.LOW, "1.12.2", null));
        String written = new String(Files.readAllBytes(game.resolve("options.txt")), StandardCharsets.UTF_8);
        assertFalse(written.contains("version:"));
        assertTrue(written.contains("fancyGraphics:false"));
    }

    @Test
    public void testReadsTheDataVersion(@TempDir Path dir) throws Exception {
        assertEquals(Integer.valueOf(4671), GameOptionsDefaults.readDataVersion(jarWithDataVersion(dir, 4671)));
        assertNull(GameOptionsDefaults.readDataVersion(dir.resolve("nothing.jar")));
    }
}
