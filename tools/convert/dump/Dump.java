// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//
// Dumps the identifier tables of Chunker's Java and Bedrock resolvers as plain text, one directory per version.
// The resolvers are executed unmodified (all loops, version gates and builders are evaluated by Java itself) and
// their final lookup maps are read through reflection. See tools/convert/README.md.
//
// Usage: java -cp <classes>:<jars> Dump <outDir> <edition: java|bedrock> <version> [<version>...]

import com.hivemc.chunker.conversion.encoding.base.Version;
import com.hivemc.chunker.conversion.encoding.base.resolver.identifier.ChunkerBlockIdentifierResolver;
import com.hivemc.chunker.conversion.encoding.base.resolver.identifier.ChunkerItemIdentifierResolver;
import com.hivemc.chunker.conversion.encoding.base.resolver.identifier.state.StateMapping;
import com.hivemc.chunker.conversion.encoding.base.resolver.identifier.state.StateMappingGroup;
import com.hivemc.chunker.conversion.encoding.base.resolver.identifier.state.TypeMapping;
import com.hivemc.chunker.conversion.encoding.base.resolver.identifier.state.VersionedStateMappingGroup;
import com.hivemc.chunker.conversion.intermediate.column.chunk.identifier.type.block.ChunkerVanillaBlockType;
import com.hivemc.chunker.conversion.intermediate.column.chunk.identifier.type.block.states.BlockState;
import com.hivemc.chunker.conversion.intermediate.column.chunk.identifier.type.block.states.vanilla.VanillaBlockStates;
import com.hivemc.chunker.util.InvertibleMap;

import java.io.IOException;
import java.io.PrintWriter;
import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.*;

public class Dump {
    static final String P = "com.hivemc.chunker.conversion.encoding.";

    // Names of the shared constants, found by reflection over the *StateGroups / *StateTypes classes.
    static final IdentityHashMap<Object, String> GROUP_NAMES = new IdentityHashMap<>();
    static final IdentityHashMap<Object, String> TYPE_NAMES = new IdentityHashMap<>();
    // Every group / type seen in any version (name -> text), written once per edition.
    static final TreeMap<String, String> GROUP_TEXT = new TreeMap<>();
    static final TreeMap<String, String> TYPE_TEXT = new TreeMap<>();
    static final List<Map.Entry<String, BlockState<?>>> STATE_FIELDS = new ArrayList<>();
    static int unnamedGroups = 0, unnamedTypes = 0;

    public static void main(String[] args) throws Exception {
        Path out = Path.of(args[0]);
        String edition = args[1];
        List<String> versions = new ArrayList<>(Arrays.asList(args).subList(2, args.length));
        setupNames(edition);
        dumpPivot(out);
        for (String v : versions) {
            Version version = Version.fromString(v);
            Path dir = out.resolve(edition).resolve(v);
            Files.createDirectories(dir);
            if (edition.equals("java")) {
                dumpJava(version, dir);
            } else {
                dumpBedrock(version, dir);
            }
        }
        Path ed = out.resolve(edition);
        Files.createDirectories(ed);
        write(ed.resolve("groups.tsv"), GROUP_TEXT.values());
        write(ed.resolve("types.tsv"), TYPE_TEXT.values());
        System.err.println("unnamed groups " + unnamedGroups + ", unnamed types " + unnamedTypes);
    }

    // ---- names ----------------------------------------------------------------------------------------------

    static void setupNames(String edition) throws Exception {
        for (Field f : VanillaBlockStates.class.getDeclaredFields()) {
            if (Modifier.isStatic(f.getModifiers()) && BlockState.class.isAssignableFrom(f.getType())) {
                STATE_FIELDS.add(Map.entry(f.getName(), (BlockState<?>) f.get(null)));
            }
        }
        String[] classes = edition.equals("java")
                ? new String[]{P + "java.base.resolver.identifier.JavaStateGroups", P + "java.base.resolver.identifier.legacy.JavaLegacyStateGroups",
                P + "java.base.resolver.identifier.JavaBlockIdentifierResolver", P + "java.base.resolver.identifier.legacy.JavaLegacyBlockIdentifierResolver"}
                : new String[]{P + "bedrock.base.resolver.identifier.BedrockStateGroups", P + "bedrock.base.resolver.identifier.legacy.BedrockLegacyStateGroups",
                P + "bedrock.base.resolver.identifier.BedrockBlockIdentifierResolver", P + "bedrock.base.resolver.identifier.legacy.BedrockLegacyBlockIdentifierResolver"};
        String[] types = edition.equals("java")
                ? new String[]{P + "java.base.resolver.identifier.JavaStateTypes", P + "java.base.resolver.identifier.legacy.JavaLegacyStateTypes"}
                : new String[]{P + "bedrock.base.resolver.identifier.BedrockStateTypes", P + "bedrock.base.resolver.identifier.legacy.BedrockLegacyStateTypes"};
        // Plain constants first so that they win over the nested members of versioned groups.
        List<Object[]> versioned = new ArrayList<>();
        for (String cn : classes) {
            Class<?> c = Class.forName(cn);
            String pre = cn.contains("Legacy") ? "legacy:" : "";
            for (Field f : c.getDeclaredFields()) {
                if (!Modifier.isStatic(f.getModifiers())) continue;
                f.setAccessible(true);
                Object o = f.get(null);
                if (o instanceof StateMappingGroup g) {
                    GROUP_NAMES.putIfAbsent(g, pre + f.getName());
                } else if (o instanceof VersionedStateMappingGroup w) {
                    versioned.add(new Object[]{pre + f.getName(), w});
                }
            }
        }
        for (Object[] vw : versioned) {
            String name = (String) vw[0];
            Object impl = vw[1];
            Object def = field(impl, "defaultStateMappingGroup");
            if (def != null) GROUP_NAMES.putIfAbsent(def, name + "@default");
            SortedMap<Version, StateMappingGroup> map = castMap(field(impl, "versions"));
            for (Map.Entry<Version, StateMappingGroup> e : map.entrySet()) {
                Version v = e.getKey();
                GROUP_NAMES.putIfAbsent(e.getValue(), name + "@" + v.getMajor() + "." + v.getMinor() + "." + v.getPatch());
            }
        }
        for (String cn : types) {
            Class<?> c = Class.forName(cn);
            String pre = cn.contains("Legacy") ? "legacy:" : "";
            for (Field f : c.getDeclaredFields()) {
                if (!Modifier.isStatic(f.getModifiers())) continue;
                f.setAccessible(true);
                Object o = f.get(null);
                if (o instanceof TypeMapping<?, ?> t) TYPE_NAMES.putIfAbsent(t, pre + f.getName());
            }
        }
    }

    @SuppressWarnings("unchecked")
    static <T> T castMap(Object o) {
        return (T) o;
    }

    static Object field(Object o, String name) throws Exception {
        for (Class<?> c = o.getClass(); c != null; c = c.getSuperclass()) {
            try {
                Field f = c.getDeclaredField(name);
                f.setAccessible(true);
                return f.get(o);
            } catch (NoSuchFieldException ignored) {
            }
        }
        throw new NoSuchFieldException(name + " in " + o.getClass());
    }

    static String stateName(BlockState<?> s) {
        for (Map.Entry<String, BlockState<?>> e : STATE_FIELDS) if (e.getValue() == s) return e.getKey();
        for (Map.Entry<String, BlockState<?>> e : STATE_FIELDS) if (e.getValue().equals(s)) return e.getKey();
        throw new IllegalStateException("unknown state " + s.getName());
    }

    // ---- value formatting -----------------------------------------------------------------------------------

    static String chk(String s) {
        if (s.indexOf('\t') >= 0 || s.indexOf('\n') >= 0) throw new IllegalStateException("bad char in " + s);
        return s;
    }

    /** Typed raw value: b:true, i:5, s:text; enums by name. */
    static String val(Object o) {
        if (o == null) return "~";
        if (o instanceof Boolean b) return "b:" + b;
        if (o instanceof Integer i) return "i:" + i;
        if (o instanceof Short i) return "i:" + i;
        if (o instanceof Byte i) return "i:" + i;
        if (o instanceof Long i) return "l:" + i;
        if (o instanceof String s) {
            if (s.contains(",") || s.contains("=") || s.contains("|")) throw new IllegalStateException("bad char in value " + s);
            return "s:" + chk(s);
        }
        if (o instanceof Enum<?> e) return e.name();
        return "?:" + chk(String.valueOf(o));
    }

    static String id(Object o) {
        if (o instanceof String s) return chk(s);
        if (o instanceof Enum<?> e) {
            String kind = o.getClass().getSimpleName().contains("Item") ? "I:" : "B:";
            return kind + e.name();
        }
        if (o instanceof BlockState<?> b) return stateName(b);
        if (o instanceof com.hivemc.chunker.conversion.intermediate.column.chunk.itemstack.ChunkerItemProperty<?> p) return "P:" + p.getName();
        return chk(String.valueOf(o));
    }

    /** Block identifiers: edition ids as they are, pivot block types by constant name. */
    static String bid(Object o) {
        return o instanceof Enum<?> e ? e.name() : id(o);
    }

    static String key(Object o) {
        if (o instanceof BlockState<?> b) return "S:" + stateName(b);
        if (o instanceof com.hivemc.chunker.conversion.intermediate.column.chunk.itemstack.ChunkerItemProperty<?> p) return "P:" + p.getName();
        return chk(String.valueOf(o));
    }

    static String list(Collection<?> c) {
        if (c == null) return "~";
        StringJoiner j = new StringJoiner("|");
        for (Object o : c) j.add(val(o));
        return j.toString();
    }

    // ---- pivot ----------------------------------------------------------------------------------------------

    static void dumpPivot(Path out) throws IOException {
        Files.createDirectories(out);
        List<String> states = new ArrayList<>();
        for (Map.Entry<String, BlockState<?>> e : STATE_FIELDS) {
            BlockState<?> s = e.getValue();
            StringJoiner vals = new StringJoiner(",");
            for (Object v : s.getValues()) vals.add(((Enum<?>) v).name());
            states.add("S\t" + e.getKey() + "\t" + s.getName() + "\t" + ((Enum<?>) s.getDefault()).name() + "\t" + vals);
        }
        List<String> types = new ArrayList<>();
        for (ChunkerVanillaBlockType t : ChunkerVanillaBlockType.values()) {
            TreeSet<String> names = new TreeSet<>();
            for (BlockState<?> s : t.getStates()) names.add(stateName(s));
            types.add("T\t" + t.name() + "\t" + String.join(",", names));
        }
        write(out.resolve("pivot_states.tsv"), states);
        write(out.resolve("pivot_types.tsv"), types);
    }

    // ---- state groups / type mappings -----------------------------------------------------------------------

    static String typeName(TypeMapping<?, ?> t) throws Exception {
        String text = typeText(t);
        String name = TYPE_NAMES.get(t);
        if (name == null) {
            name = "T#" + sha(text);
            unnamedTypes++;
        }
        String full = "T\t" + name + "\n" + text;
        String old = TYPE_TEXT.putIfAbsent(name, full);
        if (old != null && !old.equals(full)) throw new IllegalStateException("type " + name + " differs between uses");
        return name;
    }

    static String typeText(TypeMapping<?, ?> t) {
        StringBuilder sb = new StringBuilder();
        sb.append("A\t").append(t.getInputTypes().size()).append("\t").append(t.getOutputTypes().size()).append("\n");
        TreeSet<String> rows = new TreeSet<>();
        for (Map.Entry<?, ?> e : t.getInputToOutput().entrySet())
            rows.add("M\t" + list((Collection<?>) e.getKey()) + "\t" + list((Collection<?>) e.getValue()));
        for (Map.Entry<?, ?> e : t.getOutputToInput().entrySet())
            rows.add("R\t" + list((Collection<?>) e.getValue()) + "\t" + list((Collection<?>) e.getKey()));
        if (t.getDefaultInput() != null || t.getDefaultOutput() != null)
            rows.add("D\t" + list(t.getDefaultInput()) + "\t" + list(t.getDefaultOutput()));
        for (String r : rows) sb.append(r).append("\n");
        return sb.toString().stripTrailing();
    }

    static String sha(String text) throws Exception {
        byte[] d = java.security.MessageDigest.getInstance("SHA-1").digest(text.getBytes(StandardCharsets.UTF_8));
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < 6; i++) sb.append(String.format("%02x", d[i]));
        return sb.toString();
    }

    static String groupName(StateMappingGroup g) throws Exception {
        if (g == null) return "-";
        String text = groupText(g);
        String name = GROUP_NAMES.get(g);
        if (name == null) {
            name = "G#" + sha(text);
            unnamedGroups++;
        }
        String full = "G\t" + name + "\n" + text;
        String old = GROUP_TEXT.putIfAbsent(name, full);
        if (old != null && !old.equals(full)) throw new IllegalStateException("group " + name + " differs between versions");
        return name;
    }

    @SuppressWarnings("unchecked")
    static String groupText(StateMappingGroup g) throws Exception {
        StringBuilder sb = new StringBuilder();
        List<StateMapping<?, ?>> states = (List<StateMapping<?, ?>>) field(g, "states");
        for (StateMapping<?, ?> sm : states) {
            StringJoiner outs = new StringJoiner(",");
            for (BlockState<?> b : sm.getOutputBlockStates()) outs.add(stateName(b));
            sb.append("S\t").append(String.join(",", sm.getInputStateNames())).append("\t").append(outs).append("\t")
                    .append(typeName(sm.getTypeMapping())).append("\n");
        }
        Map<String, Object> di = (Map<String, Object>) field(g, "defaultInputs");
        for (String k : new TreeSet<>(di.keySet())) sb.append("DI\t").append(chk(k)).append("\t").append(val(di.get(k))).append("\n");
        Map<BlockState<?>, Object> dout = (Map<BlockState<?>, Object>) field(g, "defaultOutputs");
        TreeMap<String, String> sorted = new TreeMap<>();
        for (Map.Entry<BlockState<?>, Object> e : dout.entrySet()) sorted.put(stateName(e.getKey()), val(e.getValue()));
        for (Map.Entry<String, String> e : sorted.entrySet()) sb.append("DO\t").append(e.getKey()).append("\t").append(e.getValue()).append("\n");
        return sb.toString().stripTrailing();
    }

    // ---- block / item identifier tables ---------------------------------------------------------------------

    static Object construct(String className, Object... args) throws Exception {
        Class<?> c = Class.forName(className);
        for (Constructor<?> k : c.getConstructors()) {
            Class<?>[] pt = k.getParameterTypes();
            if (pt.length != args.length) continue;
            boolean ok = true;
            for (int i = 0; i < pt.length; i++) {
                if (args[i] != null && !wrap(pt[i]).isInstance(args[i])) ok = false;
            }
            if (ok) return k.newInstance(args);
        }
        throw new NoSuchMethodException(className + " with " + args.length + " args");
    }

    static Class<?> wrap(Class<?> c) {
        if (c == boolean.class) return Boolean.class;
        if (c == int.class) return Integer.class;
        return c;
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    static void dumpBlockResolver(ChunkerBlockIdentifierResolver r, Path file) throws Exception {
        TreeSet<String> rows = new TreeSet<>();
        for (String dir : new String[]{"I", "C"}) {
            Map<Object, Object> map = (Map<Object, Object>) field(r, dir.equals("I") ? "inputToChunker" : "chunkerToInput");
            for (Map.Entry<Object, Object> e : map.entrySet()) {
                ChunkerBlockIdentifierResolver.InputStateGrouping<?, ?, ?, ?, ?> g = (ChunkerBlockIdentifierResolver.InputStateGrouping<?, ?, ?, ?, ?>) e.getValue();
                for (Map.Entry<SortedSet<Object>, Map<List<Object>, ChunkerBlockIdentifierResolver.OutputMapping<Object, Object, Object>>> grp
                        : ((Map<SortedSet<Object>, Map<List<Object>, ChunkerBlockIdentifierResolver.OutputMapping<Object, Object, Object>>>) (Map) g.getGroups()).entrySet()) {
                    List<Object> names = new ArrayList<>(grp.getKey());
                    for (Map.Entry<List<Object>, ChunkerBlockIdentifierResolver.OutputMapping<Object, Object, Object>> m : grp.getValue().entrySet()) {
                        StringJoiner ins = new StringJoiner(",");
                        for (int i = 0; i < names.size(); i++) {
                            Object n = names.get(i);
                            String nn = n instanceof BlockState<?> b ? stateName(b) : (String) n;
                            Object v = m.getKey().get(i);
                            ins.add(nn + "=" + (dir.equals("C") ? ((Enum<?>) v).name() : val(v)));
                        }
                        ChunkerBlockIdentifierResolver.OutputMapping<Object, Object, Object> om = m.getValue();
                        TreeMap<String, String> outs = new TreeMap<>();
                        for (Map.Entry<Object, Object> o : om.getOutputs().entrySet()) {
                            Object k = o.getKey();
                            String kn = k instanceof BlockState<?> b ? stateName(b) : (String) k;
                            outs.put(kn, dir.equals("I") ? ((Enum<?>) o.getValue()).name() : val(o.getValue()));
                        }
                        StringJoiner os = new StringJoiner(",");
                        for (Map.Entry<String, String> o : outs.entrySet()) os.add(o.getKey() + "=" + o.getValue());
                        rows.add(dir + "\t" + bid(e.getKey()) + "\t" + ins + "\t" + bid(om.getIdentifier()) + "\t" + os + "\t"
                                + groupName(om.getGroup()));
                    }
                }
            }
        }
        Object extra = field(r, "extraStateMappingGroup");
        rows.add("X\t" + (extra == null ? "-" : groupName((StateMappingGroup) extra)));
        write(file, rows);
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    static void dumpItemResolver(ChunkerItemIdentifierResolver r, Path file) throws Exception {
        TreeSet<String> rows = new TreeSet<>();
        for (String dir : new String[]{"I", "C"}) {
            Map<Object, Object> map = (Map<Object, Object>) field(r, dir.equals("I") ? "inputToChunker" : "chunkerToInput");
            for (Map.Entry<Object, Object> e : map.entrySet()) {
                ChunkerItemIdentifierResolver.InputStateGrouping<?, ?, ?, ?, ?> g = (ChunkerItemIdentifierResolver.InputStateGrouping<?, ?, ?, ?, ?>) e.getValue();
                for (Map.Entry<SortedSet<Object>, Map<List<Object>, ChunkerItemIdentifierResolver.OutputMapping<Object, Object, Object>>> grp
                        : ((Map<SortedSet<Object>, Map<List<Object>, ChunkerItemIdentifierResolver.OutputMapping<Object, Object, Object>>>) (Map) g.getGroups()).entrySet()) {
                    List<Object> names = new ArrayList<>(grp.getKey());
                    for (Map.Entry<List<Object>, ChunkerItemIdentifierResolver.OutputMapping<Object, Object, Object>> m : grp.getValue().entrySet()) {
                        StringJoiner ins = new StringJoiner(",");
                        for (int i = 0; i < names.size(); i++) ins.add(key(names.get(i)) + "=" + val(m.getKey().get(i)));
                        ChunkerItemIdentifierResolver.OutputMapping<Object, Object, Object> om = m.getValue();
                        TreeMap<String, String> outs = new TreeMap<>();
                        for (Map.Entry<Object, Object> o : om.getOutputs().entrySet()) outs.put(key(o.getKey()), val(o.getValue()));
                        StringJoiner os = new StringJoiner(",");
                        for (Map.Entry<String, String> o : outs.entrySet()) os.add(o.getKey() + "=" + o.getValue());
                        rows.add(dir + "\t" + id(e.getKey()) + "\t" + ins + "\t" + id(om.getIdentifier()) + "\t" + os);
                    }
                }
            }
        }
        write(file, rows);
    }

    // ---- simple tables --------------------------------------------------------------------------------------

    static void dumpSimple(String className, Path dir, String label, Object... args) {
        try {
            Object r = construct(className, args);
            TreeSet<String> rows = new TreeSet<>();
            if (r.getClass().getSimpleName().equals("JavaMapColorsResolver")) {
                @SuppressWarnings("unchecked") List<java.awt.Color> colors = (List<java.awt.Color>) field(r, "mapColors");
                for (int i = 0; i < colors.size(); i++) {
                    java.awt.Color c = colors.get(i);
                    rows.add(String.format("C\t%03d\t%d,%d,%d,%d", i, c.getRed(), c.getGreen(), c.getBlue(), c.getAlpha()));
                }
            } else {
                InvertibleMap<?, ?> im = (InvertibleMap<?, ?>) field(r, "mapping");
                Map<Object, Object> fwd = castMap(im.forward());
                Map<Object, Object> inv = castMap(im.inverse());
                for (Map.Entry<Object, Object> e : fwd.entrySet()) rows.add("F\t" + val(e.getKey()) + "\t" + val(e.getValue()));
                // The inverse is dumped completely: when several keys share a value, only the last put wins there.
                for (Map.Entry<Object, Object> e : inv.entrySet()) rows.add("I\t" + val(e.getKey()) + "\t" + val(e.getValue()));
            }
            write(dir.resolve("simple_" + label + ".tsv"), rows);
        } catch (Throwable t) {
            System.err.println("SKIP " + label + " in " + dir + ": " + t);
        }
    }

    static void dumpBlocks(String className, Version v, Path file) {
        try {
            dumpBlockResolver((ChunkerBlockIdentifierResolver) construct(className, null, v, true, false), file);
        } catch (Throwable t) {
            System.err.println("SKIP " + file + ": " + t + (t.getCause() != null ? " / " + t.getCause() : ""));
        }
    }

    static void dumpItems(String className, Version v, Path file) {
        try {
            dumpItemResolver((ChunkerItemIdentifierResolver) construct(className, null, v, true), file);
        } catch (Throwable t) {
            System.err.println("SKIP " + file + ": " + t + (t.getCause() != null ? " / " + t.getCause() : ""));
        }
    }

    static void dumpJava(Version v, Path dir) {
        String b = P + "java.base.resolver.";
        dumpBlocks(b + "identifier.JavaBlockIdentifierResolver", v, dir.resolve("blocks.tsv"));
        dumpItems(b + "identifier.JavaItemIdentifierResolver", v, dir.resolve("items.tsv"));
        dumpBlocks(b + "identifier.legacy.JavaLegacyBlockIdentifierResolver", v, dir.resolve("legacy_blocks.tsv"));
        dumpItems(b + "identifier.legacy.JavaLegacyItemIdentifierResolver", v, dir.resolve("legacy_items.tsv"));
        String[] simple = {
                "biome.JavaNamedBiomeResolver:biome_name:b", "biome.JavaBiomeIDResolver:biome_id",
                "entity.JavaEntityTypeResolver:entity_type:b", "entity.legacy.JavaLegacyEntityTypeResolver:legacy_entity_type",
                "entity.legacy.JavaLegacyEntityTypeIDResolver:legacy_entity_id", "entity.JavaPaintingMotiveResolver:painting",
                "entity.legacy.JavaLegacyPaintingMotiveResolver:legacy_painting",
                "itemstack.JavaEffectResolver:effect", "itemstack.JavaEffectIDResolver:effect_id",
                "itemstack.JavaEnchantmentResolver:enchantment", "itemstack.JavaEnchantmentIDResolver:enchantment_id",
                "itemstack.JavaHornInstrumentResolver:horn", "itemstack.JavaPotionTypeResolver:potion",
                "itemstack.legacy.JavaLegacyPotionIDResolver:legacy_potion_id",
                "itemstack.JavaMapDecorationResolver:map_decoration", "itemstack.JavaMapDecorationIDResolver:map_decoration_id",
                "itemstack.JavaTrimPatternResolver:trim_pattern", "itemstack.JavaTrimMaterialResolver:trim_material",
                "itemstack.JavaBannerPatternResolver:banner_pattern", "itemstack.JavaBannerPatternShortNameResolver:banner_short",
                "itemstack.JavaMapColorsResolver:map_colors",
                "identifier.legacy.JavaLegacyBlockIDResolver:legacy_block_id", "identifier.legacy.JavaLegacyItemIDResolver:legacy_item_id",
        };
        simple(simple, b, v, dir);
    }

    static void dumpBedrock(Version v, Path dir) {
        String b = P + "bedrock.base.resolver.";
        dumpBlocks(b + "identifier.BedrockBlockIdentifierResolver", v, dir.resolve("blocks.tsv"));
        dumpItems(b + "identifier.BedrockItemIdentifierResolver", v, dir.resolve("items.tsv"));
        dumpBlocks(b + "identifier.legacy.BedrockLegacyBlockIdentifierResolver", v, dir.resolve("legacy_blocks.tsv"));
        String[] simple = {
                "biome.BedrockBiomeIDResolver:biome_id:b", "entity.BedrockEntityTypeResolver:entity_type:b",
                "entity.BedrockEntityTypeIDResolver:entity_id", "entity.BedrockPaintingMotiveResolver:painting",
                "itemstack.BedrockEffectIDResolver:effect_id", "itemstack.BedrockEnchantmentIDResolver:enchantment_id",
                "itemstack.BedrockPotionIDResolver:potion_id", "itemstack.BedrockTrimPatternResolver:trim_pattern",
                "itemstack.BedrockTrimMaterialResolver:trim_material", "itemstack.BedrockBannerPatternResolver:banner_pattern",
        };
        simple(simple, b, v, dir);
    }

    static void simple(String[] specs, String base, Version v, Path dir) {
        for (String spec : specs) {
            String[] p = spec.split(":");
            if (p.length > 2) dumpSimple(base + p[0], dir, p[1], v, false);
            else dumpSimple(base + p[0], dir, p[1], v);
        }
    }

    static void write(Path file, Collection<String> lines) throws IOException {
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(file, StandardCharsets.UTF_8))) {
            for (String l : lines) {
                w.print(l);
                w.print('\n');
            }
        }
    }
}
