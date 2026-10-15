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
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.time.Instant;
import java.util.Arrays;
import java.util.Collections;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import com.blockforge.launcher.utils.PcProfile.Gpu;
import com.blockforge.launcher.utils.PcProfile.Tier;

public class PcProfileTest {
    @Test
    public void testIntegratedGraphics() {
        assertEquals(Gpu.INTEGRATED, PcProfile.classifyGpu("Intel Corporation", "Intel(R) UHD Graphics 620"));
        assertEquals(Gpu.INTEGRATED, PcProfile.classifyGpu("Intel Corporation", "Intel(R) HD Graphics 4000"));
        assertEquals(Gpu.INTEGRATED, PcProfile.classifyGpu("Intel Corporation", "Intel(R) Iris(R) Xe Graphics"));
        assertEquals(Gpu.INTEGRATED, PcProfile.classifyGpu("Intel Corporation", "Intel(R) Arc(TM) Graphics"));
        assertEquals(Gpu.INTEGRATED,
            PcProfile.classifyGpu("Advanced Micro Devices, Inc.", "AMD Radeon(TM) Graphics"));
        assertEquals(Gpu.INTEGRATED, PcProfile.classifyGpu("AMD", "AMD Radeon(TM) Vega 8 Graphics"));
        assertEquals(Gpu.INTEGRATED, PcProfile.classifyGpu("AMD", "AMD Radeon 780M"));
        assertEquals(Gpu.INTEGRATED,
            PcProfile.classifyGpu("Advanced Micro Devices, Inc. [AMD/ATI]", "Rembrandt [Radeon 680M]"));
        assertEquals(Gpu.INTEGRATED, PcProfile.classifyGpu("Apple", "Apple M2"));
        assertEquals(Gpu.INTEGRATED, PcProfile.classifyGpu("", "Apple M3 Pro"));
        assertEquals(Gpu.INTEGRATED, PcProfile.classifyGpu("VMware, Inc.", "SVGA3D; build: RELEASE; LLVM;"));
        assertEquals(Gpu.INTEGRATED, PcProfile.classifyGpu("Mesa", "llvmpipe (LLVM 17.0.6, 256 bits)"));
    }

    @Test
    public void testDedicatedGraphics() {
        assertEquals(Gpu.DEDICATED, PcProfile.classifyGpu("NVIDIA", "NVIDIA GeForce RTX 4060 Laptop GPU"));
        assertEquals(Gpu.DEDICATED, PcProfile.classifyGpu("NVIDIA Corporation", "GeForce GTX 1050 Ti"));
        assertEquals(Gpu.DEDICATED,
            PcProfile.classifyGpu("Advanced Micro Devices, Inc.", "AMD Radeon RX 6700 XT"));
        assertEquals(Gpu.DEDICATED, PcProfile.classifyGpu("AMD", "Radeon RX Vega 64"));
        assertEquals(Gpu.DEDICATED, PcProfile.classifyGpu("Intel Corporation", "Intel(R) Arc(TM) A770 Graphics"));
    }

    @Test
    public void testUnknownGraphics() {
        assertEquals(Gpu.UNKNOWN, PcProfile.classifyGpu(null, null));
        assertEquals(Gpu.UNKNOWN, PcProfile.classifyGpu("", ""));
        assertEquals(Gpu.UNKNOWN, PcProfile.classifyGpu("Moore Threads", "MTT S80"));
    }

    @Test
    public void testTheDedicatedCardWins() {
        assertEquals(Gpu.DEDICATED, PcProfile.bestOf(Arrays.asList(Gpu.INTEGRATED, Gpu.DEDICATED)));
        assertEquals(Gpu.INTEGRATED, PcProfile.bestOf(Arrays.asList(Gpu.UNKNOWN, Gpu.INTEGRATED)));
        assertEquals(Gpu.UNKNOWN, PcProfile.bestOf(Collections.emptyList()));
    }

    @Test
    public void testTiers() {
        // low: little memory, few cores, or a budget laptop with integrated graphics
        assertEquals(Tier.LOW, PcProfile.tierFor(3790, 4, Gpu.DEDICATED));
        assertEquals(Tier.LOW, PcProfile.tierFor(16000, 2, Gpu.DEDICATED));
        assertEquals(Tier.LOW, PcProfile.tierFor(7850, 4, Gpu.INTEGRATED));
        // medium: integrated graphics on a better computer, or graphics that couldn't be identified
        assertEquals(Tier.MEDIUM, PcProfile.tierFor(15900, 8, Gpu.INTEGRATED));
        assertEquals(Tier.MEDIUM, PcProfile.tierFor(7850, 8, Gpu.INTEGRATED));
        assertEquals(Tier.MEDIUM, PcProfile.tierFor(7850, 4, Gpu.DEDICATED));
        assertEquals(Tier.MEDIUM, PcProfile.tierFor(32000, 16, Gpu.UNKNOWN));
        assertEquals(Tier.MEDIUM, PcProfile.tierFor(7850, 4, Gpu.UNKNOWN));
        // high: a gaming PC
        assertEquals(Tier.HIGH, PcProfile.tierFor(15900, 6, Gpu.DEDICATED));
        assertEquals(Tier.HIGH, PcProfile.tierFor(32000, 16, Gpu.DEDICATED));
    }

    @Test
    public void testDownloadThreads() {
        assertEquals(8, PcProfile.downloadThreads(3790, 4));
        assertEquals(8, PcProfile.downloadThreads(16000, 2));
        assertEquals(16, PcProfile.downloadThreads(7850, 4));
        assertEquals(16, PcProfile.downloadThreads(-1, 8));
        // a number the player set is kept, the default is fitted to the computer
        assertEquals(32, PcProfile.downloadThreads(32, 16, 3790, 2));
        assertEquals(8, PcProfile.downloadThreads(16, 16, 3790, 2));
        assertEquals(16, PcProfile.downloadThreads(16, 16, 32000, 16));
    }

    @Test
    public void testTheSavedProfileIsUsedUntilTheComputerChanges(@TempDir Path dir) {
        PcProfile.Saved saved = new PcProfile.Saved();
        saved.totalMb = 16000;
        saved.cores = 8;
        saved.gpu = "DEDICATED";
        saved.gpuName = "GeForce";
        saved.checkedAt = Instant.parse("2026-10-01T00:00:00Z").toString();

        Instant soon = Instant.parse("2026-10-10T00:00:00Z");
        assertTrue(PcProfile.isStillValid(saved, 16100, 8, soon), "the reported memory wobbles a little");
        assertFalse(PcProfile.isStillValid(saved, 32000, 8, soon), "more memory: look again");
        assertFalse(PcProfile.isStillValid(saved, 16000, 12, soon), "another processor: look again");
        assertFalse(PcProfile.isStillValid(saved, 16000, 8, soon.plus(Duration.ofDays(40))), "a month later");

        Path file = dir.resolve("cache/pc-profile.json");
        PcProfile.writeSaved(file, saved);
        assertTrue(Files.exists(file));
        PcProfile.Saved back = PcProfile.readSaved(file);
        assertNotNull(back);
        assertEquals("DEDICATED", back.gpu);
        assertEquals(Gpu.DEDICATED, PcProfile.parseGpu(back.gpu));
        assertEquals(Gpu.UNKNOWN, PcProfile.parseGpu("nonsense"));
        assertNull(PcProfile.readSaved(dir.resolve("missing.json")));
    }
}
