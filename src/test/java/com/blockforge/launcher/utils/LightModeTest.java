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

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;

import com.blockforge.launcher.App;
import com.blockforge.launcher.data.Settings;

class LightModeTest {
    private final Settings original = App.settings;

    @AfterEach
    void restore() {
        App.settings = original;
    }

    @Test
    void lowEndIsLittleMemoryOrFewCores() {
        assertTrue(LightMode.isLowEndComputer(2048, 4), "2 GB is low end");
        assertTrue(LightMode.isLowEndComputer(3891, 4), "a 4 GB machine reports a bit under 4096 MB");
        assertTrue(LightMode.isLowEndComputer(16384, 2), "2 cores is low end");
        assertTrue(LightMode.isLowEndComputer(16384, 1));
        assertFalse(LightMode.isLowEndComputer(8192, 4));
        assertFalse(LightMode.isLowEndComputer(16384, 8));
        // unknown values never switch it on by themselves
        assertFalse(LightMode.isLowEndComputer(-1, 8));
        assertFalse(LightMode.isLowEndComputer(8192, 0));
    }

    @Test
    void theSettingWinsOverDetection() {
        App.settings = new Settings();
        App.settings.lightMode = true;
        assertTrue(LightMode.isOn());
        assertEquals(16L * 1024 * 1024, LightMode.imageCacheBytes());
        assertEquals(50, LightMode.frameDelayMs());

        App.settings.lightMode = false;
        assertFalse(LightMode.isOn());
        assertEquals(48L * 1024 * 1024, LightMode.imageCacheBytes());
        assertEquals(16, LightMode.frameDelayMs());

        App.settings.lightMode = null;
        assertEquals(LightMode.detectedLowEnd(), LightMode.isOn());
    }
}
