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
import java.util.Comparator;
import java.util.List;

import com.blockforge.launcher.data.modrinth.ModrinthChannel;
import com.blockforge.launcher.data.modrinth.ModrinthVersion;

/**
 * Picks the version of a project to install without asking: the newest one made for this Minecraft version and
 * loader, preferring full releases over betas.
 */
public final class VersionPicker {
    private VersionPicker() {
    }

    /** Newest first. Modrinth dates are ISO 8601 in UTC, so they sort as text. */
    public static final Comparator<ModrinthVersion> NEWEST_FIRST = (a, b) -> {
        String da = a.datePublished == null ? "" : a.datePublished;
        String db = b.datePublished == null ? "" : b.datePublished;
        return db.compareTo(da);
    };

    /**
     * Picks a version.
     *
     * @param versions        every version of the project
     * @param acceptedLoaders loaders that work on the instance; empty for resource and shader packs, where any goes
     * @param gameVersion     the instance's Minecraft version
     * @param fallbackVersions other Minecraft versions that are close enough when nothing is made for the exact one
     *                        (empty to insist on the exact version)
     * @return the version, or null if none fits
     */
    public static ModrinthVersion pick(List<ModrinthVersion> versions, Collection<String> acceptedLoaders,
        String gameVersion, Collection<String> fallbackVersions) {
        List<ModrinthVersion> candidates = compatible(versions, acceptedLoaders, gameVersion, fallbackVersions);
        return candidates.isEmpty() ? null : candidates.get(0);
    }

    /** Every version that fits, best first. */
    public static List<ModrinthVersion> compatible(List<ModrinthVersion> versions,
        Collection<String> acceptedLoaders, String gameVersion, Collection<String> fallbackVersions) {
        if (versions == null || versions.isEmpty()) {
            return Collections.emptyList();
        }
        List<ModrinthVersion> sorted = new ArrayList<>(versions);
        sorted.sort(NEWEST_FIRST);

        List<ModrinthVersion> out = new ArrayList<>();
        java.util.Set<ModrinthVersion> taken = java.util.Collections.newSetFromMap(new java.util.IdentityHashMap<>());
        // exact game version releases, exact game version betas, then the same for the close-enough versions
        for (int tier = 0; tier < 4; tier++) {
            boolean exact = tier < 2;
            boolean releasesOnly = tier % 2 == 0;
            for (ModrinthVersion v : sorted) {
                if (taken.contains(v) || !loaderFits(v, acceptedLoaders)) {
                    continue;
                }
                if (releasesOnly && v.versionType != null && v.versionType != ModrinthChannel.RELEASE) {
                    continue;
                }
                boolean fits = exact ? madeFor(v, gameVersion) : madeForAny(v, fallbackVersions);
                if (fits) {
                    out.add(v);
                    taken.add(v);
                }
            }
        }
        return out;
    }

    /** True when {@code candidate} is a different and newer version than the one installed. */
    public static boolean isNewer(ModrinthVersion candidate, ModrinthVersion installed) {
        if (candidate == null || candidate.id == null) {
            return false;
        }
        if (installed == null || installed.id == null) {
            return true;
        }
        if (candidate.id.equals(installed.id)) {
            return false;
        }
        String a = candidate.datePublished == null ? "" : candidate.datePublished;
        String b = installed.datePublished == null ? "" : installed.datePublished;
        return a.compareTo(b) > 0;
    }

    static boolean loaderFits(ModrinthVersion v, Collection<String> acceptedLoaders) {
        if (acceptedLoaders == null || acceptedLoaders.isEmpty()) {
            return true;
        }
        if (v.loaders == null || v.loaders.isEmpty()) {
            return false;
        }
        for (String loader : v.loaders) {
            if (acceptedLoaders.contains(loader)) {
                return true;
            }
        }
        return false;
    }

    static boolean madeFor(ModrinthVersion v, String gameVersion) {
        return gameVersion == null || (v.gameVersions != null && v.gameVersions.contains(gameVersion));
    }

    static boolean madeForAny(ModrinthVersion v, Collection<String> gameVersions) {
        if (gameVersions == null || gameVersions.isEmpty() || v.gameVersions == null) {
            return false;
        }
        for (String g : gameVersions) {
            if (v.gameVersions.contains(g)) {
                return true;
            }
        }
        return false;
    }
}
