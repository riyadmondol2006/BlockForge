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
package com.blockforge.launcher.dependencies;

import com.blockforge.launcher.data.modrinth.ModrinthDependency;
import com.blockforge.launcher.data.modrinth.ModrinthVersion;

/** Turns Modrinth's description of a version into the form the planner works with. */
public final class ModrinthAdapter {
    private ModrinthAdapter() {
    }

    public static ModVersion toModVersion(ModrinthVersion version) {
        String name = version.name != null ? version.name : version.projectId + " " + version.versionNumber;
        ModVersion result = new ModVersion(version.projectId, version.id, name);

        if (version.loaders != null) {
            result.loaders.addAll(version.loaders);
        }
        if (version.gameVersions != null) {
            result.gameVersions.addAll(version.gameVersions);
        }

        if (version.dependencies != null) {
            for (ModrinthDependency dependency : version.dependencies) {
                // dependencies without a project (a bare file) or of a kind this launcher doesn't know can't be planned
                if (dependency.projectId == null || dependency.dependencyType == null) {
                    continue;
                }

                ModVersion.Relation relation = dependency.dependencyType == com.blockforge.launcher.data.modrinth.ModrinthDependencyType.REQUIRED
                    ? ModVersion.Relation.REQUIRED : ModVersion.Relation.OPTIONAL;
                result.dependencies.add(new ModVersion.Dependency(dependency.projectId, relation, dependency.versionId));
            }
        }
        return result;
    }
}
