package com.blockforge.lite.core;

import java.io.File;
import java.lang.instrument.ClassFileTransformer;
import java.util.Map;
import java.util.function.Function;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Opcodes;

/**
 * Base of a patch's self-test. Put each patch's test in its own class, `XxxTest extends PatchTest`, and add it to
 * {@link Tests#PATCH_TESTS}. Every method of it that takes no arguments and returns nothing (not private, not
 * static) is run as one test; the tests of the framework itself are in {@link Tests}.
 *
 * A test typically builds a stand-in of the target class (and of what it calls) with ASM ({@link #writer()}), loads
 * it through the agent ({@link #load}) into a fresh class loader (the JVM verifies the changed code there), runs it
 * through reflection, and checks the result with {@link #expect}. Also test at least one shape the patch must leave
 * alone, and that the report says so ({@link #reportPath}, {@link #line}).
 */
abstract class PatchTest implements Opcodes {
    /** Makes the agent's transformer (the real entry point, with its name filter) for some options. */
    Function<String, ClassFileTransformer> factory;
    /** A folder for report and list files; deleted afterwards. */
    File tmp;
    private Tests kit;

    void init(Tests kit, Function<String, ClassFileTransformer> factory, File tmp) {
        this.kit = kit;
        this.factory = factory;
        this.tmp = tmp;
    }

    /** One check: counted, and printed when it fails. */
    void expect(boolean ok, String what) {
        kit.expect(ok, what);
    }

    /** The classes, loaded through a transformer (null: as they are), like a mod loader would. */
    static Tests.Game load(ClassFileTransformer t, Map<String, byte[]> classes) throws Exception {
        return Tests.load(t, classes);
    }

    /** A fresh report file path (for report=). */
    String reportPath(String name) {
        return kit.reportPath(name);
    }

    /** The line of a report for a patch id ("" when missing). */
    static String line(String report, String id) throws Exception {
        return Tests.line(report, id);
    }

    static void write(File f, String text) throws Exception {
        Tests.write(f, text);
    }

    /** A ClassWriter that computes frames and sizes for stand-ins (type merges become Object). */
    static ClassWriter writer() {
        return Tests.writer();
    }

    /** A public no-argument constructor calling Object's. */
    static void constructor(ClassWriter w) {
        Tests.constructor(w);
    }

    /** Whether this JVM can load class files of this version. */
    static boolean runs(int version) {
        return Tests.runs(version);
    }
}
