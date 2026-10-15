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
package com.blockforge.launcher.gui.modern;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

class ImageCacheScaleTest {
    @Test
    void ordinaryScreensKeepPicturesAtTheSizeShown() {
        assertEquals(1, ImageCache.clampScale(1.0));
    }

    @Test
    void highDensityAndFractionalScreensGetDoubleSize() {
        assertEquals(2, ImageCache.clampScale(1.25));
        assertEquals(2, ImageCache.clampScale(1.5));
        assertEquals(2, ImageCache.clampScale(2.0));
        assertEquals(2, ImageCache.clampScale(3.0), "more than double isn't worth the memory");
    }

    @Test
    void unknownScaleKeepsTheOldDoubleSize() {
        assertEquals(2, ImageCache.clampScale(0));
        assertEquals(2, ImageCache.clampScale(Double.NaN));
    }
}
