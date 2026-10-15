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
package com.blockforge.launcher.diagnostics;

import java.io.IOException;
import java.io.OutputStream;
import java.io.RandomAccessFile;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Comparator;
import java.util.List;
import java.util.stream.Collectors;
import java.util.stream.Stream;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

/**
 * Builds one ZIP with everything useful for finding out why something failed, with all personal details removed. The
 * person sending it can open the ZIP first and see exactly what is in it.
 */
public final class SupportBundle {
    /** only the end of a log is useful, and it keeps the bundle small */
    private static final int MAX_LOG_BYTES = 768 * 1024;

    /** What goes into a bundle. Everything is optional. */
    public static final class Input {
        /** the launcher's own log */
        public Path launcherLog;
        /** the instance's folder, for its game log, crash reports and mod list */
        public Path instanceRoot;
        public String instanceName;
        /** lines of facts about the launcher and system, such as versions and the chosen Java */
        public final List<String> facts = new ArrayList<>();
        /** values that must never appear in the bundle, like the player's name */
        public final List<String> secrets = new ArrayList<>();
    }

    private SupportBundle() {
    }

    /** Writes the bundle and returns the diagnoses found in it. */
    public static List<Diagnosis> create(Path zip, Input input) throws IOException {
        String launcherLog = input.launcherLog == null ? "" : tail(input.launcherLog);
        String gameLog = "";
        String crashReport = "";
        Path crashFile = null;

        if (input.instanceRoot != null) {
            gameLog = tail(input.instanceRoot.resolve("logs/latest.log"));
            crashFile = newestCrashReport(input.instanceRoot);
            crashReport = crashFile == null ? "" : tail(crashFile);
        }

        List<Diagnosis> diagnoses = CrashClassifier.classify(crashReport + "\n" + gameLog);

        if (zip.getParent() != null) {
            Files.createDirectories(zip.getParent());
        }

        try (OutputStream out = Files.newOutputStream(zip); ZipOutputStream z = new ZipOutputStream(out)) {
            add(z, "summary.txt", summary(input, diagnoses, crashFile), input.secrets);
            add(z, "launcher.log", launcherLog, input.secrets);

            if (input.instanceRoot != null) {
                add(z, "latest.log", gameLog, input.secrets);
                if (crashFile != null) {
                    add(z, "crash-report.txt", crashReport, input.secrets);
                }
                add(z, "instance.json", tail(input.instanceRoot.resolve("instance.json")), input.secrets);
                add(z, "mods.txt", modList(input.instanceRoot), input.secrets);
            }
        }

        return diagnoses;
    }

    private static String summary(Input input, List<Diagnosis> diagnoses, Path crashFile) {
        StringBuilder sb = new StringBuilder();
        sb.append("BlockForge support bundle\n");
        sb.append("Personal details (player name, account IDs, tokens, e-mail, IP addresses, your user name) have been ")
            .append("removed. Open the files to check.\n\n");

        if (input.instanceName != null) {
            sb.append("Instance: ").append(input.instanceName).append('\n');
        }
        input.facts.forEach(f -> sb.append(f).append('\n'));
        if (crashFile != null) {
            sb.append("Crash report: ").append(crashFile.getFileName()).append('\n');
        }

        sb.append("\nLikely causes\n");
        if (diagnoses.isEmpty()) {
            sb.append("  Nothing recognised in the logs.\n");
        }
        for (Diagnosis d : diagnoses) {
            sb.append("  [").append(d.code).append("] ").append(d.title).append('\n');
            sb.append("    ").append(d.explanation).append('\n');
            sb.append("    Try: ").append(d.fix).append('\n');
            if (!d.evidence.isEmpty()) {
                sb.append("    Log: ").append(d.evidence).append('\n');
            }
        }
        return sb.toString();
    }

    private static void add(ZipOutputStream z, String name, String content, Collection<String> secrets)
        throws IOException {
        z.putNextEntry(new ZipEntry(name));
        z.write(Redactor.scrub(content, secrets).getBytes(StandardCharsets.UTF_8));
        z.closeEntry();
    }

    private static String modList(Path instanceRoot) {
        Path mods = instanceRoot.resolve("mods");
        if (!Files.isDirectory(mods)) {
            return "No mods folder\n";
        }

        try (Stream<Path> files = Files.list(mods)) {
            return files.filter(Files::isRegularFile).sorted().map(p -> {
                try {
                    return p.getFileName() + "  (" + Files.size(p) + " bytes)";
                } catch (IOException e) {
                    return p.getFileName().toString();
                }
            }).collect(Collectors.joining("\n", "", "\n"));
        } catch (IOException e) {
            return "Couldn't read the mods folder\n";
        }
    }

    private static Path newestCrashReport(Path instanceRoot) {
        Path reports = instanceRoot.resolve("crash-reports");
        if (!Files.isDirectory(reports)) {
            return null;
        }

        try (Stream<Path> files = Files.list(reports)) {
            return files.filter(Files::isRegularFile).max(Comparator.comparingLong(p -> {
                try {
                    return Files.getLastModifiedTime(p).toMillis();
                } catch (IOException e) {
                    return 0L;
                }
            })).orElse(null);
        } catch (IOException e) {
            return null;
        }
    }

    /** The last part of a text file, or an empty string if it can't be read. */
    static String tail(Path file) {
        if (file == null || !Files.isRegularFile(file)) {
            return "";
        }

        try (RandomAccessFile raf = new RandomAccessFile(file.toFile(), "r")) {
            long length = raf.length();
            long start = Math.max(0, length - MAX_LOG_BYTES);
            raf.seek(start);
            byte[] bytes = new byte[(int) (length - start)];
            raf.readFully(bytes);
            String text = new String(bytes, StandardCharsets.UTF_8);
            if (start > 0) {
                int firstLine = text.indexOf('\n');
                text = "[earlier lines left out]\n" + (firstLine >= 0 ? text.substring(firstLine + 1) : text);
            }
            return text;
        } catch (IOException e) {
            return "";
        }
    }
}
