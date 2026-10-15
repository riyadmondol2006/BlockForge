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

import java.util.concurrent.atomic.AtomicLong;

import org.junit.jupiter.api.Test;

public class LaunchTimingsTest {
    @Test
    public void testSummary() {
        AtomicLong now = new AtomicLong(1000);
        LaunchTimings timings = new LaunchTimings(now::get);

        now.addAndGet(5);
        timings.step("libraries");
        timings.step("java");
        now.addAndGet(120);
        timings.step("assets");

        assertEquals(125, timings.total());
        assertEquals("125 ms (libraries 5 ms, assets 120 ms)", timings.summary());
    }
}
