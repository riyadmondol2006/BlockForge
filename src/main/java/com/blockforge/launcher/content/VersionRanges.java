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
package com.blockforge.launcher.content;

import java.util.ArrayList;
import java.util.List;

/**
 * Checks a Minecraft version against the version requirements mods write in their metadata: Fabric and Quilt style
 * ({@code >=1.21 <1.22}, {@code ~1.20.1}, {@code 1.21.x}) and Forge's Maven ranges ({@code [1.20.1,1.21)}).
 * It is only used to warn, so anything it can't read counts as unknown rather than wrong.
 */
public final class VersionRanges {
    private VersionRanges() {
    }

    /**
     * Whether a Minecraft version meets a requirement.
     *
     * @return true or false, or null when the requirement or the version can't be read
     */
    public static Boolean matches(String requirement, String version) {
        if (requirement == null || version == null) {
            return null;
        }
        String req = requirement.trim();
        if (req.isEmpty() || req.contains("${")) {
            return null;
        }
        if (parse(version) == null) {
            return null;
        }
        if (req.startsWith("[") || req.startsWith("(")) {
            return maven(req, version);
        }
        Boolean any = null;
        for (String alternative : req.split("\\|\\|")) {
            Boolean one = all(alternative.trim(), version);
            if (Boolean.TRUE.equals(one)) {
                return true;
            }
            if (one != null) {
                any = false;
            }
        }
        return any;
    }

    /** Several requirements, any of which is enough (Fabric's array form). */
    public static Boolean matchesAny(List<String> requirements, String version) {
        Boolean result = null;
        for (String requirement : requirements) {
            Boolean one = matches(requirement, version);
            if (Boolean.TRUE.equals(one)) {
                return true;
            }
            if (one != null) {
                result = false;
            }
        }
        return result;
    }

    /** Space separated predicates that must all hold. */
    private static Boolean all(String predicates, String version) {
        if (predicates.isEmpty()) {
            return null;
        }
        for (String predicate : predicates.split("\\s+")) {
            Boolean one = predicate(predicate, version);
            if (one == null) {
                return null;
            }
            if (!one) {
                return false;
            }
        }
        return true;
    }

    private static Boolean predicate(String p, String version) {
        if (p.equals("*") || p.equals("x")) {
            return true;
        }
        int[] v = parse(version);
        if (p.startsWith(">=")) {
            int[] b = parse(p.substring(2));
            return b == null ? null : compare(v, b) >= 0;
        }
        if (p.startsWith("<=")) {
            int[] b = parse(p.substring(2));
            return b == null ? null : compare(v, b) <= 0;
        }
        if (p.startsWith(">")) {
            int[] b = parse(p.substring(1));
            return b == null ? null : compare(v, b) > 0;
        }
        if (p.startsWith("<")) {
            int[] b = parse(p.substring(1));
            return b == null ? null : compare(v, b) < 0;
        }
        if (p.startsWith("~")) {
            // same major and minor, at least this patch
            int[] b = parse(p.substring(1));
            if (b == null) {
                return null;
            }
            return compare(v, b) >= 0 && part(v, 0) == part(b, 0) && part(v, 1) == part(b, 1);
        }
        if (p.startsWith("^")) {
            int[] b = parse(p.substring(1));
            if (b == null) {
                return null;
            }
            // for Minecraft's 1.x versions the minor number is what really changes, so ^1.20 stays on 1.20
            boolean sameLine = part(b, 0) == 1 ? part(v, 1) == part(b, 1) : part(v, 0) == part(b, 0);
            return compare(v, b) >= 0 && part(v, 0) == part(b, 0) && sameLine;
        }
        String body = p.startsWith("=") ? p.substring(1) : p;
        if (body.endsWith(".x") || body.endsWith(".*") || body.endsWith(".X")) {
            int[] prefix = parse(body.substring(0, body.length() - 2));
            if (prefix == null) {
                return null;
            }
            for (int i = 0; i < prefix.length; i++) {
                if (part(v, i) != prefix[i]) {
                    return false;
                }
            }
            return true;
        }
        int[] b = parse(body);
        return b == null ? null : compare(v, b) == 0;
    }

    /** Maven ranges: [a,b) and friends, several joined by commas meaning any of them. */
    private static Boolean maven(String req, String version) {
        int[] v = parse(version);
        List<String> ranges = new ArrayList<>();
        int depth = 0;
        int start = 0;
        for (int i = 0; i < req.length(); i++) {
            char c = req.charAt(i);
            if (c == '[' || c == '(') {
                if (depth == 0) {
                    start = i;
                }
                depth++;
            } else if (c == ']' || c == ')') {
                depth--;
                if (depth == 0) {
                    ranges.add(req.substring(start, i + 1));
                }
            }
        }
        if (ranges.isEmpty()) {
            return null;
        }
        for (String range : ranges) {
            boolean lowInclusive = range.charAt(0) == '[';
            boolean highInclusive = range.charAt(range.length() - 1) == ']';
            String inner = range.substring(1, range.length() - 1);
            String[] bounds = inner.split(",", -1);
            if (bounds.length == 1) {
                int[] exact = parse(bounds[0].trim());
                if (exact == null) {
                    return null;
                }
                if (compare(v, exact) == 0) {
                    return true;
                }
                continue;
            }
            String low = bounds[0].trim();
            String high = bounds[1].trim();
            boolean ok = true;
            if (!low.isEmpty()) {
                int[] l = parse(low);
                if (l == null) {
                    return null;
                }
                int c = compare(v, l);
                ok = lowInclusive ? c >= 0 : c > 0;
            }
            if (ok && !high.isEmpty()) {
                int[] h = parse(high);
                if (h == null) {
                    return null;
                }
                int c = compare(v, h);
                ok = highInclusive ? c <= 0 : c < 0;
            }
            if (ok) {
                return true;
            }
        }
        return false;
    }

    /** "1.20.1" as {1, 20, 1}; text after a dash (pre-releases) is ignored. Null if it isn't a numbered version. */
    static int[] parse(String version) {
        if (version == null) {
            return null;
        }
        String v = version.trim();
        int dash = v.indexOf('-');
        if (dash > 0) {
            v = v.substring(0, dash);
        }
        int plus = v.indexOf('+');
        if (plus > 0) {
            v = v.substring(0, plus);
        }
        if (v.isEmpty()) {
            return null;
        }
        String[] parts = v.split("\\.");
        int[] out = new int[parts.length];
        for (int i = 0; i < parts.length; i++) {
            if (!parts[i].matches("\\d{1,6}")) {
                return null;
            }
            out[i] = Integer.parseInt(parts[i]);
        }
        return out;
    }

    private static int part(int[] v, int i) {
        return i < v.length ? v[i] : 0;
    }

    static int compare(int[] a, int[] b) {
        int n = Math.max(a.length, b.length);
        for (int i = 0; i < n; i++) {
            int c = Integer.compare(part(a, i), part(b, i));
            if (c != 0) {
                return c;
            }
        }
        return 0;
    }
}
