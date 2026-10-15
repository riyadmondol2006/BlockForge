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
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.file.DirectoryStream;
import java.nio.file.FileVisitResult;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.nio.file.SimpleFileVisitor;
import java.nio.file.StandardCopyOption;
import java.nio.file.attribute.BasicFileAttributes;
import java.text.SimpleDateFormat;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.Comparator;
import java.util.Date;
import java.util.Enumeration;
import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Set;
import java.util.TreeSet;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;
import java.util.zip.ZipOutputStream;

/**
 * Saves the parts of an instance that a modpack update can change (mods, configs, scripts), and puts them back. An
 * update never touches worlds or personal settings, so they aren't included.
 */
public final class InstanceSnapshots {
    /** top level entries that belong to the pack and are changed by updates */
    static final Set<String> MANAGED = new HashSet<>(Arrays.asList("mods", "config", "defaultconfigs", "kubejs",
        "scripts", "bin", "instance.json"));

    /** how many snapshots are kept for an instance, the oldest go first */
    public static final int KEEP = 8;

    /** One saved state. */
    public static final class Snapshot {
        public final Path file;
        public final long createdAt;
        public final long size;

        Snapshot(Path file, long createdAt, long size) {
            this.file = file;
            this.createdAt = createdAt;
            this.size = size;
        }

        public String label() {
            String name = file.getFileName().toString();
            name = name.substring(0, name.length() - ".zip".length());
            // the name is date-time-millis-label
            String[] parts = name.split("-", 4);
            return parts.length == 4 ? parts[3] : name;
        }
    }

    private InstanceSnapshots() {
    }

    /** Saves the instance's managed files into a new snapshot and trims old ones. */
    public static Snapshot create(Path instanceRoot, Path snapshotsDir, String label) throws IOException {
        Files.createDirectories(snapshotsDir);

        String stamp = new SimpleDateFormat("yyyyMMdd-HHmmss-SSS", Locale.ROOT).format(new Date());
        String safeLabel = label.replaceAll("[^A-Za-z0-9_.-]", "_");
        Path zip = snapshotsDir.resolve(stamp + "-" + safeLabel + ".zip");

        try (OutputStream out = Files.newOutputStream(zip); ZipOutputStream z = new ZipOutputStream(out)) {
            // mods are already compressed jars, so storing them is much faster and hardly larger
            z.setLevel(1);
            try (DirectoryStream<Path> top = Files.newDirectoryStream(instanceRoot)) {
                for (Path entry : top) {
                    if (!MANAGED.contains(entry.getFileName().toString()) || Files.isSymbolicLink(entry)) {
                        continue;
                    }
                    addTree(z, instanceRoot, entry);
                }
            }
        } catch (IOException | RuntimeException e) {
            Files.deleteIfExists(zip);
            throw e;
        }

        trim(snapshotsDir);
        return new Snapshot(zip, Files.getLastModifiedTime(zip).toMillis(), Files.size(zip));
    }

    private static void addTree(ZipOutputStream z, Path root, Path start) throws IOException {
        Files.walkFileTree(start, new SimpleFileVisitor<Path>() {
            @Override
            public FileVisitResult preVisitDirectory(Path dir, BasicFileAttributes attrs) throws IOException {
                if (Files.isSymbolicLink(dir)) {
                    return FileVisitResult.SKIP_SUBTREE;
                }
                // folders get an entry of their own, so an empty mods or config folder comes back on a restore
                z.putNextEntry(new ZipEntry(root.relativize(dir).toString().replace('\\', '/') + "/"));
                z.closeEntry();
                return FileVisitResult.CONTINUE;
            }

            @Override
            public FileVisitResult visitFile(Path file, BasicFileAttributes attrs) throws IOException {
                if (!attrs.isRegularFile() || attrs.isSymbolicLink()) {
                    return FileVisitResult.CONTINUE;
                }

                z.putNextEntry(new ZipEntry(root.relativize(file).toString().replace('\\', '/')));
                Files.copy(file, z);
                z.closeEntry();
                return FileVisitResult.CONTINUE;
            }
        });
    }

    /** Snapshots of an instance, newest first. */
    public static List<Snapshot> list(Path snapshotsDir) {
        List<Snapshot> found = new ArrayList<>();
        if (!Files.isDirectory(snapshotsDir)) {
            return found;
        }

        try (DirectoryStream<Path> files = Files.newDirectoryStream(snapshotsDir, "*.zip")) {
            for (Path file : files) {
                found.add(new Snapshot(file, Files.getLastModifiedTime(file).toMillis(), Files.size(file)));
            }
        } catch (IOException ignored) {
            // nothing readable means nothing to offer
        }

        // the names start with a timestamp, which is a better order than modified times that copying can change
        found.sort(Comparator.comparing((Snapshot s) -> s.file.getFileName().toString()).reversed());
        return found;
    }

    private static void trim(Path snapshotsDir) {
        List<Snapshot> all = list(snapshotsDir);
        for (int i = KEEP; i < all.size(); i++) {
            try {
                Files.deleteIfExists(all.get(i).file);
            } catch (IOException ignored) {
                // an old snapshot that can't be removed now will be next time
            }
        }
    }

    /**
     * Puts a snapshot back. The current state is saved as a snapshot first, so a restore can itself be undone, and if
     * the restore fails halfway the instance is returned to how it was.
     */
    public static void restore(Snapshot snapshot, Path instanceRoot, Path snapshotsDir) throws IOException {
        Path staging = Files.createTempDirectory(instanceRoot.toAbsolutePath().getParent(), ".restore-");
        Snapshot before = null;

        try {
            // unpack first: saving the current state trims the oldest snapshots, which could be this one
            extract(snapshot.file, staging);
            before = create(instanceRoot, snapshotsDir, "before-restore");

            // every part a snapshot manages goes back to how it was, so something an update added since (a new
            // kubejs folder, say) is taken away again. Worlds and anything else in the instance are left alone.
            Set<String> names = new TreeSet<>(MANAGED);
            try (DirectoryStream<Path> staged = Files.newDirectoryStream(staging)) {
                for (Path entry : staged) {
                    names.add(entry.getFileName().toString());
                }
            }

            List<Path> replaced = new ArrayList<>();
            Path parked = Files.createTempDirectory(instanceRoot.toAbsolutePath().getParent(), ".parked-");
            try {
                for (String name : names) {
                    Path entry = staging.resolve(name);
                    Path current = instanceRoot.resolve(name);
                    boolean inSnapshot = Files.exists(entry, LinkOption.NOFOLLOW_LINKS);

                    // links are never saved in a snapshot, so one the snapshot knows nothing about is left as it is
                    if (!inSnapshot && Files.isSymbolicLink(current)) {
                        continue;
                    }
                    if (Files.exists(current, LinkOption.NOFOLLOW_LINKS)) {
                        Files.move(current, parked.resolve(name));
                    }
                    if (inSnapshot) {
                        Files.move(entry, current);
                        replaced.add(current);
                    }
                }
            } catch (IOException | RuntimeException e) {
                // put back what was moved out of the way
                for (Path done : replaced) {
                    deleteTree(done);
                }
                try (DirectoryStream<Path> held = Files.newDirectoryStream(parked)) {
                    for (Path entry : held) {
                        Files.move(entry, instanceRoot.resolve(entry.getFileName().toString()),
                            StandardCopyOption.REPLACE_EXISTING);
                    }
                }
                throw e;
            } finally {
                deleteTree(parked);
            }
        } catch (IOException | RuntimeException e) {
            if (before != null) {
                Files.deleteIfExists(before.file);
            }
            throw e;
        } finally {
            deleteTree(staging);
        }
    }

    /** unpacks a snapshot, refusing any entry that would land outside the destination */
    static void extract(Path zip, Path destination) throws IOException {
        Path root = destination.toAbsolutePath().normalize();
        try (ZipFile z = new ZipFile(zip.toFile())) {
            Enumeration<? extends ZipEntry> entries = z.entries();
            while (entries.hasMoreElements()) {
                ZipEntry entry = entries.nextElement();
                Path target = root.resolve(entry.getName()).normalize();
                if (!target.startsWith(root)) {
                    throw new IOException("The snapshot contains an unsafe path: " + entry.getName());
                }

                if (entry.isDirectory()) {
                    Files.createDirectories(target);
                    continue;
                }

                Files.createDirectories(target.getParent());
                try (InputStream in = z.getInputStream(entry)) {
                    Files.copy(in, target, StandardCopyOption.REPLACE_EXISTING);
                }
            }
        }
    }

    private static void deleteTree(Path path) {
        if (path == null || !Files.exists(path, LinkOption.NOFOLLOW_LINKS)) {
            return;
        }
        try {
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

    /** the newest first, convenient for menus */
    public static List<Snapshot> latest(Path snapshotsDir, int count) {
        List<Snapshot> all = list(snapshotsDir);
        return all.size() <= count ? all : Collections.unmodifiableList(all.subList(0, count));
    }
}
