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

import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.List;
import java.util.Locale;

import com.blockforge.launcher.data.minecraft.loaders.LoaderType;
import com.blockforge.launcher.data.minecraft.loaders.LoaderVersion;

/** What each mod loader can run, in Modrinth's terms, and what a shader pack needs to work. */
public final class LoaderSupport {
    public static final String FABRIC = "fabric";
    public static final String QUILT = "quilt";
    public static final String FORGE = "forge";
    public static final String NEOFORGE = "neoforge";
    public static final String LEGACY_FABRIC = "legacy-fabric";

    public static final String IRIS = "YL57xq9U";
    public static final String SODIUM = "AANobbMI";
    public static final String OCULUS = "GchcoXML";
    public static final String EMBEDDIUM = "sk9rgfiA";

    /** What a shader pack needs before it does anything. */
    public enum ShaderSetup {
        /** Iris, Oculus or OptiFine is already there */
        READY,
        /** Iris (with Sodium) can be added from Modrinth */
        ADD_IRIS,
        /** Forge: Oculus can be added from Modrinth if it exists for this version, otherwise OptiFine is needed */
        ADD_OCULUS,
        /** no mod loader, or one no shader loader is made for */
        NEEDS_MOD_LOADER
    }

    private LoaderSupport() {
    }

    /** Modrinth's name for an instance's loader, or null for plain Minecraft and server-only loaders. */
    public static String key(LoaderVersion loader) {
        if (loader == null || loader.type == null) {
            return null;
        }
        if (loader.isFabric()) {
            return FABRIC;
        }
        if (loader.isQuilt()) {
            return QUILT;
        }
        if (loader.isNeoForge()) {
            return NEOFORGE;
        }
        if (loader.isForge()) {
            return FORGE;
        }
        if (loader.isLegacyFabric()) {
            return LEGACY_FABRIC;
        }
        return null;
    }

    /** Modrinth's name for a loader picked on the New instance page. */
    public static String key(LoaderType type) {
        if (type == null) {
            return null;
        }
        switch (type) {
            case FABRIC:
                return FABRIC;
            case QUILT:
                return QUILT;
            case NEOFORGE:
                return NEOFORGE;
            case FORGE:
                return FORGE;
            case LEGACY_FABRIC:
                return LEGACY_FABRIC;
            default:
                return null;
        }
    }

    /**
     * The loaders whose mods run on this loader: Quilt runs Fabric mods, and NeoForge runs Forge mods on the
     * Minecraft versions where the two are still compatible.
     */
    public static List<String> acceptedLoaders(String key, String minecraftVersion,
        Collection<String> neoForgeRunsForgeModsOn) {
        if (key == null) {
            return Collections.emptyList();
        }
        List<String> loaders = new ArrayList<>();
        loaders.add(key);
        if (QUILT.equals(key)) {
            loaders.add(FABRIC);
        }
        if (NEOFORGE.equals(key) && neoForgeRunsForgeModsOn != null
            && neoForgeRunsForgeModsOn.contains(minecraftVersion)) {
            loaders.add(FORGE);
        }
        return loaders;
    }

    /** Whether a mod loader is there that shader packs work with. */
    public static boolean hasShaderLoader(Collection<String> modrinthProjectIds, Collection<String> fileNames) {
        if (modrinthProjectIds != null
            && (modrinthProjectIds.contains(IRIS) || modrinthProjectIds.contains(OCULUS))) {
            return true;
        }
        if (fileNames != null) {
            for (String file : fileNames) {
                String name = file == null ? "" : file.toLowerCase(Locale.ROOT);
                if (name.startsWith("iris") || name.startsWith("oculus") || name.contains("optifine")
                    || name.startsWith("optifabric")) {
                    return true;
                }
            }
        }
        return false;
    }

    /** What has to happen before shaders work on an instance with this loader. */
    public static ShaderSetup shaderSetup(String key, boolean hasShaderLoader) {
        if (hasShaderLoader) {
            return ShaderSetup.READY;
        }
        if (FABRIC.equals(key) || QUILT.equals(key) || NEOFORGE.equals(key)) {
            return ShaderSetup.ADD_IRIS;
        }
        if (FORGE.equals(key)) {
            return ShaderSetup.ADD_OCULUS;
        }
        return ShaderSetup.NEEDS_MOD_LOADER;
    }

    /** A readable name for the loader, for messages. */
    public static String displayName(String key) {
        if (key == null) {
            return "Minecraft";
        }
        switch (key) {
            case FABRIC:
                return "Fabric";
            case QUILT:
                return "Quilt";
            case FORGE:
                return "Forge";
            case NEOFORGE:
                return "NeoForge";
            case LEGACY_FABRIC:
                return "Legacy Fabric";
            default:
                return key;
        }
    }
}
