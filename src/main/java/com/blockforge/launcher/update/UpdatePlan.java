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
package com.blockforge.launcher.update;

import java.util.Arrays;
import java.util.List;
import java.util.Locale;

/**
 * Decides whether a GitHub release is newer than the running copy, and which of its files fits this computer. Has no
 * side effects so it can be tested without a network or a window.
 */
public final class UpdatePlan {
    /** How this copy of BlockForge was installed, which decides what file an update needs. */
    public enum Kind {
        /** a plain jar, updated by swapping the jar */
        JAR(".jar"),
        /** the Windows setup program */
        WINDOWS_EXE(".exe"),
        /** the macOS disk image */
        MAC_DMG(".dmg"),
        /** the Debian/Ubuntu package */
        DEB(".deb"),
        /** the Fedora/openSUSE package */
        RPM(".rpm"),
        /** can't update in place (running from source, Flatpak...), only point at the release page */
        PAGE_ONLY(null);

        public final String extension;

        Kind(String extension) {
            this.extension = extension;
        }
    }

    private static final List<String> ARM_TOKENS = Arrays.asList("arm64", "aarch64");
    private static final List<String> X64_TOKENS = Arrays.asList("x64", "x86_64", "amd64", "intel");

    private UpdatePlan() {
    }

    /**
     * Reads "v3.4.42.0", "3.4.42" or "v3.4.42.0-rc1" into four numbers (missing ones are 0). Returns null when the tag
     * isn't a version, like "nightly".
     */
    public static int[] parseVersion(String tag) {
        if (tag == null) {
            return null;
        }
        String clean = tag.trim();
        if (clean.startsWith("v") || clean.startsWith("V")) {
            clean = clean.substring(1);
        }
        int cut = indexOfAny(clean, '-', '+', ' ');
        if (cut >= 0) {
            clean = clean.substring(0, cut);
        }
        if (clean.toLowerCase(Locale.ROOT).endsWith(".beta")) {
            clean = clean.substring(0, clean.length() - ".beta".length());
        }

        String[] parts = clean.split("\\.");
        if (parts.length == 0 || parts.length > 4) {
            return null;
        }
        int[] version = new int[4];
        for (int i = 0; i < parts.length; i++) {
            if (!parts[i].matches("\\d{1,9}")) {
                return null;
            }
            version[i] = Integer.parseInt(parts[i]);
        }
        return version;
    }

    /** True when the release tag is a higher version than {@code current}. The same version is never an update. */
    public static boolean isNewer(String tag, int[] current) {
        int[] other = parseVersion(tag);
        if (other == null || current == null) {
            return false;
        }
        for (int i = 0; i < 4; i++) {
            int mine = i < current.length ? current[i] : 0;
            if (other[i] != mine) {
                return other[i] > mine;
            }
        }
        return false;
    }

    /**
     * Works out how this copy was installed.
     *
     * @param installed   whether it was started by an installer's launcher (-Dblockforge.installed=true)
     * @param osName      the os.name property
     * @param runningFrom the file BlockForge is running from (a jar, or a classes folder when run from source)
     * @param flatpak     whether it runs inside Flatpak
     * @param osRelease   the contents of /etc/os-release on Linux, or null
     */
    public static Kind detectKind(boolean installed, String osName, String runningFrom, boolean flatpak,
        String osRelease) {
        if (flatpak) {
            return Kind.PAGE_ONLY;
        }
        if (!installed) {
            return runningFrom != null && runningFrom.toLowerCase(Locale.ROOT).endsWith(".jar") ? Kind.JAR
                : Kind.PAGE_ONLY;
        }

        String os = osName == null ? "" : osName.toLowerCase(Locale.ROOT);
        if (os.startsWith("windows")) {
            return Kind.WINDOWS_EXE;
        }
        if (os.startsWith("mac")) {
            return Kind.MAC_DMG;
        }
        if (osRelease != null) {
            String ids = linuxFamily(osRelease);
            if (containsAny(ids, "debian", "ubuntu")) {
                return Kind.DEB;
            }
            if (containsAny(ids, "fedora", "rhel", "centos", "suse", "opensuse", "mageia")) {
                return Kind.RPM;
            }
        }
        return Kind.PAGE_ONLY;
    }

    /** The ID and ID_LIKE values from /etc/os-release, lower case and space separated. */
    static String linuxFamily(String osRelease) {
        StringBuilder ids = new StringBuilder();
        for (String line : osRelease.split("\\r?\\n")) {
            String trimmed = line.trim();
            if (trimmed.startsWith("ID=") || trimmed.startsWith("ID_LIKE=")) {
                String value = trimmed.substring(trimmed.indexOf('=') + 1).replace("\"", "").replace("'", "");
                ids.append(' ').append(value.toLowerCase(Locale.ROOT));
            }
        }
        return ids.toString();
    }

    /**
     * Picks the release file for this kind of install and CPU. Prefers a file named for this CPU, accepts one that
     * doesn't name a CPU, and never picks one built for a different CPU.
     */
    public static Release.Asset pickAsset(Release release, Kind kind, String osArch) {
        if (release == null || release.assets == null || kind.extension == null) {
            return null;
        }
        boolean arm = osArch != null && containsAny(osArch.toLowerCase(Locale.ROOT), "aarch64", "arm64");
        List<String> mine = arm ? ARM_TOKENS : X64_TOKENS;
        List<String> other = arm ? X64_TOKENS : ARM_TOKENS;

        Release.Asset neutral = null;
        for (Release.Asset asset : release.assets) {
            if (asset == null || asset.name == null || asset.url == null) {
                continue;
            }
            String name = asset.name.toLowerCase(Locale.ROOT);
            if (!name.endsWith(kind.extension) || name.contains("sources") || name.contains("javadoc")) {
                continue;
            }
            if (kind == Kind.JAR) {
                // the jar runs anywhere
                return asset;
            }
            if (containsToken(name, mine)) {
                return asset;
            }
            if (!containsToken(name, other) && neutral == null) {
                neutral = asset;
            }
        }
        return neutral;
    }

    private static boolean containsToken(String name, List<String> tokens) {
        for (String token : tokens) {
            if (name.matches(".*(^|[^a-z0-9])" + token + "([^a-z0-9]|$).*")) {
                return true;
            }
        }
        return false;
    }

    private static boolean containsAny(String haystack, String... needles) {
        for (String needle : needles) {
            if (haystack.matches(".*(^|[^a-z0-9])" + needle + "([^a-z0-9]|$).*")) {
                return true;
            }
        }
        return false;
    }

    private static int indexOfAny(String s, char... chars) {
        int best = -1;
        for (char c : chars) {
            int i = s.indexOf(c);
            if (i >= 0 && (best < 0 || i < best)) {
                best = i;
            }
        }
        return best;
    }
}
