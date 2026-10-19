#!/usr/bin/env python3
"""Generate src/convert/model/generated.rs from Chunker's Java enum sources.

Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

Usage (stdlib only, run isolated):

    python3 -I tools/convert/gen_model.py [--chunker PATH] [--out PATH] [--check]

PATH is a clone of HiveGamesOSS/Chunker (default: $CHUNKER_DIR, else ../Chunker next to the repository). Only
`cli/src/main/java` is read. The output is deterministic: running the script twice yields the same bytes. With
--check the file is not written; the exit status says whether it is up to date.

What is generated, all with the Java constant order (ordinals) preserved, because mapping tables enumerate the
cartesian product of state values and rely on it:

* StateId (133 block states), their typed handles, defaults, value names and the 36 state groups;
* the 70 block state value enums;
* BlockType (all vanilla block types) with map colour, solid flag, state set, block entity kind and block groups;
* ItemType, VanillaEntityType, Motive, VanillaBiome and the small item stack enums (dye colour, banner pattern,
  enchantment, effect, potion, firework shape, horn instrument, explorer map, trim material and pattern).
"""

import argparse
import os
import re
import subprocess
import sys

COLUMN = "cli/src/main/java/com/hivemc/chunker/conversion/intermediate/column"
BLOCK_DIR = COLUMN + "/chunk/identifier/type/block"
STATES_DIR = BLOCK_DIR + "/states/vanilla"
ITEMSTACK_DIR = COLUMN + "/chunk/itemstack"

STATE_WORDS = 3  # StateMask is [u64; 3]
MAX_STATES = STATE_WORDS * 64


# ---------------------------------------------------------------------------------------------------------------------
# Java parsing
# ---------------------------------------------------------------------------------------------------------------------


def strip_comments(text):
    """Remove // and /* */ comments and annotations, leaving string literals untouched."""
    out = []
    i = 0
    n = len(text)
    while i < n:
        c = text[i]
        if c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            out.append(text[i : j + 1])
            i = j + 1
        elif text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
        elif text.startswith("/*", i):
            j = text.find("*/", i)
            i = n if j < 0 else j + 2
        else:
            out.append(c)
            i += 1
    text = "".join(out)
    return re.sub(r"@\w+(\([^)]*\))?", "", text)


def split_top(text, sep=","):
    """Split on `sep` outside of parentheses, braces and string literals."""
    parts = []
    depth = 0
    cur = []
    i = 0
    n = len(text)
    while i < n:
        c = text[i]
        if c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            cur.append(text[i : j + 1])
            i = j + 1
            continue
        if c in "({[":
            depth += 1
        elif c in ")}]":
            depth -= 1
        if c == sep and depth == 0:
            parts.append("".join(cur))
            cur = []
        else:
            cur.append(c)
        i += 1
    parts.append("".join(cur))
    return [p.strip() for p in parts]


def enum_body(text, name):
    """Return the constant list text of `enum name` (up to the `;` that ends it or the closing brace)."""
    m = re.search(r"\benum\s+" + re.escape(name) + r"\b[^{]*\{", text)
    if not m:
        raise SystemExit("enum %s not found" % name)
    i = m.end()
    depth = 0
    j = i
    n = len(text)
    while j < n:
        c = text[j]
        if c == '"':
            j += 1
            while text[j] != '"':
                j += 2 if text[j] == "\\" else 1
        elif c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
        elif depth == 0 and c in ";}":
            break
        j += 1
    return text[i:j]


def enum_constants(text, name):
    """Parse an enum into [(NAME, [args])] in declaration order."""
    body = enum_body(strip_comments(text), name)
    out = []
    for part in split_top(body):
        part = part.strip()
        if not part:
            continue
        m = re.fullmatch(r"([A-Za-z_][A-Za-z_0-9]*)\s*(?:\((.*)\))?", part, re.S)
        if not m:
            raise SystemExit("cannot parse enum constant %r in %s" % (part[:60], name))
        args = split_top(m.group(2)) if m.group(2) is not None and m.group(2).strip() else []
        out.append((m.group(1), args))
    return out


def read(root, rel):
    with open(os.path.join(root, rel), encoding="utf-8") as f:
        return f.read()


def jstr(arg):
    """Decode a Java string literal (or None for the null literal)."""
    if arg == "null":
        return None
    assert arg.startswith('"') and arg.endswith('"'), arg
    return arg[1:-1]


def jint(arg):
    return int(arg, 16) if arg.lower().startswith("0x") else int(arg)


def rust_str(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


# ---------------------------------------------------------------------------------------------------------------------
# Rust emission helpers
# ---------------------------------------------------------------------------------------------------------------------


def wrap_items(items, indent, width=118, sep=", "):
    """Join items with `sep`, wrapping lines at `width` columns."""
    lines = []
    cur = indent
    for k, item in enumerate(items):
        piece = item + ("," if k + 1 < len(items) else "")
        if len(cur) + len(piece) + 1 > width and cur.strip():
            lines.append(cur.rstrip())
            cur = indent
        cur += piece + " "
    if cur.strip():
        lines.append(cur.rstrip())
    return "\n".join(lines)


def emit_enum(out, name, names, repr_, sorted_order=False, state=False, doc=None):
    """Emit one `named_enum!` / `state_enum!` invocation."""
    macro = "state_enum" if state else "named_enum"
    out.append("%s! {" % macro)
    if doc:
        out.append("    /// " + doc)
    out.append("    enum %s: %s {" % (name, repr_))
    out.append(wrap_items(names, "        "))
    out.append("    }")
    if sorted_order:
        order = sorted(range(len(names)), key=lambda i: names[i])
        out.append("    sorted [")
        out.append(wrap_items([str(i) for i in order], "        "))
        out.append("    ]")
    out.append("}")
    out.append("")


def mask_words(ids):
    words = [0] * STATE_WORDS
    for i in ids:
        words[i // 64] |= 1 << (i % 64)
    return "StateMask([%s])" % ", ".join("0x%x" % w for w in words)


def be_kind(cls):
    assert cls.endswith("BlockEntity.class"), cls
    kind = cls[: -len("BlockEntity.class")]
    return "CommandBlock" if kind == "Command" else kind


# ---------------------------------------------------------------------------------------------------------------------
# Generation
# ---------------------------------------------------------------------------------------------------------------------


def generate(root, commit):
    out = []
    w = out.append

    w("// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.")
    w("//")
    w("// @generated by tools/convert/gen_model.py from HiveGamesOSS/Chunker commit %s." % commit)
    w("// Do not edit by hand: change the script or the Java source and run `python3 -I tools/convert/gen_model.py`.")
    w("//")
    w("// Enum variants keep the Java constant names, and the Java ordinals: the order is part of the contract.")
    w("")
    w("#![allow(non_camel_case_types, clippy::all, dead_code)]")
    w("")
    w("use super::block::BlockGroup;")
    w("use super::blockentity::BlockEntityKind as K;")
    w("use super::state::{State, StateMask};")
    w("")

    # --- block states ------------------------------------------------------------------------------------------
    states_src = read(root, STATES_DIR + "/VanillaBlockStates.java")
    state_decls = re.findall(
        r"public static final BlockState<(\w+)> (\w+) = new BlockState<>\(\"([^\"]*)\", ([\w.]+), (\w+)::values\);",
        strip_comments(states_src),
    )
    if len(state_decls) != 133:
        print("warning: expected 133 block states, found %d" % len(state_decls), file=sys.stderr)
    if len(state_decls) > MAX_STATES:
        raise SystemExit("more than %d block states: widen StateMask" % MAX_STATES)
    state_ids = {}
    for ordinal, (tname, cname, sname, default, tname2) in enumerate(state_decls):
        assert tname == tname2, cname
        assert default.startswith(tname + "."), (cname, default)
        state_ids[cname] = ordinal

    # value enums (every class in the types directory)
    types_dir = os.path.join(root, STATES_DIR, "types")
    value_enums = {}
    for fname in sorted(os.listdir(types_dir)):
        if not fname.endswith(".java"):
            continue
        tname = fname[:-5]
        consts = enum_constants(read(root, STATES_DIR + "/types/" + fname), tname)
        value_enums[tname] = [c[0] for c in consts]
    used_types = {d[0] for d in state_decls}
    missing = used_types - set(value_enums)
    if missing:
        raise SystemExit("state value enums missing: %s" % sorted(missing))

    w("// ---- Block states: %d ids ----------------------------------------------------------------------------" % len(state_decls))
    w("")
    emit_enum(out, "StateId", [d[1] for d in state_decls], "u8", sorted_order=False,
              doc="Every block state Chunker knows (`VanillaBlockStates`), in declaration order.")

    w("/// Typed handles for the block states (`VanillaBlockStates.NAME`).")
    w("pub mod states {")
    w("    use super::*;")
    w("")
    for tname, cname, sname, default, _ in state_decls:
        w("    pub const %s: State<%s> = State::new(StateId::%s);" % (cname, tname, cname))
    w("}")
    w("")

    w("/// Name, default (an ordinal in the value enum) and value names of one block state.")
    w("#[derive(Clone, Copy, Debug)]")
    w("pub struct StateDef {")
    w("    pub name: &'static str,")
    w("    pub default: u8,")
    w("    pub values: &'static [&'static str],")
    w("}")
    w("")
    w("/// Indexed by `StateId as usize`.")
    w("pub static STATE_DEFS: [StateDef; %d] = [" % len(state_decls))
    for tname, cname, sname, default, _ in state_decls:
        dname = default.split(".", 1)[1]
        dord = value_enums[tname].index(dname)
        w("    StateDef { name: %s, default: %d, values: &%s::NAMES }," % (rust_str(sname), dord, tname))
    w("];")
    w("")

    # state groups
    groups_src = strip_comments(read(root, STATES_DIR + "/VanillaBlockStateGroups.java"))
    state_groups = {}
    for gname, inner in re.findall(r"public static final Set<BlockState<\?>> (\w+) = Set\.of\(([^;]*)\);", groups_src):
        ids = []
        for item in split_top(inner):
            if not item:
                continue
            m = re.fullmatch(r"VanillaBlockStates\.(\w+)", item)
            if not m:
                raise SystemExit("state group %s: cannot parse %r" % (gname, item))
            ids.append(state_ids[m.group(1)])
        state_groups[gname] = ids
    w("/// `VanillaBlockStateGroups`: convenience unions of block states.")
    w("pub mod state_groups {")
    w("    use super::*;")
    w("")
    for gname, ids in state_groups.items():
        w("    pub const %s: StateMask = %s;" % (gname, mask_words(ids)))
    w("}")
    w("")

    # --- value enums ----------------------------------------------------------------------------------------
    w("// ---- Block state value enums: %d types ---------------------------------------------------------------" % len(value_enums))
    w("")
    for tname in sorted(value_enums):
        emit_enum(out, tname, value_enums[tname], "u8", state=True)

    # --- block types ------------------------------------------------------------------------------------------
    block_src = read(root, BLOCK_DIR + "/ChunkerVanillaBlockType.java")
    block_consts = enum_constants(block_src, "ChunkerVanillaBlockType")
    block_names = [c[0] for c in block_consts]
    assert len(set(block_names)) == len(block_names)
    waterlogged = state_ids["WATERLOGGED"]
    set_index = {}
    set_list = []

    def intern_set(ids):
        key = tuple(sorted(set(ids) | {waterlogged}))
        if key not in set_index:
            set_index[key] = len(set_list)
            set_list.append(key)
        return set_index[key]

    intern_set([])
    group_defs = {}
    for gname, inner in re.findall(
        r"public static final Set<ChunkerBlockType> (\w+) = Set\.of\(([^;]*)\);",
        strip_comments(read(root, BLOCK_DIR + "/ChunkerVanillaBlockGroups.java")),
    ):
        members = []
        for item in split_top(inner):
            m = re.fullmatch(r"ChunkerVanillaBlockType\.(\w+)", item)
            if not m:
                raise SystemExit("block group %s: cannot parse %r" % (gname, item))
            members.append(m.group(1))
        group_defs[gname] = members
    group_names = list(group_defs)
    if len(group_names) > 16:
        raise SystemExit("more than 16 block groups: widen the group mask")
    group_bits = {}
    for gi, gname in enumerate(group_names):
        for member in group_defs[gname]:
            if member not in block_names:
                raise SystemExit("group %s names unknown block %s" % (gname, member))
            group_bits[member] = group_bits.get(member, 0) | (1 << gi)

    block_rows = []
    be_kinds = set()
    for cname, args in block_consts:
        rgb = jint(args[0])
        solid = args[-1]
        assert solid in ("true", "false"), (cname, args)
        ids = []
        be = None
        for a in args[1:-1]:
            if a.endswith(".class"):
                be = be_kind(a)
                be_kinds.add(be)
            elif a.startswith("Set.of("):
                for item in split_top(a[len("Set.of(") : -1]):
                    if item:
                        m = re.fullmatch(r"VanillaBlockStates\.(\w+)", item)
                        ids.append(state_ids[m.group(1)])
            else:
                m = re.fullmatch(r"VanillaBlockStateGroups\.(\w+)", a)
                if not m:
                    raise SystemExit("block %s: cannot parse %r" % (cname, a))
                ids.extend(state_groups[m.group(1)])
        block_rows.append((rgb, solid == "true", intern_set(ids), group_bits.get(cname, 0), be))

    w("// ---- Block types: %d ----------------------------------------------------------------------------------" % len(block_names))
    w("")
    emit_enum(out, "BlockType", block_names, "u16", sorted_order=True,
              doc="Every vanilla block type Chunker knows (`ChunkerVanillaBlockType`), in Java order.")
    w("/// Static facts about a block type (`ChunkerVanillaBlockType` constructor arguments).")
    w("#[derive(Clone, Copy, Debug)]")
    w("pub struct BlockInfo {")
    w("    /// Map colour (0xRRGGBB).")
    w("    pub rgb: u32,")
    w("    /// Every face is solid (a full cube).")
    w("    pub solid: bool,")
    w("    /// Index into `STATE_SETS`.")
    w("    pub set: u16,")
    w("    /// `BlockGroup` bit set.")
    w("    pub groups: u16,")
    w("    /// The block entity the block uses, if any.")
    w("    pub entity: Option<K>,")
    w("}")
    w("")
    w("const fn b(rgb: u32, solid: bool, set: u16, groups: u16, entity: Option<K>) -> BlockInfo {")
    w("    BlockInfo { rgb, solid, set, groups, entity }")
    w("}")
    w("")
    w("/// Indexed by `BlockType as usize`.")
    w("pub static BLOCK_INFO: [BlockInfo; %d] = [" % len(block_rows))
    for rgb, solid, sidx, groups, be in block_rows:
        w("    b(0x%x, %s, %d, %d, %s)," % (rgb, "true" if solid else "false", sidx, groups,
                                          "Some(K::%s)" % be if be else "None"))
    w("];")
    w("")
    w("/// Distinct state sets of the block types (always contain `WATERLOGGED`); `BlockInfo::set` indexes it.")
    w("pub static STATE_SETS: [StateMask; %d] = [" % len(set_list))
    for key in set_list:
        w("    %s," % mask_words(key))
    w("];")
    w("")
    w("/// `ChunkerVanillaBlockGroups`: membership is a bit in `BlockInfo::groups`.")
    w("pub mod block_groups {")
    w("    use super::BlockGroup;")
    w("")
    for gi, gname in enumerate(group_names):
        w("    pub const %s: BlockGroup = BlockGroup(1 << %d);" % (gname, gi))
    w("}")
    w("")

    # --- item types --------------------------------------------------------------------------------------------
    item_src = read(root, COLUMN + "/chunk/identifier/type/item/ChunkerVanillaItemType.java")
    item_consts = enum_constants(item_src, "ChunkerVanillaItemType")
    item_names = [c[0] for c in item_consts]
    w("// ---- Item types: %d ------------------------------------------------------------------------------------" % len(item_names))
    w("")
    emit_enum(out, "ItemType", item_names, "u16", sorted_order=True,
              doc="Every vanilla item that is not a block item (`ChunkerVanillaItemType`).")
    w("/// Block entity kind of each item type, indexed by `ItemType as usize`.")
    w("pub static ITEM_ENTITY: [Option<K>; %d] = [" % len(item_names))
    ents = []
    for cname, args in item_consts:
        if args:
            ents.append("Some(K::%s)" % be_kind(args[0]))
            be_kinds.add(be_kind(args[0]))
        else:
            ents.append("None")
    w(wrap_items(ents, "    "))
    w("];")
    w("")

    # --- entity types ------------------------------------------------------------------------------------------
    ent_consts = enum_constants(read(root, COLUMN + "/entity/type/ChunkerVanillaEntityType.java"),
                                "ChunkerVanillaEntityType")
    ent_names = [c[0] for c in ent_consts]
    w("// ---- Entity types: %d -----------------------------------------------------------------------------------" % len(ent_names))
    w("")
    emit_enum(out, "VanillaEntityType", ent_names, "u16", sorted_order=True,
              doc="Every vanilla entity type (`ChunkerVanillaEntityType`).")
    w("/// Whether the entity type has a vanilla spawn egg, indexed by `VanillaEntityType as usize`.")
    w("pub static ENTITY_SPAWN_EGG: [bool; %d] = [" % len(ent_names))
    w(wrap_items(["true" if args == ["true"] else "false" for _, args in ent_consts], "    "))
    w("];")
    w("")

    motives = enum_constants(read(root, COLUMN + "/entity/PaintingEntity.java"), "Motive")
    emit_enum(out, "Motive", [m[0] for m in motives], "u8", doc="Painting motives (`PaintingEntity.Motive`).")
    w("impl Motive {")
    w("    /// Width and height in blocks, indexed by ordinal.")
    w("    pub const SIZES: [(u8, u8); %d] = [" % len(motives))
    w(wrap_items(["(%d, %d)" % (jint(a[0]), jint(a[1])) for _, a in motives], "        "))
    w("    ];")
    w("}")
    w("")

    # --- biomes ------------------------------------------------------------------------------------------------
    biome_consts = enum_constants(read(root, COLUMN + "/biome/ChunkerBiome.java"), "ChunkerVanillaBiome")
    biome_names = [c[0] for c in biome_consts]
    biome_set = set(biome_names)
    rows = []
    for cname, args in biome_consts:
        fallback = None
        if len(args) == 1:
            java = bedrock = jstr(args[0])
        elif len(args) == 2:
            java, bedrock = jstr(args[0]), jstr(args[1])
        elif len(args) == 3:
            assert args[0] in biome_set, args
            fallback = args[0]
            java, bedrock = jstr(args[1]), jstr(args[2])
        else:
            raise SystemExit("biome %s: unexpected arguments %r" % (cname, args))
        rows.append((cname, java, bedrock, fallback))
    w("// ---- Biomes: %d ---------------------------------------------------------------------------------------" % len(biome_names))
    w("")
    emit_enum(out, "VanillaBiome", biome_names, "u8", doc="Every vanilla biome (`ChunkerBiome.ChunkerVanillaBiome`).")
    w("/// Identifiers and fallback of a vanilla biome.")
    w("#[derive(Clone, Copy, Debug)]")
    w("pub struct BiomeInfo {")
    w("    pub java: Option<&'static str>,")
    w("    pub bedrock: Option<&'static str>,")
    w("    pub fallback: Option<VanillaBiome>,")
    w("}")
    w("")
    w("/// Indexed by `VanillaBiome as usize`.")
    w("pub static BIOME_INFO: [BiomeInfo; %d] = [" % len(rows))
    for cname, java, bedrock, fallback in rows:
        w("    BiomeInfo { java: %s, bedrock: %s, fallback: %s }," % (
            "Some(%s)" % rust_str(java) if java else "None",
            "Some(%s)" % rust_str(bedrock) if bedrock else "None",
            "Some(VanillaBiome::%s)" % fallback if fallback else "None"))
    w("];")
    w("")
    # identifier lookup: Java name first, then Bedrock alias; a later put replaces an earlier one
    lookup = {}
    for cname, java, bedrock, _ in rows:
        if java is not None:
            lookup[java] = cname
        if bedrock is not None and bedrock != java:
            lookup[bedrock] = cname
    w("/// Identifier (Java name, or Bedrock alias) to biome, sorted by identifier for binary search.")
    w("pub static BIOME_IDS: [(&str, VanillaBiome); %d] = [" % len(lookup))
    for ident in sorted(lookup):
        w("    (%s, VanillaBiome::%s)," % (rust_str(ident), lookup[ident]))
    w("];")
    w("")

    # --- item stack enums --------------------------------------------------------------------------------------
    w("// ---- Item stack enums -------------------------------------------------------------------------------------")
    w("")
    dye = enum_constants(read(root, ITEMSTACK_DIR + "/ChunkerDyeColor.java"), "ChunkerDyeColor")
    emit_enum(out, "DyeColor", [d[0] for d in dye], "u8", doc="The 16 dye colours (`ChunkerDyeColor`).")
    w("impl DyeColor {")
    w("    /// Lower case names (`light_blue`).")
    w("    pub const LOWER_NAMES: [&'static str; %d] = [" % len(dye))
    w(wrap_items([rust_str(jstr(a[0])) for _, a in dye], "        "))
    w("    ];")
    w("    /// Bedrock RGB values.")
    w("    pub const BEDROCK_RGB: [u32; %d] = [" % len(dye))
    w(wrap_items(["0x%06x" % jint(a[1]) for _, a in dye], "        "))
    w("    ];")
    w("    /// Java RGB values.")
    w("    pub const JAVA_RGB: [u32; %d] = [" % len(dye))
    w(wrap_items(["0x%06x" % jint(a[2]) for _, a in dye], "        "))
    w("    ];")
    w("}")
    w("")

    simple = [
        ("BannerPattern", "banner/ChunkerBannerPattern.java", "ChunkerBannerPattern", "u8"),
        ("EnchantmentType", "enchantment/ChunkerEnchantmentType.java", "ChunkerEnchantmentType", "u8"),
        ("EffectType", "potion/ChunkerEffectType.java", "ChunkerEffectType", "u8"),
        ("HornInstrument", "horn/ChunkerHornInstrument.java", "ChunkerHornInstrument", "u8"),
        ("ExplorerMap", "map/ChunkerExplorerMap.java", "ChunkerExplorerMap", "u8"),
        ("TrimMaterial", "trim/ChunkerTrimMaterial.java", "ChunkerTrimMaterial", "u8"),
        ("TrimPattern", "trim/ChunkerTrimPattern.java", "ChunkerTrimPattern", "u8"),
    ]
    for rname, rel, jname, repr_ in simple:
        consts = enum_constants(read(root, ITEMSTACK_DIR + "/" + rel), jname)
        emit_enum(out, rname, [c[0] for c in consts], repr_, doc="`%s`." % jname)

    potion = enum_constants(read(root, ITEMSTACK_DIR + "/potion/ChunkerPotionType.java"), "ChunkerPotionType")
    emit_enum(out, "PotionType", [p[0] for p in potion], "u8", doc="`ChunkerPotionType`.")
    w("impl PotionType {")
    w("    /// Identifier without namespace (`long_swiftness`), indexed by ordinal.")
    w("    pub const IDENTIFIERS: [&'static str; %d] = [" % len(potion))
    w(wrap_items([rust_str(jstr(a[0])) for _, a in potion], "        "))
    w("    ];")
    w("}")
    w("")

    shape = enum_constants(read(root, ITEMSTACK_DIR + "/firework/ChunkerFireworkShape.java"), "ChunkerFireworkShape")
    emit_enum(out, "FireworkShape", [s[0] for s in shape], "u8", doc="`ChunkerFireworkShape`.")
    w("impl FireworkShape {")
    w("    /// Lower case names (`small_ball`), indexed by ordinal.")
    w("    pub const LOWER_NAMES: [&'static str; %d] = [" % len(shape))
    w(wrap_items([rust_str(jstr(a[0])) for _, a in shape], "        "))
    w("    ];")
    w("}")
    w("")

    bottle = enum_constants(read(root, ITEMSTACK_DIR + "/potion/ChunkerPotionBottleType.java"),
                            "ChunkerPotionBottleType")
    emit_enum(out, "PotionBottleType", [b_[0] for b_ in bottle], "u8", doc="`ChunkerPotionBottleType`.")

    # block entity kinds referenced by the tables, as a trailing comment the hand-written enum is checked against
    w("// Block entity kinds referenced above (BlockEntityKind must have a variant for each):")
    w("// %s" % ", ".join(sorted(be_kinds)))

    text = "\n".join(out)
    while "\n\n\n" in text:
        text = text.replace("\n\n\n", "\n\n")
    return text.rstrip("\n") + "\n", {
        "states": len(state_decls),
        "value_enums": len(value_enums),
        "blocks": len(block_names),
        "items": len(item_names),
        "entities": len(ent_names),
        "biomes": len(biome_names),
        "state_sets": len(set_list),
        "block_entity_kinds": sorted(be_kinds),
    }


def main():
    repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--chunker", default=os.environ.get("CHUNKER_DIR") or os.path.join(repo, "..", "Chunker"))
    ap.add_argument("--out", default=os.path.join(repo, "src", "convert", "model", "generated.rs"))
    ap.add_argument("--check", action="store_true", help="do not write; exit 1 when the file is out of date")
    args = ap.parse_args()

    root = os.path.abspath(args.chunker)
    if not os.path.isdir(os.path.join(root, "cli", "src", "main", "java")):
        raise SystemExit("not a Chunker clone: %s (use --chunker or CHUNKER_DIR)" % root)
    commit = subprocess.run(["git", "-C", root, "rev-parse", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()

    text, stats = generate(root, commit)
    if args.check:
        try:
            with open(args.out, encoding="utf-8") as f:
                same = f.read() == text
        except FileNotFoundError:
            same = False
        print("up to date" if same else "OUT OF DATE")
        sys.exit(0 if same else 1)
    with open(args.out, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print("wrote %s (%d lines): %s" % (args.out, text.count("\n"),
          ", ".join("%s=%s" % (k, v) for k, v in stats.items() if k != "block_entity_kinds")))
    print("block entity kinds: " + " ".join(stats["block_entity_kinds"]))


if __name__ == "__main__":
    main()
