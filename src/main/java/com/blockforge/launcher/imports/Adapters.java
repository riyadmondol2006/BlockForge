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

import java.io.IOException;
import java.io.InputStreamReader;
import java.io.Reader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Properties;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParseException;
import com.google.gson.JsonParser;

/** The launchers BlockForge can import from. */
public final class Adapters {
    private Adapters() {
    }

    static JsonObject json(Path file) throws IOException {
        try (Reader reader = new InputStreamReader(Files.newInputStream(file), StandardCharsets.UTF_8)) {
            JsonElement element = JsonParser.parseReader(reader);
            if (!element.isJsonObject()) {
                throw new IOException(file.getFileName() + " isn't a valid instance file");
            }
            return element.getAsJsonObject();
        } catch (JsonParseException e) {
            throw new IOException(file.getFileName() + " couldn't be read: " + e.getMessage(), e);
        }
    }

    static String string(JsonObject o, String key) {
        JsonElement e = o == null ? null : o.get(key);
        return e == null || e.isJsonNull() || !e.isJsonPrimitive() ? null : e.getAsString();
    }

    static JsonObject object(JsonObject o, String key) {
        JsonElement e = o == null ? null : o.get(key);
        return e != null && e.isJsonObject() ? e.getAsJsonObject() : null;
    }

    static String nonBlank(String value, String fallback) {
        return value == null || value.trim().isEmpty() ? fallback : value.trim();
    }

    /** Prism Launcher and MultiMC share a format: instance.cfg plus mmc-pack.json. */
    public static final class Prism implements ImportAdapter {
        @Override
        public String source() {
            return "Prism / MultiMC";
        }

        @Override
        public boolean matches(Path folder) {
            return Files.isRegularFile(folder.resolve("instance.cfg")) && Files.isRegularFile(folder.resolve("mmc-pack.json"));
        }

        @Override
        public ImportedInstance read(Path folder) throws IOException {
            Properties cfg = new Properties();
            try (Reader reader = new InputStreamReader(Files.newInputStream(folder.resolve("instance.cfg")),
                StandardCharsets.UTF_8)) {
                cfg.load(reader);
            }

            JsonObject pack = json(folder.resolve("mmc-pack.json"));
            String minecraft = null;
            String loaderType = null;
            String loaderVersion = null;

            JsonElement components = pack.get("components");
            if (components != null && components.isJsonArray()) {
                JsonArray array = components.getAsJsonArray();
                for (JsonElement component : array) {
                    if (!component.isJsonObject()) {
                        continue;
                    }
                    String uid = string(component.getAsJsonObject(), "uid");
                    String version = string(component.getAsJsonObject(), "version");
                    if ("net.minecraft".equals(uid)) {
                        minecraft = version;
                    } else if ("net.fabricmc.fabric-loader".equals(uid)) {
                        loaderType = "fabric";
                        loaderVersion = version;
                    } else if ("net.minecraftforge".equals(uid)) {
                        loaderType = "forge";
                        loaderVersion = version;
                    } else if ("net.neoforged".equals(uid)) {
                        loaderType = "neoforge";
                        loaderVersion = version;
                    } else if ("org.quiltmc.quilt-loader".equals(uid)) {
                        loaderType = "quilt";
                        loaderVersion = version;
                    }
                }
            }

            if (minecraft == null) {
                throw new IOException("mmc-pack.json doesn't say which Minecraft version the instance uses");
            }

            Path gameDir = Files.isDirectory(folder.resolve(".minecraft")) ? folder.resolve(".minecraft")
                : folder.resolve("minecraft");
            ImportedInstance result = new ImportedInstance(source(),
                nonBlank(cfg.getProperty("name"), folder.getFileName().toString()), minecraft, loaderType,
                loaderVersion, folder, gameDir);

            if ("true".equalsIgnoreCase(cfg.getProperty("OverrideMemory")) && cfg.getProperty("MaxMemAlloc") != null) {
                try {
                    result.maximumMemoryMb = Integer.parseInt(cfg.getProperty("MaxMemAlloc").trim());
                } catch (NumberFormatException ignored) {
                    result.warnings.add("The memory setting couldn't be read, so the default will be used.");
                }
            }
            if ("true".equalsIgnoreCase(cfg.getProperty("OverrideJavaArgs")) && cfg.getProperty("JvmArgs") != null) {
                result.javaArguments = cfg.getProperty("JvmArgs").trim();
            }
            return result;
        }
    }

    /** The CurseForge app keeps minecraftinstance.json in each instance folder. */
    public static final class CurseForge implements ImportAdapter {
        @Override
        public String source() {
            return "CurseForge";
        }

        @Override
        public boolean matches(Path folder) {
            return Files.isRegularFile(folder.resolve("minecraftinstance.json"));
        }

        @Override
        public ImportedInstance read(Path folder) throws IOException {
            JsonObject o = json(folder.resolve("minecraftinstance.json"));
            JsonObject loader = object(o, "baseModLoader");

            String minecraft = string(o, "gameVersion");
            if (minecraft == null && loader != null) {
                minecraft = string(loader, "minecraftVersion");
            }
            if (minecraft == null) {
                throw new IOException("minecraftinstance.json doesn't say which Minecraft version the instance uses");
            }

            String loaderType = null;
            String loaderVersion = null;
            if (loader != null) {
                String loaderName = nonBlank(string(loader, "name"), "").toLowerCase();
                if (loaderName.startsWith("forge")) {
                    loaderType = "forge";
                } else if (loaderName.startsWith("fabric")) {
                    loaderType = "fabric";
                } else if (loaderName.startsWith("quilt")) {
                    loaderType = "quilt";
                } else if (loaderName.startsWith("neoforge")) {
                    loaderType = "neoforge";
                }
                int dash = loaderName.indexOf('-');
                if (loaderType != null && dash >= 0 && dash + 1 < loaderName.length()) {
                    loaderVersion = loaderName.substring(dash + 1);
                }
            }

            ImportedInstance result = new ImportedInstance(source(),
                nonBlank(string(o, "name"), folder.getFileName().toString()), minecraft, loaderType, loaderVersion,
                folder, folder);
            if (o.has("allocatedMemory") && o.get("allocatedMemory").isJsonPrimitive()) {
                try {
                    int mb = o.get("allocatedMemory").getAsInt();
                    if (mb > 0) {
                        result.maximumMemoryMb = mb;
                    }
                } catch (NumberFormatException ignored) {
                    // leave the default
                }
            }
            return result;
        }
    }

    /** ATLauncher (and older BlockForge) instances keep instance.json with the Minecraft version as "id". */
    public static final class Atlauncher implements ImportAdapter {
        @Override
        public String source() {
            return "ATLauncher";
        }

        @Override
        public boolean matches(Path folder) {
            return Files.isRegularFile(folder.resolve("instance.json"));
        }

        @Override
        public ImportedInstance read(Path folder) throws IOException {
            JsonObject o = json(folder.resolve("instance.json"));
            JsonObject launcher = object(o, "launcher");
            String minecraft = string(o, "id");
            if (launcher == null || minecraft == null) {
                throw new IOException("instance.json isn't an ATLauncher instance");
            }

            JsonObject loader = object(launcher, "loaderVersion");
            String loaderType = null;
            String loaderVersion = null;
            if (loader != null) {
                String type = nonBlank(string(loader, "type"), "").toLowerCase().replace(" ", "");
                if (type.equals("fabric") || type.equals("forge") || type.equals("neoforge") || type.equals("quilt")) {
                    loaderType = type;
                    loaderVersion = string(loader, "version");
                } else if (!type.isEmpty()) {
                    loaderVersion = null;
                }
            }

            ImportedInstance result = new ImportedInstance(source(),
                nonBlank(string(launcher, "name"), folder.getFileName().toString()), minecraft, loaderType,
                loaderVersion, folder, folder);
            if (launcher.has("maximumMemory") && launcher.get("maximumMemory").isJsonPrimitive()) {
                try {
                    result.maximumMemoryMb = launcher.get("maximumMemory").getAsInt();
                } catch (NumberFormatException ignored) {
                    // leave the default
                }
            }
            return result;
        }
    }

    /** GDLauncher keeps config.json with the game's files in an "instance" folder. */
    public static final class GdLauncher implements ImportAdapter {
        @Override
        public String source() {
            return "GDLauncher";
        }

        @Override
        public boolean matches(Path folder) {
            return Files.isRegularFile(folder.resolve("config.json")) && Files.isDirectory(folder.resolve("instance"));
        }

        @Override
        public ImportedInstance read(Path folder) throws IOException {
            JsonObject o = json(folder.resolve("config.json"));
            JsonObject version = object(object(o, "game_configuration"), "version");
            String minecraft = string(version, "release");
            if (minecraft == null) {
                throw new IOException("config.json doesn't say which Minecraft version the instance uses");
            }

            String loaderType = null;
            String loaderVersion = null;
            JsonElement loaders = version.get("modloaders");
            if (loaders != null && loaders.isJsonArray() && loaders.getAsJsonArray().size() > 0
                && loaders.getAsJsonArray().get(0).isJsonObject()) {
                JsonObject first = loaders.getAsJsonArray().get(0).getAsJsonObject();
                String type = nonBlank(string(first, "type"), "").toLowerCase();
                if (type.equals("fabric") || type.equals("forge") || type.equals("neoforge") || type.equals("quilt")) {
                    loaderType = type;
                    loaderVersion = string(first, "version");
                }
            }

            return new ImportedInstance(source(), nonBlank(string(o, "name"), folder.getFileName().toString()),
                minecraft, loaderType, loaderVersion, folder, folder.resolve("instance"));
        }
    }
}
