package com.blockforge.lite.core;

/**
 * One change to the game. A patch finds its target by what the code contains (strings, the JDK or LWJGL methods it
 * calls, its shape), with the stable class names (Mojang's, Fabric's intermediary) only as a cheap first filter,
 * because names differ between vanilla, Fabric, Quilt, Forge and NeoForge and between versions. Anything unexpected
 * means the patch leaves the class alone.
 *
 * How to write one (see README "Adding a patch"; IdleFps, SlimSections, ShortFade are small examples):
 * <pre>
 * final class MyPatch extends Patch {
 *     MyPatch() {
 *         super("my-patch", 1, "what it does, in plain words",
 *                 new String[][] {{"some/Owner", "someMethod"}},            // constant pool texts (one set must match)
 *                 new String[] {"sodium"},                                  // Mods.KNOWN keys that do the same
 *                 "net/minecraft/client/Thing", "net/minecraft/class_1234"); // Mojang + intermediary class names
 *     }
 *     void apply(Target t, Ctx ctx) throws Skip {
 *         List&lt;Object[]&gt; calls = t.calls(Opcodes.INVOKEVIRTUAL, "some/Owner", "someMethod", "()V");
 *         if (calls.isEmpty()) throw Skip.notTarget("no call");        // not this class: silent
 *         ... check the exact shape with Code.prev/next/isCall/isVar/isInt/changedByMixin ...
 *         if (unexpected) throw Skip.because("why");                   // the target, but different: reported
 *         ... change the MethodNode ..., t.changed(method);            // or t.add(helper), t.addField(field)
 *     }
 * }
 * </pre>
 * Then add it to {@link Engine#all()} and a test class to {@link Tests#PATCH_TESTS}.
 */
abstract class Patch {
    /** Short name, used in the options (off=, only=) and the report. Lower case with dashes. */
    final String id;
    /** 1 = conservative (on by default), 2 = balanced (only with level=2). */
    final int level;
    /** What it does, in plain words, for the report. */
    final String what;
    /**
     * The cheap first filter: a class is looked at more closely only when its constant pool has every text of one of
     * these sets (whole entries: class names like "java/lang/System", member names, descriptors, string constants).
     */
    final String[][] markers;
    /**
     * Mods that already do this or that change the same code ({@link Mods#KNOWN} keys); when one is installed, this
     * patch stays off.
     */
    final String[] conflictsWith;
    /**
     * The names the target class has where names are stable: Mojang's (Forge, NeoForge, 26.1 and newer) and Fabric's
     * intermediary (Fabric, Quilt <= 1.21.11), or none for any class. Classes in no package (an obfuscated vanilla
     * game) are always looked at. This only makes the agent cheaper; the markers and the shape decide.
     */
    final java.util.Set<String> names;

    Patch(String id, int level, String what, String[][] markers, String[] conflictsWith, String... names) {
        this.id = id;
        this.level = level;
        this.what = what;
        this.markers = markers;
        this.conflictsWith = conflictsWith;
        this.names = names.length == 0 ? null : new java.util.HashSet<String>(java.util.Arrays.asList(names));
    }

    /** Whether a class of this name may be the target (before its bytes are looked at). */
    boolean mayBe(String name) {
        return names == null || name.indexOf('/') < 0 || names.contains(name);
    }

    /**
     * Whether the patch is done once it changed one class (or decided it can't). Override with false for a patch
     * that changes several classes.
     */
    boolean once() {
        return true;
    }

    /**
     * Changes the class: reads the methods it needs from the target, changes them and marks them changed, or adds
     * methods and fields. Those are checked with ASM's verifier before the new class is used. Throws {@link Skip}
     * when the class is not the target, or is the target but looks different from what the patch expects. Any other
     * exception counts as a failure of the patch (reported as a problem); the class then loads without it.
     */
    abstract void apply(Target t, Ctx ctx) throws Skip;

    /** Why a patch did not change a class. */
    static final class Skip extends Exception {
        private static final long serialVersionUID = 1L;
        /** True: this class is simply not the target (not reported). False: it is, and the patch can't be applied. */
        final boolean notTarget;

        Skip(String reason, boolean notTarget) {
            super(reason, null, false, false);
            this.notTarget = notTarget;
        }

        /** This class is not the target; keep looking (nothing is reported). */
        static Skip notTarget(String reason) {
            return new Skip(reason, true);
        }

        /** This is the target, but it can't be patched safely; reported as "skipped" with the reason. */
        static Skip because(String reason) {
            return new Skip(reason, false);
        }
    }

    /** What a patch may want to know about the class being loaded. */
    static final class Ctx {
        /** The class loader that defines the class (the game's: app, Knot, ModLauncher or LaunchWrapper). */
        final ClassLoader loader;
        /** The installed mods, if a patch needs more than {@link Patch#conflictsWith}. */
        final Mods mods;
        /** The level the player chose (1 or 2), for a patch that does more on level 2. */
        final int level;

        Ctx(ClassLoader loader, Mods mods, int level) {
            this.loader = loader;
            this.mods = mods;
            this.level = level;
        }
    }
}
