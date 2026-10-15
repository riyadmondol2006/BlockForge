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

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

public class GameMemoryTest {
    @Test
    public void testThatTheDefaultIsHalfTheMemoryUpTo4Gb() {
        assertEquals(1024, GameMemory.defaultMaximumMb(2048, true));
        assertEquals(2048, GameMemory.defaultMaximumMb(4096, true));
        assertEquals(4096, GameMemory.defaultMaximumMb(8192, true));
        assertEquals(4096, GameMemory.defaultMaximumMb(16384, true));
        assertEquals(4096, GameMemory.defaultMaximumMb(65536, true));
    }

    @Test
    public void testThatTheDefaultCopesWithOddAmountsOfMemory() {
        // what a "4 GB" laptop with shared graphics memory reports
        assertEquals(1792, GameMemory.defaultMaximumMb(3790, true));
        // what an "8 GB" computer usually reports
        assertEquals(3840, GameMemory.defaultMaximumMb(7850, true));
        // tiny
        assertEquals(512, GameMemory.defaultMaximumMb(700, true));
        // unknown
        assertEquals(GameMemory.UNKNOWN_DEFAULT_MB, GameMemory.defaultMaximumMb(0, true));
    }

    @Test
    public void testThat32BitJavaNeverGetsMoreThan1Gb() {
        assertEquals(1024, GameMemory.defaultMaximumMb(8192, false));
        assertEquals(1024, GameMemory.defaultMaximumMb(0, false));
        assertEquals(1024, GameMemory.safeMaximumMb(16384, false));
    }

    @Test
    public void testThatWhatIsSetIsUsedWhenNothingAsksForMore() {
        GameMemory.Plan plan = GameMemory.plan(3072, 0, 16384, true);

        assertEquals(3072, plan.heapMb);
        assertTrue(plan.warnings.isEmpty());
    }

    @Test
    public void testThatAModpackGetsWhatItRecommendsWhenTheComputerCanSpareIt() {
        GameMemory.Plan plan = GameMemory.plan(4096, 6144, 16384, true);

        assertEquals(6144, plan.heapMb);
        assertTrue(plan.warnings.isEmpty());
    }

    @Test
    public void testThatAModpackNeverGetsMoreThanHalfTheMemory() {
        // 8 GB computer, a pack asking for 6 GB, settings at 2 GB: raised, but only to what can be spared
        GameMemory.Plan plan = GameMemory.plan(2048, 6144, 8192, true);

        assertEquals(4096, plan.heapMb);
        assertEquals(1, plan.warnings.size());
        assertTrue(plan.warnings.get(0).contains("6144"));
    }

    @Test
    public void testThatAModpackNeverLowersWhatIsSet() {
        GameMemory.Plan plan = GameMemory.plan(4096, 2048, 8192, true);

        assertEquals(4096, plan.heapMb);
    }

    @Test
    public void testThatSettingMoreThanHalfIsKeptButWarnedAbout() {
        GameMemory.Plan plan = GameMemory.plan(6144, 0, 8192, true);

        assertEquals(6144, plan.heapMb, "what the person chose is respected");
        assertEquals(1, plan.warnings.size());
    }

    @Test
    public void testThatUnknownMemoryChangesNothing() {
        GameMemory.Plan plan = GameMemory.plan(4096, 8192, 0, true);

        assertEquals(8192, plan.heapMb);
        assertTrue(plan.warnings.isEmpty());
    }

    @Test
    public void testAvailabilityWarning() {
        assertNull(GameMemory.availabilityWarning(2048, 4000));
        assertNull(GameMemory.availabilityWarning(2048, 0));
        assertNotNull(GameMemory.availabilityWarning(4096, 1500));
    }

    @Test
    public void testIsLowMemoryComputer() {
        assertTrue(GameMemory.isLowMemoryComputer(2048));
        assertTrue(GameMemory.isLowMemoryComputer(3790));
        assertTrue(GameMemory.isLowMemoryComputer(4096));
        assertFalse(GameMemory.isLowMemoryComputer(8192));
        assertFalse(GameMemory.isLowMemoryComputer(0));
    }
}
