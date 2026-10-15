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
package com.blockforge.launcher.managers;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

import org.junit.jupiter.api.Test;

class LogManagerTest {
    @Test
    void keepsAStartUpsWorthOfLinesUntilTheLoggingThreadRuns() {
        // the console is built alongside the loading, so hundreds of lines can arrive before anything reads them
        int lines = 2000;
        int before = LogManager.pending();
        assumeTrue(before + lines <= LogManager.QUEUE_CAPACITY, "the queue is already being filled by other tests");

        for (int i = 0; i < lines; i++) {
            LogManager.info("line " + i);
        }

        assertEquals(before + lines, LogManager.pending());
    }
}
