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
package com.blockforge.launcher.runtime;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

class RuntimeResolverTest {
    private static JavaRuntime java(String home, int major, String arch) {
        return new JavaRuntime(Paths.get(home), major, arch, "test");
    }

    @Test
    void readsTheReleaseFile(@TempDir Path temp) throws IOException {
        Files.write(temp.resolve("release"),
            "JAVA_VERSION=\"17.0.9\"\nOS_ARCH=\"aarch64\"\nIMPLEMENTOR=\"Eclipse Adoptium\"\n".getBytes(StandardCharsets.UTF_8));

        JavaRuntime r = JavaRuntime.fromHome(temp, "test");
        assertEquals(17, r.major);
        assertEquals(JavaRuntime.ARM64, r.arch);
    }

    @Test
    void parsesVersionsAndArchitectures() {
        assertEquals(8, JavaRuntime.parseMajor("1.8.0_392"));
        assertEquals(21, JavaRuntime.parseMajor("21.0.1"));
        assertEquals(-1, JavaRuntime.parseMajor(null));
        assertEquals(JavaRuntime.X64, JavaRuntime.normaliseArch("amd64"));
        assertEquals(JavaRuntime.ARM64, JavaRuntime.normaliseArch("arm64"));
        assertEquals(JavaRuntime.X86, JavaRuntime.normaliseArch("i386"));
        assertNull(JavaRuntime.normaliseArch(null));
        assertEquals(-1, JavaRuntime.fromHome(Paths.get("/does/not/exist"), "x").major);
    }

    @Test
    void usersChoiceWinsButIsChecked() {
        RuntimeResolver.Request req = new RuntimeResolver.Request();
        req.userChoice = java("/jdk8", 8, "x64");
        req.minecraftRuntime = java("/mc17", 17, "x64");
        req.requiredMajor = 17;
        req.hostArch = "x64";

        RuntimeResolver.Choice c = RuntimeResolver.resolve(req);
        assertEquals("/jdk8", c.java.home.toString().replace('\\', '/'));
        assertTrue(c.warnings.get(0).contains("needs Java 17"));
    }

    @Test
    void minecraftsOwnJavaIsPreferredWhenNothingIsChosen() {
        RuntimeResolver.Request req = new RuntimeResolver.Request();
        req.minecraftRuntime = java("/mc17", 17, "arm64");
        req.installed.add(java("/jdk21", 21, "arm64"));
        req.requiredMajor = 17;
        req.hostArch = "arm64";

        RuntimeResolver.Choice c = RuntimeResolver.resolve(req);
        assertEquals(17, c.java.major);
        assertTrue(c.warnings.isEmpty());
        assertTrue(c.explain().contains("why:"));
    }

    @Test
    void picksTheClosestInstalledJavaThatMeetsTheRequirement() {
        RuntimeResolver.Request req = new RuntimeResolver.Request();
        req.installed.add(java("/jdk8", 8, "x64"));
        req.installed.add(java("/jdk21", 21, "x64"));
        req.installed.add(java("/jdk17", 17, "x64"));
        req.requiredMajor = 17;
        req.hostArch = "x64";

        assertEquals(17, RuntimeResolver.resolve(req).java.major);
    }

    @Test
    void prefersTheNativeProcessorOnArmComputers() {
        RuntimeResolver.Request req = new RuntimeResolver.Request();
        req.installed.add(java("/x64-17", 17, "x64"));
        req.installed.add(java("/arm-21", 21, "arm64"));
        req.requiredMajor = 17;
        req.hostArch = "arm64";

        RuntimeResolver.Choice c = RuntimeResolver.resolve(req);
        assertEquals(JavaRuntime.ARM64, c.java.arch);
        assertTrue(c.warnings.isEmpty());
    }

    @Test
    void warnsAboutEmulationAndFallsBackToTheLauncherJava() {
        RuntimeResolver.Request armWithIntel = new RuntimeResolver.Request();
        armWithIntel.userChoice = java("/x64-17", 17, "x64");
        armWithIntel.hostArch = "arm64";
        assertTrue(RuntimeResolver.resolve(armWithIntel).warnings.get(0).contains("emulation"));

        RuntimeResolver.Request nothing = new RuntimeResolver.Request();
        nothing.launcherJava = java("/launcher", 21, "x64");
        assertEquals("/launcher", RuntimeResolver.resolve(nothing).java.home.toString().replace('\\', '/'));

        RuntimeResolver.Choice none = RuntimeResolver.resolve(new RuntimeResolver.Request());
        assertNull(none.java);
        assertTrue(none.warnings.get(0).contains("no Java"));
    }
}
