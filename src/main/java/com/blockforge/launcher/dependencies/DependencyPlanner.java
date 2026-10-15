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

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * Works out what has to be installed alongside a mod, and whether anything stops it, before a single file is
 * downloaded. The result is a plan that can be shown to the user, with the reason for every line.
 */
public final class DependencyPlanner {
    /** Looks up the versions of a project, newest first. Each provider supplies its own. */
    public interface Catalog {
        List<ModVersion> versionsOf(String projectId);
    }

    public static final class Request {
        public ModVersion wanted;
        public String loader;
        public String gameVersion;
        /** projects already in the instance (project id to the version installed) */
        public final Map<String, String> installed = new HashMap<>();
        /** versions the user insists on (project id to version id) */
        public final Map<String, String> pinned = new HashMap<>();
    }

    /** One mod that will be added, and why. */
    public static final class Step {
        public final ModVersion version;
        public final List<String> requiredBy = new ArrayList<>();

        Step(ModVersion version) {
            this.version = version;
        }
    }

    public static final class Plan {
        /** in install order: dependencies come before what needs them */
        public final List<Step> adds = new ArrayList<>();
        /** things that stop the install, with the reason */
        public final List<String> conflicts = new ArrayList<>();
        /** needs that can't be met, with the reason */
        public final List<String> problems = new ArrayList<>();
        /** extras that go well with this mod but aren't needed */
        public final List<String> suggestions = new ArrayList<>();
        /** needs that are already met by what is installed */
        public final List<String> alreadyInstalled = new ArrayList<>();

        /** true if it is safe to go ahead */
        public boolean ok() {
            return conflicts.isEmpty() && problems.isEmpty();
        }
    }

    private DependencyPlanner() {
    }

    public static Plan plan(Request request, Catalog catalog) {
        Plan plan = new Plan();
        Map<String, Step> planned = new LinkedHashMap<>();
        Set<String> visiting = new HashSet<>();

        add(request, catalog, plan, planned, visiting, request.wanted, null);

        // anything that is incompatible with something that's installed or planned
        for (Step step : planned.values()) {
            for (ModVersion.Dependency dep : step.version.dependencies) {
                if (dep.relation == ModVersion.Relation.INCOMPATIBLE
                    && (request.installed.containsKey(dep.projectId) || planned.containsKey(dep.projectId))) {
                    plan.conflicts.add(step.version.name + " doesn't work together with " + dep.projectId
                        + (request.installed.containsKey(dep.projectId) ? ", which you have installed"
                            : ", which would also be installed"));
                }
            }
        }

        // the other way round: something installed that rules out what's being added
        for (Step step : planned.values()) {
            for (String id : request.installed.keySet()) {
                List<ModVersion> installedVersions = catalog.versionsOf(id);
                for (ModVersion installed : installedVersions) {
                    if (!installed.versionId.equals(request.installed.get(id))) {
                        continue;
                    }
                    for (ModVersion.Dependency dep : installed.dependencies) {
                        if (dep.relation == ModVersion.Relation.INCOMPATIBLE
                            && dep.projectId.equals(step.version.projectId)) {
                            String message = installed.name + " (installed) doesn't work together with "
                                + step.version.name;
                            if (!plan.conflicts.contains(message)) {
                                plan.conflicts.add(message);
                            }
                        }
                    }
                }
            }
        }

        // dependencies first
        List<Step> ordered = new ArrayList<>(planned.values());
        java.util.Collections.reverse(ordered);
        plan.adds.addAll(ordered);
        return plan;
    }

    private static void add(Request request, Catalog catalog, Plan plan, Map<String, Step> planned,
        Set<String> visiting, ModVersion version, String requiredBy) {
        Step existing = planned.get(version.projectId);
        if (existing != null) {
            if (requiredBy != null && !existing.requiredBy.contains(requiredBy)) {
                existing.requiredBy.add(requiredBy);
            }
            return;
        }

        if (!visiting.add(version.projectId)) {
            return;
        }

        if (!version.worksWith(request.loader, request.gameVersion)) {
            plan.problems.add(version.name + " isn't made for " + describe(request));
        }

        Step step = new Step(version);
        if (requiredBy != null) {
            step.requiredBy.add(requiredBy);
        }
        planned.put(version.projectId, step);

        Deque<ModVersion.Dependency> queue = new ArrayDeque<>(version.dependencies);
        while (!queue.isEmpty()) {
            ModVersion.Dependency dep = queue.poll();

            switch (dep.relation) {
                case OPTIONAL:
                    plan.suggestions.add(version.name + " works well with " + dep.projectId);
                    break;
                case REQUIRED:
                    resolve(request, catalog, plan, planned, visiting, version, dep);
                    break;
                default:
                    break;
            }
        }
    }

    private static void resolve(Request request, Catalog catalog, Plan plan, Map<String, Step> planned,
        Set<String> visiting, ModVersion parent, ModVersion.Dependency dep) {
        String installedVersion = request.installed.get(dep.projectId);
        if (installedVersion != null) {
            String pin = dep.versionId != null ? dep.versionId : request.pinned.get(dep.projectId);
            if (pin != null && !pin.equals(installedVersion)) {
                plan.conflicts.add(parent.name + " needs version " + pin + " of " + dep.projectId + " but "
                    + installedVersion + " is installed");
            } else {
                plan.alreadyInstalled.add(dep.projectId + " (needed by " + parent.name + ")");
            }
            return;
        }

        List<ModVersion> versions = catalog.versionsOf(dep.projectId);
        String wanted = dep.versionId != null ? dep.versionId : request.pinned.get(dep.projectId);

        ModVersion chosen = null;
        for (ModVersion candidate : versions) {
            if (wanted != null && !wanted.equals(candidate.versionId)) {
                continue;
            }
            if (candidate.worksWith(request.loader, request.gameVersion)) {
                chosen = candidate;
                break;
            }
        }

        if (chosen == null) {
            plan.problems.add(parent.name + " needs " + dep.projectId + ", but "
                + (versions.isEmpty() ? "it couldn't be found"
                    : wanted != null ? "version " + wanted + " isn't available for " + describe(request)
                        : "no version of it works with " + describe(request)));
            return;
        }

        add(request, catalog, plan, planned, visiting, chosen, parent.name);
    }

    private static String describe(Request request) {
        StringBuilder sb = new StringBuilder();
        if (request.loader != null) {
            sb.append(request.loader).append(' ');
        }
        sb.append(request.gameVersion == null ? "this Minecraft version" : "Minecraft " + request.gameVersion);
        return sb.toString();
    }
}
