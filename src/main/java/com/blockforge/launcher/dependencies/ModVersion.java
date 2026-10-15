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

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

/** One downloadable version of a mod, from any provider, described by what matters for dependency planning. */
public final class ModVersion {
    public enum Relation {
        REQUIRED, OPTIONAL, INCOMPATIBLE, EMBEDDED
    }

    /** a link from one mod version to another project */
    public static final class Dependency {
        public final String projectId;
        public final Relation relation;
        /** a specific version that is asked for, or null for any version */
        public final String versionId;

        public Dependency(String projectId, Relation relation, String versionId) {
            this.projectId = projectId;
            this.relation = relation;
            this.versionId = versionId;
        }
    }

    public final String projectId;
    public final String versionId;
    public final String name;
    /** loaders it works with, empty meaning any */
    public final Set<String> loaders = new HashSet<>();
    /** Minecraft versions it works with, empty meaning any */
    public final Set<String> gameVersions = new HashSet<>();
    public final List<Dependency> dependencies = new ArrayList<>();

    public ModVersion(String projectId, String versionId, String name) {
        this.projectId = projectId;
        this.versionId = versionId;
        this.name = name;
    }

    public ModVersion loaders(String... values) {
        Collections.addAll(loaders, values);
        return this;
    }

    public ModVersion gameVersions(String... values) {
        Collections.addAll(gameVersions, values);
        return this;
    }

    public ModVersion depends(String projectId, Relation relation) {
        dependencies.add(new Dependency(projectId, relation, null));
        return this;
    }

    public boolean worksWith(String loader, String gameVersion) {
        boolean loaderOk = loaders.isEmpty() || loader == null || loaders.contains(loader);
        boolean versionOk = gameVersions.isEmpty() || gameVersion == null || gameVersions.contains(gameVersion);
        return loaderOk && versionOk;
    }
}
