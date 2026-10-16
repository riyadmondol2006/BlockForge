package com.blockforge.lite.core;

import java.io.BufferedReader;
import java.io.File;
import java.io.FileInputStream;
import java.io.InputStreamReader;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.WeakHashMap;

/**
 * Finds the performance mods that already do what a patch does, so the patch stays off instead of doing the work
 * twice or fighting with the mod. A mod counts as installed when any of these says so:
 *   - the list the launcher passed with mods=FILE (file names or mod ids),
 *   - the file names in the game's mods folder,
 *   - one of the mod's classes can be found by the class loader of the class being patched (this also finds mods
 *     that came inside a modpack's jar or were installed some other way).
 */
final class Mods {
    /** Key, name for the report, words of its file names / mod ids, classes it ships. */
    static final String[][] KNOWN = {
        {"sodium", "Sodium", "sodium", "net/caffeinemc/mods/sodium/client/SodiumClientMod.class",
            "me/jellysquid/mods/sodium/client/SodiumClientMod.class"},
        {"embeddium", "Embeddium", "embeddium", "org/embeddedt/embeddium/impl/Embeddium.class"},
        {"rubidium", "Rubidium", "rubidium", "me/jellysquid/mods/rubidium/RubidiumMod.class"},
        {"lithium", "Lithium", "lithium", "net/caffeinemc/mods/lithium/common/LithiumMod.class",
            "me/jellysquid/mods/lithium/common/LithiumMod.class"},
        {"radium", "Radium", "radium", "me/jellysquid/mods/lithium/common/LithiumMod.class"},
        {"canary", "Canary", "canary", "com/abdelaziz/canary/common/CanaryMod.class"},
        {"ferritecore", "FerriteCore", "ferritecore", "malte0811/ferritecore/ducks/FastMapStateHolder.class"},
        {"modernfix", "ModernFix", "modernfix", "org/embeddedt/modernfix/ModernFix.class"},
        {"immediatelyfast", "ImmediatelyFast", "immediatelyfast", "net/raphimc/immediatelyfast/ImmediatelyFast.class"},
        {"entityculling", "EntityCulling", "entityculling", "dev/tr7zw/entityculling/EntityCullingModBase.class"},
        {"dynamicfps", "Dynamic FPS", "dynamicfps", "dynamic_fps/impl/DynamicFPSMod.class",
            "net/lostluma/dynamic_fps/impl/DynamicFPSMod.class"},
        {"fpsreducer", "FPS Reducer", "fpsreducer", "bre2el/fpsreducer/FpsReducer.class"},
        {"moreculling", "MoreCulling", "moreculling", "ca/fxco/moreculling/MoreCulling.class"},
        {"c2me", "C2ME", "c2me", "com/ishland/c2me/base/ModuleEntryPoint.class"},
        {"starlight", "Starlight", "starlight", "ca/spottedleaf/starlight/common/light/StarLightEngine.class"},
        {"nvidium", "Nvidium", "nvidium", "me/cortex/nvidium/Nvidium.class"},
        {"optifine", "OptiFine", "optifine", "net/optifine/Config.class", "optifine/OptiFineClassTransformer.class",
            "Config.class"},
        {"vulkanmod", "VulkanMod", "vulkanmod", "net/vulkanmod/Initializer.class"},
        {"iris", "Iris", "iris", "net/irisshaders/iris/Iris.class", "net/coderbot/iris/Iris.class"},
        {"oculus", "Oculus", "oculus"},
        {"distanthorizons", "Distant Horizons", "distanthorizons"},
        {"scalablelux", "ScalableLux", "scalablelux"},
        {"bobby", "Bobby", "bobby"},
        {"krypton", "Krypton", "krypton"},
        {"soundphysics", "Sound Physics Remastered", "soundphysics"},
        {"ambientsounds", "AmbientSounds", "ambientsounds"},
        {"presencefootsteps", "Presence Footsteps", "presencefootsteps"},
    };

    private final String listFile;
    private final java.util.Set<String> ids;
    private final boolean idsGiven;
    private List<String> names;
    private Boolean noneListed;
    private final Map<ClassLoader, Map<String, Boolean>> probed = new WeakHashMap<ClassLoader, Map<String, Boolean>>();

    Mods(String listFile, java.util.Set<String> ids) {
        this(listFile, ids, false);
    }

    /** idsGiven: modids= was passed (an empty one means the launcher says there are no mods). */
    Mods(String listFile, java.util.Set<String> ids, boolean idsGiven) {
        this.listFile = listFile;
        this.ids = ids;
        this.idsGiven = idsGiven;
    }

    /**
     * Whether the launcher said there are no mods: a mods= list (or modids=) was given and it is empty. Any entry
     * counts as a mod, also a loader's own API library. Without a list, or when it can't be read, the answer is no.
     */
    synchronized boolean noneListed() {
        if (noneListed == null) {
            boolean none = (listFile != null || idsGiven) && ids.isEmpty();
            if (none && listFile != null) {
                try {
                    BufferedReader r = new BufferedReader(new InputStreamReader(new FileInputStream(listFile),
                            "UTF-8"));
                    try {
                        for (String line; (line = r.readLine()) != null && none; ) {
                            none = line.trim().isEmpty();
                        }
                    } finally {
                        r.close();
                    }
                } catch (Throwable e) {
                    none = false;
                }
            }
            noneListed = none;
        }
        return noneListed;
    }

    /** The name of the first of these mods that is installed, or null. */
    synchronized String installed(String[] keys, ClassLoader loader) {
        for (String key : keys) {
            String[] mod = find(key);
            if (mod == null) {
                continue;
            }
            if (listed(mod[2]) || probe(mod, loader)) {
                return mod[1];
            }
        }
        return null;
    }

    private static String[] find(String key) {
        for (String[] m : KNOWN) {
            if (m[0].equals(key)) {
                return m;
            }
        }
        return null;
    }

    private boolean listed(String word) {
        if (names == null) {
            names = readNames();
        }
        for (String n : names) {
            if (n.contains(word)) {
                return true;
            }
        }
        return false;
    }

    private boolean probe(String[] mod, ClassLoader loader) {
        if (loader == null) {
            return false;
        }
        Map<String, Boolean> seen = probed.get(loader);
        if (seen == null) {
            seen = new HashMap<String, Boolean>();
            probed.put(loader, seen);
        }
        Boolean known = seen.get(mod[0]);
        if (known == null) {
            known = Boolean.FALSE;
            for (int i = 3; i < mod.length && !known; i++) {
                // "Config.class" alone would match other things; old OptiFine is also listed by file name
                if (mod[i].indexOf('/') < 0) {
                    continue;
                }
                try {
                    known = loader.getResource(mod[i]) != null;
                } catch (Throwable e) {
                    // a loader that can't answer: count the mod as missing
                }
            }
            seen.put(mod[0], known);
        }
        return known;
    }

    /** Lower-case names with only letters and digits ("Dynamic-FPS-3.4.jar" -> "dynamicfps34jar"). */
    private List<String> readNames() {
        List<String> out = new ArrayList<String>();
        for (String id : ids) {
            out.add(simple(id));
        }
        try {
            if (listFile != null) {
                BufferedReader r = new BufferedReader(new InputStreamReader(new FileInputStream(listFile), "UTF-8"));
                try {
                    for (String line; (line = r.readLine()) != null; ) {
                        if (!line.trim().isEmpty()) {
                            out.add(simple(line));
                        }
                    }
                } finally {
                    r.close();
                }
            }
        } catch (Throwable e) {
            Engine.problem("could not read the mod list " + listFile + ": " + e);
        }
        for (File dir : modFolders()) {
            String[] files = dir.list();
            if (files == null) {
                continue;
            }
            for (String f : files) {
                String l = f.toLowerCase(Locale.ROOT);
                if (l.endsWith(".jar") || l.endsWith(".zip") || l.endsWith(".litemod")) {
                    out.add(simple(f));
                }
            }
        }
        return out;
    }

    static String simple(String s) {
        StringBuilder b = new StringBuilder(s.length());
        for (char c : s.toLowerCase(Locale.ROOT).toCharArray()) {
            if ((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9')) {
                b.append(c);
            }
        }
        return b.toString();
    }

    /** The mods folder of the game directory (--gameDir, else the working directory). */
    private static List<File> modFolders() {
        List<File> dirs = new ArrayList<File>();
        try {
            String cmd = System.getProperty("sun.java.command", "");
            int i = cmd.indexOf("--gameDir ");
            if (i >= 0) {
                String rest = cmd.substring(i + 10).trim();
                // the path ends at the next option; it may contain spaces
                int next = rest.indexOf(" --");
                String dir = next < 0 ? rest : rest.substring(0, next);
                dirs.add(new File(dir.trim(), "mods"));
            }
            dirs.add(new File(System.getProperty("user.dir", "."), "mods"));
        } catch (Throwable ignored) {
            // no folder then
        }
        return dirs;
    }
}
