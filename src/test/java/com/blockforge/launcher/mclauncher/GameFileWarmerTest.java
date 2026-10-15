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

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.List;
import java.util.concurrent.atomic.AtomicInteger;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

public class GameFileWarmerTest {
    static final long MB = 1024 * 1024;

    @Test
    public void testBudget() {
        assertEquals(1024 * MB, GameFileWarmer.budgetBytes(16000, false), "at most 1 GB");
        assertEquals(500 * MB, GameFileWarmer.budgetBytes(2000, false), "a quarter of what is free");
        assertEquals(256 * MB, GameFileWarmer.budgetBytes(8000, true), "small computers read less");
        assertEquals(0, GameFileWarmer.budgetBytes(900, false), "too little free to bother");
        assertEquals(512 * MB, GameFileWarmer.budgetBytes(-1, false), "unknown: half the cap");
    }

    @Test
    public void testHowManyGamesAreKeptWarm() {
        assertEquals(1, GameFileWarmer.instancesToWarm(true, 32000), "light mode: only the latest");
        assertEquals(1, GameFileWarmer.instancesToWarm(false, 3790), "4 GB: only the latest");
        assertEquals(1, GameFileWarmer.instancesToWarm(false, 7850));
        assertEquals(2, GameFileWarmer.instancesToWarm(false, 15900));
    }

    @Test
    public void testReadsUpToTheBudgetAndSkipsMissingFiles(@TempDir Path dir) throws Exception {
        Path a = dir.resolve("a.jar");
        Path b = dir.resolve("b.jar");
        Files.write(a, new byte[3 * 1024 * 1024]);
        Files.write(b, new byte[2 * 1024 * 1024]);
        List<Path> files = Arrays.asList(dir.resolve("missing.jar"), a, b);

        assertEquals(5 * MB, GameFileWarmer.readAll(files, 100 * MB, () -> false));
        assertEquals(3 * MB, GameFileWarmer.readAll(files, 3 * MB, () -> false), "stops at the budget");
    }

    @Test
    public void testStopsAsSoonAsPlayIsPressed(@TempDir Path dir) throws Exception {
        Path a = dir.resolve("a.jar");
        Files.write(a, new byte[8 * 1024 * 1024]);
        AtomicInteger checks = new AtomicInteger();
        // "Play" after the second check: no more than a chunk or two is read
        long read = GameFileWarmer.readAll(Arrays.asList(a, a), 100 * MB, () -> checks.incrementAndGet() > 2);
        assertEquals(2 * MB, read);
    }

    @Test
    public void testLanguageFromOptions(@TempDir Path dir) throws Exception {
        assertEquals("en_us", GameFileWarmer.languageOf(dir));
        Files.write(dir.resolve("options.txt"), "version:4671\nlang:de_de\n".getBytes(StandardCharsets.UTF_8));
        assertEquals("de_de", GameFileWarmer.languageOf(dir));
    }
}
