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
import java.nio.file.DirectoryStream;
import java.nio.file.FileVisitResult;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.nio.file.SimpleFileVisitor;
import java.nio.file.StandardCopyOption;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.HashSet;
import java.util.List;
import java.util.Optional;
import java.util.Set;

/**
 * Finds instances made by other launchers and brings their files across. Nothing is changed until the files have all
 * been copied somewhere safe, so a failed import leaves the instance exactly as it was.
 */
public final class InstanceImporter {
    private static final List<ImportAdapter> ADAPTERS = Collections.unmodifiableList(Arrays.asList(
        new Adapters.Prism(), new Adapters.CurseForge(), new Adapters.GdLauncher(), new Adapters.Atlauncher()));

    /** things that are rebuilt by the launcher or only hold history, so they aren't worth carrying over */
    private static final Set<String> SKIPPED = new HashSet<>(Arrays.asList("logs", "crash-reports", "debug", "natives",
        "bin", "cache", ".cache", ".fabric", ".mixin.out", "backups", "libraries", "versions", "assets", "temp",
        "instance.json", "instance.cfg", "mmc-pack.json", "minecraftinstance.json", "config.json", "patches",
        "icon.png", "instance.png", ".git", "hs_err_pid"));

    private InstanceImporter() {
    }

    /** the first adapter that recognises the folder, if any */
    public static Optional<ImportAdapter> detect(Path folder) {
        if (folder == null || !Files.isDirectory(folder)) {
            return Optional.empty();
        }
        return ADAPTERS.stream().filter(a -> a.matches(folder)).findFirst();
    }

    public static ImportedInstance read(Path folder) throws IOException {
        ImportAdapter adapter = detect(folder)
            .orElseThrow(() -> new IOException("This folder doesn't look like an instance from a supported launcher"));
        ImportedInstance instance = adapter.read(folder);

        if (!Files.isDirectory(instance.gameDir)) {
            throw new IOException("The instance has no game folder, so there is nothing to import");
        }
        return instance;
    }

    /** What an import would bring in, worked out without changing anything. */
    public static final class Preview {
        public int mods;
        public int worlds;
        public int resourcePacks;
        public int shaderPacks;
        public int files;
        public long bytes;
        public final List<String> topLevel = new ArrayList<>();
    }

    public static Preview preview(ImportedInstance instance) throws IOException {
        Preview preview = new Preview();

        try (DirectoryStream<Path> top = Files.newDirectoryStream(instance.gameDir)) {
            for (Path entry : top) {
                if (isSkipped(entry) || Files.isSymbolicLink(entry)) {
                    continue;
                }

                String name = entry.getFileName().toString();
                preview.topLevel.add(name);

                if (Files.isDirectory(entry)) {
                    if (name.equals("mods")) {
                        preview.mods = countFiles(entry, ".jar", ".zip", ".disabled");
                    } else if (name.equals("saves")) {
                        preview.worlds = countDirectories(entry);
                    } else if (name.equals("resourcepacks")) {
                        preview.resourcePacks = countChildren(entry);
                    } else if (name.equals("shaderpacks")) {
                        preview.shaderPacks = countChildren(entry);
                    }
                }
            }
        }

        Collections.sort(preview.topLevel);
        Files.walkFileTree(instance.gameDir, new SimpleFileVisitor<Path>() {
            @Override
            public FileVisitResult preVisitDirectory(Path dir, BasicFileAttributes attrs) {
                return !dir.equals(instance.gameDir) && (isSkipped(dir) || Files.isSymbolicLink(dir))
                    ? FileVisitResult.SKIP_SUBTREE : FileVisitResult.CONTINUE;
            }

            @Override
            public FileVisitResult visitFile(Path file, BasicFileAttributes attrs) {
                if (!isSkipped(file) && attrs.isRegularFile()) {
                    preview.files++;
                    preview.bytes += attrs.size();
                }
                return FileVisitResult.CONTINUE;
            }
        });

        return preview;
    }

    private static boolean isSkipped(Path path) {
        String name = path.getFileName().toString();
        return SKIPPED.contains(name) || name.startsWith("hs_err_pid") || name.endsWith(".log");
    }

    private static int countFiles(Path dir, String... suffixes) throws IOException {
        int count = 0;
        try (DirectoryStream<Path> stream = Files.newDirectoryStream(dir)) {
            for (Path p : stream) {
                String name = p.getFileName().toString().toLowerCase();
                for (String suffix : suffixes) {
                    if (Files.isRegularFile(p) && name.endsWith(suffix)) {
                        count++;
                        break;
                    }
                }
            }
        }
        return count;
    }

    private static int countDirectories(Path dir) throws IOException {
        int count = 0;
        try (DirectoryStream<Path> stream = Files.newDirectoryStream(dir)) {
            for (Path p : stream) {
                if (Files.isDirectory(p)) {
                    count++;
                }
            }
        }
        return count;
    }

    private static int countChildren(Path dir) throws IOException {
        int count = 0;
        try (DirectoryStream<Path> stream = Files.newDirectoryStream(dir)) {
            for (Path ignored : stream) {
                count++;
            }
        }
        return count;
    }

    /**
     * Brings the instance's files into {@code target}. The files are copied to a staging folder first and only moved in
     * once everything has been copied; if anything goes wrong what was moved in is taken out again.
     */
    public static void copyInto(ImportedInstance instance, Path target) throws IOException {
        Path root = instance.gameDir.toRealPath();
        Path staging = Files.createTempDirectory(target.toAbsolutePath().getParent(), ".import-");

        try {
            try (DirectoryStream<Path> top = Files.newDirectoryStream(root)) {
                for (Path entry : top) {
                    if (isSkipped(entry) || Files.isSymbolicLink(entry)) {
                        continue;
                    }
                    copyTree(root, entry, staging);
                }
            }

            List<Path> placed = new ArrayList<>();
            try {
                try (DirectoryStream<Path> staged = Files.newDirectoryStream(staging)) {
                    for (Path entry : staged) {
                        moveInto(entry, target.resolve(entry.getFileName().toString()), placed);
                    }
                }
            } catch (IOException | RuntimeException e) {
                for (int i = placed.size() - 1; i >= 0; i--) {
                    deleteQuietly(placed.get(i));
                }
                throw e;
            }
        } finally {
            deleteQuietly(staging);
        }
    }

    /** copies a file or folder under {@code to}, keeping its path relative to {@code from}, and never leaves {@code from} */
    private static void copyTree(Path from, Path source, Path to) throws IOException {
        Files.walkFileTree(source, new SimpleFileVisitor<Path>() {
            private Path destination(Path path) throws IOException {
                Path relative = from.relativize(path);
                Path dest = to.resolve(relative.toString()).normalize();
                // a path that climbs out of the folder it is meant to stay in must never be written
                if (!dest.startsWith(to)) {
                    throw new IOException("Refusing to write outside the instance: " + relative);
                }
                return dest;
            }

            @Override
            public FileVisitResult preVisitDirectory(Path dir, BasicFileAttributes attrs) throws IOException {
                if (Files.isSymbolicLink(dir) || (!dir.equals(source) && isSkipped(dir))) {
                    return FileVisitResult.SKIP_SUBTREE;
                }
                Files.createDirectories(destination(dir));
                return FileVisitResult.CONTINUE;
            }

            @Override
            public FileVisitResult visitFile(Path file, BasicFileAttributes attrs) throws IOException {
                if (attrs.isRegularFile() && !attrs.isSymbolicLink() && !isSkipped(file)) {
                    Path dest = destination(file);
                    Files.createDirectories(dest.getParent());
                    Files.copy(file, dest, StandardCopyOption.COPY_ATTRIBUTES);
                }
                return FileVisitResult.CONTINUE;
            }
        });
    }

    private static void moveInto(Path staged, Path destination, List<Path> placed) throws IOException {
        if (!Files.exists(destination, LinkOption.NOFOLLOW_LINKS)) {
            Files.move(staged, destination);
            placed.add(destination);
            return;
        }

        if (Files.isDirectory(staged) && Files.isDirectory(destination)) {
            try (DirectoryStream<Path> children = Files.newDirectoryStream(staged)) {
                for (Path child : children) {
                    moveInto(child, destination.resolve(child.getFileName().toString()), placed);
                }
            }
            return;
        }

        // a file that is already there is replaced by the imported one
        Files.move(staged, destination, StandardCopyOption.REPLACE_EXISTING);
        placed.add(destination);
    }

    private static void deleteQuietly(Path path) {
        try {
            if (!Files.exists(path, LinkOption.NOFOLLOW_LINKS)) {
                return;
            }
            Files.walkFileTree(path, new SimpleFileVisitor<Path>() {
                @Override
                public FileVisitResult visitFile(Path file, BasicFileAttributes attrs) throws IOException {
                    Files.deleteIfExists(file);
                    return FileVisitResult.CONTINUE;
                }

                @Override
                public FileVisitResult postVisitDirectory(Path dir, IOException exc) throws IOException {
                    Files.deleteIfExists(dir);
                    return FileVisitResult.CONTINUE;
                }
            });
        } catch (IOException ignored) {
            // best effort clean up
        }
    }

    /** Looks inside the folders where launchers keep their instances for ones that can be imported. */
    public static List<ImportedInstance> scan(List<Path> roots) {
        List<ImportedInstance> found = new ArrayList<>();
        for (Path root : roots) {
            if (!Files.isDirectory(root)) {
                continue;
            }

            try (DirectoryStream<Path> children = Files.newDirectoryStream(root)) {
                for (Path child : children) {
                    if (!Files.isDirectory(child)) {
                        continue;
                    }
                    try {
                        found.add(read(child));
                    } catch (IOException ignored) {
                        // not an instance, or unreadable: leave it out of the list
                    }
                }
            } catch (IOException ignored) {
                // an unreadable launcher folder just has nothing to offer
            }
        }
        found.sort((a, b) -> a.name.compareToIgnoreCase(b.name));
        return found;
    }

    /** where other launchers usually keep their instances on this computer */
    public static List<Path> defaultRoots() {
        Path home = Path.of(System.getProperty("user.home"));
        String os = System.getProperty("os.name", "").toLowerCase();
        List<Path> roots = new ArrayList<>();

        if (os.contains("win")) {
            String appData = System.getenv("APPDATA");
            Path data = appData == null ? home.resolve("AppData/Roaming") : Path.of(appData);
            roots.add(data.resolve("PrismLauncher/instances"));
            roots.add(data.resolve(".atlauncher/instances"));
            roots.add(data.resolve("gdlauncher_carbon/data/instances"));
            roots.add(home.resolve("curseforge/minecraft/Instances"));
            roots.add(home.resolve("Documents/curseforge/minecraft/Instances"));
        } else if (os.contains("mac")) {
            Path support = home.resolve("Library/Application Support");
            roots.add(support.resolve("PrismLauncher/instances"));
            roots.add(support.resolve("MultiMC/instances"));
            roots.add(support.resolve("ATLauncher/instances"));
            roots.add(support.resolve("gdlauncher_carbon/data/instances"));
            roots.add(home.resolve("curseforge/minecraft/Instances"));
        } else {
            roots.add(home.resolve(".local/share/PrismLauncher/instances"));
            roots.add(home.resolve(".var/app/org.prismlauncher.PrismLauncher/data/PrismLauncher/instances"));
            roots.add(home.resolve(".local/share/multimc/instances"));
            roots.add(home.resolve(".atlauncher/instances"));
            roots.add(home.resolve(".local/share/gdlauncher_carbon/data/instances"));
            roots.add(home.resolve("curseforge/minecraft/Instances"));
        }
        return roots;
    }
}
