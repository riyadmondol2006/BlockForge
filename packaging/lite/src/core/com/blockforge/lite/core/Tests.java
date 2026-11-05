package com.blockforge.lite.core;

import java.io.File;
import java.io.FileOutputStream;
import java.lang.instrument.ClassFileTransformer;
import java.lang.reflect.Field;
import java.lang.reflect.InvocationTargetException;
import java.nio.charset.Charset;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Function;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.tree.InsnNode;
import org.objectweb.asm.tree.MethodNode;

/**
 * The self-test (run by com.blockforge.lite.SelfTest). It builds small stand-ins for the game classes each patch
 * targets (and for GLFW and RenderSystem), loads them through the agent's transformer into a fresh class loader, runs
 * them, and checks the results. The JVM verifies those classes as it loads them, so a patch that writes bad bytecode
 * fails here. Also checks that every kind of trouble leaves the class untouched.
 */
public final class Tests implements Opcodes {
    static final String MC = "net/minecraft/client/Minecraft";
    private static final String GLFW = IdleFps.GLFW;
    private static final String RS = IdleFps.RENDER_SYSTEM;
    private static final String DISPLAY = IdleFps.DISPLAY;

    private final Function<String, ClassFileTransformer> factory;
    private final File tmp;
    private int failures;
    private int checks;

    private Tests(Function<String, ClassFileTransformer> factory, File tmp) {
        this.factory = factory;
        this.tmp = tmp;
    }

    /**
     * Runs every test; returns the number of failures. The factory makes the agent's transformer for some options
     * (the real entry point, with its name filter); tmp is a folder for report files.
     */
    public static int run(Function<String, ClassFileTransformer> factory, File tmp) {
        Tests t = new Tests(factory, tmp);
        // the framework's own tests (with idle-fps as the example patch)
        String[] names = {"idleFpsWorks", "idleFpsOldClassFiles", "idleFpsNewestClassFiles", "idleFpsLwjgl2",
            "idleFpsShapeMismatch", "idleFpsCalledTwice", "idleFpsChangedByMixin", "idleFpsTooNewClassFile",
            "corruptBytesUntouched", "patchThatThrows", "patchThatBreaksCode", "onePatchFailsOtherStays", "offOption",
            "onlyOption", "modInstalled", "modIdsAndTargets", "strictOption", "notAGameClass", "alreadyPatched", "constantPoolFilter",
            "reportFile"};
        for (String n : names) {
            if (n.equals("corruptBytesUntouched")) {
                System.out.println("      (the next tests break things on purpose; their \"problem\" lines are expected)");
            }
            try {
                t.one(n, t, Tests.class.getDeclaredMethod(n));
            } catch (NoSuchMethodException e) {
                t.fail(n + " is missing");
            }
        }
        // every patch's own tests
        for (Class<? extends PatchTest> c : PATCH_TESTS) {
            try {
                PatchTest p = c.getDeclaredConstructor().newInstance();
                p.init(t, factory, tmp);
                java.lang.reflect.Method[] ms = c.getDeclaredMethods();
                Arrays.sort(ms, new java.util.Comparator<java.lang.reflect.Method>() {
                    public int compare(java.lang.reflect.Method a, java.lang.reflect.Method b) {
                        return a.getName().compareTo(b.getName());
                    }
                });
                for (java.lang.reflect.Method m : ms) {
                    int mod = m.getModifiers();
                    if (m.getParameterTypes().length == 0 && m.getReturnType() == void.class && !m.isSynthetic()
                            && !java.lang.reflect.Modifier.isStatic(mod) && !java.lang.reflect.Modifier.isPrivate(mod)) {
                        t.one(c.getSimpleName() + "." + m.getName(), p, m);
                    }
                }
            } catch (Throwable e) {
                t.fail(c.getSimpleName() + " could not run: " + e);
            }
        }
        System.out.println(t.checks + " checks, " + t.failures + " failed");
        return t.failures;
    }

    /** The tests of the patches, one class each (see {@link PatchTest}). Add a new patch's test here. */
    static final List<Class<? extends PatchTest>> PATCH_TESTS = Arrays.<Class<? extends PatchTest>>asList(
            NoWorldGcTest.class, SlimSectionsTest.class, ShortFadeTest.class, OutlineSkipTest.class,
            WorkerPriorityTest.class,
            LazyCollisionShapeTest.class, EmptyEffectListsTest.class, SharedStateTablesTest.class, ShortSpinTest.class,
            SoundNoWaitTest.class, QuietMusicTest.class,
            // wave 1 W1
            TintMemoTest.class, HiddenEntitiesTest.class, BlockEntityViewTest.class, SignLinesTest.class);

    private void one(String name, Object on, java.lang.reflect.Method m) {
        int before = failures;
        try {
            m.setAccessible(true);
            m.invoke(on);
        } catch (InvocationTargetException e) {
            fail(name + " threw " + e.getCause());
            e.getCause().printStackTrace(System.out);
        } catch (Throwable e) {
            fail(name + " could not run: " + e);
        }
        System.out.println((failures == before ? "ok    " : "FAIL  ") + name);
    }

    // ---- the tests ----

    void idleFpsWorks() throws Exception {
        Game g = load(factory.apply("quiet"), minecraft(V1_8, Shape.NORMAL));
        expect(g.transformed.contains(MC), "the Minecraft stand-in was patched");
        idleBehaviour(g);
    }

    void idleFpsOldClassFiles() throws Exception {
        // Java 5 class files have no stack map frames; 1.8.9-era games use them
        Game g = load(factory.apply("quiet"), minecraft(V1_5, Shape.NORMAL));
        expect(g.transformed.contains(MC), "patched an old class file");
        idleBehaviour(g);
    }

    void idleFpsNewestClassFiles() throws Exception {
        // the newest class files this JVM can run (Java 8: 52, Java 25: 69)
        String spec = System.getProperty("java.specification.version", "1.8");
        int feature = Integer.parseInt(spec.startsWith("1.") ? spec.substring(2) : spec);
        int version = Math.min(44 + feature, V27);
        Game g = load(factory.apply("quiet"), minecraft(version, Shape.NORMAL));
        expect(g.transformed.contains(MC), "patched a class file of version " + version);
        idleBehaviour(g);
    }

    private void idleBehaviour(Game g) throws Exception {
        g.window(0, 0, 0, 1);
        expect(g.frame(120, true) == 120, "no GL context: the limit stays");
        g.window(7, 0, 0, 1);
        expect(g.frame(120, true) == 120, "visible window: the limit stays");
        expect(g.frame(120, false) == 60, "menu outside a world: 60 as before");
        g.window(7, 1, 0, 1);
        expect(g.frame(120, true) == 10, "minimised: 10");
        expect(g.frame(120, false) == 10, "minimised in a menu: 10");
        expect(g.frame(260, true) == 10, "minimised with no frame limit: 10");
        expect(g.frame(5, true) == 5, "a limit below 10 stays");
        g.window(7, 0, 0, 1);
        expect(g.frame(260, true) == -1, "visible with no frame limit: the limiter is not called");
        g.window(7, 0, 3, 1);
        expect(g.frame(120, true) == 120, "full screen and focused: the limit stays");
        g.window(7, 0, 3, 0);
        expect(g.frame(120, true) == 10, "full screen and not focused: 10");
        g.window(7, 0, 0, 0);
        expect(g.frame(120, true) == 120, "a window without focus: the limit stays");
        g.setStatic(GLFW, "boom", 1);
        expect(g.frame(120, true) == 120, "GLFW throws: the limit stays, nothing thrown");
    }

    void idleFpsLwjgl2() throws Exception {
        for (int version : new int[] {V1_6, V1_8}) {
            Game g = load(factory.apply("quiet"), minecraft(version, Shape.LEGACY));
            expect(g.transformed.contains(MC), "the LWJGL 2 stand-in was patched (class version " + version + ")");
            g.display(1, 0, 1);
            expect(g.frame(120, true) == 120, "visible: the limit stays");
            expect(g.frame(120, false) == 30, "menu outside a world: 30 as before");
            g.display(0, 0, 0);
            expect(g.frame(120, true) == 10, "minimised: 10");
            expect(g.frame(260, true) == -1, "no frame limit: sync is not called, as before");
            g.display(1, 1, 0);
            expect(g.frame(120, true) == 10, "full screen, switched away: 10");
            g.display(1, 1, 1);
            expect(g.frame(120, true) == 120, "full screen, active: the limit stays");
            g.display(1, 0, 0);
            expect(g.frame(120, true) == 120, "a window without focus: the limit stays");
            g.setStatic(DISPLAY, "boom", 1);
            expect(g.frame(120, true) == 120, "Display throws: the limit stays, nothing thrown");
        }
    }

    static boolean runs(int version) {
        String spec = System.getProperty("java.specification.version", "1.8");
        return 44 + Integer.parseInt(spec.startsWith("1.") ? spec.substring(2) : spec) >= version;
    }

    void idleFpsShapeMismatch() throws Exception {
        String report = reportPath("mismatch");
        Game g = load(factory.apply("quiet,report=" + report), minecraft(V1_8, Shape.FIELD));
        expect(!g.transformed.contains(MC), "a different shape is left alone");
        g.window(7, 1, 0, 1);
        expect(g.frame(120, true) == 120, "and still works as before");
        expect(line(report, "idle-fps").startsWith("idle-fps\tskipped\t" + MC), "reported as skipped");
    }

    void idleFpsCalledTwice() throws Exception {
        String report = reportPath("twice");
        Game g = load(factory.apply("quiet,report=" + report), minecraft(V1_8, Shape.TWICE));
        expect(!g.transformed.contains(MC), "two limiter calls: left alone");
        expect(line(report, "idle-fps").contains("2 times"), "reported with the reason");
    }

    void idleFpsChangedByMixin() throws Exception {
        String report = reportPath("mixin");
        Game g = load(factory.apply("quiet,report=" + report), minecraft(V1_8, Shape.MIXIN));
        expect(!g.transformed.contains(MC), "a getFramerateLimit changed by a mod is left alone");
        expect(line(report, "idle-fps").contains("changed by a mod"), "reported with the reason");
    }

    void idleFpsTooNewClassFile() throws Exception {
        byte[] b = minecraft(V21, Shape.NORMAL).get(MC);
        b[6] = 0;
        b[7] = 99; // a class file of Java 55
        String report = reportPath("toonew");
        ClassFileTransformer t = factory.apply("quiet,report=" + report);
        expect(t.transform(null, MC, null, null, b) == null, "a class file ASM can't read is left alone");
        expect(line(report, "idle-fps").contains("skipped"), "reported as skipped");
    }

    void corruptBytesUntouched() throws Exception {
        byte[] good = minecraft(V1_8, Shape.NORMAL).get(MC);
        // cut short: late cuts pass the cheap filter, so ASM has to deal with the broken rest
        for (int cut : new int[] {good.length - 1, good.length - 40, good.length * 3 / 4, good.length / 2, 12, 3, 0}) {
            byte[] b = Arrays.copyOf(good, cut);
            ClassFileTransformer t = factory.apply("quiet");
            expect(t.transform(null, MC, null, null, b) == null, "cut at " + cut + " of " + good.length + ": untouched");
        }
        expect(factory.apply("quiet").transform(null, MC, null, null, null) == null, "no bytes: untouched");
    }

    void patchThatThrows() throws Exception {
        Engine e = new Engine("quiet", null, Arrays.<Patch>asList(new Throwing()));
        byte[] b = minecraft(V1_8, Shape.NORMAL).get(MC);
        expect(e.transform(null, MC, null, null, b) == null, "a patch that throws changes nothing");
    }

    void patchThatBreaksCode() throws Exception {
        Engine e = new Engine("quiet", null, Arrays.<Patch>asList(new Breaking()));
        byte[] b = minecraft(V1_8, Shape.NORMAL).get(MC);
        expect(e.transform(null, MC, null, null, b) == null, "a patch whose code fails the check changes nothing");
    }

    void onePatchFailsOtherStays() throws Exception {
        Engine e = new Engine("quiet", null, Arrays.<Patch>asList(new Throwing(), new IdleFps(), new Breaking()));
        Map<String, byte[]> classes = minecraft(V1_8, Shape.NORMAL);
        byte[] out = e.transform(null, MC, null, null, classes.get(MC));
        expect(out != null, "the good patch is applied");
        if (out != null) {
            classes.put(MC, out);
            Game g = load(null, classes);
            g.window(7, 1, 0, 1);
            expect(g.frame(120, true) == 10, "and works");
        }
    }

    void offOption() throws Exception {
        String report = reportPath("off");
        Game g = load(factory.apply("quiet,off=idle-fps,report=" + report), minecraft(V1_8, Shape.NORMAL));
        expect(!g.transformed.contains(MC), "off=idle-fps: not patched");
        expect(line(report, "idle-fps").contains("off="), "reported as switched off");
        g = load(factory.apply("quiet,off=something-else;other"), minecraft(V1_8, Shape.NORMAL));
        expect(g.transformed.contains(MC), "off= of other patches: patched");
    }

    void onlyOption() throws Exception {
        Game g = load(factory.apply("quiet,only=other"), minecraft(V1_8, Shape.NORMAL));
        expect(!g.transformed.contains(MC), "only=other: not patched");
        g = load(factory.apply("quiet,only=idle-fps"), minecraft(V1_8, Shape.NORMAL));
        expect(g.transformed.contains(MC), "only=idle-fps: patched");
        g = load(factory.apply("quiet,only=idle-fps,off=idle-fps"), minecraft(V1_8, Shape.NORMAL));
        expect(!g.transformed.contains(MC), "off= wins over only=");
    }

    void modInstalled() throws Exception {
        File list = new File(tmp, "mods.txt");
        write(list, "fabric-api-0.100.jar\nDynamic-FPS-3.6.3+minecraft-1.21.0-fabric.jar\n");
        String report = reportPath("mods");
        Game g = load(factory.apply("quiet,mods=" + list + ",report=" + report), minecraft(V1_8, Shape.NORMAL));
        expect(!g.transformed.contains(MC), "Dynamic FPS in the mod list: not patched");
        expect(line(report, "idle-fps").contains("Dynamic FPS"), "the report names the mod");
        write(list, "sodium-fabric-0.6.jar\n");
        g = load(factory.apply("quiet,mods=" + list), minecraft(V1_8, Shape.NORMAL));
        expect(g.transformed.contains(MC), "an unrelated mod: patched");
        // found by a class of the mod, as the game's class loader sees it
        Map<String, byte[]> classes = minecraft(V1_8, Shape.NORMAL);
        classes.put("dynamic_fps/impl/DynamicFPSMod", emptyClass("dynamic_fps/impl/DynamicFPSMod"));
        g = load(factory.apply("quiet"), classes);
        expect(!g.transformed.contains(MC), "Dynamic FPS found by its class: not patched");
    }

    void modIdsAndTargets() throws Exception {
        Game g = load(factory.apply("quiet,modids=fabric-api;dynamic_fps"), minecraft(V1_8, Shape.NORMAL));
        expect(!g.transformed.contains(MC), "modids=...;dynamic_fps: not patched");
        File targets = new File(tmp, "targets.txt");
        write(targets, "# patch class\nidle-fps fgo\n");
        byte[] b = minecraft(V1_8, Shape.NORMAL).get(MC);
        expect(factory.apply("quiet,targets=" + targets).transform(null, MC, null, null, b) == null,
                "targets= names another class: this one is not looked at");
        expect(factory.apply("quiet,targets=" + targets).transform(null, "fgo", null, null, b) != null,
                "the class targets= names is patched");
        expect(factory.apply("quiet,targets=" + new File(tmp, "missing.txt")).transform(null, MC, null, null, b)
                != null, "a missing targets file: as without it");
    }

    void strictOption() throws Exception {
        byte[] old = minecraft(V1_8, Shape.NORMAL).get(MC);
        byte[] modern = minecraft(V17, Shape.NORMAL).get(MC);
        ClassFileTransformer t = factory.apply("quiet,strict");
        expect(t.transform(null, MC, null, null, old) == null, "strict: Java 8 class files are left alone");
        expect(factory.apply("quiet,strict").transform(null, "fgo", null, null, modern) == null,
                "strict: obfuscated names are left alone");
        expect(factory.apply("quiet,strict").transform(null, "net/minecraft/class_310", null, null, modern) == null,
                "strict: intermediary names are left alone");
        expect(factory.apply("quiet,strict").transform(null, MC, null, null, modern) != null,
                "strict: the Mojang name of a 1.17+ class is patched");
    }

    void notAGameClass() throws Exception {
        byte[] b = minecraft(V1_8, Shape.NORMAL).get(MC);
        ClassFileTransformer t = factory.apply("quiet");
        expect(t.transform(null, "com/example/Minecraft", null, null, b) == null, "outside the game's packages");
        expect(t.transform(null, MC, String.class, null, b) == null, "a class being redefined");
        expect(t.transform(null, "net/minecraft/client/Other", null, null, b) == null, "another Minecraft class");
        // the stand-in still calls itself net/minecraft/client/Minecraft inside; only the given name differs
        expect(factory.apply("quiet").transform(null, "fgo", null, null, b) != null, "an obfuscated name");
        expect(factory.apply("quiet").transform(null, "net/minecraft/class_310", null, null, b) != null,
                "the intermediary name");
    }

    void alreadyPatched() throws Exception {
        byte[] b = minecraft(V1_8, Shape.NORMAL).get(MC);
        byte[] once = factory.apply("quiet").transform(null, MC, null, null, b);
        expect(once != null, "patched once");
        if (once != null) {
            expect(factory.apply("quiet").transform(null, MC, null, null, once) == null, "not twice");
        }
    }

    void constantPoolFilter() throws Exception {
        ConstantPool cp = new ConstantPool(new String[] {"limitDisplayFPS", "nothing-like-this"});
        boolean[] found = new boolean[2];
        expect(cp.scan(minecraft(V1_8, Shape.NORMAL).get(MC), found), "reads a class file");
        expect(found[0] && !found[1], "finds exactly the texts that are there");
        // every class of the JDK this test can read
        int n = 0;
        long t0 = System.nanoTime();
        for (Class<?> c : new Class<?>[] {String.class, HashMap.class, ArrayList.class, Thread.class, Charset.class,
                ClassWriter.class, ClassReader.class, MethodNode.class}) {
            byte[] b = bytesOf(c);
            if (b != null) {
                boolean[] f = new boolean[2];
                expect(cp.scan(b, f) && !f[0], "scans " + c.getName());
                n++;
            }
        }
        System.out.println("      scanned " + n + " real classes in " + (System.nanoTime() - t0) / 1000 + " us");
    }

    void reportFile() throws Exception {
        String report = reportPath("report");
        load(factory.apply("quiet,report=" + report), minecraft(V1_8, Shape.NORMAL));
        String l = line(report, "idle-fps");
        expect(l.startsWith("idle-fps\tapplied\t" + MC + "\t"), "applied line: " + l);
        String[] columns = l.split("\t", -1);
        expect(columns.length == 5, "five columns");
        expect(columns[4].contains("minimised"), "says what it does: " + columns[4]);
    }

    // ---- stand-ins for the game ----

    /** NORMAL: 1.14 - 1.21.1; LEGACY: 1.8 - 1.12 (LWJGL 2); the others: shapes the patch must leave alone. */
    enum Shape { NORMAL, LEGACY, FIELD, TWICE, MIXIN }

    /** GLFW, RenderSystem and a Minecraft class whose frame end looks like the real one in the given way. */
    static Map<String, byte[]> minecraft(int version, Shape shape) {
        Map<String, byte[]> m = new HashMap<String, byte[]>();
        m.put(GLFW, glfw());
        m.put(RS, renderSystem());
        m.put(DISPLAY, display());
        boolean legacy = shape == Shape.LEGACY;

        ClassWriter w = writer();
        w.visit(version, ACC_PUBLIC | ACC_SUPER, MC, null, "java/lang/Object", null);
        w.visitField(ACC_PUBLIC, "level", "Ljava/lang/Object;", null, null).visitEnd();
        w.visitField(ACC_PUBLIC, "limit", "I", null, null).visitEnd();
        constructor(w);

        MethodVisitor v = w.visitMethod(ACC_PRIVATE, "getFramerateLimit", "()I", null, null);
        v.visitCode();
        if (shape == Shape.MIXIN) {
            v.visitMethodInsn(INVOKESTATIC, MC, "handler$zza000$dynamicfps$limit", "()V", false);
        }
        Label menu = new Label();
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, MC, "level", "Ljava/lang/Object;");
        v.visitJumpInsn(IFNULL, menu);
        v.visitVarInsn(ALOAD, 0);
        v.visitFieldInsn(GETFIELD, MC, "limit", "I");
        v.visitInsn(IRETURN);
        v.visitLabel(menu);
        v.visitIntInsn(BIPUSH, legacy ? 30 : 60);
        v.visitInsn(IRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        if (shape == Shape.MIXIN) {
            v = w.visitMethod(ACC_PRIVATE | ACC_STATIC, "handler$zza000$dynamicfps$limit", "()V", null, null);
            v.visitCode();
            v.visitInsn(RETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }

        v = w.visitMethod(ACC_PUBLIC, "runTick", "()V", null, null);
        v.visitCode();
        v.visitLdcInsn("Pre render");
        v.visitInsn(POP);
        if (legacy) {
            // if (this.limit < 260) Display.sync(this.getLimitFramerate());
            Label skip = new Label();
            v.visitVarInsn(ALOAD, 0);
            v.visitFieldInsn(GETFIELD, MC, "limit", "I");
            v.visitIntInsn(SIPUSH, 260);
            v.visitJumpInsn(IF_ICMPGE, skip);
            v.visitVarInsn(ALOAD, 0);
            v.visitMethodInsn(INVOKEVIRTUAL, MC, "getFramerateLimit", "()I", false);
            v.visitMethodInsn(INVOKESTATIC, DISPLAY, "sync", "(I)V", false);
            v.visitLabel(skip);
        }
        for (int i = 0; i < (shape == Shape.TWICE ? 2 : legacy ? 0 : 1); i++) {
            Label skip = new Label();
            v.visitVarInsn(ALOAD, 0);
            if (shape == Shape.FIELD) {
                v.visitFieldInsn(GETFIELD, MC, "limit", "I");
                v.visitInsn(ICONST_0);
                v.visitInsn(IADD);
            } else {
                v.visitMethodInsn(INVOKESPECIAL, MC, "getFramerateLimit", "()I", false);
            }
            v.visitVarInsn(ISTORE, 1);
            v.visitVarInsn(ILOAD, 1);
            v.visitIntInsn(SIPUSH, 260);
            v.visitJumpInsn(IF_ICMPGE, skip);
            v.visitVarInsn(ILOAD, 1);
            v.visitMethodInsn(INVOKESTATIC, RS, "limitDisplayFPS", "(I)V", false);
            v.visitLabel(skip);
        }
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        m.put(MC, w.toByteArray());
        return m;
    }

    /** A GLFW whose answers the test sets: the context, iconified, monitor and focused attributes, or an error. */
    private static byte[] glfw() {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, GLFW, null, "java/lang/Object", null);
        for (String f : new String[] {"ctx:J", "iconified:I", "monitor:J", "focused:I", "boom:I"}) {
            String[] p = f.split(":");
            w.visitField(ACC_PUBLIC | ACC_STATIC, p[0], p[1], null, null).visitEnd();
        }
        constructor(w);
        MethodVisitor v = w.visitMethod(ACC_PUBLIC | ACC_STATIC, "glfwGetCurrentContext", "()J", null, null);
        v.visitCode();
        Label ok = new Label();
        v.visitFieldInsn(GETSTATIC, GLFW, "boom", "I");
        v.visitJumpInsn(IFEQ, ok);
        v.visitTypeInsn(NEW, "java/lang/IllegalStateException");
        v.visitInsn(DUP);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/IllegalStateException", "<init>", "()V", false);
        v.visitInsn(ATHROW);
        v.visitLabel(ok);
        v.visitFieldInsn(GETSTATIC, GLFW, "ctx", "J");
        v.visitInsn(LRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        v = w.visitMethod(ACC_PUBLIC | ACC_STATIC, "glfwGetWindowAttrib", "(JI)I", null, null);
        v.visitCode();
        Label notIconified = new Label();
        Label notFocused = new Label();
        v.visitVarInsn(ILOAD, 2);
        v.visitLdcInsn(IdleFps.GLFW_ICONIFIED);
        v.visitJumpInsn(IF_ICMPNE, notIconified);
        v.visitFieldInsn(GETSTATIC, GLFW, "iconified", "I");
        v.visitInsn(IRETURN);
        v.visitLabel(notIconified);
        v.visitVarInsn(ILOAD, 2);
        v.visitLdcInsn(IdleFps.GLFW_FOCUSED);
        v.visitJumpInsn(IF_ICMPNE, notFocused);
        v.visitFieldInsn(GETSTATIC, GLFW, "focused", "I");
        v.visitInsn(IRETURN);
        v.visitLabel(notFocused);
        v.visitInsn(ICONST_0);
        v.visitInsn(IRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();

        v = w.visitMethod(ACC_PUBLIC | ACC_STATIC, "glfwGetWindowMonitor", "(J)J", null, null);
        v.visitCode();
        v.visitFieldInsn(GETSTATIC, GLFW, "monitor", "J");
        v.visitInsn(LRETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        return w.toByteArray();
    }

    /** A RenderSystem that remembers the frame limit it was last given (-1: not called). */
    private static byte[] renderSystem() {
        return limiter(RS, "limitDisplayFPS");
    }

    /** LWJGL 2's Display: answers set by the test (visible, fullscreen, active, or an error), remembers sync(). */
    private static byte[] display() {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, DISPLAY, null, "java/lang/Object", null);
        w.visitField(ACC_PUBLIC | ACC_STATIC, "last", "I", null, null).visitEnd();
        constructor(w);
        for (String f : new String[] {"visible", "fullscreen", "active", "boom"}) {
            w.visitField(ACC_PUBLIC | ACC_STATIC, f, "Z", null, null).visitEnd();
            if (f.equals("boom")) {
                continue;
            }
            String name = "is" + Character.toUpperCase(f.charAt(0)) + f.substring(1);
            MethodVisitor v = w.visitMethod(ACC_PUBLIC | ACC_STATIC, name, "()Z", null, null);
            v.visitCode();
            Label ok = new Label();
            v.visitFieldInsn(GETSTATIC, DISPLAY, "boom", "Z");
            v.visitJumpInsn(IFEQ, ok);
            v.visitTypeInsn(NEW, "java/lang/IllegalStateException");
            v.visitInsn(DUP);
            v.visitMethodInsn(INVOKESPECIAL, "java/lang/IllegalStateException", "<init>", "()V", false);
            v.visitInsn(ATHROW);
            v.visitLabel(ok);
            v.visitFieldInsn(GETSTATIC, DISPLAY, f, "Z");
            v.visitInsn(IRETURN);
            v.visitMaxs(0, 0);
            v.visitEnd();
        }
        MethodVisitor v = w.visitMethod(ACC_PUBLIC | ACC_STATIC, "sync", "(I)V", null, null);
        v.visitCode();
        v.visitVarInsn(ILOAD, 0);
        v.visitFieldInsn(PUTSTATIC, DISPLAY, "last", "I");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        return w.toByteArray();
    }

    private static byte[] limiter(String cls, String method) {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, cls, null, "java/lang/Object", null);
        w.visitField(ACC_PUBLIC | ACC_STATIC, "last", "I", null, null).visitEnd();
        constructor(w);
        MethodVisitor v = w.visitMethod(ACC_PUBLIC | ACC_STATIC, method, "(I)V", null, null);
        v.visitCode();
        v.visitVarInsn(ILOAD, 0);
        v.visitFieldInsn(PUTSTATIC, cls, "last", "I");
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
        w.visitEnd();
        return w.toByteArray();
    }

    private static byte[] emptyClass(String name) {
        ClassWriter w = writer();
        w.visit(V1_8, ACC_PUBLIC | ACC_SUPER, name, null, "java/lang/Object", null);
        constructor(w);
        w.visitEnd();
        return w.toByteArray();
    }

    static void constructor(ClassWriter w) {
        MethodVisitor v = w.visitMethod(ACC_PUBLIC, "<init>", "()V", null, null);
        v.visitCode();
        v.visitVarInsn(ALOAD, 0);
        v.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
        v.visitInsn(RETURN);
        v.visitMaxs(0, 0);
        v.visitEnd();
    }

    static ClassWriter writer() {
        return new ClassWriter(ClassWriter.COMPUTE_FRAMES | ClassWriter.COMPUTE_MAXS) {
            @Override
            protected String getCommonSuperClass(String a, String b) {
                return "java/lang/Object";
            }
        };
    }

    /** The stand-ins, loaded through a transformer (null: as they are), like a mod loader would. */
    static Game load(ClassFileTransformer t, Map<String, byte[]> classes) throws Exception {
        GameLoader l = new GameLoader(t, classes);
        for (String n : classes.keySet()) {
            Class.forName(n.replace('/', '.'), true, l);
        }
        return new Game(l);
    }

    static final class GameLoader extends ClassLoader {
        final ClassFileTransformer transformer;
        final Map<String, byte[]> classes;
        final List<String> transformed = new ArrayList<String>();

        GameLoader(ClassFileTransformer t, Map<String, byte[]> classes) {
            super(null);
            this.transformer = t;
            this.classes = classes;
        }

        @Override
        protected Class<?> findClass(String name) throws ClassNotFoundException {
            String internal = name.replace('.', '/');
            byte[] b = classes.get(internal);
            if (b == null) {
                throw new ClassNotFoundException(name);
            }
            if (transformer != null) {
                try {
                    byte[] out = transformer.transform(this, internal, null, null, b);
                    if (out != null) {
                        b = out;
                        transformed.add(internal);
                    }
                } catch (Exception e) {
                    throw new ClassNotFoundException(name, e);
                }
            }
            return defineClass(name, b, 0, b.length);
        }

        /** The stand-ins' class files, as a mod loader's class loader has them (for patches that look up members). */
        @Override
        public java.io.InputStream getResourceAsStream(String name) {
            byte[] b = name.endsWith(".class") ? classes.get(name.substring(0, name.length() - 6)) : null;
            return b != null ? new java.io.ByteArrayInputStream(b) : super.getResourceAsStream(name);
        }

        @Override
        protected java.net.URL findResource(String name) {
            // lets Mods find a mod by its class, as a mod loader's class loader would
            if (name.endsWith(".class") && classes.containsKey(name.substring(0, name.length() - 6))) {
                try {
                    return new java.net.URL("file:/" + name);
                } catch (Exception e) {
                    return null;
                }
            }
            return null;
        }
    }

    /** Drives the loaded stand-ins. */
    static final class Game {
        final GameLoader loader;
        final List<String> transformed;

        Game(GameLoader loader) {
            this.loader = loader;
            this.transformed = loader.transformed;
        }

        void setStatic(String cls, String field, Object value) throws Exception {
            Field f = loader.loadClass(cls.replace('/', '.')).getField(field);
            if (f.getType() == long.class) {
                f.setLong(null, ((Number) value).longValue());
            } else if (f.getType() == boolean.class) {
                f.setBoolean(null, ((Number) value).intValue() != 0);
            } else {
                f.setInt(null, ((Number) value).intValue());
            }
        }

        /** What GLFW says: the current context, iconified, the full screen monitor, focused. */
        void window(long ctx, int iconified, long monitor, int focused) throws Exception {
            setStatic(GLFW, "ctx", ctx);
            setStatic(GLFW, "iconified", iconified);
            setStatic(GLFW, "monitor", monitor);
            setStatic(GLFW, "focused", focused);
            setStatic(GLFW, "boom", 0);
        }

        /** What LWJGL 2's Display says. */
        void display(int visible, int fullscreen, int active) throws Exception {
            setStatic(DISPLAY, "visible", visible);
            setStatic(DISPLAY, "fullscreen", fullscreen);
            setStatic(DISPLAY, "active", active);
            setStatic(DISPLAY, "boom", 0);
        }

        /** Runs one frame end; returns the limit the limiter got, or -1 when it was not called. */
        int frame(int limit, boolean inWorld) throws Exception {
            setStatic(RS, "last", -1);
            setStatic(DISPLAY, "last", -1);
            Class<?> c = loader.loadClass(MC.replace('/', '.'));
            Object mc = c.newInstance();
            c.getField("limit").setInt(mc, limit);
            c.getField("level").set(mc, inWorld ? new Object() : null);
            c.getMethod("runTick").invoke(mc);
            int modern = loader.loadClass(RS.replace('/', '.')).getField("last").getInt(null);
            int legacy = loader.loadClass(DISPLAY.replace('/', '.')).getField("last").getInt(null);
            return modern != -1 ? modern : legacy;
        }
    }

    /** A patch that throws on the Minecraft stand-in. */
    static final class Throwing extends Patch {
        Throwing() {
            super("test-throws", 1, "throws", new String[][] {{"limitDisplayFPS"}}, new String[0]);
        }

        @Override
        void apply(Target t, Ctx ctx) {
            throw new IllegalStateException("on purpose");
        }
    }

    /** A patch that writes code the verifier must refuse (a POP on an empty stack). */
    static final class Breaking extends Patch {
        Breaking() {
            super("test-breaks", 1, "breaks", new String[][] {{"limitDisplayFPS"}}, new String[0]);
        }

        @Override
        void apply(Target t, Ctx ctx) {
            MethodNode m = t.method("runTick", "()V");
            m.instructions.insert(new InsnNode(POP));
            t.changed(m);
        }
    }

    // ---- helpers ----

    void expect(boolean ok, String what) {
        checks++;
        if (!ok) {
            fail(what);
        }
    }

    private void fail(String what) {
        failures++;
        System.out.println("      failed: " + what);
    }

    String reportPath(String name) {
        File f = new File(tmp, name + ".report");
        f.delete();
        return f.getPath();
    }

    static String line(String report, String id) throws Exception {
        byte[] b = java.nio.file.Files.readAllBytes(new File(report).toPath());
        for (String l : new String(b, "UTF-8").split("\n")) {
            if (l.startsWith(id + "\t")) {
                return l;
            }
        }
        return "";
    }

    static void write(File f, String text) throws Exception {
        FileOutputStream o = new FileOutputStream(f);
        try {
            o.write(text.getBytes("UTF-8"));
        } finally {
            o.close();
        }
    }

    private static byte[] bytesOf(Class<?> c) {
        try {
            java.io.InputStream in = c.getResourceAsStream("/" + c.getName().replace('.', '/') + ".class");
            if (in == null) {
                in = ClassLoader.getSystemResourceAsStream(c.getName().replace('.', '/') + ".class");
            }
            if (in == null) {
                return null;
            }
            java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
            byte[] buf = new byte[8192];
            for (int r; (r = in.read(buf)) > 0; ) {
                out.write(buf, 0, r);
            }
            in.close();
            return out.toByteArray();
        } catch (Exception e) {
            return null;
        }
    }
}
