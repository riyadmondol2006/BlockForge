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
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

import com.blockforge.launcher.mclauncher.AutoMemory.Game;
import com.blockforge.launcher.mclauncher.AutoMemory.Loader;
import com.blockforge.launcher.mclauncher.AutoMemory.Pc;
import com.blockforge.launcher.mclauncher.AutoMemory.Result;

public class AutoMemoryTest {
    // computers, as they report themselves (a "4 GB" laptop shows a little less, shared graphics memory takes some)
    static final Pc TWO_GB = new Pc(1950, 900, 2, false);
    static final Pc FOUR_GB = new Pc(3790, 2000, 4, false);
    static final Pc EIGHT_GB = new Pc(7850, 5000, 8, false);
    static final Pc EIGHT_GB_DUAL_CORE = new Pc(7850, 5000, 2, false);
    static final Pc SIXTEEN_GB = new Pc(15900, 11000, 12, false);
    static final Pc THIRTY_TWO_GB = new Pc(32000, 24000, 16, false);
    static final Pc EIGHT_GB_32_BIT_JAVA = new Pc(7850, 5000, 4, true);
    static final Pc UNKNOWN = new Pc(0, -1, 0, false);

    // games
    static final Game OLD_VANILLA = Game.vanilla("1.8.9");
    static final Game ANCIENT_VANILLA = Game.vanilla("1.6.4");
    static final Game VANILLA_1_16 = Game.vanilla("1.16.5");
    static final Game MODERN_VANILLA = Game.vanilla("1.21.11");
    static final Game YEAR_VANILLA = Game.vanilla("26.3");
    static final Game LIGHT_FABRIC = new Game("1.21.11", Loader.FABRIC, 6, false, 0, 0, 0);
    static final Game FABRIC_SHADERS = new Game("1.21.11", Loader.FABRIC, 6, true, 0, 0, 0);
    static final Game BIG_FABRIC = new Game("1.21.11", Loader.FABRIC, 150, false, 0, 0, 0);
    static final Game FORGE_200 = new Game("1.20.1", Loader.FORGE, 200, false, 0, 0, 0);
    static final Game NEOFORGE_350 = new Game("1.21.1", Loader.NEOFORGE, 350, false, 0, 0, 0);
    static final Game FORGE_1_12_150 = new Game("1.12.2", Loader.FORGE, 150, false, 0, 0, 0);
    static final Game PACK_ASKS_10_GB = new Game("1.20.1", Loader.FORGE, 300, false, 0, 10240, 0);
    static final Game PACK_NEEDS_6_GB = new Game("1.20.1", Loader.FORGE, 120, false, 0, 0, 6144);
    static final Game HIGH_RES_PACK = new Game("1.21.11", Loader.NONE, 0, false, 300, 0, 0);

    static int heap(Pc pc, Game game) {
        return AutoMemory.choose(pc, game).heapMb;
    }

    @Test
    public void testWhatEachGameWouldLike() {
        assertEquals(1024, AutoMemory.idealMb(ANCIENT_VANILLA));
        assertEquals(1536, AutoMemory.idealMb(OLD_VANILLA));
        assertEquals(1536, AutoMemory.idealMb(Game.vanilla("1.12.2")));
        assertEquals(2048, AutoMemory.idealMb(VANILLA_1_16));
        assertEquals(2560, AutoMemory.idealMb(MODERN_VANILLA));
        assertEquals(2560, AutoMemory.idealMb(YEAR_VANILLA));
        assertEquals(3072, AutoMemory.idealMb(LIGHT_FABRIC), "a light Fabric list stays around 3 GB");
        assertEquals(3584, AutoMemory.idealMb(FABRIC_SHADERS));
        assertEquals(4864, AutoMemory.idealMb(BIG_FABRIC));
        assertEquals(7424, AutoMemory.idealMb(FORGE_200));
        assertEquals(8192, AutoMemory.idealMb(NEOFORGE_350), "never past 8 GB on its own");
        assertEquals(4864, AutoMemory.idealMb(FORGE_1_12_150));
        assertEquals(10240, AutoMemory.idealMb(PACK_ASKS_10_GB), "the modpack's own recommendation wins");
        assertEquals(3584, AutoMemory.idealMb(HIGH_RES_PACK));
    }

    @Test
    public void testTheTableOfComputersAndGames() {
        // { computer, game, expected heap }
        Object[][] table = {
            { EIGHT_GB, MODERN_VANILLA, 2560 },
            { EIGHT_GB, LIGHT_FABRIC, 3072 },
            { EIGHT_GB, OLD_VANILLA, 1536 },
            { SIXTEEN_GB, MODERN_VANILLA, 2560 },
            { SIXTEEN_GB, FORGE_200, 7424 },
            { SIXTEEN_GB, BIG_FABRIC, 4864 },
            { THIRTY_TWO_GB, MODERN_VANILLA, 2560 },
            { THIRTY_TWO_GB, NEOFORGE_350, 8192 },
            { THIRTY_TWO_GB, PACK_ASKS_10_GB, 10240 },
            // 4 GB laptop: at most 2 GB, and only 2 GB free
            { FOUR_GB, MODERN_VANILLA, 1792 },
            { FOUR_GB, LIGHT_FABRIC, 2048 },
            { FOUR_GB, OLD_VANILLA, 1280 },
            // 8 GB with a big Forge pack: 5.75 GB can be spared, but only 4.9 GB is free, so it gets what it needs
            { EIGHT_GB, FORGE_200, 5376 },
            // two cores: smaller heap so collections stay short
            { EIGHT_GB_DUAL_CORE, FORGE_200, 4096 },
            // 32-bit Java can't use more than about 1.4 GB
            { EIGHT_GB_32_BIT_JAVA, FORGE_200, 1408 },
            { EIGHT_GB_32_BIT_JAVA, OLD_VANILLA, 1408 },
            // tiny computer: the least a modern game starts with
            { TWO_GB, MODERN_VANILLA, 1024 },
            { TWO_GB, OLD_VANILLA, 768 },
            // nothing known about the computer: modest
            { UNKNOWN, MODERN_VANILLA, 2048 },
            { UNKNOWN, OLD_VANILLA, 1536 },
        };
        for (Object[] row : table) {
            Pc pc = (Pc) row[0];
            Game game = (Game) row[1];
            assertEquals((int) (Integer) row[2], heap(pc, game),
                () -> pc.totalMb + " MB computer, " + AutoMemory.describeGame(game) + " " + game.minecraftVersion);
        }
    }

    @Test
    public void testNeverMoreThanTheComputerCanSpare() {
        Pc[] pcs = { TWO_GB, FOUR_GB, EIGHT_GB, EIGHT_GB_DUAL_CORE, SIXTEEN_GB, THIRTY_TWO_GB, EIGHT_GB_32_BIT_JAVA };
        Game[] games = { ANCIENT_VANILLA, OLD_VANILLA, VANILLA_1_16, MODERN_VANILLA, LIGHT_FABRIC, BIG_FABRIC,
            FORGE_200, NEOFORGE_350, FORGE_1_12_150, PACK_ASKS_10_GB, PACK_NEEDS_6_GB, HIGH_RES_PACK };
        for (Pc pc : pcs) {
            for (Game game : games) {
                Result r = AutoMemory.choose(pc, game);
                int startMinimum = AutoMemory.minorVersion(game.minecraftVersion) <= 12 ? 768 : 1024;
                assertTrue(r.heapMb <= Math.max(r.ceilingMb, startMinimum), pc.totalMb + " " + game.minecraftVersion);
                assertTrue(r.heapMb <= pc.totalMb - 768 || r.heapMb <= startMinimum);
                assertEquals(0, r.heapMb % 128, "a round number");
                if (pc.java32Bit) {
                    assertTrue(r.heapMb <= AutoMemory.JAVA_32_BIT_CAP_MB);
                }
            }
        }
    }

    @Test
    public void testTheSystemKeepsAtLeastOneAndAHalfGb() {
        assertEquals(2048, AutoMemory.ceilingMb(FOUR_GB));
        assertEquals(5888, AutoMemory.ceilingMb(EIGHT_GB));
        assertEquals(11776, AutoMemory.ceilingMb(SIXTEEN_GB));
        assertEquals(27904, AutoMemory.ceilingMb(THIRTY_TWO_GB), "big computers keep 4 GB back");
        assertEquals(Integer.MAX_VALUE, AutoMemory.ceilingMb(UNKNOWN));
    }

    @Test
    public void testLittleFreeMemoryLowersTheHeapButNotBelowWhatTheGameNeeds() {
        Pc busy = new Pc(15900, 2500, 8, false);
        Result r = AutoMemory.choose(busy, FORGE_200);
        assertTrue(r.limitedByFreeMemory);
        assertEquals(5376, r.heapMb, "three quarters of the 7.25 GB it would like");
        assertFalse(r.warnings.isEmpty());
        assertTrue(r.isShort());

        Result fine = AutoMemory.choose(new Pc(15900, 12000, 8, false), FORGE_200);
        assertFalse(fine.limitedByFreeMemory);
        assertTrue(fine.warnings.isEmpty());
        assertFalse(fine.isShort());
    }

    @Test
    public void testAPackThatNeedsMoreThanTheComputerHasIsFlagged() {
        Result r = AutoMemory.choose(FOUR_GB, PACK_NEEDS_6_GB);
        assertTrue(r.belowPackNeeds);
        assertTrue(r.isShort());
        assertTrue(r.warnings.get(r.warnings.size() - 1).contains("6144"));

        Result enough = AutoMemory.choose(THIRTY_TWO_GB, PACK_NEEDS_6_GB);
        assertFalse(enough.belowPackNeeds);
        assertTrue(enough.heapMb >= 6144);
    }

    @Test
    public void testMinorVersions() {
        assertEquals(8, AutoMemory.minorVersion("1.8.9"));
        assertEquals(12, AutoMemory.minorVersion("1.12.2"));
        assertEquals(21, AutoMemory.minorVersion("1.21.11"));
        assertEquals(100, AutoMemory.minorVersion("26.3"));
        assertEquals(100, AutoMemory.minorVersion("26.4-snapshot-4"));
        assertEquals(21, AutoMemory.minorVersion("24w14a"));
        assertEquals(12, AutoMemory.minorVersion("17w06a"));
        assertEquals(0, AutoMemory.minorVersion("b1.7.3"));
        assertEquals(0, AutoMemory.minorVersion("a1.0.4"));
        assertEquals(20, AutoMemory.minorVersion("1.20.5-pre1"));
        assertEquals(100, AutoMemory.minorVersion(null));
    }

    @Test
    public void testLoaderNames() {
        assertEquals(Loader.NONE, Loader.of(null));
        assertEquals(Loader.FABRIC, Loader.of("Fabric"));
        assertEquals(Loader.FABRIC, Loader.of("LegacyFabric"));
        assertEquals(Loader.QUILT, Loader.of("Quilt"));
        assertEquals(Loader.FORGE, Loader.of("Forge"));
        assertEquals(Loader.NEOFORGE, Loader.of("NeoForge"));
        assertEquals(Loader.OTHER, Loader.of("Paper"));
    }

    @Test
    public void testDescriptions() {
        assertEquals("3 GB", AutoMemory.describeMb(3072));
        assertEquals("2.5 GB", AutoMemory.describeMb(2560));
        assertEquals("1.75 GB", AutoMemory.describeMb(1792));
        assertEquals("5.25 GB", AutoMemory.describeMb(5376));
        assertEquals("768 MB", AutoMemory.describeMb(768));
        assertEquals("a modern vanilla game", AutoMemory.describeGame(MODERN_VANILLA));
        assertEquals("a modern Fabric game with 6 mods", AutoMemory.describeGame(LIGHT_FABRIC));
        assertEquals("an older Forge game with 150 mods", AutoMemory.describeGame(FORGE_1_12_150));
    }
}
