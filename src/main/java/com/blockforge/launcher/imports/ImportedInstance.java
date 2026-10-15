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

import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

/**
 * An instance from another launcher, described in BlockForge's own terms so that every launcher is handled the same
 * way after it has been read.
 */
public final class ImportedInstance {
    /** which launcher it came from, for display: "Prism Launcher", "CurseForge" ... */
    public final String source;
    public final String name;
    public final String minecraftVersion;
    /** fabric, forge, neoforge or quilt, or null for vanilla */
    public final String loaderType;
    public final String loaderVersion;
    /** where the game's own files (mods, saves, config ...) are */
    public final Path gameDir;
    /** the instance's folder in the other launcher */
    public final Path root;
    public Integer maximumMemoryMb;
    public String javaArguments;
    public final List<String> warnings = new ArrayList<>();

    public ImportedInstance(String source, String name, String minecraftVersion, String loaderType,
        String loaderVersion, Path root, Path gameDir) {
        this.source = source;
        this.name = name;
        this.minecraftVersion = minecraftVersion;
        this.loaderType = loaderType;
        this.loaderVersion = loaderVersion;
        this.root = root;
        this.gameDir = gameDir;
    }

    public String loaderLabel() {
        if (loaderType == null) {
            return "Vanilla";
        }
        String pretty = loaderType.substring(0, 1).toUpperCase() + loaderType.substring(1);
        if (loaderType.equals("neoforge")) {
            pretty = "NeoForge";
        }
        return loaderVersion == null ? pretty : pretty + " " + loaderVersion;
    }
}
