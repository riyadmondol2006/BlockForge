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

import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

import com.blockforge.launcher.utils.PcProfile;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * Graphics settings for a brand new game, picked for this computer, so a budget laptop doesn't start at Minecraft's
 * 12 chunks and fancy graphics and a gaming PC isn't held back. Written only when the game has no options.txt yet:
 * the player's own settings, and a modpack's, are never touched.
 */
public final class GameOptionsDefaults {
    private GameOptionsDefaults() {
    }

    /**
     * The settings for a computer of this tier, with the option names this Minecraft version uses. Empty for
     * versions older than 1.7, whose options are too different to be worth it.
     */
    public static Map<String, String> forTier(PcProfile.Tier tier, String minecraftVersion) {
        return forTier(tier, minecraftVersion, false);
    }

    /**
     * @param graphicsPresets if the game has graphics presets (1.21.11 and newer): there the preset decides most
     *                        settings when the game starts, so the preset is what has to be chosen
     */
    public static Map<String, String> forTier(PcProfile.Tier tier, String minecraftVersion, boolean graphicsPresets) {
        Map<String, String> options = new LinkedHashMap<>();
        int minor = AutoMemory.minorVersion(minecraftVersion);
        if (minor < 7) {
            return options;
        }

        if (graphicsPresets) {
            switch (tier) {
                case LOW:
                    // Mojang's own "Fast" preset: 8 chunks, simulation 6, decreased particles, no entity shadows,
                    // no smooth lighting; and 60 FPS to keep a laptop cool
                    options.put("graphicsPreset", "\"fast\"");
                    options.put("maxFps", "60");
                    return options;
                case HIGH:
                    // "Fancy" (16 chunks, simulation 12) is what the game picks itself, and right for a gaming PC
                    options.put("graphicsPreset", "\"fancy\"");
                    options.put("maxFps", "260");
                    return options;
                case MEDIUM:
                default:
                    // "Fancy" with a shorter view: Fancy's 16 chunks are too much for most laptops
                    options.put("graphicsPreset", "\"custom\"");
                    options.put("renderDistance", "10");
                    options.put("simulationDistance", "8");
                    options.put("particles", "0");
                    options.put("maxFps", "120");
                    options.put("entityShadows", "true");
                    return options;
            }
        }

        int render;
        int simulation;
        boolean fancy;
        int particles; // 0 all, 1 decreased, 2 minimal
        int maxFps; // 260 means unlimited
        boolean shadows;
        switch (tier) {
            case LOW:
                // a budget laptop: half of the default view, fast graphics and fewer particles keep it smooth, and
                // 60 FPS keeps it cool and quiet
                render = 6;
                simulation = 5;
                fancy = false;
                particles = 1;
                maxFps = 60;
                shadows = false;
                break;
            case HIGH:
                // a gaming PC: a little more view than the default (each step adds a ring of chunks, so 14 is about a
                // third more than 12 and fits in the automatic memory); fabulous graphics stay off, as they cost a
                // lot and clash with shader packs
                render = minor <= 12 ? 12 : 14;
                simulation = 12;
                fancy = true;
                particles = 0;
                maxFps = 260;
                shadows = true;
                break;
            case MEDIUM:
            default:
                // a bit under Minecraft's own defaults (12 chunks), which are too much for most laptops
                render = 10;
                simulation = 8;
                fancy = true;
                particles = 0;
                maxFps = 120;
                shadows = true;
                break;
        }

        options.put("renderDistance", String.valueOf(render));
        if (minor >= 18) {
            options.put("simulationDistance", String.valueOf(simulation));
        }
        if (minor >= 16) {
            options.put("graphicsMode", fancy ? "1" : "0");
        } else {
            options.put("fancyGraphics", String.valueOf(fancy));
        }
        options.put("particles", String.valueOf(particles));
        if (minor >= 8) {
            options.put("maxFps", String.valueOf(maxFps));
            options.put("entityShadows", String.valueOf(shadows));
        }
        return options;
    }

    /**
     * The text of options.txt. From 1.14 on the file has to say which data version it was written for: without it,
     * Minecraft treats the file as ancient and "upgrades" it, which turns graphicsMode:1 back into fast graphics.
     */
    public static String toText(Map<String, String> options, Integer dataVersion) {
        StringBuilder sb = new StringBuilder();
        if (dataVersion != null) {
            sb.append("version:").append(dataVersion).append('\n');
        }
        for (Map.Entry<String, String> e : options.entrySet()) {
            sb.append(e.getKey()).append(':').append(e.getValue()).append('\n');
        }
        return sb.toString();
    }

    /** If the game already has settings, or a mod that hands out a modpack's default settings is set up. */
    public static boolean hasOwnOptions(Path gameDirectory) {
        return Files.exists(gameDirectory.resolve("options.txt"))
            // "Your Options Shall Be Respected" and "Default Options" copy the pack's settings in on the first start,
            // but only when there is no options.txt yet
            || Files.exists(gameDirectory.resolve("config/yosbr/options.txt"))
            || Files.exists(gameDirectory.resolve("config/defaultoptions/options.txt"));
    }

    /**
     * Writes options.txt for a game that has none yet.
     *
     * @param clientJar the game's jar, to read its data version from
     * @return what was written, or null when nothing was
     */
    public static Map<String, String> writeIfMissing(Path gameDirectory, PcProfile.Tier tier, String minecraftVersion,
        Path clientJar) throws IOException {
        if (hasOwnOptions(gameDirectory)) {
            return null;
        }

        Integer dataVersion = clientJar == null ? null : readDataVersion(clientJar);
        Map<String, String> options = forTier(tier, minecraftVersion,
            clientJar != null && hasGraphicsPresets(clientJar));
        if (options.isEmpty()) {
            return null;
        }

        // 1.14 and newer need the data version (see toText); if it can't be found, leave the defaults alone
        if (dataVersion == null && AutoMemory.minorVersion(minecraftVersion) >= 14) {
            return null;
        }

        Files.createDirectories(gameDirectory);
        try {
            // CREATE_NEW: never overwrite, even if the game made the file in the meantime
            Files.write(gameDirectory.resolve("options.txt"),
                toText(options, dataVersion).getBytes(StandardCharsets.UTF_8), StandardOpenOption.CREATE_NEW,
                StandardOpenOption.WRITE);
        } catch (java.nio.file.FileAlreadyExistsException e) {
            return null;
        }
        return options;
    }

    /**
     * If the game has graphics presets (Fast, Fancy, Fabulous, Custom; 1.21.11 and newer). Told from its English
     * texts, as class names are obfuscated in some versions.
     */
    public static boolean hasGraphicsPresets(Path clientJar) {
        if (clientJar == null || !Files.isRegularFile(clientJar)) {
            return false;
        }
        try (ZipFile zip = new ZipFile(clientJar.toFile())) {
            ZipEntry entry = zip.getEntry("assets/minecraft/lang/en_us.json");
            if (entry == null) {
                return false;
            }
            try (InputStream in = zip.getInputStream(entry)) {
                JsonElement json = JsonParser.parseReader(new InputStreamReader(in, StandardCharsets.UTF_8));
                return json.isJsonObject() && json.getAsJsonObject().has("options.graphics.preset");
            }
        } catch (Exception e) {
            return false;
        }
    }

    /** The "world_version" from version.json in a client jar (1.14 and newer), or null. */
    public static Integer readDataVersion(Path clientJar) {
        if (clientJar == null || !Files.isRegularFile(clientJar)) {
            return null;
        }
        try (ZipFile zip = new ZipFile(clientJar.toFile())) {
            ZipEntry entry = zip.getEntry("version.json");
            if (entry == null) {
                return null;
            }
            try (InputStream in = zip.getInputStream(entry)) {
                JsonElement json = JsonParser.parseReader(new InputStreamReader(in, StandardCharsets.UTF_8));
                if (json.isJsonObject()) {
                    JsonObject o = json.getAsJsonObject();
                    if (o.has("world_version")) {
                        return o.get("world_version").getAsInt();
                    }
                }
            }
        } catch (Exception e) {
            // not a readable jar
        }
        return null;
    }
}
