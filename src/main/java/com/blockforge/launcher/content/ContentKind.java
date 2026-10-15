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

import com.blockforge.launcher.data.Type;
import com.blockforge.launcher.data.modrinth.ModrinthProjectType;

/** The kinds of things a player adds to an instance, and where each one lives inside it. */
public enum ContentKind {
    MODS(ModrinthProjectType.MOD, Type.mods, "mods"),
    RESOURCE_PACKS(ModrinthProjectType.RESOURCEPACK, Type.resourcepack, "resourcepacks"),
    SHADERS(ModrinthProjectType.SHADER, Type.shaderpack, "shaderpacks");

    /** the project type used when searching Modrinth */
    public final ModrinthProjectType projectType;
    /** how the launcher records a file of this kind */
    public final Type type;
    /** the folder inside the instance */
    public final String folder;

    ContentKind(ModrinthProjectType projectType, Type type, String folder) {
        this.projectType = projectType;
        this.type = type;
        this.folder = folder;
    }

    /** The kind that a launcher file type belongs to, or null if it isn't one of these. */
    public static ContentKind of(Type type) {
        if (type == null) {
            return null;
        }
        for (ContentKind kind : values()) {
            if (kind.type == type) {
                return kind;
            }
        }
        return null;
    }

    /** The kind for a Modrinth project type, or null for modpacks, plugins and data packs. */
    public static ContentKind of(ModrinthProjectType projectType) {
        if (projectType == null) {
            return null;
        }
        for (ContentKind kind : values()) {
            if (kind.projectType == projectType) {
                return kind;
            }
        }
        return null;
    }
}
