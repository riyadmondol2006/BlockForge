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

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Locale;
import java.util.Properties;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * A Java installation as far as choosing one is concerned: where it is, which version and what kind of processor it
 * is built for.
 */
public final class JavaRuntime {
    public static final String X64 = "x64";
    public static final String ARM64 = "arm64";
    public static final String X86 = "x86";

    public final Path home;
    /** 8, 11, 17, 21 ... or -1 if unknown */
    public final int major;
    /** x64, arm64, x86 or null if unknown */
    public final String arch;
    /** where it came from, shown to the user: "chosen in settings", "Minecraft's own Java" ... */
    public final String source;

    public JavaRuntime(Path home, int major, String arch, String source) {
        this.home = home;
        this.major = major;
        this.arch = arch;
        this.source = source;
    }

    /** Reads a Java home's "release" file, which every JDK and most JREs ship, without starting Java. */
    public static JavaRuntime fromHome(Path home, String source) {
        Path release = home.resolve("release");
        if (!Files.isRegularFile(release)) {
            return new JavaRuntime(home, -1, null, source);
        }

        Properties props = new Properties();
        try (InputStream in = Files.newInputStream(release)) {
            props.load(in);
        } catch (IOException e) {
            return new JavaRuntime(home, -1, null, source);
        }

        return new JavaRuntime(home, parseMajor(unquote(props.getProperty("JAVA_VERSION"))),
            normaliseArch(unquote(props.getProperty("OS_ARCH"))), source);
    }

    private static String unquote(String value) {
        return value == null ? null : value.replace("\"", "").trim();
    }

    /** "1.8.0_392" is 8, "17.0.9" is 17 */
    public static int parseMajor(String version) {
        if (version == null) {
            return -1;
        }
        Matcher m = Pattern.compile("^(?:1\\.)?(\\d+)").matcher(version);
        return m.find() ? Integer.parseInt(m.group(1)) : -1;
    }

    public static String normaliseArch(String arch) {
        if (arch == null) {
            return null;
        }
        String a = arch.toLowerCase(Locale.ROOT);
        if (a.equals("aarch64") || a.equals("arm64")) {
            return ARM64;
        }
        if (a.equals("x86_64") || a.equals("amd64") || a.equals("x64")) {
            return X64;
        }
        if (a.equals("x86") || a.equals("i386") || a.equals("i586") || a.equals("i686")) {
            return X86;
        }
        return a;
    }

    public String label() {
        return (major > 0 ? "Java " + major : "Java") + (arch != null ? ", " + arch : "") + " at " + home;
    }
}
