package com.blockforge.lite;

import java.io.File;
import java.io.IOException;
import java.lang.instrument.ClassFileTransformer;
import java.util.function.Function;

/**
 * java -cp blockforge-lite.jar com.blockforge.lite.SelfTest
 *
 * Checks the agent on this JVM and this system: the off switches, that ASM stays hidden from the class path, and then
 * every patch on stand-ins of its target (the tests themselves live with the patch code, see core/Tests.java). Exits
 * with 0 when everything passed, 1 otherwise.
 */
public final class SelfTest {
    private static int failures;

    private SelfTest() {}

    public static void main(String[] args) throws Exception {
        System.out.println("BlockForge Lite self-test on Java " + System.getProperty("java.version") + " ("
                + System.getProperty("java.vendor") + "), " + System.getProperty("os.name") + " "
                + System.getProperty("os.arch"));
        offSwitches();
        hidden();
        File tmp = tempDir();
        try {
            Class<?> tests = Class.forName("com.blockforge.lite.core.Tests", true, CoreLoader.get());
            Function<String, ClassFileTransformer> factory = new Function<String, ClassFileTransformer>() {
                public ClassFileTransformer apply(String options) {
                    return new Entry(options);
                }
            };
            failures += (Integer) tests.getMethod("run", Function.class, File.class).invoke(null, factory, tmp);
        } finally {
            delete(tmp);
        }
        System.out.println(failures == 0 ? "PASSED" : "FAILED (" + failures + ")");
        System.exit(failures == 0 ? 0 : 1);
    }

    private static void offSwitches() throws IOException {
        expect(!Agent.switchedOff(), "on by default");
        System.setProperty("blockforge.lite", "off");
        expect(Agent.switchedOff(), "-Dblockforge.lite=off switches it off");
        System.setProperty("blockforge.lite", "on");
        expect(!Agent.switchedOff(), "-Dblockforge.lite=on leaves it on");
        System.clearProperty("blockforge.lite");
        File f = File.createTempFile("blockforge-lite-off", ".txt");
        System.setProperty("blockforge.lite.off", f.getPath());
        expect(Agent.switchedOff(), "the off file exists: off");
        f.delete();
        expect(!Agent.switchedOff(), "the off file is gone: on");
        System.clearProperty("blockforge.lite.off");
        expect(Agent.hasFlag("a=1, quiet ,b", "quiet") && !Agent.hasFlag("quiet2", "quiet"), "flags are found");
        System.out.println("ok    offSwitches");
    }

    /** The game must never be able to load our ASM or the patch code from the class path. */
    private static void hidden() throws Exception {
        ClassLoader app = SelfTest.class.getClassLoader();
        expect(app.getResource("org/objectweb/asm/ClassReader.class") == null, "ASM is not on the class path");
        expect(app.getResource("com/blockforge/lite/core/Engine.class") == null, "the patch code is not either");
        ClassLoader core = CoreLoader.get();
        Class<?> asm = Class.forName("org.objectweb.asm.ClassReader", false, core);
        expect(asm.getClassLoader() == core, "ASM comes from the private loader");
        boolean seesEntry;
        try {
            Class.forName("com.blockforge.lite.Entry", false, core);
            seesEntry = true;
        } catch (ClassNotFoundException e) {
            seesEntry = false;
        }
        expect(!seesEntry, "the private loader can't see the class path");
        System.out.println("ok    hidden");
    }

    private static File tempDir() throws IOException {
        File f = File.createTempFile("blockforge-lite-test", "");
        f.delete();
        if (!f.mkdirs()) {
            throw new IOException("can't make " + f);
        }
        return f;
    }

    private static void delete(File f) {
        File[] kids = f.listFiles();
        if (kids != null) {
            for (File k : kids) {
                delete(k);
            }
        }
        f.delete();
    }

    private static void expect(boolean ok, String what) {
        if (!ok) {
            failures++;
            System.out.println("      failed: " + what);
        }
    }
}
