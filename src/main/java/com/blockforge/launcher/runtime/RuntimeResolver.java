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
package com.blockforge.launcher.runtime;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Optional;

/**
 * Picks the Java that Minecraft will run on and keeps the reasons, so the launcher can say what it chose and why
 * instead of leaving people to guess.
 */
public final class RuntimeResolver {
    private RuntimeResolver() {
    }

    /** What the choice is based on. */
    public static final class Request {
        /** a Java the user chose, in the instance or in settings; null if they left it automatic */
        public JavaRuntime userChoice;
        /** the Java that Mojang provides for this Minecraft version, if it's installed and allowed */
        public JavaRuntime minecraftRuntime;
        /** Javas found on the computer */
        public final List<JavaRuntime> installed = new ArrayList<>();
        /** the Java that launches BlockForge itself, as a last resort */
        public JavaRuntime launcherJava;
        /** the version Minecraft needs at least, or -1 if unknown */
        public int requiredMajor = -1;
        /** the processor this computer has: x64, arm64 or x86 */
        public String hostArch;
    }

    /** The decision and the thinking behind it. */
    public static final class Choice {
        public JavaRuntime java;
        public final List<String> reasons = new ArrayList<>();
        public final List<String> warnings = new ArrayList<>();

        /** one line per fact, for the log */
        public String explain() {
            StringBuilder sb = new StringBuilder();
            sb.append(java == null ? "No Java found" : "Using " + java.label());
            reasons.forEach(r -> sb.append("\n  why: ").append(r));
            warnings.forEach(w -> sb.append("\n  warning: ").append(w));
            return sb.toString();
        }
    }

    public static Choice resolve(Request request) {
        Choice choice = new Choice();

        if (request.userChoice != null) {
            choice.java = request.userChoice;
            choice.reasons.add("you picked this Java in the settings");
        } else if (request.minecraftRuntime != null) {
            choice.java = request.minecraftRuntime;
            choice.reasons.add("it's the Java Minecraft provides for this version, so it is the one it was tested with");
        } else {
            Optional<JavaRuntime> best = best(request);
            if (best.isPresent()) {
                choice.java = best.get();
                choice.reasons.add(request.requiredMajor > 0
                    ? "it is the closest installed Java that meets Minecraft's requirement of Java " + request.requiredMajor
                    : "it is the newest Java installed here");
                if (request.hostArch != null && request.hostArch.equals(best.get().arch)) {
                    choice.reasons.add("it matches this computer's processor (" + request.hostArch + ")");
                }
            } else if (request.launcherJava != null) {
                choice.java = request.launcherJava;
                choice.reasons.add("no other Java was found, so BlockForge's own Java is being used");
            }
        }

        check(choice, request);
        return choice;
    }

    /** the best installed Java: meets the requirement, matches the processor, and is as close to the requirement as possible */
    private static Optional<JavaRuntime> best(Request request) {
        List<JavaRuntime> candidates = new ArrayList<>(request.installed);
        candidates.removeIf(j -> j.major > 0 && request.requiredMajor > 0 && j.major < request.requiredMajor);

        return candidates.stream().min(Comparator
            .comparing((JavaRuntime j) -> request.hostArch != null && !request.hostArch.equals(j.arch))
            .thenComparingInt(j -> request.requiredMajor > 0 && j.major > 0 ? j.major - request.requiredMajor : 0)
            .thenComparing(j -> j.major <= 0));
    }

    private static void check(Choice choice, Request request) {
        JavaRuntime java = choice.java;
        if (java == null) {
            choice.warnings.add("no Java could be found, install one or turn on \"Use Java provided by Minecraft\"");
            return;
        }

        if (request.requiredMajor > 0 && java.major > 0 && java.major < request.requiredMajor) {
            choice.warnings.add("this Minecraft needs Java " + request.requiredMajor + " or newer but this is Java "
                + java.major + ", so it will probably fail to start");
        }

        if (request.hostArch != null && java.arch != null && !request.hostArch.equals(java.arch)) {
            if (request.hostArch.equals(JavaRuntime.ARM64) && java.arch.equals(JavaRuntime.X64)) {
                choice.warnings.add("this Java is built for Intel/AMD processors and runs through emulation on this "
                    + "computer, which makes Minecraft noticeably slower. An ARM Java is recommended.");
            } else if (java.arch.equals(JavaRuntime.X86) && !request.hostArch.equals(JavaRuntime.X86)) {
                choice.warnings.add("this is a 32-bit Java, which limits Minecraft to about 1.5 GB of memory");
            } else {
                choice.warnings.add("this Java is built for " + java.arch + " but this computer is " + request.hostArch);
            }
        }
    }
}
