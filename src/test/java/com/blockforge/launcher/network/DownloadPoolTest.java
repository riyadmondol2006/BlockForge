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
package com.blockforge.launcher.network;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;

import org.junit.jupiter.api.Test;

import com.blockforge.launcher.data.minecraft.JavaRuntimeManifestFileDownloads;

public class DownloadPoolTest {
    @Test
    public void testDownloadThreads() {
        // the setting on a normal computer
        assertEquals(16, DownloadPool.downloadThreads(16, 8, 4000));
        // 2 cores: no more than 8 at a time
        assertEquals(8, DownloadPool.downloadThreads(16, 2, 4000));
        // a lower setting is always respected
        assertEquals(2, DownloadPool.downloadThreads(2, 16, 4000));
        // never more threads than files
        assertEquals(3, DownloadPool.downloadThreads(16, 8, 3));
        assertEquals(1, DownloadPool.downloadThreads(0, 8, 0));
    }

    @Test
    public void testThatTheBiggestFilesStartFirstAndTheRestKeepTheirOrder() {
        List<Download> downloads = new ArrayList<>();
        long[] sizes = { 10, 500, 20, 900, 30, 40 };
        for (int i = 0; i < sizes.length; i++) {
            downloads.add(new Download().downloadTo(Paths.get("file" + i)).size(sizes[i]));
        }

        List<Download> order = DownloadPool.startOrder(downloads, 2);

        assertEquals(6, order.size());
        assertEquals(900, order.get(0).size);
        assertEquals(500, order.get(1).size);
        assertEquals(10, order.get(2).size);
        assertEquals(20, order.get(3).size);
        assertEquals(30, order.get(4).size);
        assertEquals(40, order.get(5).size);
    }

    @Test
    public void testCheckThreads() {
        assertEquals(4, DownloadPool.checkThreads(2, 4000));
        assertEquals(16, DownloadPool.checkThreads(32, 4000));
        assertEquals(2, DownloadPool.checkThreads(1, 4000));
        assertEquals(1, DownloadPool.checkThreads(8, 1));
    }

    @Test
    public void testThatJavaRuntimeFilesAreFetchedCompressedWhenOffered() {
        JavaRuntimeManifestFileDownloads downloads = new JavaRuntimeManifestFileDownloads();
        downloads.raw = new com.blockforge.launcher.data.minecraft.Download();
        downloads.raw.url = "https://example.com/raw";
        downloads.raw.sha1 = "aaaa";
        downloads.raw.size = 1000;

        Download plain = Download.forJavaRuntimeFile(downloads, Paths.get("lib/modules"));
        assertEquals("https://example.com/raw", plain.url);
        assertEquals(1000, plain.size);
        assertFalse(plain.isLzmaCompressed());

        downloads.lzma = new com.blockforge.launcher.data.minecraft.Download();
        downloads.lzma.url = "https://example.com/lzma";
        downloads.lzma.sha1 = "bbbb";
        downloads.lzma.size = 300;

        Download compressed = Download.forJavaRuntimeFile(downloads, Paths.get("lib/modules"));
        assertEquals("https://example.com/lzma", compressed.url);
        assertEquals(300, compressed.size, "progress counts what comes over the network");
        assertTrue(compressed.isLzmaCompressed());
        assertEquals("aaaa", compressed.getHash(), "checked against the unpacked file");
    }
}
