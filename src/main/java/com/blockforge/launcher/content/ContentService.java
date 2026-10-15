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

import java.io.File;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.EnumMap;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.stream.Collectors;
import java.util.stream.Stream;

import com.blockforge.launcher.App;
import com.blockforge.launcher.constants.Constants;
import com.blockforge.launcher.data.AddModRestriction;
import com.blockforge.launcher.data.DisableableMod;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.data.modrinth.ModrinthDownloadMetadata;
import com.blockforge.launcher.data.modrinth.ModrinthProject;
import com.blockforge.launcher.data.modrinth.ModrinthSearchHit;
import com.blockforge.launcher.data.modrinth.ModrinthSearchResult;
import com.blockforge.launcher.data.modrinth.ModrinthVersion;
import com.blockforge.launcher.dependencies.DependencyPlanner;
import com.blockforge.launcher.dependencies.ModVersion;
import com.blockforge.launcher.dependencies.ModrinthAdapter;
import com.blockforge.launcher.managers.ConfigManager;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.managers.MinecraftManager;
import com.blockforge.launcher.network.Analytics;
import com.blockforge.launcher.utils.Hashing;
import com.blockforge.launcher.utils.ModrinthApi;
import com.blockforge.launcher.utils.Utils;

/**
 * Everything the content manager does to an instance's mods, resource packs and shaders: searching Modrinth,
 * installing the right version with whatever it needs, updating, switching on and off, removing and adding files.
 * It uses the same download and bookkeeping code as the rest of the launcher, and is safe to call off the interface
 * thread (the network and disk work happens wherever it is called).
 */
public final class ContentService {
    /** Progress of one install, as text and as a share done (0 to 1, or below 0 when unknown). */
    public interface Progress {
        void status(String text);

        void fraction(double fraction);
    }

    public static final Progress NO_PROGRESS = new Progress() {
        @Override
        public void status(String text) {
        }

        @Override
        public void fraction(double fraction) {
        }
    };

    /** What an install did. */
    public static final class InstallResult {
        /** names of everything that was added, the thing asked for last */
        public final List<String> installed = new ArrayList<>();
        /** things that didn't go to plan but didn't stop the install */
        public final List<String> warnings = new ArrayList<>();
        /** names of things that were skipped because nothing fits this instance */
        public final List<String> skipped = new ArrayList<>();
    }

    /** What happened to files that were dropped in. */
    public static final class AddFilesResult {
        public final Map<ContentKind, List<String>> added = new EnumMap<>(ContentKind.class);
        /** files that already exist in the instance */
        public final List<String> duplicates = new ArrayList<>();
        /** files that aren't a mod, resource pack or shader pack */
        public final List<String> rejected = new ArrayList<>();
        /** data packs added, with the world each went into */
        public final List<String> dataPacks = new ArrayList<>();

        public int addedCount() {
            int n = 0;
            for (List<String> names : added.values()) {
                n += names.size();
            }
            return n + dataPacks.size();
        }
    }

    /** Thrown when nothing of a project is made for the instance. */
    public static final class NoCompatibleVersionException extends IOException {
        private static final long serialVersionUID = 1L;

        public NoCompatibleVersionException(String message) {
            super(message);
        }
    }

    private final Instance instance;
    private final Map<String, List<ModrinthVersion>> versionCache = new ConcurrentHashMap<>();
    private final Map<String, ModrinthProject> projectCache = new ConcurrentHashMap<>();

    public ContentService(Instance instance) {
        this.instance = instance;
    }

    public Instance instance() {
        return instance;
    }

    public String minecraftVersion() {
        return instance.getMinecraftVersion();
    }

    /** Modrinth's name for the instance's loader, or null for plain Minecraft. */
    public String loaderKey() {
        return LoaderSupport.key(instance.getLoaderVersion());
    }

    public List<String> acceptedLoaders() {
        return LoaderSupport.acceptedLoaders(loaderKey(), minecraftVersion(), ConfigManager
            .getConfigItem("loaders.neoforge.forgeCompatibleMinecraftVersions", new ArrayList<String>()));
    }

    private AddModRestriction restriction() {
        AddModRestriction r = App.settings == null ? null : App.settings.addModRestriction;
        return r == null ? AddModRestriction.STRICT : r;
    }

    /** Minecraft versions near enough to the instance's when nothing is made for the exact one. */
    private List<String> nearbyVersions() {
        if (restriction() != AddModRestriction.LAX) {
            return Collections.emptyList();
        }
        try {
            return MinecraftManager.getMajorMinecraftVersions(minecraftVersion()).stream().map(v -> v.id)
                .collect(Collectors.toList());
        } catch (Exception e) {
            return Collections.emptyList();
        }
    }

    // ---------------------------------------------------------------- searching

    /**
     * Searches Modrinth for one kind of content made for this instance.
     *
     * @param sort Modrinth's sort: relevance, downloads, follows, newest or updated
     * @return the results, or null if Modrinth couldn't be reached
     */
    public ModrinthSearchResult search(ContentKind kind, String query, int page, String sort) {
        List<String> gameVersions = new ArrayList<>();
        switch (restriction()) {
            case NONE:
                gameVersions = null;
                break;
            case LAX:
                gameVersions.addAll(nearbyVersions());
                if (gameVersions.isEmpty()) {
                    gameVersions.add(minecraftVersion());
                }
                break;
            default:
                gameVersions.add(minecraftVersion());
                break;
        }

        List<List<String>> categories = null;
        if (kind == ContentKind.MODS && !acceptedLoaders().isEmpty()) {
            categories = Collections.singletonList(acceptedLoaders());
        }

        ModrinthSearchResult result = ModrinthApi.searchModrinth(gameVersions, query == null ? "" : query, page,
            sort == null ? "relevance" : sort, categories, kind.projectType);
        if (result != null && result.hits == null) {
            result.hits = new ArrayList<>();
        }
        return result;
    }

    /** The most downloaded projects of a kind for this instance, for empty pages. Null if offline. */
    public List<ModrinthSearchHit> popular(ContentKind kind, int count) {
        ModrinthSearchResult result = search(kind, "", 0, "downloads");
        if (result == null) {
            return null;
        }
        return new ArrayList<>(result.hits.subList(0, Math.min(count, result.hits.size())));
    }

    /** Every version of a project, or null if Modrinth couldn't be reached. Remembered for this window. */
    public List<ModrinthVersion> versions(String projectId) {
        List<ModrinthVersion> cached = versionCache.get(projectId);
        if (cached != null) {
            return cached;
        }
        List<ModrinthVersion> fetched = ModrinthApi.getVersions(projectId);
        if (fetched != null) {
            versionCache.put(projectId, fetched);
        }
        return fetched;
    }

    public ModrinthProject project(String projectId) {
        ModrinthProject cached = projectCache.get(projectId);
        if (cached != null) {
            return cached;
        }
        ModrinthProject fetched = ModrinthApi.getProject(projectId);
        if (fetched != null) {
            projectCache.put(projectId, fetched);
            if (fetched.slug != null) {
                projectCache.put(fetched.slug, fetched);
            }
        }
        return fetched;
    }

    /** The version that would be installed, or null if none fits. */
    public ModrinthVersion pick(ContentKind kind, List<ModrinthVersion> versions) {
        if (kind != ContentKind.MODS) {
            return VersionPicker.pick(versions, Collections.<String>emptyList(), minecraftVersion(),
                nearbyVersions());
        }
        // some projects publish one file that is both a mod and a data pack; the launcher would file that as a data
        // pack, so a mod-only file is preferred
        List<ModrinthVersion> modOnly = versions == null ? null
            : versions.stream().filter(v -> v.loaders == null || !v.loaders.contains("datapack"))
                .collect(Collectors.toList());
        ModrinthVersion version = VersionPicker.pick(modOnly, acceptedLoaders(), minecraftVersion(),
            nearbyVersions());
        return version != null ? version
            : VersionPicker.pick(versions, acceptedLoaders(), minecraftVersion(), nearbyVersions());
    }

    /** Whether something of a project is made for this instance. Needs the network. */
    public boolean isAvailable(String projectId, ContentKind kind) {
        List<ModrinthVersion> versions = versions(projectId);
        return versions != null && pick(kind, versions) != null;
    }

    // ---------------------------------------------------------------- what's installed

    /** The instance's content of one kind, sorted by name. */
    public List<DisableableMod> installed(ContentKind kind) {
        List<DisableableMod> all = snapshot();
        return all.stream().filter(m -> m != null && m.type == kind.type && m.wasSelected() && !m.skipped)
            .sorted((a, b) -> String.CASE_INSENSITIVE_ORDER.compare(nameOf(a), nameOf(b)))
            .collect(Collectors.toList());
    }

    public static String nameOf(DisableableMod mod) {
        if (mod.isFromModrinth() && mod.modrinthProject.title != null) {
            return mod.modrinthProject.title;
        }
        return mod.name == null ? mod.file : mod.name;
    }

    /** A copy of the instance's list; installs on other threads may be changing it. */
    private List<DisableableMod> snapshot() {
        for (int attempt = 0; attempt < 3; attempt++) {
            try {
                return new ArrayList<>(instance.getMods());
            } catch (RuntimeException e) {
                // changed while copying, try again
            }
        }
        return new ArrayList<>();
    }

    /** Modrinth projects that are in the instance, counting stand-ins (Quilt's QSL does the job of Fabric API). */
    public Set<String> installedProjectIds() {
        Set<String> ids = new HashSet<>();
        for (DisableableMod mod : snapshot()) {
            if (mod == null) {
                continue;
            }
            if (mod.isFromModrinth()) {
                ids.add(mod.modrinthProject.id);
                if (Constants.MODRINTH_QSL_MOD_ID.equals(mod.modrinthProject.id)) {
                    ids.add(Constants.MODRINTH_FABRIC_MOD_ID);
                }
            }
            if (mod.isFromCurseForge() && mod.getCurseForgeModId() != null
                && mod.getCurseForgeModId() == Constants.CURSEFORGE_FABRIC_MOD_ID) {
                ids.add(Constants.MODRINTH_FABRIC_MOD_ID);
            }
        }
        return ids;
    }

    /** Projects installed from Modrinth, to the version installed. */
    private Map<String, ModrinthVersion> installedVersions() {
        Map<String, ModrinthVersion> versions = new HashMap<>();
        for (DisableableMod mod : snapshot()) {
            if (mod != null && mod.isFromModrinth()) {
                versions.put(mod.modrinthProject.id, mod.modrinthVersion);
            }
        }
        return versions;
    }

    // ---------------------------------------------------------------- installing

    /**
     * Installs the newest version of a project that is made for this instance, and for mods everything it needs.
     */
    public InstallResult install(String projectId, ContentKind kind, Progress progress) throws IOException {
        progress.status("Looking up versions");
        progress.fraction(-1);
        ModrinthProject project = project(projectId);
        List<ModrinthVersion> versions = versions(projectId);
        if (project == null || versions == null) {
            throw new IOException("Couldn't reach Modrinth. Check your connection and try again.");
        }
        ModrinthVersion chosen = pick(kind, versions);
        if (chosen == null) {
            throw new NoCompatibleVersionException(project.title + " has no version for Minecraft "
                + minecraftVersion() + (kind == ContentKind.MODS && loaderKey() != null
                    ? " with " + LoaderSupport.displayName(loaderKey()) : ""));
        }
        return installVersion(project, chosen, kind, progress);
    }

    /** Installs one exact version, and for mods whatever it needs. */
    public InstallResult installVersion(ModrinthProject project, ModrinthVersion chosen, ContentKind kind,
        Progress progress) throws IOException {
        InstallResult result = new InstallResult();
        List<ModrinthVersion> toInstall = new ArrayList<>();

        if (kind == ContentKind.MODS) {
            Map<String, ModrinthVersion> known = new HashMap<>();
            known.put(chosen.id, chosen);
            DependencyPlanner.Plan plan = planDependencies(chosen, known);
            for (DependencyPlanner.Step step : plan.adds) {
                ModrinthVersion version = known.get(step.version.versionId);
                if (version != null) {
                    toInstall.add(version);
                }
            }
            result.warnings.addAll(plan.problems);
            result.warnings.addAll(plan.conflicts);
            if (toInstall.isEmpty() || !toInstall.get(toInstall.size() - 1).id.equals(chosen.id)) {
                toInstall.remove(chosen);
                toInstall.add(chosen);
            }
        } else {
            toInstall.add(chosen);
        }

        int index = 0;
        for (ModrinthVersion version : toInstall) {
            index++;
            boolean main = version == chosen;
            ModrinthProject p = main ? project : project(version.projectId);
            if (p == null) {
                throw new IOException("Couldn't reach Modrinth. Check your connection and try again.");
            }
            String label = toInstall.size() > 1 ? p.title + " (" + index + "/" + toInstall.size() + ")" : p.title;
            progress.status(label);
            download(p, version, main ? kind : ContentKind.MODS,
                main ? ModrinthDownloadMetadata.Reason.STANDALONE : ModrinthDownloadMetadata.Reason.DEPENDENCY,
                main ? null : chosen.id, progress);
            result.installed.add(p.title);
        }
        return result;
    }

    /** Works out the dependencies of a mod version, filling {@code known} with every version the plan refers to. */
    private DependencyPlanner.Plan planDependencies(ModrinthVersion chosen, Map<String, ModrinthVersion> known) {
        Map<String, ModrinthVersion> installed = installedVersions();
        Set<String> provided = installedProjectIds();

        DependencyPlanner.Request request = new DependencyPlanner.Request();
        ModVersion wanted = ModrinthAdapter.toModVersion(chosen);
        // the version was already picked for this instance, maybe from a nearby Minecraft version
        wanted.loaders.clear();
        wanted.gameVersions.clear();
        request.wanted = wanted;
        request.gameVersion = minecraftVersion();
        for (String id : provided) {
            ModrinthVersion v = installed.get(id);
            request.installed.put(id, v != null && v.id != null ? v.id : "installed");
        }
        request.installed.remove(chosen.projectId);
        // a mod that needs one exact version of another (Iris and Sodium) gets it, replacing the one installed
        if (chosen.dependencies != null) {
            for (com.blockforge.launcher.data.modrinth.ModrinthDependency dep : chosen.dependencies) {
                String have = request.installed.get(dep.projectId);
                if (dep.dependencyType == com.blockforge.launcher.data.modrinth.ModrinthDependencyType.REQUIRED
                    && dep.versionId != null && have != null && !have.equals(dep.versionId)
                    && installed.containsKey(dep.projectId)) {
                    request.installed.remove(dep.projectId);
                }
            }
        }

        List<String> loaders = acceptedLoaders();
        return DependencyPlanner.plan(request, projectId -> {
            ModrinthVersion local = installed.get(projectId);
            if (request.installed.containsKey(projectId)) {
                // only asked about to look for clashes: what's installed is known without the network
                return local == null ? Collections.<ModVersion>emptyList()
                    : Collections.singletonList(ModrinthAdapter.toModVersion(local));
            }
            List<ModrinthVersion> all = versions(projectId);
            List<ModVersion> out = new ArrayList<>();
            for (ModrinthVersion v : VersionPicker.compatible(all, loaders, minecraftVersion(),
                Collections.<String>emptyList())) {
                known.put(v.id, v);
                out.add(ModrinthAdapter.toModVersion(v));
            }
            return out;
        });
    }

    private void download(ModrinthProject project, ModrinthVersion version, ContentKind kind,
        ModrinthDownloadMetadata.Reason reason, String dependentVersionId, Progress progress) throws IOException {
        if (version.files == null || version.files.isEmpty()) {
            throw new IOException(project.title + " has no file to download");
        }
        SilentProgress dialog = SilentProgress.create(progress);
        try {
            try {
                Analytics.trackEvent(project.getAnalyticsEventForAdded(version, kind.type));
            } catch (RuntimeException e) {
                // statistics never stop an install
            }
            instance.addFileFromModrinth(project, version, null, kind.type, reason, dependentVersionId, dialog);
        } finally {
            dialog.close();
        }

        boolean added = snapshot().stream().anyMatch(m -> m != null && m.isFromModrinth()
            && project.id.equals(m.modrinthProject.id) && version.id.equals(m.modrinthVersion.id));
        if (!added) {
            throw new IOException("Couldn't download " + project.title + ". Please try again.");
        }
    }

    // ---------------------------------------------------------------- updates

    /**
     * Finds newer versions of the content of one kind that came from Modrinth.
     *
     * @return installed item to its newest version, or null if Modrinth couldn't be reached
     */
    public Map<DisableableMod, ModrinthVersion> findUpdates(ContentKind kind) {
        // a mod that needs one exact version of another (Iris needs a matching Sodium) would break if that one moved on
        Set<String> pinned = new HashSet<>();
        for (DisableableMod mod : snapshot()) {
            if (mod != null && mod.isFromModrinth() && mod.modrinthVersion.dependencies != null) {
                for (com.blockforge.launcher.data.modrinth.ModrinthDependency dep : mod.modrinthVersion.dependencies) {
                    if (dep.dependencyType == com.blockforge.launcher.data.modrinth.ModrinthDependencyType.REQUIRED
                        && dep.versionId != null && dep.projectId != null) {
                        pinned.add(dep.projectId);
                    }
                }
            }
        }

        Map<String, DisableableMod> byHash = new LinkedHashMap<>();
        for (DisableableMod mod : installed(kind)) {
            if (!mod.isFromModrinth() || pinned.contains(mod.modrinthProject.id)) {
                continue;
            }
            Path path = mod.getPath(instance);
            if (path == null || !Files.exists(path)) {
                continue;
            }
            byHash.put(Hashing.sha1(path).toString(), mod);
        }
        Map<DisableableMod, ModrinthVersion> updates = new LinkedHashMap<>();
        if (byHash.isEmpty()) {
            return updates;
        }

        Map<String, ModrinthVersion> latest = ModrinthApi.getLatestVersionsFromSha1Hashes(
            byHash.keySet().toArray(new String[0]), kind == ContentKind.MODS ? acceptedLoaders() : null,
            Collections.singletonList(minecraftVersion()));
        if (latest == null) {
            return null;
        }
        for (Map.Entry<String, DisableableMod> entry : byHash.entrySet()) {
            ModrinthVersion newest = latest.get(entry.getKey());
            ModrinthVersion current = entry.getValue().modrinthVersion;
            // someone on a full release isn't moved to a beta; installs pick releases first too
            boolean riskier = newest != null && newest.versionType != null
                && newest.versionType != com.blockforge.launcher.data.modrinth.ModrinthChannel.RELEASE
                && (current.versionType == null
                    || current.versionType == com.blockforge.launcher.data.modrinth.ModrinthChannel.RELEASE);
            if (newest != null && !riskier && VersionPicker.isNewer(newest, current)) {
                updates.put(entry.getValue(), newest);
            }
        }
        return updates;
    }

    /** Replaces an installed item with a newer version, keeping it switched off if it was. */
    public void update(DisableableMod mod, ModrinthVersion version, Progress progress) throws IOException {
        ContentKind kind = ContentKind.of(mod.type);
        ModrinthProject project = project(mod.modrinthProject.id);
        if (project == null) {
            project = mod.modrinthProject;
        }
        boolean wasDisabled = mod.isDisabled();
        if (wasDisabled) {
            // the download code only clears out the switched-on copy of the old version
            Files.deleteIfExists(mod.getDisabledFile(instance).toPath());
        }
        progress.status(project.title);
        download(project, version, kind == null ? ContentKind.MODS : kind, ModrinthDownloadMetadata.Reason.UPDATE,
            null, progress);
        if (wasDisabled) {
            for (DisableableMod now : snapshot()) {
                if (now != null && now.isFromModrinth() && project.id.equals(now.modrinthProject.id)) {
                    now.disable(instance);
                }
            }
            instance.save();
        }
    }

    // ---------------------------------------------------------------- switching on and off, removing

    public void setEnabled(DisableableMod mod, boolean enabled) {
        if (enabled) {
            mod.enable(instance);
        } else {
            mod.disable(instance);
        }
        instance.save();
    }

    public void remove(DisableableMod mod) {
        instance.removeMod(mod);
    }

    // ---------------------------------------------------------------- files from the computer

    /** What to do when a dropped file has the same name as one already in the instance. */
    public enum Conflict {
        REPLACE, KEEP_BOTH, SKIP
    }

    /** Asked, on the thread doing the copying, what to do with a file that is already there. */
    public interface ConflictHandler {
        Conflict onConflict(DroppedFile file, String where);
    }

    /**
     * Copies dropped files into the right place: mods, resource packs and shader packs into their folders, where they
     * show up in the content manager, and data packs into the given worlds.
     *
     * @param files           what the detector found; modpacks and unknown files are reported as rejected
     * @param dataPackWorlds  folder names of the worlds that get the data packs
     * @param conflicts       decides what happens when a file of the same name is already there
     */
    public AddFilesResult addDropped(List<DroppedFile> files, List<String> dataPackWorlds,
        ConflictHandler conflicts) {
        AddFilesResult result = new AddFilesResult();
        List<DisableableMod> added = new ArrayList<>();
        for (DroppedFile file : files) {
            if (file.kind == DroppedFile.Kind.DATA_PACK) {
                for (String world : dataPackWorlds == null ? Collections.<String>emptyList() : dataPackWorlds) {
                    Path folder = instance.getRoot().resolve("saves").resolve(world).resolve("datapacks");
                    Path target = place(file, folder, null, world, conflicts, result);
                    if (target != null) {
                        result.dataPacks.add(file.fileName + " (" + world + ")");
                    }
                }
                continue;
            }
            ContentKind kind = file.kind.contentKind();
            if (kind == null) {
                result.rejected.add(file.fileName);
                continue;
            }
            Path folder = instance.getRoot().resolve(kind.folder);
            Path target = place(file, folder, kind, instance.launcher.name, conflicts, result);
            if (target == null) {
                continue;
            }
            DisableableMod mod = DisableableMod.generateMod(target.toFile(), kind.type, true);
            if (file.name != null && (mod.name == null || mod.name.equals(target.getFileName().toString()))) {
                mod.name = file.name;
            }
            if (file.version != null && (mod.version == null || "Unknown".equals(mod.version))) {
                mod.version = file.version;
            }
            instance.addMod(mod);
            added.add(mod);
            result.added.computeIfAbsent(kind, k -> new ArrayList<>()).add(file.fileName);
        }
        if (!added.isEmpty()) {
            identify(added);
            instance.save();
        }
        return result;
    }

    /** Copies one file into a folder, asking what to do if one of that name is already there. */
    private Path place(DroppedFile file, Path folder, ContentKind kind, String where, ConflictHandler conflicts,
        AddFilesResult result) {
        Path target = folder.resolve(file.fileName);
        Path disabled = kind == null ? null : instance.getRoot().resolve("disabledmods").resolve(file.fileName);
        boolean taken = Files.exists(target) || (disabled != null && Files.exists(disabled));
        try {
            // dragged out of this very folder: it's already where it belongs, and replacing would delete it
            if (taken && ((Files.exists(target) && Files.isSameFile(file.path, target))
                || (disabled != null && Files.exists(disabled) && Files.isSameFile(file.path, disabled)))) {
                result.duplicates.add(file.fileName);
                return null;
            }
            if (taken) {
                Conflict choice = conflicts == null ? Conflict.SKIP : conflicts.onConflict(file, where);
                if (choice == Conflict.SKIP || choice == null) {
                    result.duplicates.add(file.fileName);
                    return null;
                }
                if (choice == Conflict.REPLACE) {
                    Files.deleteIfExists(target);
                    if (disabled != null) {
                        Files.deleteIfExists(disabled);
                    }
                    if (kind != null) {
                        for (DisableableMod old : snapshot()) {
                            if (old != null && old.type == kind.type && file.fileName.equals(old.file)) {
                                instance.getMods().remove(old);
                            }
                        }
                    }
                } else {
                    target = folder.resolve(uniqueName(file.fileName, name -> Files.exists(folder.resolve(name))
                        || (disabled != null && Files.exists(disabled.getParent().resolve(name)))));
                }
            }
            Files.createDirectories(folder);
            Files.copy(file.path, target, java.nio.file.StandardCopyOption.REPLACE_EXISTING);
            return target;
        } catch (IOException | RuntimeException e) {
            LogManager.logStackTrace("Couldn't copy " + file.fileName, e);
            result.rejected.add(file.fileName);
            return null;
        }
    }

    /** "mod.jar" becomes "mod (2).jar", "mod (3).jar" and so on until the name is free. */
    public static String uniqueName(String fileName, java.util.function.Predicate<String> taken) {
        int dot = fileName.lastIndexOf('.');
        String base = dot > 0 ? fileName.substring(0, dot) : fileName;
        String ext = dot > 0 ? fileName.substring(dot) : "";
        for (int i = 2; i < 1000; i++) {
            String candidate = base + " (" + i + ")" + ext;
            if (!taken.test(candidate)) {
                return candidate;
            }
        }
        return base + " (" + System.currentTimeMillis() + ")" + ext;
    }

    /** The folder names of the instance's worlds. */
    public List<String> worlds() {
        Path saves = instance.getRoot().resolve("saves");
        if (!Files.isDirectory(saves)) {
            return Collections.emptyList();
        }
        try (Stream<Path> stream = Files.list(saves)) {
            return stream.filter(w -> Files.isRegularFile(w.resolve("level.dat")))
                .map(w -> w.getFileName().toString()).sorted(String.CASE_INSENSITIVE_ORDER)
                .collect(Collectors.toList());
        } catch (IOException e) {
            return Collections.emptyList();
        }
    }

    /**
     * Records files that were put into the instance's folders by hand, so they show up and can be switched off.
     *
     * @return true if anything new was found
     */
    public boolean scanUntracked() {
        List<DisableableMod> mods = snapshot();
        List<DisableableMod> found = new ArrayList<>();
        for (ContentKind kind : ContentKind.values()) {
            Path folder = instance.getRoot().resolve(kind.folder);
            if (!Files.isDirectory(folder)) {
                continue;
            }
            try (Stream<Path> stream = Files.list(folder)) {
                for (Path file : stream.collect(Collectors.toList())) {
                    String name = file.getFileName().toString();
                    if (Files.isDirectory(file) || !Utils.isAcceptedModFile(name)) {
                        continue;
                    }
                    boolean known = mods.stream().anyMatch(m -> m != null && name.equals(m.file)
                        && (m.type == kind.type || ContentKind.of(m.type) == null));
                    if (!known) {
                        found.add(DisableableMod.generateMod(file.toFile(), kind.type, true));
                    }
                }
            } catch (IOException e) {
                LogManager.logStackTrace("Couldn't look through " + folder, e);
            }
        }
        if (found.isEmpty()) {
            return false;
        }
        identify(found);
        instance.addMods(found);
        instance.save();
        return true;
    }

    /** Asks Modrinth what files are, so they get a proper name, a picture and update checks. */
    private void identify(List<DisableableMod> mods) {
        Map<String, DisableableMod> byHash = new HashMap<>();
        for (DisableableMod mod : mods) {
            Path path = mod.getPath(instance);
            if (path != null && Files.exists(path)) {
                byHash.put(Hashing.sha1(path).toString(), mod);
            }
        }
        if (byHash.isEmpty()) {
            return;
        }
        try {
            Map<String, ModrinthVersion> versions = ModrinthApi
                .getVersionsFromSha1Hashes(byHash.keySet().toArray(new String[0]));
            if (versions == null || versions.isEmpty()) {
                return;
            }
            Set<String> projectIds = versions.values().stream().filter(v -> v != null && v.projectId != null)
                .map(v -> v.projectId).collect(Collectors.toSet());
            Map<String, ModrinthProject> projects = ModrinthApi.getProjectsAsMap(projectIds.toArray(new String[0]));
            if (projects == null) {
                return;
            }
            for (Map.Entry<String, ModrinthVersion> entry : versions.entrySet()) {
                DisableableMod mod = byHash.get(entry.getKey());
                ModrinthVersion version = entry.getValue();
                ModrinthProject project = version == null ? null : projects.get(version.projectId);
                if (mod == null || project == null) {
                    continue;
                }
                mod.modrinthProject = project;
                mod.modrinthVersion = version;
                mod.name = project.title;
                mod.version = version.name;
                mod.description = project.description;
            }
        } catch (RuntimeException e) {
            LogManager.debug("Couldn't identify files on Modrinth: " + e.getMessage());
        }
    }

    // ---------------------------------------------------------------- shaders and the performance pack

    public LoaderSupport.ShaderSetup shaderSetup() {
        List<String> files = new ArrayList<>();
        for (DisableableMod mod : installed(ContentKind.MODS)) {
            if (!mod.isDisabled()) {
                files.add(mod.file);
            }
        }
        Set<String> ids = new HashSet<>();
        for (DisableableMod mod : installed(ContentKind.MODS)) {
            if (!mod.isDisabled() && mod.isFromModrinth()) {
                ids.add(mod.modrinthProject.id);
            }
        }
        return LoaderSupport.shaderSetup(loaderKey(), LoaderSupport.hasShaderLoader(ids, files));
    }

    /** The project that runs shaders on this instance (Iris or Oculus), or null if there is none. */
    public String shaderLoaderProject() {
        switch (LoaderSupport.shaderSetup(loaderKey(), false)) {
            case ADD_IRIS:
                return LoaderSupport.IRIS;
            case ADD_OCULUS:
                return LoaderSupport.OCULUS;
            default:
                return null;
        }
    }

    /** Installs Iris (with Sodium) or Oculus, whichever fits the loader. */
    public InstallResult installShaderLoader(Progress progress) throws IOException {
        String loader = shaderLoaderProject();
        if (loader == null) {
            throw new NoCompatibleVersionException("Shaders need a mod loader such as Fabric");
        }
        InstallResult result = install(loader, ContentKind.MODS, progress);
        String key = loaderKey();
        boolean irisLoader = LoaderSupport.IRIS.equals(loader);
        if (irisLoader && !installedProjectIds().contains(LoaderSupport.SODIUM)
            && !LoaderSupport.FORGE.equals(key) && isAvailable(LoaderSupport.SODIUM, ContentKind.MODS)) {
            InstallResult sodium = install(LoaderSupport.SODIUM, ContentKind.MODS, progress);
            result.installed.addAll(sodium.installed);
        }
        return result;
    }

    /** Installs the performance mods that are available for this instance and not there yet. */
    public InstallResult installPerformancePack(Progress progress) {
        InstallResult result = new InstallResult();
        List<PerformancePack.Slot> slots = PerformancePack.slotsFor(loaderKey());
        progress.status("Checking what's available");
        Set<String> installed = installedProjectIds();
        List<String> ids = PerformancePack.choose(slots, id -> isAvailable(id, ContentKind.MODS), installed);
        for (PerformancePack.Slot slot : slots) {
            boolean covered = false;
            for (String id : slot.projectIds) {
                covered |= ids.contains(id) || installed.contains(id);
            }
            if (!covered) {
                result.skipped.add(PerformancePack.nameOf(slot.projectIds.get(0)));
            }
        }
        for (String id : ids) {
            try {
                InstallResult one = install(id, ContentKind.MODS, progress);
                result.installed.add(PerformancePack.nameOf(id));
                result.warnings.addAll(one.warnings);
            } catch (IOException e) {
                LogManager.warn("Couldn't add " + PerformancePack.nameOf(id) + ": " + e.getMessage());
                result.warnings.add(e.getMessage());
                result.skipped.add(PerformancePack.nameOf(id));
            }
        }
        return result;
    }
}
