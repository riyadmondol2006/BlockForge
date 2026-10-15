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

import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Reads a game log or crash report and says what probably broke. Each check looks for a specific signature, so a
 * diagnosis says "likely", never "certainly", and the first one listed is the most specific.
 */
public final class CrashClassifier {
    private static final Pattern CLASS_VERSION = Pattern
        .compile("(?:class file version|major version|version) (\\d{2})\\.\\d", Pattern.CASE_INSENSITIVE);
    private static final Pattern MOD_NAMES = Pattern.compile("(?i)(?:requires|depends on|missing)[^\\n]{0,80}?(?:mod )?['\"`]?([a-z0-9_.-]{3,40})['\"`]?");
    private static final Pattern DUPLICATE_MODS = Pattern.compile("(?i)duplicate mods?|DuplicateModsFoundException|found duplicate");

    private CrashClassifier() {
    }

    public static List<Diagnosis> classify(String log) {
        List<Diagnosis> found = new ArrayList<>();
        if (log == null || log.isEmpty()) {
            return found;
        }

        wrongJava(log, found);
        badJvmArgument(log, found);
        outOfMemory(log, found);
        missingDependency(log, found);
        duplicateMods(log, found);
        mixin(log, found);
        corruptFile(log, found);
        diskFull(log, found);
        graphics(log, found);
        natives(log, found);
        permissions(log, found);

        return found;
    }

    private static String evidence(String log, Pattern pattern) {
        Matcher m = pattern.matcher(log);
        if (!m.find()) {
            return "";
        }
        int lineStart = log.lastIndexOf('\n', m.start()) + 1;
        int lineEnd = log.indexOf('\n', m.end());
        String line = log.substring(lineStart, lineEnd < 0 ? log.length() : lineEnd).trim();
        return line.length() > 300 ? line.substring(0, 300) + "…" : line;
    }

    private static boolean has(String log, String regex) {
        return Pattern.compile(regex, Pattern.CASE_INSENSITIVE).matcher(log).find();
    }

    private static void wrongJava(String log, List<Diagnosis> out) {
        Pattern p = Pattern.compile(
            "UnsupportedClassVersionError|compiled by a more recent version of the Java Runtime|requires Java \\d+",
            Pattern.CASE_INSENSITIVE);
        if (!p.matcher(log).find()) {
            return;
        }

        String line = evidence(log, p);
        Matcher v = CLASS_VERSION.matcher(line.isEmpty() ? log : line);
        String needed = "a newer version of Java";
        if (v.find()) {
            int classFile = Integer.parseInt(v.group(1));
            if (classFile >= 52) {
                needed = "Java " + (classFile - 44) + " or newer";
            }
        }

        out.add(new Diagnosis("MC-JAVA-VERSION", "Minecraft is using the wrong version of Java",
            "Something in this instance was built for " + needed + ", but an older Java is running it.",
            "In the instance settings turn on \"Use Java provided by Minecraft\", or choose a Java that is "
                + needed + ".",
            line));
    }

    private static void badJvmArgument(String log, List<Diagnosis> out) {
        Pattern p = Pattern.compile(
            "Unrecognized VM option|Unrecognized option|Could not create the Java Virtual Machine|Invalid maximum heap size|Improperly specified VM option",
            Pattern.CASE_INSENSITIVE);
        if (!p.matcher(log).find()) {
            return;
        }

        out.add(new Diagnosis("MC-JVM-ARGUMENT", "A Java argument isn't accepted",
            "Java refused to start because one of the extra arguments isn't valid for the Java that's being used.",
            "Open Settings and clear the Java arguments, or remove the option named in the log. Options for old Java "
                + "versions often don't work on new ones.",
            evidence(log, p)));
    }

    private static void outOfMemory(String log, List<Diagnosis> out) {
        Pattern p = Pattern.compile(
            "java\\.lang\\.OutOfMemoryError|insufficient memory for the Java Runtime|Could not reserve enough space for (?:object )?heap",
            Pattern.CASE_INSENSITIVE);
        if (!p.matcher(log).find()) {
            return;
        }

        boolean reserve = has(log, "Could not reserve enough space|insufficient memory for the Java Runtime");
        out.add(new Diagnosis("MC-OUT-OF-MEMORY", "Minecraft ran out of memory",
            reserve ? "Java couldn't get as much memory as it was asked for, so the game never started."
                : "The game used all the memory it was allowed to.",
            reserve ? "Lower the memory in Settings, or close other programs. A 32-bit Java can't use more than about 1.5 GB."
                : "Raise the memory in Settings. Large modpacks usually need 6 GB or more.",
            evidence(log, p)));
    }

    private static void missingDependency(String log, List<Diagnosis> out) {
        Pattern p = Pattern.compile(
            "Missing or unsupported mandatory dependencies|Unmet dependency listing|Could not find required mod|requires (?:any )?(?:version|mod)|Mod .* requires|MissingModsException|incompatible mod set",
            Pattern.CASE_INSENSITIVE);
        if (!p.matcher(log).find()) {
            return;
        }

        Set<String> names = new LinkedHashSet<>();
        Matcher m = MOD_NAMES.matcher(log);
        while (m.find() && names.size() < 5) {
            String name = m.group(1);
            if (!name.equalsIgnoreCase("version") && !name.equalsIgnoreCase("any") && !name.equalsIgnoreCase("the")) {
                names.add(name);
            }
        }

        out.add(new Diagnosis("MC-MISSING-DEPENDENCY", "A mod is missing something it needs",
            "A mod needs another mod, or a different version of one, that isn't installed."
                + (names.isEmpty() ? "" : " Mentioned in the log: " + String.join(", ", names) + "."),
            "Install the mod named in the log (and the right loader and Minecraft version for it), or remove the mod "
                + "that asked for it.",
            evidence(log, p)));
    }

    private static void duplicateMods(String log, List<Diagnosis> out) {
        if (!DUPLICATE_MODS.matcher(log).find()) {
            return;
        }

        out.add(new Diagnosis("MC-DUPLICATE-MOD", "The same mod is installed twice",
            "Two files in the mods folder are the same mod, usually two versions of it.",
            "Open the instance's mods folder and delete the older copy.", evidence(log, DUPLICATE_MODS)));
    }

    private static void mixin(String log, List<Diagnosis> out) {
        Pattern p = Pattern.compile("MixinApplyError|InvalidMixinException|Mixin apply failed|Mixin transformation of|MixinTransformerError",
            Pattern.CASE_INSENSITIVE);
        if (!p.matcher(log).find()) {
            return;
        }

        out.add(new Diagnosis("MC-MIXIN-CONFLICT", "Two mods are fighting over the same part of the game",
            "A mod's changes to Minecraft couldn't be applied, which usually means it clashes with another mod or was "
                + "made for a different version.",
            "Look for the mod named next to \"Mixin\" in the log. Update it, or remove it and any similar mod, then try again.",
            evidence(log, p)));
    }

    private static void corruptFile(String log, List<Diagnosis> out) {
        Pattern p = Pattern.compile("Invalid or corrupt jarfile|zip END header not found|ZipException|error in opening zip file|Unexpected end of ZLIB",
            Pattern.CASE_INSENSITIVE);
        if (!p.matcher(log).find()) {
            return;
        }

        out.add(new Diagnosis("MC-CORRUPT-FILE", "A downloaded file is damaged",
            "A jar file couldn't be opened, which normally means it didn't finish downloading.",
            "Delete the mod named in the log and add it again. If it is a library, use Tools > Clear downloads.",
            evidence(log, p)));
    }

    private static void diskFull(String log, List<Diagnosis> out) {
        Pattern p = Pattern.compile("No space left on device|not enough space on the disk|There is not enough space",
            Pattern.CASE_INSENSITIVE);
        if (!p.matcher(log).find()) {
            return;
        }

        out.add(new Diagnosis("SYS-DISK-FULL", "The disk is full", "There's no room left to write files.",
            "Free up space on the drive BlockForge uses, then try again.", evidence(log, p)));
    }

    private static void graphics(String log, List<Diagnosis> out) {
        Pattern p = Pattern.compile(
            "GLFW error|Pixel format not accepted|does not appear to support OpenGL|GL_INVALID|Failed to create window|OpenGL 3\\.\\d? is required",
            Pattern.CASE_INSENSITIVE);
        if (!p.matcher(log).find()) {
            return;
        }

        out.add(new Diagnosis("MC-GRAPHICS", "Minecraft couldn't start its graphics",
            "The graphics driver didn't give Minecraft a window it can draw in.",
            "Update your graphics driver. On a laptop, make sure Java runs on the dedicated GPU. Turn off overlays and screen recorders.",
            evidence(log, p)));
    }

    private static void natives(String log, List<Diagnosis> out) {
        Pattern p = Pattern.compile("UnsatisfiedLinkError|Failed to locate library|Could not load native|lwjgl.*(?:not found|failed)",
            Pattern.CASE_INSENSITIVE);
        if (!p.matcher(log).find() || has(log, "GLFW error")) {
            return;
        }

        out.add(new Diagnosis("MC-NATIVES", "A native library failed to load",
            "Minecraft couldn't load one of its system libraries (for example LWJGL or OpenAL).",
            "Use Tools > Delete libraries and start again so they're downloaded fresh. On ARM or Apple Silicon, make sure "
                + "the instance runs an ARM Java.",
            evidence(log, p)));
    }

    private static void permissions(String log, List<Diagnosis> out) {
        Pattern p = Pattern.compile("AccessDeniedException|Permission denied|Access is denied", Pattern.CASE_INSENSITIVE);
        if (!p.matcher(log).find()) {
            return;
        }

        out.add(new Diagnosis("SYS-PERMISSION", "BlockForge isn't allowed to use a file",
            "A file or folder couldn't be opened because of its permissions, or because another program has it open.",
            "Close other programs using the instance folder, and make sure it is inside a folder you own.",
            evidence(log, p)));
    }
}
