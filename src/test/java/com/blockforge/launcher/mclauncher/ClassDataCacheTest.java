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
import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import com.blockforge.launcher.mclauncher.ClassDataCache.Action;
import com.blockforge.launcher.mclauncher.ClassDataCache.State;

public class ClassDataCacheTest {
    static final Path ARCHIVE = Paths.get("/cache/class-cache/Vanilla-25.jsa");
    static final Path LIST = Paths.get("/cache/class-cache/Vanilla-25.classlist");
    static final List<String> NONE = Collections.emptyList();

    static State state(String fingerprint, String status) {
        State s = new State();
        s.fingerprint = fingerprint;
        s.status = status;
        return s;
    }

    @Test
    public void testOnlyJava17AndNewer() {
        for (int java : new int[] { -1, 0, 8, 11, 12, 16 }) {
            assertEquals(Action.NONE, ClassDataCache.decide(true, java, NONE, null, "f", -1), "Java " + java);
        }
        for (int java : new int[] { 17, 21, 25 }) {
            assertEquals(Action.RECORD, ClassDataCache.decide(true, java, NONE, null, "f", -1), "Java " + java);
        }
    }

    @Test
    public void testTheArchiveIsUsedOnlyWhenEverythingMatches() {
        assertEquals(Action.USE, ClassDataCache.decide(true, 25, NONE, state("f", "ready"), "f", 100));
        // another Java, other libraries or other module options: record again
        assertEquals(Action.RECORD, ClassDataCache.decide(true, 25, NONE, state("old", "ready"), "f", 100));
        // the archive was deleted (Tools, or by hand)
        assertEquals(Action.RECORD, ClassDataCache.decide(true, 25, NONE, state("f", "ready"), "f", -1));
        // cut short or replaced since it was made: Java would skip it, so make it again
        State made = state("f", "ready");
        made.archiveBytes = 100;
        assertEquals(Action.USE, ClassDataCache.decide(true, 25, NONE, made, "f", 100));
        assertEquals(Action.RECORD, ClassDataCache.decide(true, 25, NONE, made, "f", 5));
        // still waiting for the game to close, or the game crashed before the list was long enough
        assertEquals(Action.RECORD, ClassDataCache.decide(true, 25, NONE, state("f", "recording"), "f", -1));
        // it couldn't be made for this combination: don't try at every start
        assertEquals(Action.NONE, ClassDataCache.decide(true, 25, NONE, state("f", "failed"), "f", -1));
        assertEquals(Action.RECORD, ClassDataCache.decide(true, 25, NONE, state("old", "failed"), "f", -1),
            "but do try again after an update");
    }

    @Test
    public void testTheSettingTurnsItOff() {
        assertEquals(Action.NONE, ClassDataCache.decide(false, 25, NONE, state("f", "ready"), "f", 100));
    }

    @Test
    public void testThePersonsOwnSharingArgumentsWin() {
        String[] own = { "-Xshare:off", "-Xshare:auto", "-XX:SharedArchiveFile=/my.jsa",
            "-XX:ArchiveClassesAtExit=/my.jsa", "-XX:+AutoCreateSharedArchive", "-XX:AOTCache=/app.aot",
            "-XX:DumpLoadedClassList=/x" };
        for (String argument : own) {
            assertEquals(Action.NONE,
                ClassDataCache.decide(true, 21, Arrays.asList("-XX:+UseZGC", argument), null, "f", -1), argument);
        }
        // other arguments of their own don't matter
        assertEquals(Action.RECORD,
            ClassDataCache.decide(true, 21, Arrays.asList("-XX:+UseZGC", "-Dfoo=bar"), null, "f", -1));
        assertFalse(ClassDataCache.userControlsSharing(null));
    }

    @Test
    public void testFlags() {
        assertEquals(Arrays.asList("-XX:SharedArchiveFile=" + ARCHIVE.toAbsolutePath(), "-XX:+VerifySharedSpaces",
            "-Xlog:cds=off"),
            ClassDataCache.flags(Action.USE, ARCHIVE, LIST));
        assertEquals(Arrays.asList("-XX:DumpLoadedClassList=" + LIST.toAbsolutePath()),
            ClassDataCache.flags(Action.RECORD, ARCHIVE, LIST));
        assertTrue(ClassDataCache.flags(Action.NONE, ARCHIVE, LIST).isEmpty());
    }

    @Test
    public void testTheOptionsTheArchiveDependsOn() {
        List<String> javaOptions = Arrays.asList("-Xmx2560M", "-XX:+UseG1GC", "--enable-native-access=ALL-UNNAMED",
            "--add-exports", "java.base/jdk.internal.misc=ALL-UNNAMED", "-Djava.library.path=/tmp/natives-1",
            "-cp", "/libs/a.jar:/libs/b.jar");
        List<String> options = ClassDataCache.archiveOptions(javaOptions);
        assertEquals(Arrays.asList("--enable-native-access=ALL-UNNAMED", "--add-exports",
            "java.base/jdk.internal.misc=ALL-UNNAMED", "-cp", "/libs/a.jar:/libs/b.jar"), options);

        // memory and the natives folder (new every start) don't change the archive; libraries and Java do
        String f = ClassDataCache.fingerprint("/java/bin/java", "JAVA_VERSION=25", options);
        assertEquals(f, ClassDataCache.fingerprint("/java/bin/java", "JAVA_VERSION=25",
            ClassDataCache.archiveOptions(Arrays.asList("-Xmx4G", "--enable-native-access=ALL-UNNAMED",
                "--add-exports", "java.base/jdk.internal.misc=ALL-UNNAMED", "-Djava.library.path=/tmp/natives-2",
                "-cp", "/libs/a.jar:/libs/b.jar"))));
        assertNotEquals(f, ClassDataCache.fingerprint("/java/bin/java", "JAVA_VERSION=25.0.2", options));
        assertNotEquals(f, ClassDataCache.fingerprint("/java/bin/java", "JAVA_VERSION=25",
            ClassDataCache.archiveOptions(Arrays.asList("-cp", "/libs/a.jar:/libs/c.jar"))));
    }

    @Test
    public void testAReDownloadedLibraryChangesTheStamp(@TempDir Path dir) throws Exception {
        Path jar = dir.resolve("lib.jar");
        Files.write(jar, new byte[100]);
        List<String> options = Arrays.asList("-cp", jar.toString());
        String before = ClassDataCache.classPathStamp(options);
        Files.setLastModifiedTime(jar, java.nio.file.attribute.FileTime.fromMillis(1_000_000L));
        assertNotEquals(before, ClassDataCache.classPathStamp(options));
        assertEquals("", ClassDataCache.classPathStamp(Arrays.asList("--add-exports", "x")));
    }

    @Test
    public void testTheClassListLeavesOutLambdas() {
        List<String> lines = Arrays.asList("# NOTE", "java/lang/Object id: 1",
            "@lambda-proxy java/lang/Thread run ()Ljava/lang/Runnable;", "@lambda-form-invoker [LF_RESOLVE] x",
            "com/google/common/base/Preconditions id: 2 super: 1 source: /libs/guava.jar");
        assertEquals(Arrays.asList("# NOTE", "java/lang/Object id: 1",
            "com/google/common/base/Preconditions id: 2 super: 1 source: /libs/guava.jar"),
            ClassDataCache.usableClassList(lines));
        assertEquals(2, ClassDataCache.countClasses(lines));
    }

    @Test
    public void testFileNames() {
        assertEquals("Fabric12111-21.jsa", ClassDataCache.fileName("Fabric12111", 21));
        assertEquals("instance-25.jsa", ClassDataCache.fileName("", 25));
        assertEquals("ab-21.jsa", ClassDataCache.fileName("a/b", 21));
    }

    @Test
    public void testTheStateIsSavedAndRead(@TempDir Path dir) {
        State s = state("abc", "ready");
        s.java = "/java/bin/java";
        s.javaMajor = 25;
        s.options = Arrays.asList("-cp", "/a.jar");
        Path file = dir.resolve("Vanilla-25.json");
        ClassDataCache.writeState(file, s);
        State back = ClassDataCache.readState(file);
        assertEquals("abc", back.fingerprint);
        assertEquals("ready", back.status);
        assertEquals(25, back.javaMajor);
        assertEquals(s.options, back.options);
    }

    @Test
    public void testDeletingOneInstanceLeavesTheOthers(@TempDir Path dir) throws Exception {
        for (String name : new String[] { "Pack-21.jsa", "Pack-25.jsa", "Pack-25.json", "Pack-2-21.jsa",
            "Other-21.jsa", "notes.txt" }) {
            Files.write(dir.resolve(name), new byte[10]);
        }
        // Java writes its archives read-only
        assertTrue(dir.resolve("Pack-21.jsa").toFile().setReadOnly());

        assertEquals(30, ClassDataCache.deleteFor(dir, "Pack"));
        assertFalse(Files.exists(dir.resolve("Pack-21.jsa")));
        assertFalse(Files.exists(dir.resolve("Pack-25.jsa")));
        assertFalse(Files.exists(dir.resolve("Pack-25.json")));
        assertTrue(Files.exists(dir.resolve("Pack-2-21.jsa")), "another instance whose name starts the same");
        assertTrue(Files.exists(dir.resolve("Other-21.jsa")));

        assertEquals(20, ClassDataCache.deleteMatching(dir, null));
        assertTrue(Files.exists(dir.resolve("notes.txt")), "only the cache's own files are deleted");
    }
}
