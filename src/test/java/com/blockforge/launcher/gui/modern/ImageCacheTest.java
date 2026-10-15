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
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.awt.image.BufferedImage;

import org.junit.jupiter.api.Test;

class ImageCacheTest {
    private static BufferedImage image(int w, int h) {
        return new BufferedImage(w, h, BufferedImage.TYPE_INT_ARGB);
    }

    @Test
    void staysWithinItsByteLimitAndDropsTheLeastRecentlyUsed() {
        // room for exactly three 100x100 images
        ImageCache.Lru cache = new ImageCache.Lru(3 * 100 * 100 * 4);
        cache.put("a", image(100, 100));
        cache.put("b", image(100, 100));
        cache.put("c", image(100, 100));
        cache.get("a"); // a was used recently, so b is now the oldest

        cache.put("d", image(100, 100));

        assertNull(cache.get("b"));
        assertNotNull(cache.get("a"));
        assertNotNull(cache.get("c"));
        assertNotNull(cache.get("d"));
        assertTrue(cache.bytes() <= 3 * 100 * 100 * 4);
    }

    @Test
    void manyBigBannersDoNotGrowWithoutLimit() {
        ImageCache.Lru cache = new ImageCache.Lru(ImageCache.MAX_BYTES);
        for (int i = 0; i < 1000; i++) {
            cache.put("instance:" + i, image(544, 256));
        }
        assertTrue(cache.bytes() <= ImageCache.MAX_BYTES);
        assertEquals(ImageCache.MAX_BYTES / ImageCache.Lru.sizeOf(image(544, 256)), cache.size());
    }

    @Test
    void replacingAnImageKeepsTheCountRight() {
        ImageCache.Lru cache = new ImageCache.Lru(1024 * 1024);
        cache.put("a", image(10, 10));
        cache.put("a", image(20, 20));
        assertEquals(20 * 20 * 4, cache.bytes());

        // one image larger than the whole limit is still kept, it is what was just asked for
        cache.put("huge", image(1000, 1000));
        assertNotNull(cache.get("huge"));
        assertEquals(1, cache.size());
    }
}
