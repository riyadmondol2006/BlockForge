/*
 * ATLauncher - https://github.com/ATLauncher/ATLauncher
 * Copyright (C) 2013-2022 ATLauncher
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
import static org.junit.jupiter.api.Assertions.assertNull;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

public class JavaTest {
    @Test
    public void testThatJavaVersionNumberIsReturnedCorrectly() {
        assertEquals(7, Java.parseJavaVersionNumber("1.7.0_64"));
        assertEquals(8, Java.parseJavaVersionNumber("1.8.0_212"));
        assertEquals(9, Java.parseJavaVersionNumber("9.0.4"));
        assertEquals(10, Java.parseJavaVersionNumber("10.0.2"));
        assertEquals(11, Java.parseJavaVersionNumber("11.0.3"));
        assertEquals(12, Java.parseJavaVersionNumber("12.0.1"));
        assertEquals(13, Java.parseJavaVersionNumber("13-ea"));
    }

    @Test
    public void testThatJavaBuildNumberIsReturnedCorrectly() {
        assertEquals(64, Java.parseJavaBuildVersion("1.7.0_64"));
        assertEquals(212, Java.parseJavaBuildVersion("1.8.0_212"));
        assertEquals(4, Java.parseJavaBuildVersion("9.0.4"));
        assertEquals(2, Java.parseJavaBuildVersion("10.0.2"));
        assertEquals(3, Java.parseJavaBuildVersion("11.0.3"));
        assertEquals(1, Java.parseJavaBuildVersion("12.0.1"));
        assertEquals(0, Java.parseJavaBuildVersion("13-ea"));
    }

    @Test
    public void testVersionFromReleaseFile(@TempDir Path home) throws Exception {
        assertNull(Java.versionFromReleaseFile(home));

        Files.write(home.resolve("release"),
            "IMPLEMENTOR=\"Eclipse Adoptium\"\r\nJAVA_VERSION=\"17.0.15\"\r\nOS_ARCH=\"x86_64\"\r\n"
                .getBytes(StandardCharsets.ISO_8859_1));
        assertEquals("17.0.15", Java.versionFromReleaseFile(home));
        assertEquals(17, Java.parseJavaVersionNumber(Java.versionFromReleaseFile(home)));

        Files.write(home.resolve("release"), "JAVA_VERSION=\"1.8.0_202\"\nOS_NAME=\"Linux\"\n"
            .getBytes(StandardCharsets.ISO_8859_1));
        assertEquals("1.8.0_202", Java.versionFromReleaseFile(home));
        assertEquals("1.8.0_202", Java.getVersionForJavaPath(home.toFile()), "no need to start Java");

        Files.write(home.resolve("release"), "JAVA_VERSION_DATE=\"2024-01-16\"\n"
            .getBytes(StandardCharsets.ISO_8859_1));
        assertNull(Java.versionFromReleaseFile(home));
    }

}
