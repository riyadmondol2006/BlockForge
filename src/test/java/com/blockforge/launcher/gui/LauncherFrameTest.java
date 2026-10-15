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
package com.blockforge.launcher.gui;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.awt.Dimension;
import java.awt.Point;
import java.awt.Rectangle;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;

import org.junit.jupiter.api.Test;

class LauncherFrameTest {
    @Test
    void theWindowNeverOpensBiggerThanTheScreen() {
        // 1920x1080 at 150% scaling with a 48 pixel task bar
        Rectangle laptop = new Rectangle(0, 0, 1280, 672);

        assertEquals(new Dimension(1080, 672), LauncherFrame.fit(new Dimension(1080, 680), laptop));
        assertEquals(new Dimension(1240, 672), LauncherFrame.fit(new Dimension(1240, 780), laptop));
        assertEquals(new Dimension(1240, 780),
            LauncherFrame.fit(new Dimension(1240, 780), new Rectangle(0, 0, 2560, 1400)));
    }

    @Test
    void aRememberedPositionOnAScreenThatIsGoneIsIgnored() {
        Dimension size = new Dimension(1240, 780);
        List<Rectangle> oneScreen = Collections.singletonList(new Rectangle(0, 0, 1920, 1080));

        assertTrue(LauncherFrame.isReachable(new Point(100, 100), size, oneScreen));
        // was on a second monitor to the right, which isn't connected any more
        assertFalse(LauncherFrame.isReachable(new Point(2200, 100), size, oneScreen));
        // title bar above the top of the screen
        assertFalse(LauncherFrame.isReachable(new Point(100, -400), size, oneScreen));

        List<Rectangle> twoScreens = Arrays.asList(new Rectangle(0, 0, 1920, 1080),
            new Rectangle(1920, 0, 1920, 1080));
        assertTrue(LauncherFrame.isReachable(new Point(2200, 100), size, twoScreens));
    }
}
