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

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.stream.Collectors;

import org.junit.jupiter.api.Test;

import com.blockforge.launcher.dependencies.ModVersion.Relation;

class DependencyPlannerTest {
    private final Map<String, List<ModVersion>> catalog = new HashMap<>();

    private ModVersion mod(String project, String version) {
        ModVersion m = new ModVersion(project, version, project + " " + version).loaders("fabric").gameVersions("1.20.1");
        catalog.computeIfAbsent(project, k -> new ArrayList<>()).add(m);
        return m;
    }

    private DependencyPlanner.Plan plan(ModVersion wanted, Map<String, String> installed) {
        DependencyPlanner.Request r = new DependencyPlanner.Request();
        r.wanted = wanted;
        r.loader = "fabric";
        r.gameVersion = "1.20.1";
        r.installed.putAll(installed);
        return DependencyPlanner.plan(r, id -> catalog.getOrDefault(id, new ArrayList<>()));
    }

    private static List<String> names(DependencyPlanner.Plan p) {
        return p.adds.stream().map(s -> s.version.projectId).collect(Collectors.toList());
    }

    @Test
    void addsRequiredDependenciesFirstAndExplainsWhy() {
        mod("lib", "2.0");
        mod("api", "1.0").depends("lib", Relation.REQUIRED);
        ModVersion main = mod("main", "3.0").depends("api", Relation.REQUIRED);

        DependencyPlanner.Plan p = plan(main, new HashMap<>());

        assertTrue(p.ok());
        assertEquals(Arrays.asList("lib", "api", "main"), names(p));
        assertEquals("api 1.0", p.adds.get(0).requiredBy.get(0));
    }

    @Test
    void skipsWhatIsAlreadyInstalled() {
        mod("api", "1.0");
        ModVersion main = mod("main", "3.0").depends("api", Relation.REQUIRED);
        Map<String, String> installed = new HashMap<>();
        installed.put("api", "0.9");

        DependencyPlanner.Plan p = plan(main, installed);

        assertEquals(Arrays.asList("main"), names(p));
        assertEquals(1, p.alreadyInstalled.size());
    }

    @Test
    void picksTheNewestVersionThatFitsTheLoaderAndGame() {
        catalog.put("api", new ArrayList<>(Arrays.asList(
            new ModVersion("api", "3", "api 3").loaders("forge").gameVersions("1.20.1"),
            new ModVersion("api", "2", "api 2").loaders("fabric").gameVersions("1.21"),
            new ModVersion("api", "1", "api 1").loaders("fabric").gameVersions("1.20.1"))));
        ModVersion main = mod("main", "1").depends("api", Relation.REQUIRED);

        DependencyPlanner.Plan p = plan(main, new HashMap<>());

        assertTrue(p.ok());
        assertEquals("1", p.adds.get(0).version.versionId);
    }

    @Test
    void reportsADependencyThatCannotBeMet() {
        catalog.put("api", new ArrayList<>(Arrays.asList(new ModVersion("api", "1", "api 1").loaders("forge"))));
        ModVersion main = mod("main", "1").depends("api", Relation.REQUIRED);
        main.depends("ghost", Relation.REQUIRED);

        DependencyPlanner.Plan p = plan(main, new HashMap<>());

        assertFalse(p.ok());
        assertEquals(2, p.problems.size());
        assertTrue(p.problems.get(0).contains("no version of it works with fabric Minecraft 1.20.1"));
        assertTrue(p.problems.get(1).contains("couldn't be found"));
    }

    @Test
    void reportsIncompatibleModsInBothDirections() {
        mod("rival", "1");
        ModVersion main = mod("main", "1").depends("rival", Relation.INCOMPATIBLE);
        Map<String, String> installed = new HashMap<>();
        installed.put("rival", "1");
        assertTrue(plan(main, installed).conflicts.get(0).contains("which you have installed"));

        ModVersion other = mod("other", "1");
        mod("blocker", "1").depends("other", Relation.INCOMPATIBLE);
        Map<String, String> withBlocker = new HashMap<>();
        withBlocker.put("blocker", "1");
        DependencyPlanner.Plan p = plan(other, withBlocker);
        assertFalse(p.ok());
        assertTrue(p.conflicts.get(0).contains("blocker 1 (installed)"));
    }

    @Test
    void survivesCyclesAndOnlyAddsEachModOnce() {
        ModVersion a = mod("a", "1");
        ModVersion b = mod("b", "1");
        a.depends("b", Relation.REQUIRED);
        b.depends("a", Relation.REQUIRED);

        DependencyPlanner.Plan p = plan(a, new HashMap<>());
        assertEquals(2, p.adds.size());
        assertTrue(p.ok());
    }

    @Test
    void optionalDependenciesAreOnlySuggestions() {
        mod("extra", "1");
        ModVersion main = mod("main", "1").depends("extra", Relation.OPTIONAL);

        DependencyPlanner.Plan p = plan(main, new HashMap<>());
        assertEquals(Arrays.asList("main"), names(p));
        assertEquals(1, p.suggestions.size());
    }

    @Test
    void flagsAModThatIsNotMadeForTheInstance() {
        ModVersion wrong = new ModVersion("old", "1", "old 1").loaders("forge").gameVersions("1.12.2");
        DependencyPlanner.Plan p = plan(wrong, new HashMap<>());
        assertFalse(p.ok());
        assertTrue(p.problems.get(0).contains("isn't made for fabric Minecraft 1.20.1"));
    }

    @Test
    void convertsModrinthVersions() {
        com.blockforge.launcher.data.modrinth.ModrinthVersion v = new com.blockforge.launcher.data.modrinth.ModrinthVersion();
        v.id = "v1";
        v.projectId = "sodium";
        v.name = "Sodium 0.5";
        v.loaders = Arrays.asList("fabric");
        v.gameVersions = Arrays.asList("1.20.1");
        com.blockforge.launcher.data.modrinth.ModrinthDependency dep = new com.blockforge.launcher.data.modrinth.ModrinthDependency();
        dep.projectId = "indium";
        dep.dependencyType = com.blockforge.launcher.data.modrinth.ModrinthDependencyType.REQUIRED;
        com.blockforge.launcher.data.modrinth.ModrinthDependency bare = new com.blockforge.launcher.data.modrinth.ModrinthDependency();
        v.dependencies = Arrays.asList(dep, bare);

        ModVersion m = ModrinthAdapter.toModVersion(v);

        assertEquals("sodium", m.projectId);
        assertTrue(m.worksWith("fabric", "1.20.1"));
        assertFalse(m.worksWith("forge", "1.20.1"));
        assertEquals(1, m.dependencies.size());
        assertEquals(Relation.REQUIRED, m.dependencies.get(0).relation);
    }
}
