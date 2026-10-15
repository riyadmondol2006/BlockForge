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
package com.blockforge.launcher.downloads;

import java.io.IOException;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.Optional;
import java.util.function.Predicate;
import java.util.regex.Pattern;

/**
 * Recognises a file that the user downloaded by hand (for example a CurseForge mod whose author doesn't allow
 * launchers to download it) even if the browser renamed it or it was saved under a slightly different name.
 */
public final class RestrictedFileMatcher {
    private static final String[] PARTIAL = { ".crdownload", ".part", ".tmp", ".download", ".opdownload", ".partial" };

    private RestrictedFileMatcher() {
    }

    /** The files in a folder that are finished downloads. */
    public static List<Path> finishedFiles(Path folder) {
        List<Path> files = new ArrayList<>();
        if (folder == null || !Files.isDirectory(folder)) {
            return files;
        }

        try (DirectoryStream<Path> stream = Files.newDirectoryStream(folder)) {
            for (Path p : stream) {
                if (Files.isRegularFile(p) && !isPartial(p)) {
                    files.add(p);
                }
            }
        } catch (IOException ignored) {
            // an unreadable downloads folder simply has nothing to find
        }
        return files;
    }

    static boolean isPartial(Path file) {
        String name = file.getFileName().toString().toLowerCase(Locale.ROOT);
        for (String suffix : PARTIAL) {
            if (name.endsWith(suffix)) {
                return true;
            }
        }
        return false;
    }

    /**
     * Finds the downloaded copy of a file.
     *
     * @param files the finished files in the downloads folder
     * @param fileName the name the file should have
     * @param size how big it should be, or 0 if unknown
     * @param verified checks a candidate's contents against the expected hash; a file only counts if this says yes
     */
    public static Optional<Path> find(List<Path> files, String fileName, long size, Predicate<Path> verified) {
        // a file with the right name, or the name a browser would have made from it, is the most likely
        for (Path file : files) {
            if (sameName(file.getFileName().toString(), fileName) && sizeMatches(file, size) && verified.test(file)) {
                return Optional.of(file);
            }
        }

        // otherwise any file of exactly the right size whose contents check out, whatever it was called
        if (size > 0) {
            for (Path file : files) {
                if (sizeMatches(file, size) && extension(file.getFileName().toString()).equals(extension(fileName))
                    && verified.test(file)) {
                    return Optional.of(file);
                }
            }
        }
        return Optional.empty();
    }

    private static boolean sizeMatches(Path file, long size) {
        if (size <= 0) {
            return true;
        }
        try {
            return Files.size(file) == size;
        } catch (IOException e) {
            return false;
        }
    }

    private static String extension(String name) {
        int dot = name.lastIndexOf('.');
        return dot < 0 ? "" : name.substring(dot).toLowerCase(Locale.ROOT);
    }

    /** equal ignoring case, with spaces turned into plus signs, or with a browser's " (1)" added */
    static boolean sameName(String actual, String expected) {
        String a = actual.toLowerCase(Locale.ROOT);
        String e = expected.toLowerCase(Locale.ROOT);
        if (a.equals(e) || a.equals(e.replace(' ', '+')) || a.equals(e.replace(' ', '_'))) {
            return true;
        }

        String ext = extension(e);
        String stem = e.substring(0, e.length() - ext.length());
        return Pattern.matches(Pattern.quote(stem) + "(?: \\(\\d+\\)|-\\d+|_\\d+)" + Pattern.quote(ext), a);
    }
}
