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

import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

/** What a file the player dropped on the launcher turned out to be. */
public final class DroppedFile {
    public enum Kind {
        MOD, RESOURCE_PACK, DATA_PACK, SHADER_PACK, MODRINTH_MODPACK, CURSEFORGE_MODPACK, PRISM_EXPORT, UNKNOWN;

        /** Modpacks become instances of their own instead of going into one. */
        public boolean isModpack() {
            return this == MODRINTH_MODPACK || this == CURSEFORGE_MODPACK || this == PRISM_EXPORT;
        }

        /** Whether it goes into an instance (a data pack goes into its worlds). */
        public boolean goesIntoInstance() {
            return this == MOD || this == RESOURCE_PACK || this == DATA_PACK || this == SHADER_PACK;
        }

        /** The tab of the content manager it shows on, or null for data packs and modpacks. */
        public ContentKind contentKind() {
            switch (this) {
                case MOD:
                    return ContentKind.MODS;
                case RESOURCE_PACK:
                    return ContentKind.RESOURCE_PACKS;
                case SHADER_PACK:
                    return ContentKind.SHADERS;
                default:
                    return null;
            }
        }
    }

    public static final String LITELOADER = "liteloader";

    public final Path path;
    public final String fileName;
    public final Kind kind;
    /** the loader a mod is made for (fabric, quilt, forge, neoforge, liteloader), or null if unknown */
    public final String loader;
    /** the name inside the file, or null if it has none */
    public final String name;
    public final String version;
    /** Minecraft version requirements as the mod writes them; any one is enough */
    public final List<String> minecraftVersions;

    public DroppedFile(Path path, String fileName, Kind kind, String loader, String name, String version,
        List<String> minecraftVersions) {
        this.path = path;
        this.fileName = fileName;
        this.kind = kind;
        this.loader = loader;
        this.name = name;
        this.version = version;
        this.minecraftVersions = minecraftVersions == null ? Collections.<String>emptyList()
            : Collections.unmodifiableList(new ArrayList<>(minecraftVersions));
    }

    /** The best name to show for it. */
    public String displayName() {
        return name != null && !name.trim().isEmpty() ? name : fileName;
    }

    /**
     * Why this might not work in an instance, or null when it looks fine.
     *
     * @param loaderKey        the instance's loader as Modrinth names it, null for plain Minecraft
     * @param minecraftVersion the instance's Minecraft version
     */
    public String problemFor(String loaderKey, String minecraftVersion) {
        if (kind == Kind.MOD) {
            if (loaderKey == null) {
                return "Mods need a mod loader such as Fabric";
            }
            if (LITELOADER.equals(loader)) {
                return displayName() + " is a LiteLoader mod";
            }
            if (loader != null && !loaderRuns(loaderKey, loader, minecraftVersion)) {
                return displayName() + " is made for " + LoaderSupport.displayName(loader);
            }
            Boolean fits = minecraftVersions.isEmpty() ? null
                : VersionRanges.matchesAny(minecraftVersions, minecraftVersion);
            if (Boolean.FALSE.equals(fits)) {
                return displayName() + " is made for Minecraft " + String.join(" or ", minecraftVersions);
            }
        }
        if (kind == Kind.SHADER_PACK && loaderKey == null) {
            return "Shaders need Fabric with Iris";
        }
        return null;
    }

    /** Whether an instance with one loader runs a mod made for another. */
    static boolean loaderRuns(String instanceLoader, String modLoader, String minecraftVersion) {
        if (instanceLoader.equals(modLoader)) {
            return true;
        }
        if (LoaderSupport.QUILT.equals(instanceLoader) && LoaderSupport.FABRIC.equals(modLoader)) {
            return true;
        }
        // NeoForge still loaded Forge mods on 1.20.1, where it started
        return LoaderSupport.NEOFORGE.equals(instanceLoader) && LoaderSupport.FORGE.equals(modLoader)
            && "1.20.1".equals(minecraftVersion);
    }
}
