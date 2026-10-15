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

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;

import java.util.Arrays;

import org.junit.jupiter.api.Test;

public class VersionRangesTest {
    @Test
    public void fabricStylePredicates() {
        assertEquals(true, VersionRanges.matches(">=1.21 <1.22", "1.21.4"));
        assertEquals(false, VersionRanges.matches(">=1.21 <1.22", "1.22"));
        assertEquals(true, VersionRanges.matches("~1.20.1", "1.20.4"));
        assertEquals(false, VersionRanges.matches("~1.20.1", "1.21"));
        assertEquals(true, VersionRanges.matches("1.21.x", "1.21.11"));
        assertEquals(false, VersionRanges.matches("1.21.x", "26.3"));
        assertEquals(true, VersionRanges.matches("*", "26.3"));
        assertEquals(true, VersionRanges.matches("1.20", "1.20.0"));
        assertEquals(true, VersionRanges.matches("^1.20.1", "1.20.6"));
        assertEquals(false, VersionRanges.matches("^1.20.1", "1.21"));
        assertEquals(true, VersionRanges.matches(">=26.1", "26.3"));
        assertEquals(true, VersionRanges.matches("1.19.2 || >=1.20", "1.20.1"));
        assertEquals(true, VersionRanges.matchesAny(Arrays.asList("1.20.1", "1.20.4"), "1.20.4"));
        assertEquals(false, VersionRanges.matchesAny(Arrays.asList("1.20.1", "1.20.4"), "1.21"));
    }

    @Test
    public void mavenRanges() {
        assertEquals(true, VersionRanges.matches("[1.20.1,1.21)", "1.20.4"));
        assertEquals(false, VersionRanges.matches("[1.20.1,1.21)", "1.21"));
        assertEquals(true, VersionRanges.matches("[1.20.1]", "1.20.1"));
        assertEquals(false, VersionRanges.matches("[1.20.1]", "1.20.2"));
        assertEquals(true, VersionRanges.matches("[1.21,)", "26.3"));
        assertEquals(false, VersionRanges.matches("(1.20,1.20.4]", "1.20"));
        assertEquals(true, VersionRanges.matches("[1.16,1.17),[1.18,1.19)", "1.18.2"));
    }

    @Test
    public void unreadableMeansUnknown() {
        assertNull(VersionRanges.matches("${minecraft_version}", "1.20.1"));
        assertNull(VersionRanges.matches(">=1.20", "25w14a"));
        assertNull(VersionRanges.matches(null, "1.20.1"));
        assertNull(VersionRanges.matches("", "1.20.1"));
        assertEquals(true, VersionRanges.matches(">=1.21-alpha.24.10.a", "1.21.4"));
    }
}
