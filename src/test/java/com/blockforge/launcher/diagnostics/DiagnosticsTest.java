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
package com.blockforge.launcher.diagnostics;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

class DiagnosticsTest {
    @Test
    void redactsTokensAndIdentity() {
        String text = "Launching with --accessToken abc123SECRET --uuid 11111111-2222-3333-4444-555555555555 "
            + "--username Steve123 and mail steve@example.com from 203.0.113.7 \"access_token\": \"zzzTOKENzzz\" "
            + "Authorization: Bearer abcdefghijkl.mnop";

        String out = Redactor.scrub(text, Arrays.asList("Steve123"));

        assertFalse(out.contains("abc123SECRET"));
        assertFalse(out.contains("Steve123"));
        assertFalse(out.contains("steve@example.com"));
        assertFalse(out.contains("203.0.113.7"));
        assertFalse(out.contains("zzzTOKENzzz"));
        assertFalse(out.contains("11111111-2222-3333-4444-555555555555"));
        assertTrue(out.contains("--accessToken <hidden>"));
    }

    @Test
    void keepsUsefulNumbersAndLocalAddresses() {
        String out = Redactor.scrub("Using sodium-0.5.8.1 on 127.0.0.1 with Minecraft 1.20.1");
        assertTrue(out.contains("sodium-0.5.8.1"));
        assertTrue(out.contains("127.0.0.1"));
        assertTrue(out.contains("1.20.1"));
    }

    @Test
    void replacesTheHomeFolder() {
        String home = System.getProperty("user.home");
        String out = Redactor.scrub("Loading from " + home + "/instances/pack/mods");
        assertFalse(out.contains(home));
        assertTrue(out.contains("~/instances/pack/mods"));
    }

    @Test
    void recognisesWrongJava() {
        List<Diagnosis> d = CrashClassifier.classify(
            "java.lang.UnsupportedClassVersionError: net/minecraft/client/main/Main has been compiled by a more recent "
                + "version of the Java Runtime (class file version 65.0), this version only recognizes up to 61.0");
        assertEquals("MC-JAVA-VERSION", d.get(0).code);
        assertTrue(d.get(0).explanation.contains("Java 21"));
    }

    @Test
    void recognisesMemoryAndArguments() {
        assertEquals("MC-OUT-OF-MEMORY",
            CrashClassifier.classify("Exception in thread \"main\" java.lang.OutOfMemoryError: Java heap space").get(0).code);
        assertEquals("MC-JVM-ARGUMENT",
            CrashClassifier.classify("Unrecognized VM option 'UseConcMarkSweepGC'\nError: Could not create the Java Virtual Machine.").get(0).code);
    }

    @Test
    void recognisesModProblems() {
        assertEquals("MC-MISSING-DEPENDENCY", CrashClassifier
            .classify("Mod 'Create' (create) requires version 0.5 or later of mod 'flywheel'").get(0).code);
        assertEquals("MC-DUPLICATE-MOD", CrashClassifier.classify("Found duplicate mods: sodium").get(0).code);
        assertEquals("MC-MIXIN-CONFLICT",
            CrashClassifier.classify("org.spongepowered.asm.mixin.transformer.throwables.MixinApplyError: Mixin apply failed").get(0).code);
    }

    @Test
    void saysNothingForAHealthyLog() {
        assertTrue(CrashClassifier.classify("[Render thread/INFO]: Setting user: Player\n[Render thread/INFO]: Stopping!").isEmpty());
        assertTrue(CrashClassifier.classify("").isEmpty());
    }

    @Test
    void bundleContainsScrubbedFilesAndTheDiagnosis(@TempDir Path temp) throws IOException {
        Path instance = temp.resolve("instance");
        Files.createDirectories(instance.resolve("logs"));
        Files.createDirectories(instance.resolve("crash-reports"));
        Files.createDirectories(instance.resolve("mods"));
        Files.write(instance.resolve("logs/latest.log"),
            "Setting user: Steve123\njava.lang.OutOfMemoryError: Java heap space\n".getBytes(StandardCharsets.UTF_8));
        Files.write(instance.resolve("crash-reports/crash-1.txt"), "crash for Steve123".getBytes(StandardCharsets.UTF_8));
        Files.write(instance.resolve("mods/a.jar"), new byte[10]);
        Files.write(instance.resolve("instance.json"), "{\"token\": \"SECRETVALUE\"}".getBytes(StandardCharsets.UTF_8));
        Path log = temp.resolve("launcher.log");
        Files.write(log, "launcher says hi Steve123".getBytes(StandardCharsets.UTF_8));

        SupportBundle.Input input = new SupportBundle.Input();
        input.launcherLog = log;
        input.instanceRoot = instance;
        input.instanceName = "Test";
        input.facts.add("Java: 21");
        input.secrets.add("Steve123");

        Path zip = temp.resolve("out/bundle.zip");
        List<Diagnosis> diagnoses = SupportBundle.create(zip, input);

        assertEquals("MC-OUT-OF-MEMORY", diagnoses.get(0).code);

        Map<String, String> files = new HashMap<>();
        try (ZipFile z = new ZipFile(zip.toFile())) {
            for (ZipEntry e : java.util.Collections.list(z.entries())) {
                files.put(e.getName(), new String(z.getInputStream(e).readAllBytes(), StandardCharsets.UTF_8));
            }
        }

        assertTrue(files.keySet().containsAll(Arrays.asList("summary.txt", "launcher.log", "latest.log",
            "crash-report.txt", "instance.json", "mods.txt")));
        files.values().forEach(content -> {
            assertFalse(content.contains("Steve123"));
            assertFalse(content.contains("SECRETVALUE"));
        });
        assertTrue(files.get("summary.txt").contains("MC-OUT-OF-MEMORY"));
        assertTrue(files.get("mods.txt").contains("a.jar"));
    }
}
