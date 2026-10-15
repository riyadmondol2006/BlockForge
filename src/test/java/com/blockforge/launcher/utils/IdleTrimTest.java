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
package com.blockforge.launcher.utils;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

class IdleTrimTest {
    private static final long MB = 1024 * 1024;
    private static final long START = 1_000_000_000L;

    @Test
    void waitsUntilLeftAlone() {
        long input = START;
        assertFalse(IdleTrim.due(input + IdleTrim.IDLE_MS - 1, input, 0, 200 * MB, 0));
        assertTrue(IdleTrim.due(input + IdleTrim.IDLE_MS, input, 0, 200 * MB, 0));
    }

    @Test
    void tidiesOnceAfterEachUse() {
        long input = START;
        long trimmed = input + IdleTrim.IDLE_MS;
        // nothing happened since: no second tidy up, however long it sits
        assertFalse(IdleTrim.due(trimmed + 60 * 60_000, input, trimmed, 64 * MB, 64 * MB));
        // used again, then left alone again
        long input2 = trimmed + 10_000;
        assertFalse(IdleTrim.due(input2 + 1_000, input2, trimmed, 64 * MB, 64 * MB));
        assertTrue(IdleTrim.due(input2 + IdleTrim.IDLE_MS, input2, trimmed, 64 * MB, 64 * MB));
    }

    @Test
    void backgroundGrowthIsTidiedNowAndThen() {
        long input = START;
        long trimmed = input + IdleTrim.IDLE_MS;
        long grown = 64 * MB + IdleTrim.GROWTH_BYTES;
        assertFalse(IdleTrim.due(trimmed + IdleTrim.MIN_GROWTH_INTERVAL_MS - 1, input, trimmed, grown, 64 * MB),
            "not too often");
        assertTrue(IdleTrim.due(trimmed + IdleTrim.MIN_GROWTH_INTERVAL_MS, input, trimmed, grown, 64 * MB));
        assertFalse(IdleTrim.due(trimmed + IdleTrim.MIN_GROWTH_INTERVAL_MS, input, trimmed, grown - 1, 64 * MB),
            "small growth isn't worth a collection");
    }

    @Test
    void neverWhileInUse() {
        long now = START + 10 * 60_000;
        assertFalse(IdleTrim.due(now, now - 5_000, 0, 1024 * MB, 0));
    }
}
