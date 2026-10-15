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
package com.blockforge.launcher.snapshots;

import java.io.IOException;
import java.nio.file.Path;

import com.blockforge.launcher.FileSystem;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.managers.LogManager;

/** Connects instances to their snapshot folders. */
public final class SnapshotService {
    private SnapshotService() {
    }

    public static Path folderFor(Instance instance) {
        return FileSystem.BASE_DIR.resolve("snapshots").resolve(instance.getSafeName());
    }

    /**
     * Saves the instance as it is, ahead of something that changes it. A failure is only logged: not being able to
     * make a snapshot shouldn't stop the person from updating.
     *
     * @return true if a snapshot was made
     */
    public static boolean saveBefore(Instance instance, String label) {
        try {
            InstanceSnapshots.create(instance.getRoot(), folderFor(instance), label);
            LogManager.info("Saved a snapshot of " + instance.getName() + " (" + label + ")");
            return true;
        } catch (IOException | RuntimeException e) {
            LogManager.logStackTrace("Couldn't save a snapshot of " + instance.getName(), e, false);
            return false;
        }
    }
}
