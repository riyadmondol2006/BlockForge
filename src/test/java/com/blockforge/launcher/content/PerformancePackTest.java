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
package com.blockforge.launcher.content;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.Arrays;
import java.util.Collections;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

import org.junit.jupiter.api.Test;

import com.blockforge.launcher.data.minecraft.loaders.LoaderType;

public class PerformancePackTest {
    private static final Set<String> EVERYTHING = new HashSet<>(Arrays.asList(PerformancePack.SODIUM,
        PerformancePack.EMBEDDIUM, PerformancePack.LITHIUM, PerformancePack.FERRITE_CORE, PerformancePack.MODERN_FIX));

    @Test
    public void fabricAndQuiltGetTheFullPack() {
        List<String> expected = Arrays.asList(PerformancePack.SODIUM, PerformancePack.LITHIUM,
            PerformancePack.FERRITE_CORE, PerformancePack.MODERN_FIX);
        assertEquals(expected, PerformancePack.choose(PerformancePack.slotsFor(LoaderSupport.FABRIC),
            EVERYTHING::contains, Collections.<String>emptySet()));
        assertEquals(expected, PerformancePack.choose(PerformancePack.slotsFor(LoaderSupport.QUILT),
            EVERYTHING::contains, Collections.<String>emptySet()));
        assertEquals("Sodium, Lithium, FerriteCore, ModernFix", PerformancePack.describe(LoaderSupport.FABRIC));
    }

    @Test
    public void modsWithoutAVersionForTheGameAreSkipped() {
        // ModernFix often takes a while to support a brand new Minecraft version
        Set<String> available = new HashSet<>(EVERYTHING);
        available.remove(PerformancePack.MODERN_FIX);
        assertEquals(Arrays.asList(PerformancePack.SODIUM, PerformancePack.LITHIUM, PerformancePack.FERRITE_CORE),
            PerformancePack.choose(PerformancePack.slotsFor(LoaderSupport.FABRIC), available::contains, null));
    }

    @Test
    public void neoForgeUsesEmbeddiumWhenSodiumIsntThere() {
        Set<String> available = new HashSet<>(EVERYTHING);
        assertEquals(PerformancePack.SODIUM, PerformancePack.choose(PerformancePack.slotsFor(LoaderSupport.NEOFORGE),
            available::contains, null).get(0));

        available.remove(PerformancePack.SODIUM);
        available.remove(PerformancePack.LITHIUM);
        assertEquals(Arrays.asList(PerformancePack.EMBEDDIUM, PerformancePack.FERRITE_CORE, PerformancePack.MODERN_FIX),
            PerformancePack.choose(PerformancePack.slotsFor(LoaderSupport.NEOFORGE), available::contains, null));
    }

    @Test
    public void forgeGetsEmbeddiumFerriteCoreAndModernFix() {
        assertEquals(Arrays.asList(PerformancePack.EMBEDDIUM, PerformancePack.FERRITE_CORE, PerformancePack.MODERN_FIX),
            PerformancePack.choose(PerformancePack.slotsFor(LoaderSupport.FORGE), EVERYTHING::contains, null));
    }

    @Test
    public void jobsAlreadyDoneAreSkippedAndRenderersAreNeverDoubled() {
        Set<String> installed = new HashSet<>(Arrays.asList(PerformancePack.EMBEDDIUM, PerformancePack.LITHIUM));
        // Embeddium already renders, so Sodium must not be added next to it
        assertEquals(Arrays.asList(PerformancePack.FERRITE_CORE, PerformancePack.MODERN_FIX), PerformancePack
            .choose(PerformancePack.slotsFor(LoaderSupport.NEOFORGE), EVERYTHING::contains, installed));
    }

    @Test
    public void onlyOfferedForModLoaders() {
        assertTrue(PerformancePack.offeredFor(LoaderSupport.key(LoaderType.FABRIC)));
        assertTrue(PerformancePack.offeredFor(LoaderSupport.key(LoaderType.QUILT)));
        assertTrue(PerformancePack.offeredFor(LoaderSupport.key(LoaderType.NEOFORGE)));
        assertTrue(PerformancePack.offeredFor(LoaderSupport.key(LoaderType.FORGE)));
        assertFalse(PerformancePack.offeredFor(LoaderSupport.key((LoaderType) null)));
        assertFalse(PerformancePack.offeredFor(LoaderSupport.key(LoaderType.PAPER)));
        assertFalse(PerformancePack.offeredFor(LoaderSupport.LEGACY_FABRIC));
        assertTrue(PerformancePack.choose(PerformancePack.slotsFor(null), EVERYTHING::contains, null).isEmpty());
    }
}
