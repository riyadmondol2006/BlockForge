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

import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;

import org.junit.jupiter.api.Test;

import com.blockforge.launcher.constants.Constants;

public class JvmArgumentsTest {
    @Test
    public void testSplit() {
        assertEquals(Arrays.asList("-Xss2M", "-Dfoo=bar"), JvmArguments.split("  -Xss2M   -Dfoo=bar "));
        assertEquals(Arrays.asList("-Dname=two words", "-Da=b"), JvmArguments.split("-Dname=\"two words\" -Da=b"));
        assertEquals(Collections.singletonList("-Dpath=C:\\Users\\O'Brien\\x"),
            JvmArguments.split("-Dpath=C:\\Users\\O'Brien\\x"));
        assertEquals(Collections.emptyList(), JvmArguments.split(""));
        assertEquals(Collections.emptyList(), JvmArguments.split(null));
        assertEquals(Collections.singletonList(""), JvmArguments.split("\"\""));
    }

    @Test
    public void testIsCustom() {
        assertFalse(JvmArguments.isCustom(null));
        assertFalse(JvmArguments.isCustom("   "));
        assertFalse(JvmArguments.isCustom(Constants.DEFAULT_JAVA_PARAMETERS));
        assertFalse(JvmArguments.isCustom("  " + Constants.DEFAULT_JAVA_PARAMETERS.replace(" ", "  ")));
        assertTrue(JvmArguments.isCustom("-XX:+UseZGC"));
        assertTrue(JvmArguments.isCustom(Constants.DEFAULT_JAVA_PARAMETERS + " -Dfoo=bar"));
    }

    @Test
    public void testThatPerformanceFlagsAreTheWellKnownG1Set() {
        List<String> flags = JvmArguments.performanceFlags(4096, 8);

        assertEquals(Arrays.asList("-XX:+UnlockExperimentalVMOptions", "-XX:+UseG1GC", "-XX:G1NewSizePercent=20",
            "-XX:G1ReservePercent=20", "-XX:MaxGCPauseMillis=50", "-XX:G1HeapRegionSize=32M"), flags);
        assertFalse(flags.stream().anyMatch(f -> f.startsWith("-Xms")), "never a fixed starting heap");
        assertFalse(flags.stream().anyMatch(f -> f.contains("ParallelGCThreads")), "Java sizes threads itself");
    }

    @Test
    public void testThatSingleProcessorComputersAreLeftToJava() {
        assertEquals(Collections.emptyList(), JvmArguments.performanceFlags(1024, 1));
    }

    @Test
    public void testThatSmallHeapsGetSmallerRegions() {
        assertEquals(32, JvmArguments.g1RegionSizeMb(8192));
        assertEquals(32, JvmArguments.g1RegionSizeMb(2048));
        assertEquals(16, JvmArguments.g1RegionSizeMb(1024));
        assertEquals(16, JvmArguments.g1RegionSizeMb(1536));
        assertEquals(8, JvmArguments.g1RegionSizeMb(512));
        assertEquals(0, JvmArguments.g1RegionSizeMb(32));
        assertEquals(0, JvmArguments.g1RegionSizeMb(0));
        assertTrue(JvmArguments.performanceFlags(1024, 2).contains("-XX:G1HeapRegionSize=16M"));
    }

    @Test
    public void testThatAddMissingNeverDoublesUpOrMixesCollectors() {
        List<String> arguments = new ArrayList<>(Arrays.asList("java", "-XX:+UseZGC", "-XX:MaxGCPauseMillis=10"));

        JvmArguments.addMissing(arguments, JvmArguments.performanceFlags(4096, 4));

        assertFalse(arguments.contains("-XX:+UseG1GC"));
        assertFalse(arguments.contains("-XX:MaxGCPauseMillis=50"));
        assertTrue(arguments.contains("-XX:MaxGCPauseMillis=10"));
        assertTrue(arguments.contains("-XX:G1ReservePercent=20"));
        assertEquals(1, arguments.stream().filter(a -> a.equals("-XX:+UnlockExperimentalVMOptions")).count());
    }

    @Test
    public void testOptionName() {
        assertEquals("-XX:UseG1GC", JvmArguments.optionName("-XX:+UseG1GC"));
        assertEquals("-XX:UseG1GC", JvmArguments.optionName("-XX:-UseG1GC"));
        assertEquals("-XX:G1ReservePercent", JvmArguments.optionName("-XX:G1ReservePercent=20"));
        assertEquals("-Dfoo", JvmArguments.optionName("-Dfoo=bar"));
    }

    @Test
    public void testNeedsArgumentFile() {
        List<String> shortCommand = Arrays.asList("javaw", "-cp", "a.jar;b.jar", "net.minecraft.client.main.Main");
        List<String> longCommand = new ArrayList<>(shortCommand);
        StringBuilder classpath = new StringBuilder();
        for (int i = 0; i < 600; i++) {
            classpath.append("C:\\Users\\Some Body\\AppData\\Roaming\\BlockForge\\libraries\\lib").append(i)
                .append(".jar;");
        }
        longCommand.set(2, classpath.toString());

        assertFalse(JvmArguments.needsArgumentFile(shortCommand, true, 17));
        assertTrue(JvmArguments.needsArgumentFile(longCommand, true, 17));
        assertFalse(JvmArguments.needsArgumentFile(longCommand, true, 8), "Java 8 has no argument files");
        assertFalse(JvmArguments.needsArgumentFile(longCommand, false, 17), "only Windows has the limit");
    }

    @Test
    public void testToArgumentFile() {
        assertEquals("\"-cp\"\n\"C:\\\\Program Files\\\\a.jar\"\n\"-Dq=say \\\"hi\\\"\"\n",
            JvmArguments.toArgumentFile(Arrays.asList("-cp", "C:\\Program Files\\a.jar", "-Dq=say \"hi\"")));
    }
}
