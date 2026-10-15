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

import java.util.ArrayList;
import java.util.List;

import com.blockforge.launcher.constants.Constants;

/**
 * The parts of building the game's Java command line that are pure decisions: splitting what the user typed into
 * arguments, which tuning flags to give the game's Java, and when the command line is too long for Windows.
 */
public final class JvmArguments {
    /**
     * Windows refuses to start a program whose command line is longer than 32767 characters. Big modpacks have
     * hundreds of libraries on the class path and can go over it, so stay well clear.
     */
    public static final int WINDOWS_COMMAND_LINE_BUDGET = 30000;

    private JvmArguments() {
    }

    /**
     * Splits Java arguments as typed by a person into separate arguments. Spaces separate arguments, except inside
     * double quotes, so {@code -Dname="two words"} stays one argument (without the quotes). Splitting on every space
     * used to break such arguments apart.
     */
    public static List<String> split(String arguments) {
        List<String> result = new ArrayList<>();
        if (arguments == null) {
            return result;
        }

        StringBuilder current = new StringBuilder();
        boolean inToken = false;
        char quote = 0;

        for (int i = 0; i < arguments.length(); i++) {
            char c = arguments.charAt(i);

            if (quote != 0) {
                if (c == quote) {
                    quote = 0;
                } else {
                    current.append(c);
                }
            } else if (c == '"') {
                // only double quotes group, since apostrophes turn up in names and paths (C:\Users\O'Brien)
                quote = c;
                inToken = true;
            } else if (Character.isWhitespace(c)) {
                if (inToken) {
                    result.add(current.toString());
                    current.setLength(0);
                    inToken = false;
                }
            } else {
                current.append(c);
                inToken = true;
            }
        }

        if (inToken) {
            result.add(current.toString());
        }

        return result;
    }

    /**
     * If these are the person's own Java arguments, as opposed to nothing or the ones BlockForge ships with. Their
     * own arguments are always used exactly as they are.
     */
    public static boolean isCustom(String arguments) {
        if (arguments == null || arguments.trim().isEmpty()) {
            return false;
        }

        return !split(arguments).equals(split(Constants.DEFAULT_JAVA_PARAMETERS));
    }

    /**
     * The garbage collector settings to start the game with when the person hasn't set their own Java arguments.
     * These are the flags Mojang's own launcher uses, which work on every Java from 8 to the newest on Windows, macOS
     * and Linux, with the region size fitted to the amount of memory instead of always 32 MB.
     *
     * Nothing here costs anything on a small computer: no fixed starting heap (-Xms), no thread counts (Java sizes
     * those to the processor) and smaller regions for small heaps.
     *
     * @param maximumHeapMb the -Xmx the game is given, in MB
     * @param processors    how many processors the computer has
     */
    public static List<String> performanceFlags(int maximumHeapMb, int processors) {
        List<String> flags = new ArrayList<>();

        // with a single processor, G1's background work competes with the game itself; Java then picks the Serial
        // collector on its own, which is the better choice there, so leave it alone
        if (processors <= 1) {
            return flags;
        }

        // needed for the two G1 percentages below, which are "experimental" in name only and have been in every
        // Java since 8
        flags.add("-XX:+UnlockExperimentalVMOptions");
        // G1 collects in small steps instead of stopping the game for one long pause. Java 8 would otherwise use the
        // Parallel collector, which causes the well known lag spikes, and newer Javas fall back to the Serial
        // collector on computers with few cores or little memory
        flags.add("-XX:+UseG1GC");
        // a bigger young generation suits the game, which makes huge numbers of short lived objects every tick
        flags.add("-XX:G1NewSizePercent=20");
        // keeps some memory free for sudden bursts (loading chunks) so G1 doesn't run out and do a full collection
        flags.add("-XX:G1ReservePercent=20");
        // a pause target of 50 ms keeps collections shorter than a game tick
        flags.add("-XX:MaxGCPauseMillis=50");

        // bigger regions keep chunk data arrays from being treated as "humongous", but G1 needs enough regions to
        // work with, so aim for at least 64 of them and never more than the 32 MB Java 8 allows
        int regionMb = g1RegionSizeMb(maximumHeapMb);
        if (regionMb > 0) {
            flags.add("-XX:G1HeapRegionSize=" + regionMb + "M");
        }

        return flags;
    }

    /** largest power of two no bigger than heap/64 and no bigger than 32, or 0 to let Java choose */
    static int g1RegionSizeMb(int maximumHeapMb) {
        if (maximumHeapMb <= 0) {
            return 0;
        }

        int target = maximumHeapMb / 64;
        if (target < 1) {
            return 0;
        }

        int size = 1;
        while (size * 2 <= target && size < 32) {
            size *= 2;
        }

        return size;
    }

    /** The name of a -XX option, -D property or similar, so the same setting isn't given twice with two values. */
    static String optionName(String argument) {
        if (argument.startsWith("-XX:+") || argument.startsWith("-XX:-")) {
            return "-XX:" + argument.substring(5);
        }

        int equals = argument.indexOf('=');
        return equals == -1 ? argument : argument.substring(0, equals);
    }

    /** Adds the flags whose setting isn't already in the arguments. */
    public static void addMissing(List<String> arguments, List<String> flags) {
        List<String> present = new ArrayList<>();
        for (String argument : arguments) {
            present.add(optionName(argument));
        }

        for (String flag : flags) {
            // only one collector may be chosen
            if (flag.startsWith("-XX:+Use") && flag.endsWith("GC")
                && arguments.stream().anyMatch(a -> a.startsWith("-XX:+Use") && a.endsWith("GC"))) {
                continue;
            }

            if (!present.contains(optionName(flag))) {
                arguments.add(flag);
                present.add(optionName(flag));
            }
        }
    }

    /** How long Windows will see the command line as: every argument, quoted when needed, with spaces between. */
    public static int windowsCommandLineLength(List<String> arguments) {
        int length = 0;
        for (String argument : arguments) {
            length += argument.length() + 1;
            if (argument.isEmpty() || argument.indexOf(' ') != -1 || argument.indexOf('\t') != -1) {
                length += 2;
            }
        }
        return length;
    }

    /**
     * If the Java options should go into an argument file ({@code java @file}) because the command line would be too
     * long for Windows. Argument files only exist since Java 9; on Java 8 there is nothing better to do than try.
     */
    public static boolean needsArgumentFile(List<String> arguments, boolean windows, int javaMajor) {
        return windows && javaMajor >= 9 && windowsCommandLineLength(arguments) > WINDOWS_COMMAND_LINE_BUDGET;
    }

    /**
     * Writes arguments in the format of a Java argument file: one per line, quoted, with backslashes and quotes
     * escaped as Java expects inside quotes.
     */
    public static String toArgumentFile(List<String> arguments) {
        StringBuilder sb = new StringBuilder();
        for (String argument : arguments) {
            sb.append('"').append(argument.replace("\\", "\\\\").replace("\"", "\\\"")).append('"').append('\n');
        }
        return sb.toString();
    }
}
