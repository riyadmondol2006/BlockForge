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

import java.util.Collection;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Removes anything personal or secret from text before it is shared: account names and tokens, e-mail addresses,
 * IDs, IP addresses and the name of the user's home folder.
 */
public final class Redactor {
    private static final Pattern EMAIL = Pattern.compile("[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\\.[A-Za-z]{2,}");
    private static final Pattern UUID = Pattern
        .compile("\\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\\b");
    private static final Pattern JWT = Pattern
        .compile("\\beyJ[A-Za-z0-9_-]{8,}\\.[A-Za-z0-9_-]{8,}\\.[A-Za-z0-9_-]{8,}\\b");
    private static final Pattern BEARER = Pattern.compile("(?i)\\bBearer\\s+[A-Za-z0-9._~+/=-]{8,}");
    private static final Pattern IPV4 = Pattern
        .compile("\\b(?:(?:25[0-5]|2[0-4]\\d|1?\\d?\\d)\\.){3}(?:25[0-5]|2[0-4]\\d|1?\\d?\\d)\\b");
    /** launch arguments and JSON/config fields that carry credentials or identity */
    private static final Pattern ARGUMENT = Pattern
        .compile("(?i)(--(?:accessToken|session|uuid|username|xuid|clientId)\\s+)\\S+");
    private static final Pattern FIELD = Pattern.compile(
        "(?i)(\"?(?:access_?token|refresh_?token|session_?token|id_?token|client_?secret|password|token)\"?\\s*[:=]\\s*\"?)[^\\s\",}]+");

    private Redactor() {
    }

    public static String scrub(String text) {
        return scrub(text, null);
    }

    /**
     * @param extraSecrets values that must disappear wherever they occur, such as the player's name
     */
    public static String scrub(String text, Collection<String> extraSecrets) {
        if (text == null || text.isEmpty()) {
            return text;
        }

        String result = text;

        if (extraSecrets != null) {
            for (String secret : extraSecrets) {
                if (secret != null && secret.length() >= 3) {
                    result = result.replace(secret, "<hidden>");
                }
            }
        }

        result = replaceHome(result);
        result = ARGUMENT.matcher(result).replaceAll("$1<hidden>");
        result = FIELD.matcher(result).replaceAll("$1<hidden>");
        result = BEARER.matcher(result).replaceAll("Bearer <token>");
        result = JWT.matcher(result).replaceAll("<token>");
        result = EMAIL.matcher(result).replaceAll("<email>");
        result = UUID.matcher(result).replaceAll("<uuid>");
        result = redactAddresses(result);
        return result;
    }

    /** replaces the user's home folder, and failing that their login name, wherever it appears in a path */
    private static String replaceHome(String text) {
        String result = text;

        String home = System.getProperty("user.home");
        if (home != null && home.length() > 3) {
            result = result.replace(home, "~").replace(home.replace('\\', '/'), "~");
        }

        String user = System.getProperty("user.name");
        if (user != null && user.length() >= 3) {
            // only inside paths, so an ordinary word that equals the user name isn't touched
            result = result.replaceAll("(?i)([/\\\\](?:home|Users)[/\\\\])" + Pattern.quote(user) + "(?=[/\\\\\\s\"']|$)",
                "$1<user>");
        }

        return result;
    }

    /** hides IPv4 addresses but leaves loopback and version-like numbers (such as mod-1.2.3.4) alone */
    private static String redactAddresses(String text) {
        Matcher matcher = IPV4.matcher(text);
        StringBuilder out = new StringBuilder();
        while (matcher.find()) {
            String ip = matcher.group();
            int start = matcher.start();
            char before = start > 0 ? text.charAt(start - 1) : ' ';
            boolean looksLikeVersion = Character.isLetter(before) || before == '-' || before == '_' || before == '+';
            boolean loopback = ip.startsWith("127.") || ip.equals("0.0.0.0");
            matcher.appendReplacement(out, Matcher.quoteReplacement(looksLikeVersion || loopback ? ip : "<ip>"));
        }
        matcher.appendTail(out);
        return out.toString();
    }
}
