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

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Enumeration;
import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Set;
import java.util.function.Function;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

import com.blockforge.launcher.content.DroppedFile.Kind;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * Looks inside a file the player dropped to work out what it is: a mod (and for which loader and Minecraft version),
 * a resource pack, a data pack, a shader pack or a modpack. The extension alone isn't enough: shader packs, resource
 * packs, data packs and modpacks are all zip files.
 */
public final class DroppedFileDetector {
    /** metadata files bigger than this are not read */
    static final int MAX_TEXT = 512 * 1024;

    private DroppedFileDetector() {
    }

    /** Reads a file from disk and tells what it is. Files that can't be read are UNKNOWN. */
    public static DroppedFile detect(Path file) {
        String name = file.getFileName().toString();
        String lower = name.toLowerCase(Locale.ROOT);
        if (!lower.endsWith(".jar") && !lower.endsWith(".zip") && !lower.endsWith(".mrpack")
            && !lower.endsWith(".litemod")) {
            return new DroppedFile(file, name, Kind.UNKNOWN, null, null, null, null);
        }
        try (ZipFile zip = new ZipFile(file.toFile())) {
            List<String> entries = new ArrayList<>();
            Enumeration<? extends ZipEntry> e = zip.entries();
            while (e.hasMoreElements() && entries.size() < ContentRouting.MAX_ENTRIES) {
                entries.add(e.nextElement().getName());
            }
            return classify(file, name, entries, entry -> read(zip, entry));
        } catch (IOException | RuntimeException e) {
            // not a zip at all; a .jar or .litemod that isn't one can't be a working mod either
            return new DroppedFile(file, name, Kind.UNKNOWN, null, null, null, null);
        }
    }

    private static String read(ZipFile zip, String entryName) {
        ZipEntry entry = zip.getEntry(entryName);
        if (entry == null || entry.getSize() > MAX_TEXT) {
            return null;
        }
        try (InputStream in = zip.getInputStream(entry)) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buffer = new byte[8192];
            int n;
            while ((n = in.read(buffer)) > 0 && out.size() <= MAX_TEXT) {
                out.write(buffer, 0, n);
            }
            return new String(out.toByteArray(), StandardCharsets.UTF_8);
        } catch (IOException e) {
            return null;
        }
    }

    /**
     * Tells what a file is from its name and the names inside it.
     *
     * @param read gives the text of an entry, or null if it can't be read; may itself be null
     */
    public static DroppedFile classify(Path path, String fileName, List<String> rawEntries,
        Function<String, String> read) {
        Function<String, String> reader = read == null ? x -> null : read;
        String lower = fileName.toLowerCase(Locale.ROOT);
        Set<String> entries = new HashSet<>();
        for (String raw : rawEntries) {
            entries.add(raw.replace('\\', '/'));
        }

        if (lower.endsWith(".mrpack") || entries.contains("modrinth.index.json")) {
            String index = reader.apply("modrinth.index.json");
            JsonObject json = json(index);
            return new DroppedFile(path, fileName, Kind.MODRINTH_MODPACK, null, string(json, "name"),
                string(json, "versionId"), null);
        }
        if (lower.endsWith(".litemod")) {
            JsonObject json = json(reader.apply("litemod.json"));
            return new DroppedFile(path, fileName, Kind.MOD, DroppedFile.LITELOADER, string(json, "name"),
                string(json, "version"), listOf(string(json, "mcversion")));
        }
        if (atRootOrOneDown(entries, "mmc-pack.json") || atRootOrOneDown(entries, "instance.cfg")) {
            return new DroppedFile(path, fileName, Kind.PRISM_EXPORT, null, null, null, null);
        }
        if (entries.contains("manifest.json")) {
            JsonObject json = json(reader.apply("manifest.json"));
            if (json != null && json.has("minecraft") && json.get("minecraft").isJsonObject()) {
                JsonObject mc = json.getAsJsonObject("minecraft");
                return new DroppedFile(path, fileName, Kind.CURSEFORGE_MODPACK, null, string(json, "name"),
                    string(json, "version"), listOf(string(mc, "version")));
            }
        }

        DroppedFile mod = mod(path, fileName, entries, reader);
        if (mod != null) {
            return mod;
        }
        if (lower.endsWith(".jar")) {
            // a jar without any loader's metadata: probably a library or a very old mod
            return new DroppedFile(path, fileName, Kind.MOD, null, null, null, null);
        }

        boolean mcmeta = entries.contains("pack.mcmeta");
        boolean assets = false;
        boolean data = false;
        boolean shaders = false;
        for (String entry : entries) {
            assets |= entry.startsWith("assets/");
            data |= entry.startsWith("data/");
            String[] parts = entry.split("/");
            if ((parts.length >= 2 && parts[0].equals("shaders"))
                || (parts.length >= 3 && parts[1].equals("shaders"))) {
                shaders = true;
            }
            if (entry.equals("shaders/")) {
                shaders = true;
            }
        }
        if (shaders && !assets) {
            return new DroppedFile(path, fileName, Kind.SHADER_PACK, null, null, null, null);
        }
        if (mcmeta) {
            String description = packDescription(reader.apply("pack.mcmeta"));
            if (data && !assets) {
                return new DroppedFile(path, fileName, Kind.DATA_PACK, null, null, description, null);
            }
            return new DroppedFile(path, fileName, Kind.RESOURCE_PACK, null, null, description, null);
        }
        return new DroppedFile(path, fileName, Kind.UNKNOWN, null, null, null, null);
    }

    private static boolean atRootOrOneDown(Set<String> entries, String name) {
        for (String entry : entries) {
            if (entry.equals(name)) {
                return true;
            }
            String[] parts = entry.split("/");
            if (parts.length == 2 && parts[1].equals(name)) {
                return true;
            }
        }
        return false;
    }

    /** Reads whichever loader's metadata a mod has; null if it has none. */
    private static DroppedFile mod(Path path, String fileName, Set<String> entries, Function<String, String> read) {
        if (entries.contains("quilt.mod.json")) {
            JsonObject json = json(read.apply("quilt.mod.json"));
            JsonObject loader = json != null && json.has("quilt_loader") && json.get("quilt_loader").isJsonObject()
                ? json.getAsJsonObject("quilt_loader") : null;
            JsonObject meta = loader != null && loader.has("metadata") && loader.get("metadata").isJsonObject()
                ? loader.getAsJsonObject("metadata") : null;
            List<String> mc = new ArrayList<>();
            if (loader != null && loader.has("depends") && loader.get("depends").isJsonArray()) {
                for (JsonElement dep : loader.getAsJsonArray("depends")) {
                    if (dep.isJsonObject() && "minecraft".equals(string(dep.getAsJsonObject(), "id"))) {
                        mc.addAll(versions(dep.getAsJsonObject().get("versions")));
                    }
                }
            }
            return new DroppedFile(path, fileName, Kind.MOD, LoaderSupport.QUILT, string(meta, "name"),
                clean(string(loader, "version")), mc);
        }
        if (entries.contains("fabric.mod.json")) {
            JsonObject json = json(read.apply("fabric.mod.json"));
            List<String> mc = new ArrayList<>();
            if (json != null && json.has("depends") && json.get("depends").isJsonObject()) {
                mc.addAll(versions(json.getAsJsonObject("depends").get("minecraft")));
            }
            return new DroppedFile(path, fileName, Kind.MOD, LoaderSupport.FABRIC, string(json, "name"),
                clean(string(json, "version")), mc);
        }
        if (entries.contains("META-INF/neoforge.mods.toml")) {
            return toml(path, fileName, read.apply("META-INF/neoforge.mods.toml"), LoaderSupport.NEOFORGE);
        }
        if (entries.contains("META-INF/mods.toml")) {
            return toml(path, fileName, read.apply("META-INF/mods.toml"), LoaderSupport.FORGE);
        }
        if (entries.contains("mcmod.info")) {
            JsonElement root = parse(read.apply("mcmod.info"));
            JsonObject first = null;
            if (root != null && root.isJsonArray() && root.getAsJsonArray().size() > 0
                && root.getAsJsonArray().get(0).isJsonObject()) {
                first = root.getAsJsonArray().get(0).getAsJsonObject();
            } else if (root != null && root.isJsonObject() && root.getAsJsonObject().has("modList")
                && root.getAsJsonObject().get("modList").isJsonArray()) {
                JsonArray list = root.getAsJsonObject().getAsJsonArray("modList");
                first = list.size() > 0 && list.get(0).isJsonObject() ? list.get(0).getAsJsonObject() : null;
            }
            return new DroppedFile(path, fileName, Kind.MOD, LoaderSupport.FORGE, string(first, "name"),
                clean(string(first, "version")), listOf(clean(string(first, "mcversion"))));
        }
        return null;
    }

    /**
     * Forge and NeoForge describe mods in TOML. Only the few keys needed here are read: the display name, the
     * version and the Minecraft dependency. A Forge-style file that depends on NeoForge is a NeoForge mod.
     */
    static DroppedFile toml(Path path, String fileName, String text, String loader) {
        String table = "";
        String name = null;
        String version = null;
        String depModId = null;
        String depRange = null;
        List<String> mc = new ArrayList<>();
        boolean dependsOnNeoForge = false;
        if (text != null) {
            for (String raw : text.split("\\r?\\n")) {
                String line = raw.trim();
                if (line.startsWith("#") || line.isEmpty()) {
                    continue;
                }
                if (line.startsWith("[")) {
                    if ("minecraft".equals(depModId) && depRange != null) {
                        mc.add(depRange);
                    }
                    if ("neoforge".equals(depModId)) {
                        dependsOnNeoForge = true;
                    }
                    depModId = null;
                    depRange = null;
                    table = line.replace("[", "").replace("]", "").trim();
                    continue;
                }
                int eq = line.indexOf('=');
                if (eq < 0) {
                    continue;
                }
                String key = line.substring(0, eq).trim();
                String value = unquote(line.substring(eq + 1).trim());
                if (table.equals("mods")) {
                    if (key.equals("displayName") && name == null) {
                        name = value;
                    } else if (key.equals("version") && version == null) {
                        version = clean(value);
                    }
                } else if (table.startsWith("dependencies")) {
                    if (key.equals("modId")) {
                        depModId = value;
                    } else if (key.equals("versionRange")) {
                        depRange = value;
                    }
                }
            }
            if ("minecraft".equals(depModId) && depRange != null) {
                mc.add(depRange);
            }
            if ("neoforge".equals(depModId)) {
                dependsOnNeoForge = true;
            }
        }
        String effective = dependsOnNeoForge ? LoaderSupport.NEOFORGE : loader;
        return new DroppedFile(path, fileName, Kind.MOD, effective, name, version, mc);
    }

    private static String unquote(String value) {
        String v = value;
        int hash = v.indexOf(" #");
        if (hash > 0 && !v.startsWith("\"") && !v.startsWith("'")) {
            v = v.substring(0, hash).trim();
        }
        if (v.length() >= 2 && (v.startsWith("\"") || v.startsWith("'"))) {
            char quote = v.charAt(0);
            int end = v.indexOf(quote, 1);
            return end > 0 ? v.substring(1, end) : v.substring(1);
        }
        return v;
    }

    /** Placeholders like ${version} are filled in when a mod is built; one left in means the value is unknown. */
    private static String clean(String value) {
        return value == null || value.contains("${") || value.trim().isEmpty() ? null : value.trim();
    }

    private static List<String> listOf(String value) {
        return value == null ? Collections.<String>emptyList() : Collections.singletonList(value);
    }

    private static List<String> versions(JsonElement element) {
        List<String> out = new ArrayList<>();
        if (element == null || element.isJsonNull()) {
            return out;
        }
        if (element.isJsonPrimitive()) {
            out.add(element.getAsString());
        } else if (element.isJsonArray()) {
            for (JsonElement e : element.getAsJsonArray()) {
                if (e.isJsonPrimitive()) {
                    out.add(e.getAsString());
                }
            }
        }
        return out;
    }

    private static String packDescription(String mcmeta) {
        JsonObject json = json(mcmeta);
        if (json == null || !json.has("pack") || !json.get("pack").isJsonObject()) {
            return null;
        }
        JsonElement d = json.getAsJsonObject("pack").get("description");
        return d != null && d.isJsonPrimitive() ? d.getAsString() : null;
    }

    private static JsonElement parse(String text) {
        if (text == null) {
            return null;
        }
        try {
            // some mods start their metadata with a byte order mark
            String clean = text.startsWith("﻿") ? text.substring(1) : text;
            com.google.gson.stream.JsonReader reader = new com.google.gson.stream.JsonReader(
                new java.io.StringReader(clean));
            reader.setLenient(true);
            return JsonParser.parseReader(reader);
        } catch (RuntimeException e) {
            return null;
        }
    }

    private static JsonObject json(String text) {
        JsonElement e = parse(text);
        return e != null && e.isJsonObject() ? e.getAsJsonObject() : null;
    }

    private static String string(JsonObject json, String key) {
        if (json == null || !json.has(key)) {
            return null;
        }
        JsonElement e = json.get(key);
        return e.isJsonPrimitive() ? e.getAsString() : null;
    }
}
