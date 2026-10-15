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

import java.nio.file.Path;

/** Knows how to recognise and read one other launcher's instance folder. */
public interface ImportAdapter {
    /** the launcher this reads, for display */
    String source();

    /** a cheap check on the folder's files, without reading them */
    boolean matches(Path folder);

    /** reads the folder, throwing if its files are unreadable or incomplete */
    ImportedInstance read(Path folder) throws IOException;
}
