package com.blockforge.lite.core;

import java.util.Collections;
import java.util.HashSet;
import java.util.Locale;
import java.util.Set;

/**
 * The agent's options, comma separated:
 * <pre>
 *   quiet            no summary line (problems are still printed)
 *   off=ID;ID        these patches stay off
 *   only=ID;ID       only these patches (any level)
 *   level=1|2        1 = the conservative set (the default), 2 = also the balanced patches
 *   report=FILE      write one line per patch: id, applied/skipped/failed, class, reason
 *   mods=FILE        the mods of this game, one file name or mod id per line, given by the launcher
 *   modids=ID;ID     the same, inline
 *   targets=FILE     lines "patch-id runtime/class/Name": where the launcher knows a patch's target class, only that
 *                    class is looked at for it (saves looking at the classes of an obfuscated game)
 *   strict           only classes whose runtime name is Mojang's (NeoForge, Forge 1.17+, 26.x) or that targets= names,
 *                    and only Minecraft 1.17+ class files; without it, obfuscated (vanilla) and intermediary
 *                    (Fabric/Quilt) names are matched by the code's shape too
 *   verbose          print every decision as it happens
 * </pre>
 * Unknown options are ignored, so an older agent works with a newer launcher.
 */
final class Options {
    final boolean quiet;
    final boolean verbose;
    final Set<String> off;
    final Set<String> only;
    final int level;
    final String report;
    final String mods;
    final Set<String> modIds;
    final String targets;
    final boolean strict;

    Options(String args) {
        boolean q = false;
        boolean v = false;
        Set<String> offs = new HashSet<String>();
        Set<String> onlys = null;
        int lvl = 1;
        String rep = null;
        String md = null;
        String tg = null;
        boolean st = false;
        Set<String> mids = new HashSet<String>();
        for (String part : (args == null ? "" : args).split(",")) {
            String p = part.trim();
            int eq = p.indexOf('=');
            String key = (eq < 0 ? p : p.substring(0, eq)).trim().toLowerCase(Locale.ROOT);
            String value = eq < 0 ? "" : p.substring(eq + 1).trim();
            if (key.equals("quiet")) {
                q = true;
            } else if (key.equals("verbose")) {
                v = true;
            } else if (key.equals("off")) {
                offs.addAll(ids(value));
            } else if (key.equals("only")) {
                if (onlys == null) {
                    onlys = new HashSet<String>();
                }
                onlys.addAll(ids(value));
            } else if (key.equals("level")) {
                lvl = value.equals("2") ? 2 : 1;
            } else if (key.equals("report") && !value.isEmpty()) {
                rep = value;
            } else if (key.equals("mods") && !value.isEmpty()) {
                md = value;
            } else if (key.equals("modids")) {
                mids.addAll(ids(value));
            } else if (key.equals("targets") && !value.isEmpty()) {
                tg = value;
            } else if (key.equals("strict")) {
                st = true;
            }
        }
        modIds = Collections.unmodifiableSet(mids);
        targets = tg;
        strict = st;
        quiet = q;
        verbose = v && !q;
        off = Collections.unmodifiableSet(offs);
        only = onlys == null ? null : Collections.unmodifiableSet(onlys);
        level = lvl;
        report = rep;
        mods = md;
    }

    private static Set<String> ids(String list) {
        Set<String> s = new HashSet<String>();
        for (String id : list.split("[;:\\s]+")) {
            if (!id.isEmpty()) {
                s.add(id.toLowerCase(Locale.ROOT));
            }
        }
        return s;
    }

    /** Null when the patch is on, else why it is off. */
    String whyOff(Patch p) {
        if (off.contains(p.id)) {
            return "switched off (off=)";
        }
        if (only != null) {
            return only.contains(p.id) ? null : "not in only=";
        }
        if (p.level > level) {
            return "needs level=" + p.level;
        }
        return null;
    }
}
