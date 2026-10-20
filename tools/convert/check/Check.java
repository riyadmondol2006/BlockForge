// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//
// Differential oracle for the Rust engine: runs Chunker's real block resolvers (to(...) / from(...)) on a list of
// cases and prints the answers. Usage: java -cp <classes>:<jars> Check <cases.tsv> <answers.tsv>
//
// Case line:  <mode> <version> <dir> <id or type> <states>
//   mode java|bedrock|java_legacy|bedrock_legacy; dir T (edition -> pivot, states typed b:/i:/s: pairs)
//   or F (pivot -> edition, id is the pivot type, states FIELD=CONST pairs).
// Answer line: the case line plus a tab and `NONE` or `<id or type>|<pairs>` (pairs sorted by name).

import com.hivemc.chunker.conversion.encoding.base.Converter;
import com.hivemc.chunker.conversion.encoding.base.Version;
import com.hivemc.chunker.conversion.encoding.base.resolver.identifier.ChunkerBlockIdentifierResolver;
import com.hivemc.chunker.conversion.intermediate.column.chunk.identifier.ChunkerBlockIdentifier;
import com.hivemc.chunker.conversion.intermediate.column.chunk.identifier.type.block.ChunkerVanillaBlockType;
import com.hivemc.chunker.conversion.intermediate.column.chunk.identifier.type.block.states.BlockState;
import com.hivemc.chunker.conversion.intermediate.column.chunk.identifier.type.block.states.BlockStateValue;
import com.hivemc.chunker.conversion.intermediate.column.chunk.identifier.type.block.states.vanilla.VanillaBlockStates;
import com.hivemc.chunker.mapping.identifier.Identifier;
import com.hivemc.chunker.mapping.identifier.states.StateValue;
import com.hivemc.chunker.mapping.identifier.states.StateValueBoolean;
import com.hivemc.chunker.mapping.identifier.states.StateValueInt;
import com.hivemc.chunker.mapping.identifier.states.StateValueString;

import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.lang.reflect.Proxy;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.*;

public class Check {
    static final String P = "com.hivemc.chunker.conversion.encoding.";
    static final Map<String, BlockState<?>> STATES = new HashMap<>();
    static final Map<String, ChunkerBlockIdentifierResolver> RESOLVERS = new HashMap<>();
    static final Converter CONVERTER = (Converter) Proxy.newProxyInstance(Check.class.getClassLoader(), new Class<?>[]{Converter.class},
            (proxy, method, args) -> {
                Class<?> r = method.getReturnType();
                if (r == boolean.class) return false;
                return null; // getBlockMappings() == null disables user mappings
            });

    public static void main(String[] args) throws Exception {
        for (Field f : VanillaBlockStates.class.getDeclaredFields()) {
            if (Modifier.isStatic(f.getModifiers()) && BlockState.class.isAssignableFrom(f.getType())) {
                STATES.put(f.getName(), (BlockState<?>) f.get(null));
            }
        }
        List<String> out = new ArrayList<>();
        for (String line : Files.readAllLines(Path.of(args[0]), StandardCharsets.UTF_8)) {
            if (line.isEmpty()) continue;
            String[] f = line.split("\t", -1);
            String result;
            try {
                ChunkerBlockIdentifierResolver r = resolver(f[0], f[1]);
                result = f[2].equals("T") ? to(r, f[3], f[4]) : from(r, f[3], f[4]);
            } catch (Throwable t) {
                result = "ERROR " + t;
            }
            out.add(line + "\t" + result);
        }
        Files.write(Path.of(args[1]), out, StandardCharsets.UTF_8);
    }

    static ChunkerBlockIdentifierResolver resolver(String mode, String version) throws Exception {
        String key = mode + " " + version;
        ChunkerBlockIdentifierResolver r = RESOLVERS.get(key);
        if (r == null) {
            String cls = switch (mode) {
                case "java" -> P + "java.base.resolver.identifier.JavaBlockIdentifierResolver";
                case "bedrock" -> P + "bedrock.base.resolver.identifier.BedrockBlockIdentifierResolver";
                case "java_legacy" -> P + "java.base.resolver.identifier.legacy.JavaLegacyBlockIdentifierResolver";
                default -> P + "bedrock.base.resolver.identifier.legacy.BedrockLegacyBlockIdentifierResolver";
            };
            Constructor<?> k = Class.forName(cls).getConstructors()[0];
            r = (ChunkerBlockIdentifierResolver) k.newInstance(CONVERTER, Version.fromString(version), true, false);
            RESOLVERS.put(key, r);
        }
        return r;
    }

    static StateValue<?> typed(String t) {
        String v = t.substring(2);
        return switch (t.charAt(0)) {
            case 'b' -> v.equals("true") ? StateValueBoolean.TRUE : StateValueBoolean.FALSE;
            case 'i' -> new StateValueInt(Integer.parseInt(v));
            default -> new StateValueString(v);
        };
    }

    static String typed(StateValue<?> v) {
        Object o = v.getBoxed();
        if (o instanceof Boolean b) return "b:" + b;
        if (o instanceof Integer i) return "i:" + i;
        return "s:" + o;
    }

    static String to(ChunkerBlockIdentifierResolver r, String id, String states) {
        Map<String, StateValue<?>> m = new HashMap<>();
        if (!states.isEmpty()) {
            for (String p : states.split(",")) {
                int eq = p.indexOf('=');
                m.put(p.substring(0, eq), typed(p.substring(eq + 1)));
            }
        }
        Optional<ChunkerBlockIdentifier> o = r.to(new Identifier(id, m));
        if (o.isEmpty()) return "NONE";
        TreeMap<String, String> out = new TreeMap<>();
        for (Map.Entry<BlockState<?>, BlockStateValue> e : o.get().getPresentStates().entrySet()) {
            out.put(field(e.getKey()), ((Enum<?>) e.getValue()).name());
        }
        return ((Enum<?>) o.get().getType()).name() + "|" + join(out);
    }

    static String from(ChunkerBlockIdentifierResolver r, String type, String states) {
        Map<BlockState<?>, BlockStateValue> m = new HashMap<>();
        if (!states.isEmpty()) {
            for (String p : states.split(",")) {
                int eq = p.indexOf('=');
                BlockState<?> s = STATES.get(p.substring(0, eq));
                BlockStateValue value = null;
                for (BlockStateValue v : s.getValues()) if (((Enum<?>) v).name().equals(p.substring(eq + 1))) value = v;
                if (value == null) throw new IllegalArgumentException("bad value " + p);
                m.put(s, value);
            }
        }
        Optional<Identifier> o = r.from(new ChunkerBlockIdentifier(ChunkerVanillaBlockType.valueOf(type), m));
        if (o.isEmpty()) return "NONE";
        TreeMap<String, String> out = new TreeMap<>();
        for (Map.Entry<String, StateValue<?>> e : o.get().getStates().entrySet()) out.put(e.getKey(), typed(e.getValue()));
        return o.get().getIdentifier() + "|" + join(out);
    }

    static String field(BlockState<?> s) {
        for (Map.Entry<String, BlockState<?>> e : STATES.entrySet()) if (e.getValue() == s) return e.getKey();
        for (Map.Entry<String, BlockState<?>> e : STATES.entrySet()) if (e.getValue().equals(s)) return e.getKey();
        throw new IllegalStateException("unknown state " + s.getName());
    }

    static String join(Map<String, String> m) {
        StringJoiner j = new StringJoiner(",");
        for (Map.Entry<String, String> e : m.entrySet()) j.add(e.getKey() + "=" + e.getValue());
        return j.toString();
    }
}
