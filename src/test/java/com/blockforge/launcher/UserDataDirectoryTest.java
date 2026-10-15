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
package com.blockforge.launcher;

import static org.junit.jupiter.api.Assertions.assertEquals;

import java.nio.file.Paths;

import org.junit.jupiter.api.Test;

class UserDataDirectoryTest {
    @Test
    void installedCopiesKeepFilesInTheUsersOwnFolder() {
        assertEquals(Paths.get("/home/riyad/.local/share/blockforge"),
            FileSystem.userDataDirectory("Linux", "/home/riyad", null, null));
        assertEquals(Paths.get("/data/blockforge"), FileSystem.userDataDirectory("Linux", "/home/riyad", null, "/data"));
        assertEquals(Paths.get("/Users/riyad/Library/Application Support/BlockForge"),
            FileSystem.userDataDirectory("Mac OS X", "/Users/riyad", null, null));
        assertEquals(Paths.get("C:/Users/riyad/AppData/Roaming/BlockForge"),
            FileSystem.userDataDirectory("Windows 11", "C:/Users/riyad", "C:/Users/riyad/AppData/Roaming", null));
        assertEquals(Paths.get("C:/Users/riyad", "AppData", "Roaming", "BlockForge"),
            FileSystem.userDataDirectory("Windows 10", "C:/Users/riyad", null, null));
    }
}
