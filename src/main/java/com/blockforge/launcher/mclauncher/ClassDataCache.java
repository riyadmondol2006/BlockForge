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
package com.blockforge.launcher.mclauncher;

import java.io.IOException;
import java.io.Reader;
import java.io.Writer;
import java.nio.charset.StandardCharsets;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.TimeUnit;

import com.blockforge.launcher.FileSystem;
import com.blockforge.launcher.Gsons;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.utils.OS;
import com.blockforge.launcher.utils.Utils;

/**
 * Java's class-data sharing for the game: an archive of the classes a game's start loads (Java itself and the
 * libraries), already parsed and checked, which Java maps straight into memory on the next start instead of reading
 * and verifying thousands of classes again. Measured on 26.3, 1.21.11 and 1.20.1: starts about 8% (0.5-0.8 s)
 * quicker. Minecraft's own classes can't be shared, as its jar is signed.
 *
 * <p>
 * How: a start without an archive also writes down which classes it loaded (no measurable cost); once the game has
 * closed, a background Java builds the archive from that list. Every later start of the same game with the same Java
 * and the same libraries uses it. When any of that changes (a Java or library update, another loader version), the
 * archive no longer matches: it is thrown away and made again the same way.
 *
 * <p>
 * Java's own "automatic" archives (-XX:+AutoCreateSharedArchive, Java 19+) and the Java 25 AOT cache were measured
 * too and not used: the start that makes them takes about three times as long (21 s instead of 7.7 s on 26.3), the
 * automatic archive needs a base archive Mojang's Java doesn't ship, and a stale one isn't replaced.
 *
 * <p>
 * Safe by design: Java only uses an archive that matches exactly and passes its checksum, and otherwise starts
 * normally (checked with truncated, random, damaged, foreign and stale archives on Java 17, 21 and 25). Only Java 17
 * and newer; never Java 8-16. Fabric's class list can't be archived (Java rejects it); that is remembered, so it
 * isn't tried at every start.
 */
public final class ClassDataCache {
    /** the oldest Java this is used with (1.18 and newer run on 17 or newer) */
    public static final int MINIMUM_JAVA = 17;
    /** a list shorter than this is from a start that didn't get far (a crash): record again next time */
    static final int MINIMUM_CLASSES = 2000;
    /** building the archive takes 20-50 s; give up well after that */
    static final long DUMP_TIMEOUT_MINUTES = 6;

    /** what to do for one start */
    public enum Action {
        /** nothing: turned off, an old Java, the person's own sharing options, or the archive couldn't be made */
        NONE,
        /** use the archive */
        USE,
        /** write down the classes, so the archive can be made after the game closes */
        RECORD
    }

    /** What is kept next to an archive. */
    static final class State {
        String fingerprint;
        /** "recording", "ready" or "failed" */
        String status;
        String java;
        int javaMajor;
        /** the archive's size when it was made: a different size means it was cut short or replaced */
        long archiveBytes;
        List<String> options;
    }

    /** an archive waiting to be built once its game closes */
    private static final Map<String, State> PENDING = new ConcurrentHashMap<>();

    private ClassDataCache() {
    }

    // ---- pure decisions ------------------------------------------------------------------------------------------

    /**
     * If the person's own Java arguments already decide about class-data sharing, in which case they are left to it.
     */
    public static boolean userControlsSharing(List<String> userArguments) {
        if (userArguments == null) {
            return false;
        }
        for (String argument : userArguments) {
            String a = argument.trim();
            if (a.startsWith("-Xshare") || a.startsWith("-XX:SharedArchiveFile")
                || a.startsWith("-XX:ArchiveClassesAtExit") || a.contains("AutoCreateSharedArchive")
                || a.startsWith("-XX:SharedClassListFile") || a.startsWith("-XX:DumpLoadedClassList")
                || a.startsWith("-XX:+DumpSharedSpaces") || a.startsWith("-XX:AOTCache")
                || a.startsWith("-XX:AOTMode") || a.startsWith("-XX:AOTConfiguration")) {
                return true;
            }
        }
        return false;
    }

    /**
     * What to do for a start.
     *
     * @param enabled       the setting
     * @param javaMajor     the game's Java version (0 or less when unknown)
     * @param userArguments the person's own Java arguments
     * @param state         what was saved for this game and Java, or null
     * @param fingerprint   the fingerprint of this start (Java, libraries and module options)
     * @param archiveBytes  the size of the archive file, or -1 when it isn't there
     */
    public static Action decide(boolean enabled, int javaMajor, List<String> userArguments, State state,
        String fingerprint, long archiveBytes) {
        if (!enabled || javaMajor < MINIMUM_JAVA || userControlsSharing(userArguments)) {
            return Action.NONE;
        }
        if (state != null && fingerprint.equals(state.fingerprint)) {
            if ("ready".equals(state.status) && archiveBytes > 0
                && (state.archiveBytes <= 0 || state.archiveBytes == archiveBytes)) {
                return Action.USE;
            }
            if ("failed".equals(state.status)) {
                // this combination can't be archived (some mod loaders); don't try at every start
                return Action.NONE;
            }
        }
        return Action.RECORD;
    }

    /** The Java options for an action. */
    public static List<String> flags(Action action, Path archive, Path classList) {
        List<String> flags = new ArrayList<>();
        if (action == Action.USE) {
            flags.add("-XX:SharedArchiveFile=" + archive.toAbsolutePath());
            // checks the archive's checksum first (about 0.1 s): Java only compares the header, and a file damaged
            // in the middle (a bad disk, a crash while copying) made the game crash on start in testing; with the
            // check it is skipped and the game starts normally
            flags.add("-XX:+VerifySharedSpaces");
            // a stale or broken archive is skipped with a message; it is harmless, so keep it out of the game's log
            flags.add("-Xlog:cds=off");
        } else if (action == Action.RECORD) {
            flags.add("-XX:DumpLoadedClassList=" + classList.toAbsolutePath());
        }
        return flags;
    }

    /**
     * The options from the game's Java command line that the archive depends on and that building it needs: the
     * class path and the module options. Taken from the options before the main class.
     */
    public static List<String> archiveOptions(List<String> javaOptions) {
        List<String> options = new ArrayList<>();
        for (int i = 0; i < javaOptions.size(); i++) {
            String a = javaOptions.get(i);
            boolean takesValue = a.equals("-cp") || a.equals("-classpath") || a.equals("--class-path")
                || a.equals("-p") || a.equals("--module-path") || a.equals("--add-modules")
                || a.equals("--add-exports") || a.equals("--add-opens") || a.equals("--add-reads")
                || a.equals("--enable-native-access");
            if (takesValue && i + 1 < javaOptions.size()) {
                options.add(a);
                options.add(javaOptions.get(i + 1));
                i++;
            } else if (a.startsWith("--add-") || a.startsWith("--enable-native-access=")
                || a.startsWith("--module-path=") || a.startsWith("--class-path=")) {
                options.add(a);
            }
        }
        return options;
    }

    /** A fingerprint of everything the archive depends on. */
    public static String fingerprint(String javaExecutable, String javaVersion, List<String> archiveOptions) {
        StringBuilder sb = new StringBuilder();
        sb.append(javaExecutable).append('\n').append(javaVersion).append('\n');
        for (String option : archiveOptions) {
            sb.append(option).append('\n');
        }
        try {
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            byte[] hash = digest.digest(sb.toString().getBytes(StandardCharsets.UTF_8));
            StringBuilder hex = new StringBuilder();
            for (int i = 0; i < 16; i++) {
                hex.append(String.format(Locale.ROOT, "%02x", hash[i]));
            }
            return hex.toString();
        } catch (Exception e) {
            return Integer.toHexString(sb.toString().hashCode());
        }
    }

    /**
     * The recorded class list, ready for building the archive: the lines that describe lambdas and method handles
     * are left out, as building them runs code (and fails on some libraries).
     */
    static List<String> usableClassList(List<String> lines) {
        List<String> usable = new ArrayList<>();
        for (String line : lines) {
            if (!line.startsWith("@")) {
                usable.add(line);
            }
        }
        return usable;
    }

    static int countClasses(List<String> lines) {
        int count = 0;
        for (String line : lines) {
            if (!line.isEmpty() && !line.startsWith("#") && !line.startsWith("@")) {
                count++;
            }
        }
        return count;
    }

    // ---- files -----------------------------------------------------------------------------------------------------

    /** Where the archives live: the launcher's cache folder, so they never end up in instance exports or backups. */
    public static Path folder() {
        return FileSystem.CACHE.resolve("class-cache");
    }

    static String baseName(String instanceSafeName, int javaMajor) {
        String safe = instanceSafeName == null ? "" : instanceSafeName.replaceAll("[^A-Za-z0-9._-]", "");
        if (safe.isEmpty()) {
            safe = "instance";
        }
        return safe + "-" + javaMajor;
    }

    static String fileName(String instanceSafeName, int javaMajor) {
        return baseName(instanceSafeName, javaMajor) + ".jsa";
    }

    /** The archive for one instance on one Java version. */
    public static Path archiveFor(String instanceSafeName, int javaMajor) {
        return folder().resolve(fileName(instanceSafeName, javaMajor));
    }

    static State readState(Path file) {
        if (!Files.isRegularFile(file)) {
            return null;
        }
        try (Reader reader = Files.newBufferedReader(file, StandardCharsets.UTF_8)) {
            return Gsons.DEFAULT_SLIM.fromJson(reader, State.class);
        } catch (Exception e) {
            return null;
        }
    }

    static void writeState(Path file, State state) {
        try {
            Files.createDirectories(file.getParent());
            try (Writer writer = Files.newBufferedWriter(file, StandardCharsets.UTF_8)) {
                Gsons.DEFAULT_SLIM.toJson(state, writer);
            }
        } catch (IOException e) {
            LogManager.debug("Couldn't save the class cache state: " + e.getMessage());
        }
    }

    // ---- at launch and after -----------------------------------------------------------------------------------------

    /**
     * Adds the class cache options to a game's command line.
     *
     * @param arguments      the whole command line; changed in place
     * @param javaIndex      where the java executable is in it
     * @param mainClassIndex where the main class is
     * @return how many arguments were added (before the main class)
     */
    public static int apply(List<String> arguments, int javaIndex, int mainClassIndex, boolean enabled, int javaMajor,
        String javaHome, List<String> userArguments, String instanceSafeName) {
        if (!enabled || javaMajor < MINIMUM_JAVA) {
            return 0;
        }
        try {
            List<String> archiveOptions = archiveOptions(arguments.subList(javaIndex + 1, mainClassIndex));
            String javaExecutable = arguments.get(javaIndex);
            // Java checks each jar's size and time too, so a re-downloaded library makes the archive stale
            String fingerprint = fingerprint(javaExecutable, javaVersion(javaHome) + classPathStamp(archiveOptions),
                archiveOptions);

            String base = baseName(instanceSafeName, javaMajor);
            Path archive = folder().resolve(base + ".jsa");
            Path classList = folder().resolve(base + ".classlist");
            Path stateFile = folder().resolve(base + ".json");
            State state = readState(stateFile);

            long archiveBytes = Files.isRegularFile(archive) ? Files.size(archive) : -1;
            Action action = decide(enabled, javaMajor, userArguments, state, fingerprint, archiveBytes);
            if (action == Action.NONE) {
                return 0;
            }

            Files.createDirectories(folder());
            if (action == Action.RECORD) {
                // anything there belongs to an older Java or other libraries
                quietlyDelete(archive);
                quietlyDelete(classList);
                State recording = new State();
                recording.fingerprint = fingerprint;
                recording.status = "recording";
                recording.java = javaExecutable;
                recording.javaMajor = javaMajor;
                recording.options = archiveOptions;
                writeState(stateFile, recording);
                PENDING.put(instanceSafeName, recording);
                LogManager.info("Class cache: this start writes down the classes it loads; the cache is made after "
                    + "the game closes, for quicker starts next time");
            } else {
                LogManager.info("Class cache: using " + archive.getFileName());
            }

            List<String> flags = flags(action, archive, classList);
            arguments.addAll(javaIndex + 1, flags);
            return flags.size();
        } catch (Exception e) {
            LogManager.debug("Class cache skipped: " + e.getMessage());
            return 0;
        }
    }

    /** The size and time of every jar on the class path. */
    static String classPathStamp(List<String> archiveOptions) {
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i + 1 < archiveOptions.size(); i++) {
            String option = archiveOptions.get(i);
            if (option.equals("-cp") || option.equals("-classpath") || option.equals("--class-path")) {
                for (String entry : archiveOptions.get(i + 1).split(java.io.File.pathSeparator)) {
                    try {
                        Path path = Paths.get(entry);
                        if (Files.isRegularFile(path)) {
                            sb.append(Files.size(path)).append('@')
                                .append(Files.getLastModifiedTime(path).toMillis()).append(';');
                        }
                    } catch (IOException | RuntimeException e) {
                        sb.append("?;");
                    }
                }
            }
        }
        return sb.toString();
    }

    /** The Java's exact version from its release file, so an update makes the archive stale. */
    static String javaVersion(String javaHome) {
        if (javaHome == null) {
            return "";
        }
        Path release = Paths.get(javaHome).resolve("release");
        try {
            if (Files.isRegularFile(release)) {
                for (String line : Files.readAllLines(release, StandardCharsets.UTF_8)) {
                    if (line.startsWith("JAVA_RUNTIME_VERSION=") || line.startsWith("JAVA_VERSION=")) {
                        return line.trim() + Files.getLastModifiedTime(release).toMillis();
                    }
                }
            }
        } catch (IOException | RuntimeException e) {
            // unknown
        }
        return "";
    }

    /**
     * The game has closed: if its classes were written down, build its archive in the background, at low priority.
     */
    public static void gameExited(String instanceSafeName) {
        State pending = PENDING.remove(instanceSafeName);
        if (pending == null) {
            return;
        }
        Thread t = new Thread(() -> build(instanceSafeName, pending), "class-cache-builder");
        t.setDaemon(true);
        t.setPriority(Thread.MIN_PRIORITY);
        t.start();
    }

    private static void build(String instanceSafeName, State state) {
        String base = baseName(instanceSafeName, state.javaMajor);
        Path classList = folder().resolve(base + ".classlist");
        Path usableList = folder().resolve(base + ".classlist.txt");
        Path archive = folder().resolve(base + ".jsa");
        Path building = folder().resolve(base + ".jsa.part");
        Path stateFile = folder().resolve(base + ".json");
        Path argumentFile = folder().resolve(base + ".args");
        Path output = folder().resolve(base + ".log");

        try {
            if (!Files.isRegularFile(classList)) {
                return;
            }
            List<String> lines = Files.readAllLines(classList, StandardCharsets.UTF_8);
            if (countClasses(lines) < MINIMUM_CLASSES) {
                // the game didn't get far; try again on the next start
                return;
            }
            Files.write(usableList, usableClassList(lines), StandardCharsets.UTF_8);

            List<String> options = new ArrayList<>();
            options.add("-Xshare:dump");
            options.add("-XX:SharedClassListFile=" + usableList.toAbsolutePath());
            options.add("-XX:SharedArchiveFile=" + building.toAbsolutePath());
            options.addAll(state.options);
            // the class path can be longer than Windows allows on a command line, so it all goes in a file
            Files.write(argumentFile, JvmArguments.toArgumentFile(options).getBytes(StandardCharsets.UTF_8));

            List<String> command = new ArrayList<>();
            if (!OS.isWindows() && Utils.executableInPath("nice")) {
                command.add("nice");
                command.add("-n");
                command.add("15");
            }
            command.add(state.java);
            command.add("@" + argumentFile.toAbsolutePath());

            long started = System.nanoTime();
            // Java lists every class it skips; that output isn't needed, but must go somewhere
            Process process = new ProcessBuilder(command).redirectErrorStream(true)
                .redirectOutput(output.toFile()).start();
            boolean finished = process.waitFor(DUMP_TIMEOUT_MINUTES, TimeUnit.MINUTES);
            if (!finished) {
                process.destroyForcibly();
            }
            boolean ok = finished && process.exitValue() == 0 && Files.isRegularFile(building)
                && Files.size(building) > 0;

            if (ok) {
                quietlyDelete(archive);
                Files.move(building, archive, StandardCopyOption.REPLACE_EXISTING);
                state.status = "ready";
                state.archiveBytes = Files.size(archive);
                LogManager.info(String.format(Locale.ENGLISH, "Class cache for %s made in %d s (%d MB)",
                    instanceSafeName, (System.nanoTime() - started) / 1_000_000_000L,
                    Files.size(archive) / (1024 * 1024)));
            } else {
                state.status = "failed";
                LogManager.info("Class cache for " + instanceSafeName + " couldn't be made; the game starts as usual");
            }
            writeState(stateFile, state);
        } catch (Exception e) {
            LogManager.debug("Class cache build failed: " + e.getMessage());
        } finally {
            quietlyDelete(classList);
            quietlyDelete(usableList);
            quietlyDelete(building);
            quietlyDelete(argumentFile);
            quietlyDelete(output);
        }
    }

    private static void quietlyDelete(Path file) {
        try {
            delete(file);
        } catch (IOException e) {
            // next time
        }
    }

    /** Java makes its archives read-only, which Windows won't delete as they are */
    private static void delete(Path file) throws IOException {
        if (Files.exists(file) && !Files.isWritable(file)) {
            file.toFile().setWritable(true);
        }
        Files.deleteIfExists(file);
    }

    // ---- clearing ----------------------------------------------------------------------------------------------------

    /** Deletes the archives of one instance (all Java versions). */
    public static void deleteFor(String instanceSafeName) {
        if (instanceSafeName == null) {
            return;
        }
        deleteFor(folder(), instanceSafeName);
    }

    static long deleteFor(Path folder, String instanceSafeName) {
        String prefix = baseName(instanceSafeName, 0);
        prefix = prefix.substring(0, prefix.length() - 1);
        return deleteMatching(folder, prefix);
    }

    /** Deletes every archive; they are made again after the next start of each game. Returns the bytes freed. */
    public static long clear() {
        return deleteMatching(folder(), null);
    }

    /** How much room the archives take, in bytes. */
    public static long size() {
        long total = 0;
        Path folder = folder();
        if (!Files.isDirectory(folder)) {
            return 0;
        }
        try (DirectoryStream<Path> files = Files.newDirectoryStream(folder)) {
            for (Path file : files) {
                total += Files.size(file);
            }
        } catch (IOException e) {
            // best effort
        }
        return total;
    }

    static long deleteMatching(Path folder, String prefix) {
        long freed = 0;
        if (!Files.isDirectory(folder)) {
            return 0;
        }
        try (DirectoryStream<Path> files = Files.newDirectoryStream(folder)) {
            for (Path file : files) {
                String name = file.getFileName().toString();
                if (!isOurs(name)) {
                    continue;
                }
                if (prefix == null || (name.startsWith(prefix) && isVersionSuffix(name, prefix))) {
                    try {
                        long size = Files.size(file);
                        delete(file);
                        freed += size;
                    } catch (IOException e) {
                        // in use by a running game; it goes next time
                    }
                }
            }
        } catch (IOException e) {
            // nothing to clear
        }
        return freed;
    }

    private static boolean isOurs(String name) {
        String n = name.toLowerCase(Locale.ROOT);
        return n.endsWith(".jsa") || n.endsWith(".jsa.part") || n.endsWith(".classlist")
            || n.endsWith(".classlist.txt") || n.endsWith(".json") || n.endsWith(".args") || n.endsWith(".log");
    }

    /** "Name-21.jsa" belongs to "Name-", but "Name-2-21.jsa" (another instance) doesn't */
    private static boolean isVersionSuffix(String name, String prefix) {
        String rest = name.substring(prefix.length());
        int dot = rest.indexOf('.');
        return dot > 0 && rest.substring(0, dot).matches("\\d+");
    }

    /** For tests: the archives waiting for their game to close. */
    static Map<String, State> pending() {
        return Collections.unmodifiableMap(PENDING);
    }
}
