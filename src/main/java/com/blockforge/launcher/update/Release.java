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

import java.util.ArrayList;
import java.util.List;

import com.blockforge.launcher.annot.Json;
import com.google.gson.annotations.SerializedName;

/** The parts of a GitHub release (from the releases API) that the updater needs. */
@Json
public class Release {
    @SerializedName("tag_name")
    public String tagName;

    public String name;

    /** the release notes, in Markdown */
    public String body;

    @SerializedName("html_url")
    public String htmlUrl;

    public boolean prerelease;

    public boolean draft;

    public List<Asset> assets = new ArrayList<>();

    @Json
    public static class Asset {
        public String name;

        @SerializedName("browser_download_url")
        public String url;

        public long size;

        /** "sha256:<hex>" when GitHub has computed it, otherwise null */
        public String digest;

        public Asset() {
        }

        public Asset(String name, String url, long size) {
            this.name = name;
            this.url = url;
            this.size = size;
        }

        /** The expected SHA-256 in lower case hex, or null when the release doesn't say. */
        public String sha256() {
            if (digest == null || !digest.toLowerCase(java.util.Locale.ROOT).startsWith("sha256:")) {
                return null;
            }
            String hex = digest.substring("sha256:".length()).trim().toLowerCase(java.util.Locale.ROOT);
            return hex.matches("[0-9a-f]{64}") ? hex : null;
        }
    }
}
