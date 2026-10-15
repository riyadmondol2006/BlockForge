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
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.Arrays;
import java.util.Collections;

import org.junit.jupiter.api.Test;

import com.blockforge.launcher.content.LoaderSupport.ShaderSetup;
import com.blockforge.launcher.data.minecraft.loaders.LoaderVersion;

public class LoaderSupportTest {
    @Test
    public void readsTheLoaderOfAnInstance() {
        assertEquals(LoaderSupport.FABRIC, LoaderSupport.key(new LoaderVersion("0.16", false, "Fabric")));
        assertEquals(LoaderSupport.QUILT, LoaderSupport.key(new LoaderVersion("0.26", false, "Quilt")));
        assertEquals(LoaderSupport.NEOFORGE, LoaderSupport.key(new LoaderVersion("21.1", false, "NeoForge")));
        assertEquals(LoaderSupport.FORGE, LoaderSupport.key(new LoaderVersion("47.3", false, "Forge")));
        assertNull(LoaderSupport.key((LoaderVersion) null));
        assertNull(LoaderSupport.key(new LoaderVersion("1", false, "Paper")));
    }

    @Test
    public void quiltRunsFabricModsAndNeoForgeSometimesRunsForgeMods() {
        assertEquals(Arrays.asList("quilt", "fabric"), LoaderSupport.acceptedLoaders("quilt", "26.3", null));
        assertEquals(Collections.singletonList("fabric"), LoaderSupport.acceptedLoaders("fabric", "26.3", null));
        assertEquals(Arrays.asList("neoforge", "forge"),
            LoaderSupport.acceptedLoaders("neoforge", "1.20.1", Collections.singletonList("1.20.1")));
        assertEquals(Collections.singletonList("neoforge"),
            LoaderSupport.acceptedLoaders("neoforge", "1.21.1", Collections.singletonList("1.20.1")));
        assertTrue(LoaderSupport.acceptedLoaders(null, "26.3", null).isEmpty());
    }

    @Test
    public void findsShaderLoadersByProjectOrFileName() {
        assertTrue(LoaderSupport.hasShaderLoader(Collections.singletonList(LoaderSupport.IRIS), null));
        assertTrue(LoaderSupport.hasShaderLoader(null, Collections.singletonList("iris-fabric-1.8.0.jar")));
        assertTrue(LoaderSupport.hasShaderLoader(null, Collections.singletonList("OptiFine_1.20.1_HD_U_I6.jar")));
        assertTrue(LoaderSupport.hasShaderLoader(null, Collections.singletonList("oculus-mc1.20.1-1.7.0.jar")));
        assertFalse(LoaderSupport.hasShaderLoader(Collections.singletonList(LoaderSupport.SODIUM),
            Collections.singletonList("sodium-fabric-0.6.jar")));
    }

    @Test
    public void saysWhatShadersNeedPerLoader() {
        assertEquals(ShaderSetup.READY, LoaderSupport.shaderSetup(LoaderSupport.FABRIC, true));
        assertEquals(ShaderSetup.ADD_IRIS, LoaderSupport.shaderSetup(LoaderSupport.FABRIC, false));
        assertEquals(ShaderSetup.ADD_IRIS, LoaderSupport.shaderSetup(LoaderSupport.QUILT, false));
        assertEquals(ShaderSetup.ADD_IRIS, LoaderSupport.shaderSetup(LoaderSupport.NEOFORGE, false));
        assertEquals(ShaderSetup.ADD_OCULUS, LoaderSupport.shaderSetup(LoaderSupport.FORGE, false));
        assertEquals(ShaderSetup.NEEDS_MOD_LOADER, LoaderSupport.shaderSetup(null, false));
        assertEquals(ShaderSetup.NEEDS_MOD_LOADER, LoaderSupport.shaderSetup(LoaderSupport.LEGACY_FABRIC, false));
    }
}
