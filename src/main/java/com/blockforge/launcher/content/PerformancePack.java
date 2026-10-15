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
import java.util.Arrays;
import java.util.Collection;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Predicate;

/**
 * A handful of well known mods that make Minecraft run faster, especially on slower computers. They change nothing
 * about how the game plays.
 */
public final class PerformancePack {
    public static final String SODIUM = LoaderSupport.SODIUM;
    public static final String EMBEDDIUM = LoaderSupport.EMBEDDIUM;
    public static final String LITHIUM = "gvQqBUqZ";
    public static final String FERRITE_CORE = "uXXizFIs";
    public static final String MODERN_FIX = "nmDcB62a";

    private static final Map<String, String> NAMES = new LinkedHashMap<>();

    static {
        NAMES.put(SODIUM, "Sodium");
        NAMES.put(EMBEDDIUM, "Embeddium");
        NAMES.put(LITHIUM, "Lithium");
        NAMES.put(FERRITE_CORE, "FerriteCore");
        NAMES.put(MODERN_FIX, "ModernFix");
    }

    /**
     * One job in the pack, such as "faster rendering". The first project in the list that is available for the
     * instance is used; the others do the same job and must never be installed together with it.
     */
    public static final class Slot {
        public final String purpose;
        public final List<String> projectIds;

        Slot(String purpose, String... projectIds) {
            this.purpose = purpose;
            this.projectIds = Collections.unmodifiableList(Arrays.asList(projectIds));
        }
    }

    private PerformancePack() {
    }

    /** The jobs the pack does on a loader, empty when there's no pack for it. */
    public static List<Slot> slotsFor(String loaderKey) {
        if (loaderKey == null) {
            return Collections.emptyList();
        }
        switch (loaderKey) {
            case LoaderSupport.FABRIC:
            case LoaderSupport.QUILT:
                return Arrays.asList(new Slot("rendering", SODIUM), new Slot("game logic", LITHIUM),
                    new Slot("memory", FERRITE_CORE), new Slot("start up", MODERN_FIX));
            case LoaderSupport.NEOFORGE:
                return Arrays.asList(new Slot("rendering", SODIUM, EMBEDDIUM), new Slot("game logic", LITHIUM),
                    new Slot("memory", FERRITE_CORE), new Slot("start up", MODERN_FIX));
            case LoaderSupport.FORGE:
                return Arrays.asList(new Slot("rendering", EMBEDDIUM), new Slot("memory", FERRITE_CORE),
                    new Slot("start up", MODERN_FIX));
            default:
                return Collections.emptyList();
        }
    }

    /** Whether the pack can be offered for a loader at all. Each mod is still checked against the version. */
    public static boolean offeredFor(String loaderKey) {
        return !slotsFor(loaderKey).isEmpty();
    }

    /**
     * Picks what to install.
     *
     * @param slots     the jobs, from {@link #slotsFor(String)}
     * @param available whether a project has a version for this instance's Minecraft version and loader
     * @param installed projects already in the instance
     * @return project ids to install, in order; a job already done by something installed is skipped, as is one
     *         nothing is available for
     */
    public static List<String> choose(List<Slot> slots, Predicate<String> available, Collection<String> installed) {
        List<String> picked = new ArrayList<>();
        for (Slot slot : slots) {
            boolean done = false;
            for (String id : slot.projectIds) {
                if (installed != null && installed.contains(id)) {
                    done = true;
                    break;
                }
            }
            if (done) {
                continue;
            }
            for (String id : slot.projectIds) {
                if (available.test(id)) {
                    picked.add(id);
                    break;
                }
            }
        }
        return picked;
    }

    /** A readable name for one of the pack's projects. */
    public static String nameOf(String projectId) {
        String name = NAMES.get(projectId);
        return name == null ? projectId : name;
    }

    /** The mods offered on a loader, by name, for showing next to the option. */
    public static String describe(String loaderKey) {
        List<String> names = new ArrayList<>();
        for (Slot slot : slotsFor(loaderKey)) {
            names.add(nameOf(slot.projectIds.get(0)));
        }
        return String.join(", ", names);
    }
}
