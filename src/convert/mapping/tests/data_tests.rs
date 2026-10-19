// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Data-driven tests: the packed Chunker tables through the engine, with answers that were checked against the game
//! (Bedrock `weirdo_direction` 0..3 = east, west, south, north; `facing_direction` 2..5 = north, south, west, east; Java
//! `facing`, `half`, `shape`, wall heights, ...) and against Chunker's own table code.

use std::collections::HashSet;
use std::sync::LazyLock;

use crate::convert::mapping::data;
use crate::convert::mapping::tables::{IdTable, map_colors, names};
use crate::convert::mapping::*;

static JAVA: LazyLock<BlockResolver> = LazyLock::new(|| BlockResolver::new(Edition::Java, Version::new(1, 21, 4)));
static BEDROCK: LazyLock<BlockResolver> =
    LazyLock::new(|| BlockResolver::new(Edition::Bedrock, Version::new(1, 21, 50)));

/// `id[name=value,...]`; values `b:true`, `i:5` are typed, anything else is a string.
fn spec(text: &str) -> (String, StateMap) {
    let (id, rest) = text.split_once('[').map_or((text, ""), |(i, r)| (i, r.trim_end_matches(']')));
    let mut m = StateMap::new();
    for p in rest.split(',').filter(|p| !p.is_empty()) {
        let (k, v) = p.split_once('=').unwrap();
        let v = StateValue::parse_typed(v)
            .filter(|_| v.starts_with("b:") || v.starts_with("i:"))
            .unwrap_or_else(|| StateValue::str(v));
        m.insert(k.into(), v);
    }
    (id.to_string(), m)
}

fn show(id: &str, m: &StateMap) -> String {
    let s: Vec<String> = m
        .iter()
        .map(|(k, v)| match v {
            StateValue::Bool(b) => format!("{k}=b:{b}"),
            StateValue::Int(i) => format!("{k}=i:{i}"),
            StateValue::Str(s) => format!("{k}={s}"),
        })
        .collect();
    format!("{id}[{}]", s.join(","))
}

fn show_pivot(p: &PivotBlock) -> String {
    let s: Vec<String> = p.states.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{}[{}]", p.ty, s.join(","))
}

fn convert(r: &BlockResolver, text: &str) -> Option<PivotBlock> {
    let (id, st) = spec(text);
    r.to_pivot(&id, &st)
}

fn write(r: &BlockResolver, p: &PivotBlock) -> Option<String> {
    r.from_pivot(p).map(|(id, st)| show(&id, &st))
}

/// (input, pivot, written back)
type Case = (&'static str, &'static str, &'static str);

fn check_cases(r: &BlockResolver, cases: &[Case]) {
    for (input, pivot, back) in cases {
        let p = convert(r, input).unwrap_or_else(|| panic!("no mapping for {input}"));
        assert_eq!(show_pivot(&p), *pivot, "{input} -> pivot");
        assert_eq!(write(r, &p).as_deref(), Some(*back), "{input} -> back");
    }
}

#[test]
fn java_blocks_to_pivot_and_back() {
    check_cases(&JAVA, JAVA_CASES);
}

#[test]
fn bedrock_blocks_to_pivot_and_back() {
    check_cases(&BEDROCK, BEDROCK_CASES);
}

#[test]
fn java_and_bedrock_compose_through_the_pivot() {
    for (java, bedrock, java_back) in COMPOSED_CASES {
        let p = convert(&JAVA, java).unwrap_or_else(|| panic!("no java mapping for {java}"));
        let b = write(&BEDROCK, &p).unwrap_or_else(|| panic!("no bedrock mapping for {java}"));
        assert_eq!(b, *bedrock, "{java} -> bedrock");
        let p2 = convert(&BEDROCK, &b).unwrap();
        assert_eq!(write(&JAVA, &p2).as_deref(), Some(*java_back), "{bedrock} -> java");
    }
}

#[test]
fn unknown_blocks_have_no_mapping() {
    assert!(convert(&JAVA, "minecraft:does_not_exist").is_none());
    assert!(convert(&BEDROCK, "minecraft:does_not_exist[waterlogged=b:false]").is_none());
    assert!(JAVA.from_pivot(&PivotBlock::new("NOT_A_TYPE", &[])).is_none());
    // An unknown value of a pinned state falls back to the plain `stone` row (smaller pinned set).
    let p = convert(&BEDROCK, "minecraft:stone[stone_type=bogus,waterlogged=b:false]").unwrap();
    assert_eq!(show_pivot(&p), "STONE[WATERLOGGED=FALSE]");
}

#[test]
fn bedrock_stairs_use_weirdo_direction() {
    // weirdo_direction: 0 east, 1 west, 2 south, 3 north; upside_down_bit is the half.
    for (n, dir) in [(0, "EAST"), (1, "WEST"), (2, "SOUTH"), (3, "NORTH")] {
        let p = convert(
            &BEDROCK,
            &format!("minecraft:stone_stairs[weirdo_direction=i:{n},upside_down_bit=b:false,waterlogged=b:false]"),
        )
        .unwrap();
        assert_eq!(&*p.ty, "COBBLESTONE_STAIRS");
        assert_eq!(p.states.get("FACING_HORIZONTAL").map(|v| &**v), Some(dir));
        assert_eq!(p.states.get("HALF").map(|v| &**v), Some("BOTTOM"));
    }
    let top =
        convert(&BEDROCK, "minecraft:stone_stairs[weirdo_direction=i:0,upside_down_bit=b:true,waterlogged=b:false]")
            .unwrap();
    assert_eq!(top.states.get("HALF").map(|v| &**v), Some("TOP"));
}

#[test]
fn bedrock_waterlogged_is_a_pseudo_state() {
    let p = convert(&BEDROCK, "minecraft:oak_fence[waterlogged=b:true]").unwrap();
    assert_eq!(p.states.get("WATERLOGGED").map(|v| &**v), Some("TRUE"));
    let (_, st) = BEDROCK.from_pivot(&p).unwrap();
    assert_eq!(st.get("waterlogged"), Some(&StateValue::Bool(true)));
    // Without the state it is dry, and Java blocks without a waterlogged property are dry too.
    let p = convert(&BEDROCK, "minecraft:oak_fence").unwrap();
    assert_eq!(p.states.get("WATERLOGGED").map(|v| &**v), Some("FALSE"));
    let p = convert(&JAVA, "minecraft:stone").unwrap();
    assert_eq!(p.states.get("WATERLOGGED").map(|v| &**v), Some("FALSE"));
}

#[test]
fn bedrock_facing_state_changes_with_the_version() {
    // Chests used facing_direction (2..5) until 1.20.40, then minecraft:cardinal_direction.
    let old = BlockResolver::new(Edition::Bedrock, Version::new(1, 20, 30));
    let new = BlockResolver::new(Edition::Bedrock, Version::new(1, 20, 40));
    let p = convert(&old, "minecraft:chest[facing_direction=i:3,waterlogged=b:false]").unwrap();
    assert_eq!(show_pivot(&p), "CHEST[CHEST_TYPE=SINGLE,FACING_HORIZONTAL=SOUTH,WATERLOGGED=FALSE]");
    assert_eq!(write(&old, &p).unwrap(), "minecraft:chest[facing_direction=i:3,waterlogged=b:false]");
    assert_eq!(write(&new, &p).unwrap(), "minecraft:chest[minecraft:cardinal_direction=south,waterlogged=b:false]");
    // The new table ignores the old state name; the missing cardinal direction maps to nothing.
    let ignored = convert(&new, "minecraft:chest[facing_direction=i:3,waterlogged=b:false]").unwrap();
    assert_ne!(ignored.states.get("FACING_HORIZONTAL"), p.states.get("FACING_HORIZONTAL"));
}

#[test]
fn rows_are_gated_by_version() {
    let has = |ed: Edition, v: Version, id: &str| {
        let rows = match ed {
            Edition::Java => data::java_blocks(v),
            Edition::Bedrock => data::bedrock_blocks(v),
        };
        rows.rows.iter().any(|r| &*r.id == id && r.kind != data::RowKind::FromPivot)
    };
    // Java: cherry 1.19.4, deepslate 1.17, grass renamed to short_grass in 1.20.3 (grass stays readable).
    assert!(!has(Edition::Java, Version::new(1, 19, 3), "minecraft:cherry_planks"));
    assert!(has(Edition::Java, Version::new(1, 19, 4), "minecraft:cherry_planks"));
    assert!(!has(Edition::Java, Version::new(1, 16, 5), "minecraft:deepslate"));
    assert!(has(Edition::Java, Version::new(1, 17, 0), "minecraft:deepslate"));
    assert!(!has(Edition::Java, Version::new(1, 20, 2), "minecraft:short_grass"));
    assert!(has(Edition::Java, Version::new(1, 20, 3), "minecraft:short_grass"));
    assert!(has(Edition::Java, Version::new(1, 20, 3), "minecraft:grass"));
    assert!(has(Edition::Java, Version::new(1, 13, 2), "minecraft:grass"));
    // Bedrock: pale oak 1.21.50, crafter 1.20.50.
    assert!(!has(Edition::Bedrock, Version::new(1, 21, 40), "minecraft:pale_oak_log"));
    assert!(has(Edition::Bedrock, Version::new(1, 21, 50), "minecraft:pale_oak_log"));
    assert!(!has(Edition::Bedrock, Version::new(1, 20, 40), "minecraft:crafter"));
    assert!(has(Edition::Bedrock, Version::new(1, 20, 50), "minecraft:crafter"));
    let old = BlockResolver::new(Edition::Java, Version::new(1, 16, 5));
    assert!(old.to_pivot("minecraft:cherry_planks", &StateMap::new()).is_none());
    assert!(old.to_pivot("minecraft:stone", &StateMap::new()).is_some());
}

#[test]
fn distinct_java_block_ids_match_the_survey() {
    // Distinct Java ids per version, from the survey of JavaBlockIdentifierResolver (base + all gates up to the version).
    for ((major, minor, patch), expected) in [
        ((1, 13, 2), 598),
        ((1, 16, 5), 765),
        ((1, 18, 2), 901),
        ((1, 20, 1), 1006),
        ((1, 21, 0), 1064),
        ((1, 21, 4), 1099),
        ((1, 21, 11), 1171),
        ((26, 3, 0), 1291),
    ] {
        let rows = data::java_blocks(Version::new(major, minor, patch));
        let ids: HashSet<_> = rows.rows.iter().filter(|r| r.kind != data::RowKind::FromPivot).map(|r| &r.id).collect();
        assert_eq!(ids.len(), expected, "{major}.{minor}.{patch}");
    }
}

#[test]
fn every_table_builds_at_every_probe_version() {
    for v in
        [(1, 13, 2), (1, 16, 5), (1, 18, 2), (1, 20, 1), (1, 20, 4), (1, 21, 0), (1, 21, 4), (1, 21, 11), (26, 3, 0)]
    {
        let v = Version::new(v.0, v.1, v.2);
        let r = BlockResolver::new(Edition::Java, v);
        assert!(r.row_counts().0 > 500, "{v}");
        let i = ItemResolver::new(Edition::Java, v);
        assert!(i.row_counts().0 > 200, "{v}");
    }
    for v in [
        (1, 13, 0),
        (1, 16, 0),
        (1, 17, 30),
        (1, 18, 30),
        (1, 19, 80),
        (1, 20, 0),
        (1, 20, 80),
        (1, 21, 0),
        (1, 21, 50),
        (1, 21, 90),
        (1, 21, 120),
        (1, 26, 0),
        (1, 26, 60),
    ] {
        let v = Version::new(v.0, v.1, v.2);
        let r = BlockResolver::new(Edition::Bedrock, v);
        assert!(r.row_counts().0 > 400, "{v}");
        let i = ItemResolver::new(Edition::Bedrock, v);
        assert!(i.row_counts().0 > 200, "{v}");
    }
    // Legacy tables.
    assert!(BlockResolver::new_legacy(Edition::Java, Version::new(1, 12, 2)).row_counts().0 > 400);
    assert!(BlockResolver::new_legacy(Edition::Bedrock, Version::new(1, 12, 0)).row_counts().0 > 500);
    assert!(ItemResolver::new_legacy_java(Version::new(1, 12, 2)).row_counts().0 > 100);
}

#[test]
fn legacy_java_blocks_use_numeric_data() {
    let r = BlockResolver::new_legacy(Edition::Java, Version::new(1, 12, 2));
    let get = |t: &str| convert(&r, t).map(|p| show_pivot(&p));
    assert_eq!(get("minecraft:stone[data=i:1]").as_deref(), Some("GRANITE[WATERLOGGED=FALSE]"));
    assert_eq!(get("minecraft:wool[data=i:14]").as_deref(), Some("RED_WOOL[WATERLOGGED=FALSE]"));
    assert_eq!(get("minecraft:log[data=i:5]").as_deref(), Some("SPRUCE_LOG[AXIS=X,WATERLOGGED=FALSE]"));
    assert_eq!(get("minecraft:dirt[data=i:2]").as_deref(), Some("PODZOL[SNOWY=FALSE,WATERLOGGED=FALSE]"));
    assert_eq!(
        get("minecraft:stone_stairs[data=i:5]").as_deref(),
        Some("COBBLESTONE_STAIRS[FACING_HORIZONTAL=WEST,HALF=TOP,STAIR_SHAPE=STRAIGHT,WATERLOGGED=FALSE]")
    );
    let p = convert(&r, "minecraft:stone[data=i:1]").unwrap();
    assert_eq!(write(&r, &p).as_deref(), Some("minecraft:stone[data=i:1]"));
}

#[test]
fn legacy_bedrock_blocks_use_numeric_data() {
    let r = BlockResolver::new_legacy(Edition::Bedrock, Version::new(1, 12, 0));
    let p = convert(&r, "minecraft:planks[data=i:2]").unwrap();
    assert_eq!(show_pivot(&p), "BIRCH_PLANKS[WATERLOGGED=FALSE]");
    assert_eq!(write(&r, &p).as_deref(), Some("minecraft:planks[data=i:2,waterlogged=b:false]"));
}

#[test]
fn java_items() {
    let r = ItemResolver::new(Edition::Java, Version::new(1, 21, 4));
    let get = |id: &str| r.to_pivot(id, &StateMap::new());
    assert_eq!(get("minecraft:stick"), Some(PivotItem::new(ItemKind::Item("STICK".into()), &[])));
    assert_eq!(
        get("minecraft:allay_spawn_egg"),
        Some(PivotItem::new(ItemKind::Item("SPAWN_EGG".into()), &[("P:spawn_egg_mob", "ALLAY")]))
    );
    assert_eq!(get("minecraft:cactus_green").map(|i| i.kind), Some(ItemKind::Item("GREEN_DYE".into())));
    // Block items are not in the item table: they fall through to the block table.
    assert!(get("minecraft:oak_door").is_none());
    let egg = get("minecraft:allay_spawn_egg").unwrap();
    assert_eq!(r.from_pivot(&egg).map(|(i, _)| i.to_string()).as_deref(), Some("minecraft:allay_spawn_egg"));
}

#[test]
fn bedrock_items_use_data_values() {
    let r = ItemResolver::new(Edition::Bedrock, Version::new(1, 21, 50));
    let get = |id: &str, data: Option<i32>| {
        let st = data.map_or_else(StateMap::new, |d| states(&[("data", StateValue::Int(d))]));
        let p = r.to_pivot(id, &st)?;
        let back = r.from_pivot(&p).map(|(i, s)| (i.to_string(), s));
        Some((p, back))
    };
    // banner data is the dye colour (15 white, 1 red); no data = black.
    let (p, back) = get("minecraft:banner", Some(15)).unwrap();
    assert_eq!(p.kind, ItemKind::Item("WHITE_BANNER".into()));
    assert_eq!(back.unwrap(), ("minecraft:banner".to_string(), states(&[("data", StateValue::Int(15))])));
    assert_eq!(get("minecraft:banner", Some(1)).unwrap().0.kind, ItemKind::Item("RED_BANNER".into()));
    // potion data -> potion type property
    let (p, _) = get("minecraft:potion", Some(5)).unwrap();
    assert_eq!(p.kind, ItemKind::Item("POTION".into()));
    assert_eq!(p.props.get("P:potion").map(|v| &**v), Some("NIGHT_VISION"));
    // coal data 1 is charcoal, written back as its own item
    assert_eq!(get("minecraft:coal", Some(1)).unwrap().0.kind, ItemKind::Item("CHARCOAL".into()));
    assert_eq!(get("minecraft:bed", Some(14)).unwrap().0.kind, ItemKind::Block("RED_BED".into()));
    assert!(get("minecraft:oak_door", None).is_none());
}

#[test]
fn id_tables() {
    let j = Version::new(1, 21, 4);
    let b = Version::new(1, 21, 50);
    let plains = IdTable::load(Edition::Java, names::BIOME_NAME, j);
    assert_eq!(plains.from_pivot_str("PLAINS"), Some("minecraft:plains"));
    assert_eq!(plains.to_pivot_name("plains"), Some("PLAINS")); // missing namespace means minecraft:
    assert_eq!(plains.to_pivot_name("minecraft:the_end"), Some("THE_END"));
    assert_eq!(plains.to_pivot_name("mod:custom"), None);
    let old = IdTable::load(Edition::Java, names::BIOME_ID, Version::new(1, 12, 2));
    assert_eq!(old.from_pivot_int("PLAINS"), Some(1));
    assert_eq!(old.from_pivot_int("ICE_SPIKES"), Some(140));
    let bio = IdTable::load(Edition::Bedrock, names::BIOME_ID, b);
    assert_eq!(bio.to_pivot(&StateValue::Int(1)), Some("PLAINS"));
    assert_eq!(bio.to_pivot(&StateValue::Int(0)), Some("OCEAN"));
    assert_eq!(bio.from_pivot_int("DEEP_DARK"), Some(190));
    assert_eq!(bio.from_pivot_int("CHERRY_GROVE"), Some(192));
    let ent = IdTable::load(Edition::Java, names::ENTITY_TYPE, j);
    assert_eq!(ent.from_pivot_str("ZOMBIE"), Some("minecraft:zombie"));
    assert_eq!(ent.to_pivot_name("allay"), Some("ALLAY"));
    let ids = IdTable::load(Edition::Bedrock, names::ENTITY_ID, b);
    assert_eq!(ids.from_pivot_int("ZOMBIE"), Some(32));
    assert_eq!(ids.from_pivot_int("PIG"), Some(12));
    let fx = IdTable::load(Edition::Bedrock, names::EFFECT_ID, b);
    assert_eq!((fx.from_pivot_int("SPEED"), fx.from_pivot_int("DARKNESS")), (Some(1), Some(30)));
    let ench = IdTable::load(Edition::Bedrock, names::ENCHANTMENT_ID, b);
    assert_eq!((ench.from_pivot_int("SHARPNESS"), ench.from_pivot_int("MENDING")), (Some(9), Some(26)));
    assert_eq!(IdTable::load(Edition::Java, names::TRIM_PATTERN, j).len(), 18);
    assert_eq!(IdTable::load(Edition::Java, names::HORN, j).len(), 8);
    // gated by version: the darkness effect is new in 1.19
    assert!(IdTable::load(Edition::Java, names::EFFECT, Version::new(1, 18, 2)).from_pivot("DARKNESS").is_none());
    assert!(IdTable::load(Edition::Java, names::EFFECT, Version::new(1, 19, 0)).from_pivot("DARKNESS").is_some());
    let legacy = IdTable::load(Edition::Java, names::LEGACY_BLOCK_ID, Version::new(1, 12, 2));
    assert_eq!(legacy.from_pivot_int("minecraft:stone"), Some(1));
    assert_eq!(legacy.to_pivot(&StateValue::Int(1)), Some("minecraft:stone"));
    let item_ids = IdTable::load(Edition::Java, names::LEGACY_ITEM_ID, Version::new(1, 12, 2));
    assert_eq!(item_ids.from_pivot_int("minecraft:stick"), Some(280));
    // map colours: transparent first, then grass
    let c = map_colors(Edition::Java, j);
    assert_eq!(c[0], [0, 0, 0, 0]);
    assert_eq!(c[1], [127, 178, 56, 255]);
    assert!(c.len() >= 60);
}

#[test]
fn chunker_data_is_optional() {
    // Validation against Chunker's own data folder comes in a later phase; this only shows how it will skip.
    let dir = std::env::var("CHUNKER_DATA").unwrap_or_else(|_| "../Chunker/cli/data".to_string());
    if !std::path::Path::new(&dir).is_dir() {
        eprintln!("skipping: CHUNKER_DATA ({dir}) not found");
    }
    // When present nothing is read in this phase.
}

const JAVA_CASES: &[(&str, &str, &str)] = &[
    (
        "minecraft:oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=false]",
        "OAK_STAIRS[FACING_HORIZONTAL=NORTH,HALF=BOTTOM,STAIR_SHAPE=STRAIGHT,WATERLOGGED=FALSE]",
        "minecraft:oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=false]",
    ),
    (
        "minecraft:oak_stairs[facing=west,half=top,shape=outer_right,waterlogged=true]",
        "OAK_STAIRS[FACING_HORIZONTAL=WEST,HALF=TOP,STAIR_SHAPE=OUTER_RIGHT,WATERLOGGED=TRUE]",
        "minecraft:oak_stairs[facing=west,half=top,shape=outer_right,waterlogged=true]",
    ),
    (
        "minecraft:redstone_wire[east=side,north=up,power=7,south=none,west=none]",
        "REDSTONE_WIRE[POWER=_7,REDSTONE_EAST=SIDE,REDSTONE_NORTH=UP,REDSTONE_SOUTH=NONE,REDSTONE_WEST=NONE,WATERLOGGED=FALSE]",
        "minecraft:redstone_wire[east=side,north=up,power=7,south=none,west=none]",
    ),
    (
        "minecraft:oak_door[facing=east,half=lower,hinge=left,open=false,powered=false]",
        "OAK_DOOR[DOOR_HINGE=LEFT,FACING_HORIZONTAL=EAST,HALF=BOTTOM,OPEN=FALSE,POWERED=FALSE,WATERLOGGED=FALSE]",
        "minecraft:oak_door[facing=east,half=lower,hinge=left,open=false,powered=false]",
    ),
    (
        "minecraft:oak_door[facing=north,half=upper,hinge=right,open=true,powered=true]",
        "OAK_DOOR[DOOR_HINGE=RIGHT,FACING_HORIZONTAL=NORTH,HALF=TOP,OPEN=TRUE,POWERED=TRUE,WATERLOGGED=FALSE]",
        "minecraft:oak_door[facing=north,half=upper,hinge=right,open=true,powered=true]",
    ),
    (
        "minecraft:oak_fence[east=true,north=false,south=true,waterlogged=false,west=false]",
        "OAK_FENCE[EAST=TRUE,NORTH=FALSE,SOUTH=TRUE,WATERLOGGED=FALSE,WEST=FALSE]",
        "minecraft:oak_fence[east=true,north=false,south=true,waterlogged=false,west=false]",
    ),
    (
        "minecraft:chest[facing=south,type=left,waterlogged=false]",
        "CHEST[CHEST_TYPE=LEFT,FACING_HORIZONTAL=SOUTH,WATERLOGGED=FALSE]",
        "minecraft:chest[facing=south,type=left,waterlogged=false]",
    ),
    (
        "minecraft:white_banner[rotation=5]",
        "WHITE_BANNER[ROTATION=_5,WATERLOGGED=FALSE]",
        "minecraft:white_banner[rotation=5]",
    ),
    (
        "minecraft:red_wall_banner[facing=west]",
        "RED_WALL_BANNER[FACING_HORIZONTAL=WEST,WATERLOGGED=FALSE]",
        "minecraft:red_wall_banner[facing=west]",
    ),
    (
        "minecraft:red_bed[facing=north,occupied=false,part=head]",
        "RED_BED[BED_PART=HEAD,FACING_HORIZONTAL=NORTH,OCCUPIED=FALSE,WATERLOGGED=FALSE]",
        "minecraft:red_bed[facing=north,occupied=false,part=head]",
    ),
    ("minecraft:cauldron", "CAULDRON[WATERLOGGED=FALSE]", "minecraft:cauldron[]"),
    (
        "minecraft:water_cauldron[level=2]",
        "WATER_CAULDRON[CAULDRON_LEVEL=_4,WATERLOGGED=FALSE]",
        "minecraft:water_cauldron[level=2]",
    ),
    ("minecraft:lava_cauldron", "LAVA_CAULDRON[CAULDRON_LEVEL=_6,WATERLOGGED=FALSE]", "minecraft:lava_cauldron[]"),
    ("minecraft:grass", "SHORT_GRASS[WATERLOGGED=FALSE]", "minecraft:short_grass[]"),
    ("minecraft:short_grass", "SHORT_GRASS[WATERLOGGED=FALSE]", "minecraft:short_grass[]"),
    (
        "minecraft:oak_wall_sign[facing=east,waterlogged=false]",
        "OAK_WALL_SIGN[FACING_HORIZONTAL=EAST,WATERLOGGED=FALSE]",
        "minecraft:oak_wall_sign[facing=east,waterlogged=false]",
    ),
    (
        "minecraft:oak_sign[rotation=3,waterlogged=false]",
        "OAK_SIGN[ROTATION=_3,WATERLOGGED=FALSE]",
        "minecraft:oak_sign[rotation=3,waterlogged=false]",
    ),
    (
        "minecraft:oak_hanging_sign[attached=false,rotation=4,waterlogged=false]",
        "OAK_HANGING_SIGN[ATTACHED=FALSE,ROTATION=_4,WATERLOGGED=FALSE]",
        "minecraft:oak_hanging_sign[attached=false,rotation=4,waterlogged=false]",
    ),
    ("minecraft:oak_log[axis=y]", "OAK_LOG[AXIS=Y,WATERLOGGED=FALSE]", "minecraft:oak_log[axis=y]"),
    ("minecraft:stone", "STONE[WATERLOGGED=FALSE]", "minecraft:stone[]"),
    (
        "minecraft:oak_slab[type=double,waterlogged=false]",
        "OAK_SLAB[SLAB_TYPE=DOUBLE,WATERLOGGED=FALSE]",
        "minecraft:oak_slab[type=double,waterlogged=false]",
    ),
    (
        "minecraft:oak_slab[type=top,waterlogged=true]",
        "OAK_SLAB[SLAB_TYPE=TOP,WATERLOGGED=TRUE]",
        "minecraft:oak_slab[type=top,waterlogged=true]",
    ),
    ("minecraft:wheat[age=7]", "WHEAT[AGE_7=_7,WATERLOGGED=FALSE]", "minecraft:wheat[age=7]"),
    (
        "minecraft:oak_leaves[distance=3,persistent=false,waterlogged=false]",
        "OAK_LEAVES[DISTANCE=_3,PERSISTENT=FALSE,UPDATE=FALSE,WATERLOGGED=FALSE]",
        "minecraft:oak_leaves[distance=3,persistent=false,waterlogged=false]",
    ),
    ("minecraft:water[level=0]", "WATER[FLOWING=FALSE,LIQUID_LEVEL=_0,WATERLOGGED=FALSE]", "minecraft:water[level=0]"),
    ("minecraft:lava[level=3]", "LAVA[FLOWING=FALSE,LIQUID_LEVEL=_3,WATERLOGGED=FALSE]", "minecraft:lava[level=3]"),
    (
        "minecraft:piston[extended=false,facing=up]",
        "PISTON[EXTENDED=FALSE,FACING_ALL=UP,WATERLOGGED=FALSE]",
        "minecraft:piston[extended=false,facing=up]",
    ),
    (
        "minecraft:piston_head[facing=down,short=false,type=sticky]",
        "PISTON_HEAD[FACING_ALL=DOWN,PISTON_TYPE=STICKY,SHORT=FALSE,WATERLOGGED=FALSE]",
        "minecraft:piston_head[facing=down,short=false,type=sticky]",
    ),
    (
        "minecraft:lever[face=wall,facing=north,powered=true]",
        "LEVER[ATTACHMENT_TYPE=WALL,FACING_HORIZONTAL=NORTH,POWERED=TRUE,WATERLOGGED=FALSE]",
        "minecraft:lever[face=wall,facing=north,powered=true]",
    ),
    (
        "minecraft:stone_button[face=floor,facing=east,powered=false]",
        "STONE_BUTTON[ATTACHMENT_TYPE=FLOOR,FACING_HORIZONTAL=EAST,POWERED=FALSE,WATERLOGGED=FALSE]",
        "minecraft:stone_button[face=floor,facing=east,powered=false]",
    ),
    (
        "minecraft:repeater[delay=3,facing=south,locked=false,powered=false]",
        "REPEATER[DELAY=_3,FACING_HORIZONTAL=SOUTH,LOCKED=FALSE,POWERED=FALSE,WATERLOGGED=FALSE]",
        "minecraft:repeater[delay=3,facing=south,locked=false,powered=false]",
    ),
    (
        "minecraft:comparator[facing=north,mode=subtract,powered=false]",
        "COMPARATOR[FACING_HORIZONTAL=NORTH,MODE_COMPARATOR=SUBTRACT,POWERED=FALSE,WATERLOGGED=FALSE]",
        "minecraft:comparator[facing=north,mode=subtract,powered=false]",
    ),
    ("minecraft:torch", "TORCH[WATERLOGGED=FALSE]", "minecraft:torch[]"),
    (
        "minecraft:wall_torch[facing=north]",
        "WALL_TORCH[FACING_HORIZONTAL=NORTH,WATERLOGGED=FALSE]",
        "minecraft:wall_torch[facing=north]",
    ),
    (
        "minecraft:ladder[facing=west,waterlogged=false]",
        "LADDER[FACING_HORIZONTAL=WEST,WATERLOGGED=FALSE]",
        "minecraft:ladder[facing=west,waterlogged=false]",
    ),
    (
        "minecraft:cobblestone_wall[east=low,north=tall,south=none,up=true,waterlogged=false,west=none]",
        "COBBLESTONE_WALL[UP=TRUE,WALL_EAST=LOW,WALL_NORTH=TALL,WALL_SOUTH=NONE,WALL_WEST=NONE,WATERLOGGED=FALSE]",
        "minecraft:cobblestone_wall[east=low,north=tall,south=none,up=true,waterlogged=false,west=none]",
    ),
    (
        "minecraft:oak_trapdoor[facing=north,half=top,open=true,powered=false,waterlogged=false]",
        "OAK_TRAPDOOR[FACING_HORIZONTAL=NORTH,HALF=TOP,OPEN=TRUE,POWERED=FALSE,WATERLOGGED=FALSE]",
        "minecraft:oak_trapdoor[facing=north,half=top,open=true,powered=false,waterlogged=false]",
    ),
    (
        "minecraft:oak_fence_gate[facing=south,in_wall=false,open=true,powered=false]",
        "OAK_FENCE_GATE[FACING_HORIZONTAL=SOUTH,IN_WALL=FALSE,OPEN=TRUE,POWERED=FALSE,WATERLOGGED=FALSE]",
        "minecraft:oak_fence_gate[facing=south,in_wall=false,open=true,powered=false]",
    ),
    (
        "minecraft:command_block[conditional=false,facing=up]",
        "COMMAND_BLOCK[CONDITIONAL=FALSE,FACING_ALL=UP,WATERLOGGED=FALSE]",
        "minecraft:command_block[conditional=false,facing=up]",
    ),
    (
        "minecraft:note_block[instrument=harp,note=4,powered=false]",
        "NOTE_BLOCK[NOTE=_4,NOTE_BLOCK_INSTRUMENT=HARP,POWERED=FALSE,WATERLOGGED=FALSE]",
        "minecraft:note_block[instrument=harp,note=4,powered=false]",
    ),
    (
        "minecraft:rail[shape=north_south,waterlogged=false]",
        "RAIL[RAIL_SHAPE=NORTH_SOUTH,WATERLOGGED=FALSE]",
        "minecraft:rail[shape=north_south,waterlogged=false]",
    ),
    (
        "minecraft:white_stained_glass_pane[east=true,north=true,south=false,waterlogged=false,west=false]",
        "WHITE_STAINED_GLASS_PANE[EAST=TRUE,NORTH=TRUE,SOUTH=FALSE,WATERLOGGED=FALSE,WEST=FALSE]",
        "minecraft:white_stained_glass_pane[east=true,north=true,south=false,waterlogged=false,west=false]",
    ),
    ("minecraft:tall_grass[half=upper]", "TALL_GRASS[HALF=TOP,WATERLOGGED=FALSE]", "minecraft:tall_grass[half=upper]"),
    ("minecraft:sunflower[half=lower]", "SUNFLOWER[HALF=BOTTOM,WATERLOGGED=FALSE]", "minecraft:sunflower[half=lower]"),
    (
        "minecraft:nether_portal[axis=x]",
        "NETHER_PORTAL[AXIS_HORIZONTAL=X,WATERLOGGED=FALSE]",
        "minecraft:nether_portal[axis=x]",
    ),
    (
        "minecraft:end_portal_frame[eye=true,facing=north]",
        "END_PORTAL_FRAME[EYE=TRUE,FACING_HORIZONTAL=NORTH,WATERLOGGED=FALSE]",
        "minecraft:end_portal_frame[eye=true,facing=north]",
    ),
    ("minecraft:snow[layers=3]", "SNOW[LAYERS=_3,WATERLOGGED=FALSE]", "minecraft:snow[layers=3]"),
    ("minecraft:farmland[moisture=7]", "FARMLAND[MOISTURE=_7,WATERLOGGED=FALSE]", "minecraft:farmland[moisture=7]"),
    ("minecraft:composter[level=4]", "COMPOSTER[COMPOSTER_LEVEL=_4,WATERLOGGED=FALSE]", "minecraft:composter[level=4]"),
    (
        "minecraft:bubble_column[drag=true]",
        "BUBBLE_COLUMN[DRAG=TRUE,WATERLOGGED=TRUE]",
        "minecraft:bubble_column[drag=true]",
    ),
    ("minecraft:kelp[age=5]", "KELP[AGE_25=_5,WATERLOGGED=TRUE]", "minecraft:kelp[age=5]"),
    (
        "minecraft:sea_pickle[pickles=2,waterlogged=true]",
        "SEA_PICKLE[DEAD=FALSE,PICKLES=_2,WATERLOGGED=TRUE]",
        "minecraft:sea_pickle[pickles=2,waterlogged=true]",
    ),
    (
        "minecraft:big_dripleaf[facing=north,tilt=none,waterlogged=false]",
        "BIG_DRIPLEAF[FACING_HORIZONTAL=NORTH,TILT=NONE,WATERLOGGED=FALSE]",
        "minecraft:big_dripleaf[facing=north,tilt=none,waterlogged=false]",
    ),
];

const BEDROCK_CASES: &[(&str, &str, &str)] = &[
    (
        "minecraft:stone_stairs[weirdo_direction=i:2,upside_down_bit=b:false,waterlogged=b:false]",
        "COBBLESTONE_STAIRS[FACING_HORIZONTAL=SOUTH,HALF=BOTTOM,STAIR_SHAPE=STRAIGHT,WATERLOGGED=FALSE]",
        "minecraft:stone_stairs[upside_down_bit=b:false,waterlogged=b:false,weirdo_direction=i:2]",
    ),
    (
        "minecraft:oak_stairs[weirdo_direction=i:0,upside_down_bit=b:true,waterlogged=b:true]",
        "OAK_STAIRS[FACING_HORIZONTAL=EAST,HALF=TOP,STAIR_SHAPE=STRAIGHT,WATERLOGGED=TRUE]",
        "minecraft:oak_stairs[upside_down_bit=b:true,waterlogged=b:true,weirdo_direction=i:0]",
    ),
    (
        "minecraft:wall_sign[facing_direction=i:3,waterlogged=b:false]",
        "OAK_WALL_SIGN[FACING_HORIZONTAL=SOUTH,WATERLOGGED=FALSE]",
        "minecraft:wall_sign[facing_direction=i:3,waterlogged=b:false]",
    ),
    (
        "minecraft:standing_sign[ground_sign_direction=i:5,waterlogged=b:false]",
        "OAK_SIGN[ROTATION=_5,WATERLOGGED=FALSE]",
        "minecraft:standing_sign[ground_sign_direction=i:5,waterlogged=b:false]",
    ),
    (
        "minecraft:oak_hanging_sign[attached_bit=b:false,facing_direction=i:2,ground_sign_direction=i:4,hanging=b:false,waterlogged=b:false]",
        "OAK_WALL_HANGING_SIGN[ATTACHED=FALSE,FACING_HORIZONTAL=NORTH,WATERLOGGED=FALSE]",
        "minecraft:oak_hanging_sign[attached_bit=b:false,facing_direction=i:2,ground_sign_direction=i:2,hanging=b:false,waterlogged=b:false]",
    ),
    (
        "minecraft:command_block[conditional_bit=b:false,facing_direction=i:1,waterlogged=b:false]",
        "COMMAND_BLOCK[CONDITIONAL=FALSE,FACING_ALL=UP,WATERLOGGED=FALSE]",
        "minecraft:command_block[conditional_bit=b:false,facing_direction=i:1,waterlogged=b:false]",
    ),
    (
        "minecraft:chain_command_block[conditional_bit=b:true,facing_direction=i:4,waterlogged=b:false]",
        "CHAIN_COMMAND_BLOCK[CONDITIONAL=TRUE,FACING_ALL=WEST,WATERLOGGED=FALSE]",
        "minecraft:chain_command_block[conditional_bit=b:true,facing_direction=i:4,waterlogged=b:false]",
    ),
    (
        "minecraft:oak_log[pillar_axis=y,waterlogged=b:false]",
        "OAK_LOG[AXIS=Y,WATERLOGGED=FALSE]",
        "minecraft:oak_log[pillar_axis=y,waterlogged=b:false]",
    ),
    (
        "minecraft:log[old_log_type=oak,pillar_axis=y,waterlogged=b:false]",
        "OAK_LOG[AXIS=Y,WATERLOGGED=FALSE]",
        "minecraft:oak_log[pillar_axis=y,waterlogged=b:false]",
    ),
    (
        "minecraft:stone[stone_type=granite,waterlogged=b:false]",
        "GRANITE[WATERLOGGED=FALSE]",
        "minecraft:granite[waterlogged=b:false]",
    ),
    ("minecraft:stone[waterlogged=b:false]", "STONE[WATERLOGGED=FALSE]", "minecraft:stone[waterlogged=b:false]"),
    (
        "minecraft:wool[color=red,waterlogged=b:false]",
        "RED_WOOL[WATERLOGGED=FALSE]",
        "minecraft:red_wool[waterlogged=b:false]",
    ),
    (
        "minecraft:chest[minecraft:cardinal_direction=north,waterlogged=b:false]",
        "CHEST[CHEST_TYPE=SINGLE,FACING_HORIZONTAL=NORTH,WATERLOGGED=FALSE]",
        "minecraft:chest[minecraft:cardinal_direction=north,waterlogged=b:false]",
    ),
    (
        "minecraft:wooden_door[direction=i:1,door_hinge_bit=b:false,open_bit=b:false,upper_block_bit=b:false,waterlogged=b:false]",
        "OAK_DOOR[DOOR_HINGE=LEFT,FACING_HORIZONTAL=SOUTH,HALF=BOTTOM,OPEN=FALSE,POWERED=FALSE,WATERLOGGED=FALSE]",
        "minecraft:wooden_door[direction=i:1,door_hinge_bit=b:false,open_bit=b:false,upper_block_bit=b:false,waterlogged=b:false]",
    ),
    (
        "minecraft:water[liquid_depth=i:0,waterlogged=b:false]",
        "WATER[FLOWING=FALSE,LIQUID_LEVEL=_0,WATERLOGGED=FALSE]",
        "minecraft:water[liquid_depth=i:0,waterlogged=b:false]",
    ),
    (
        "minecraft:flowing_water[liquid_depth=i:3,waterlogged=b:false]",
        "WATER[FLOWING=TRUE,LIQUID_LEVEL=_3,WATERLOGGED=FALSE]",
        "minecraft:flowing_water[liquid_depth=i:3,waterlogged=b:false]",
    ),
    (
        "minecraft:wheat[growth=i:7,waterlogged=b:false]",
        "WHEAT[AGE_7=_7,WATERLOGGED=FALSE]",
        "minecraft:wheat[growth=i:7,waterlogged=b:false]",
    ),
    (
        "minecraft:leaves[old_leaf_type=oak,persistent_bit=b:false,update_bit=b:false,waterlogged=b:false]",
        "OAK_LEAVES[DISTANCE=_1,PERSISTENT=FALSE,UPDATE=FALSE,WATERLOGGED=FALSE]",
        "minecraft:oak_leaves[persistent_bit=b:false,update_bit=b:false,waterlogged=b:false]",
    ),
    (
        "minecraft:grass_block[waterlogged=b:false]",
        "GRASS_BLOCK[SNOWY=FALSE,WATERLOGGED=FALSE]",
        "minecraft:grass_block[waterlogged=b:false]",
    ),
    (
        "minecraft:short_grass[waterlogged=b:false]",
        "SHORT_GRASS[WATERLOGGED=FALSE]",
        "minecraft:short_grass[waterlogged=b:false]",
    ),
    (
        "minecraft:tallgrass[tall_grass_type=tall,waterlogged=b:false]",
        "SHORT_GRASS[WATERLOGGED=FALSE]",
        "minecraft:short_grass[waterlogged=b:false]",
    ),
    (
        "minecraft:piston[facing_direction=i:1,waterlogged=b:false]",
        "PISTON[EXTENDED=FALSE,FACING_ALL=UP,WATERLOGGED=FALSE]",
        "minecraft:piston[facing_direction=i:1,waterlogged=b:false]",
    ),
    (
        "minecraft:lever[lever_direction=up_north_south,open_bit=b:true,waterlogged=b:false]",
        "LEVER[ATTACHMENT_TYPE=FLOOR,FACING_HORIZONTAL=SOUTH,POWERED=FALSE,WATERLOGGED=FALSE]",
        "minecraft:lever[lever_direction=up_north_south,open_bit=b:true,waterlogged=b:false]",
    ),
    (
        "minecraft:bed[direction=i:2,head_piece_bit=b:true,occupied_bit=b:false,waterlogged=b:false]",
        "WHITE_BED[BED_PART=HEAD,FACING_HORIZONTAL=NORTH,OCCUPIED=FALSE,WATERLOGGED=FALSE]",
        "minecraft:bed[direction=i:2,head_piece_bit=b:true,occupied_bit=b:false,waterlogged=b:false]",
    ),
    (
        "minecraft:cauldron[cauldron_liquid=water,fill_level=i:3,waterlogged=b:false]",
        "WATER_CAULDRON[CAULDRON_LEVEL=_3,WATERLOGGED=FALSE]",
        "minecraft:cauldron[cauldron_liquid=water,fill_level=i:3,waterlogged=b:false]",
    ),
    (
        "minecraft:oak_slab[minecraft:vertical_half=top,waterlogged=b:false]",
        "OAK_SLAB[SLAB_TYPE=TOP,WATERLOGGED=FALSE]",
        "minecraft:oak_slab[minecraft:vertical_half=top,waterlogged=b:false]",
    ),
    (
        "minecraft:oak_double_slab[minecraft:vertical_half=bottom,waterlogged=b:false]",
        "OAK_SLAB[SLAB_TYPE=DOUBLE,WATERLOGGED=FALSE]",
        "minecraft:oak_double_slab[minecraft:vertical_half=bottom,waterlogged=b:false]",
    ),
    (
        "minecraft:double_wooden_slab[wood_type=oak,waterlogged=b:false]",
        "OAK_SLAB[SLAB_TYPE=DOUBLE,WATERLOGGED=FALSE]",
        "minecraft:oak_double_slab[minecraft:vertical_half=bottom,waterlogged=b:false]",
    ),
    ("minecraft:air[waterlogged=b:false]", "AIR[WATERLOGGED=FALSE]", "minecraft:air[waterlogged=b:false]"),
    (
        "minecraft:bubble_column[drag_down=b:true,waterlogged=b:false]",
        "BUBBLE_COLUMN[DRAG=TRUE,WATERLOGGED=FALSE]",
        "minecraft:bubble_column[drag_down=b:true,waterlogged=b:false]",
    ),
    (
        "minecraft:redstone_wire[redstone_signal=i:9,waterlogged=b:false]",
        "REDSTONE_WIRE[POWER=_9,REDSTONE_EAST=NONE,REDSTONE_NORTH=NONE,REDSTONE_SOUTH=NONE,REDSTONE_WEST=NONE,WATERLOGGED=FALSE]",
        "minecraft:redstone_wire[redstone_signal=i:9,waterlogged=b:false]",
    ),
    (
        "minecraft:brick_block[waterlogged=b:false]",
        "BRICKS[WATERLOGGED=FALSE]",
        "minecraft:brick_block[waterlogged=b:false]",
    ),
    (
        "minecraft:end_rod[facing_direction=i:0,waterlogged=b:false]",
        "END_ROD[FACING_ALL=DOWN,WATERLOGGED=FALSE]",
        "minecraft:end_rod[facing_direction=i:0,waterlogged=b:false]",
    ),
    (
        "minecraft:oak_fence[waterlogged=b:true]",
        "OAK_FENCE[EAST=FALSE,NORTH=FALSE,SOUTH=FALSE,WATERLOGGED=TRUE,WEST=FALSE]",
        "minecraft:oak_fence[waterlogged=b:true]",
    ),
    (
        "minecraft:standing_banner[ground_sign_direction=i:7,waterlogged=b:false]",
        "WHITE_BANNER[ROTATION=_7,WATERLOGGED=FALSE]",
        "minecraft:standing_banner[ground_sign_direction=i:7,waterlogged=b:false]",
    ),
    (
        "minecraft:wall_banner[facing_direction=i:2,waterlogged=b:false]",
        "WHITE_WALL_BANNER[FACING_HORIZONTAL=NORTH,WATERLOGGED=FALSE]",
        "minecraft:wall_banner[facing_direction=i:2,waterlogged=b:false]",
    ),
    (
        "minecraft:trapdoor[direction=i:0,open_bit=b:true,upside_down_bit=b:false,waterlogged=b:false]",
        "OAK_TRAPDOOR[FACING_HORIZONTAL=EAST,HALF=BOTTOM,OPEN=TRUE,POWERED=FALSE,WATERLOGGED=FALSE]",
        "minecraft:trapdoor[direction=i:0,open_bit=b:true,upside_down_bit=b:false,waterlogged=b:false]",
    ),
    (
        "minecraft:fence_gate[direction=i:2,in_wall_bit=b:false,open_bit=b:true,waterlogged=b:false]",
        "OAK_FENCE_GATE[FACING_HORIZONTAL=NORTH,IN_WALL=FALSE,OPEN=TRUE,POWERED=FALSE,WATERLOGGED=FALSE]",
        "minecraft:fence_gate[direction=i:2,in_wall_bit=b:false,open_bit=b:true,waterlogged=b:false]",
    ),
    (
        "minecraft:ladder[facing_direction=i:3,waterlogged=b:false]",
        "LADDER[FACING_HORIZONTAL=SOUTH,WATERLOGGED=FALSE]",
        "minecraft:ladder[facing_direction=i:3,waterlogged=b:false]",
    ),
    (
        "minecraft:torch[torch_facing_direction=top,waterlogged=b:false]",
        "TORCH[WATERLOGGED=FALSE]",
        "minecraft:torch[torch_facing_direction=top,waterlogged=b:false]",
    ),
];

const COMPOSED_CASES: &[(&str, &str, &str)] = &[
    (
        "minecraft:oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=false]",
        "minecraft:oak_stairs[upside_down_bit=b:false,waterlogged=b:false,weirdo_direction=i:3]",
        "minecraft:oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=false]",
    ),
    (
        "minecraft:oak_stairs[facing=east,half=top,shape=straight,waterlogged=true]",
        "minecraft:oak_stairs[upside_down_bit=b:true,waterlogged=b:true,weirdo_direction=i:0]",
        "minecraft:oak_stairs[facing=east,half=top,shape=straight,waterlogged=true]",
    ),
    (
        "minecraft:cobblestone_stairs[facing=south,half=bottom,shape=straight,waterlogged=false]",
        "minecraft:stone_stairs[upside_down_bit=b:false,waterlogged=b:false,weirdo_direction=i:2]",
        "minecraft:cobblestone_stairs[facing=south,half=bottom,shape=straight,waterlogged=false]",
    ),
    (
        "minecraft:oak_slab[type=bottom,waterlogged=false]",
        "minecraft:oak_slab[minecraft:vertical_half=bottom,waterlogged=b:false]",
        "minecraft:oak_slab[type=bottom,waterlogged=false]",
    ),
    (
        "minecraft:oak_slab[type=top,waterlogged=false]",
        "minecraft:oak_slab[minecraft:vertical_half=top,waterlogged=b:false]",
        "minecraft:oak_slab[type=top,waterlogged=false]",
    ),
    (
        "minecraft:oak_slab[type=double,waterlogged=false]",
        "minecraft:oak_double_slab[minecraft:vertical_half=bottom,waterlogged=b:false]",
        "minecraft:oak_slab[type=double,waterlogged=false]",
    ),
    (
        "minecraft:oak_door[facing=east,half=lower,hinge=left,open=false,powered=false]",
        "minecraft:wooden_door[direction=i:0,door_hinge_bit=b:false,open_bit=b:false,upper_block_bit=b:false,waterlogged=b:false]",
        "minecraft:oak_door[facing=east,half=lower,hinge=left,open=false,powered=false]",
    ),
    (
        "minecraft:oak_door[facing=west,half=upper,hinge=right,open=true,powered=true]",
        "minecraft:wooden_door[direction=i:2,door_hinge_bit=b:true,open_bit=b:true,upper_block_bit=b:true,waterlogged=b:false]",
        "minecraft:oak_door[facing=west,half=upper,hinge=right,open=true,powered=false]",
    ),
    (
        "minecraft:oak_trapdoor[facing=north,half=top,open=true,powered=false,waterlogged=false]",
        "minecraft:trapdoor[direction=i:3,open_bit=b:true,upside_down_bit=b:true,waterlogged=b:false]",
        "minecraft:oak_trapdoor[facing=north,half=top,open=true,powered=false,waterlogged=false]",
    ),
    (
        "minecraft:oak_fence_gate[facing=south,in_wall=false,open=true,powered=false]",
        "minecraft:fence_gate[direction=i:0,in_wall_bit=b:false,open_bit=b:true,waterlogged=b:false]",
        "minecraft:oak_fence_gate[facing=south,in_wall=false,open=true,powered=false]",
    ),
    (
        "minecraft:chest[facing=south,type=single,waterlogged=false]",
        "minecraft:chest[minecraft:cardinal_direction=south,waterlogged=b:false]",
        "minecraft:chest[facing=south,type=single,waterlogged=false]",
    ),
    (
        "minecraft:chest[facing=west,type=left,waterlogged=false]",
        "minecraft:chest[minecraft:cardinal_direction=west,waterlogged=b:false]",
        "minecraft:chest[facing=west,type=single,waterlogged=false]",
    ),
    (
        "minecraft:trapped_chest[facing=east,type=single,waterlogged=false]",
        "minecraft:trapped_chest[minecraft:cardinal_direction=east,waterlogged=b:false]",
        "minecraft:trapped_chest[facing=east,type=single,waterlogged=false]",
    ),
    (
        "minecraft:ender_chest[facing=north,waterlogged=false]",
        "minecraft:ender_chest[minecraft:cardinal_direction=north,waterlogged=b:false]",
        "minecraft:ender_chest[facing=north,waterlogged=false]",
    ),
    (
        "minecraft:oak_wall_sign[facing=east,waterlogged=false]",
        "minecraft:wall_sign[facing_direction=i:5,waterlogged=b:false]",
        "minecraft:oak_wall_sign[facing=east,waterlogged=false]",
    ),
    (
        "minecraft:oak_sign[rotation=3,waterlogged=false]",
        "minecraft:standing_sign[ground_sign_direction=i:3,waterlogged=b:false]",
        "minecraft:oak_sign[rotation=3,waterlogged=false]",
    ),
    (
        "minecraft:spruce_hanging_sign[attached=false,rotation=4,waterlogged=false]",
        "minecraft:spruce_hanging_sign[attached_bit=b:false,facing_direction=i:4,ground_sign_direction=i:4,hanging=b:true,waterlogged=b:false]",
        "minecraft:spruce_hanging_sign[attached=false,rotation=4,waterlogged=false]",
    ),
    ("minecraft:oak_log[axis=x]", "minecraft:oak_log[pillar_axis=x,waterlogged=b:false]", "minecraft:oak_log[axis=x]"),
    (
        "minecraft:stripped_oak_log[axis=z]",
        "minecraft:stripped_oak_log[pillar_axis=z,waterlogged=b:false]",
        "minecraft:stripped_oak_log[axis=z]",
    ),
    ("minecraft:oak_planks", "minecraft:oak_planks[waterlogged=b:false]", "minecraft:oak_planks[]"),
    ("minecraft:stone", "minecraft:stone[waterlogged=b:false]", "minecraft:stone[]"),
    ("minecraft:granite", "minecraft:granite[waterlogged=b:false]", "minecraft:granite[]"),
    ("minecraft:cobblestone", "minecraft:cobblestone[waterlogged=b:false]", "minecraft:cobblestone[]"),
    ("minecraft:white_wool", "minecraft:white_wool[waterlogged=b:false]", "minecraft:white_wool[]"),
    ("minecraft:red_wool", "minecraft:red_wool[waterlogged=b:false]", "minecraft:red_wool[]"),
    ("minecraft:white_concrete", "minecraft:white_concrete[waterlogged=b:false]", "minecraft:white_concrete[]"),
    ("minecraft:glass", "minecraft:glass[waterlogged=b:false]", "minecraft:glass[]"),
    ("minecraft:water[level=0]", "minecraft:water[liquid_depth=i:0,waterlogged=b:false]", "minecraft:water[level=0]"),
    ("minecraft:lava[level=2]", "minecraft:lava[liquid_depth=i:2,waterlogged=b:false]", "minecraft:lava[level=2]"),
    ("minecraft:wheat[age=7]", "minecraft:wheat[growth=i:7,waterlogged=b:false]", "minecraft:wheat[age=7]"),
    ("minecraft:carrots[age=3]", "minecraft:carrots[growth=i:3,waterlogged=b:false]", "minecraft:carrots[age=3]"),
    (
        "minecraft:oak_leaves[distance=7,persistent=true,waterlogged=false]",
        "minecraft:oak_leaves[persistent_bit=b:true,update_bit=b:false,waterlogged=b:false]",
        "minecraft:oak_leaves[distance=1,persistent=true,waterlogged=false]",
    ),
    ("minecraft:short_grass", "minecraft:short_grass[waterlogged=b:false]", "minecraft:short_grass[]"),
    (
        "minecraft:grass_block[snowy=false]",
        "minecraft:grass_block[waterlogged=b:false]",
        "minecraft:grass_block[snowy=false]",
    ),
    ("minecraft:dirt", "minecraft:dirt[waterlogged=b:false]", "minecraft:dirt[]"),
    ("minecraft:sand", "minecraft:sand[waterlogged=b:false]", "minecraft:sand[]"),
    (
        "minecraft:redstone_wire[east=side,north=up,power=7,south=none,west=none]",
        "minecraft:redstone_wire[redstone_signal=i:7,waterlogged=b:false]",
        "minecraft:redstone_wire[east=none,north=none,power=7,south=none,west=none]",
    ),
    (
        "minecraft:repeater[delay=3,facing=south,locked=false,powered=false]",
        "minecraft:unpowered_repeater[minecraft:cardinal_direction=south,repeater_delay=i:2,waterlogged=b:false]",
        "minecraft:repeater[delay=3,facing=south,locked=false,powered=false]",
    ),
    (
        "minecraft:piston[extended=false,facing=up]",
        "minecraft:piston[facing_direction=i:1,waterlogged=b:false]",
        "minecraft:piston[extended=false,facing=up]",
    ),
    (
        "minecraft:lever[face=wall,facing=north,powered=true]",
        "minecraft:lever[lever_direction=north,open_bit=b:true,waterlogged=b:false]",
        "minecraft:lever[face=wall,facing=north,powered=true]",
    ),
    (
        "minecraft:stone_button[face=floor,facing=east,powered=false]",
        "minecraft:stone_button[button_pressed_bit=b:false,facing_direction=i:1,waterlogged=b:false]",
        "minecraft:stone_button[face=floor,facing=north,powered=false]",
    ),
    (
        "minecraft:command_block[conditional=false,facing=up]",
        "minecraft:command_block[conditional_bit=b:false,facing_direction=i:1,waterlogged=b:false]",
        "minecraft:command_block[conditional=false,facing=up]",
    ),
    (
        "minecraft:chain_command_block[conditional=true,facing=west]",
        "minecraft:chain_command_block[conditional_bit=b:true,facing_direction=i:4,waterlogged=b:false]",
        "minecraft:chain_command_block[conditional=true,facing=west]",
    ),
    ("minecraft:torch", "minecraft:torch[torch_facing_direction=top,waterlogged=b:false]", "minecraft:torch[]"),
    (
        "minecraft:wall_torch[facing=east]",
        "minecraft:torch[torch_facing_direction=west,waterlogged=b:false]",
        "minecraft:wall_torch[facing=east]",
    ),
    (
        "minecraft:ladder[facing=west,waterlogged=false]",
        "minecraft:ladder[facing_direction=i:4,waterlogged=b:false]",
        "minecraft:ladder[facing=west,waterlogged=false]",
    ),
    (
        "minecraft:red_bed[facing=north,occupied=false,part=head]",
        "minecraft:bed[direction=i:2,head_piece_bit=b:true,occupied_bit=b:false,waterlogged=b:false]",
        "minecraft:white_bed[facing=north,occupied=false,part=head]",
    ),
    (
        "minecraft:white_banner[rotation=5]",
        "minecraft:standing_banner[ground_sign_direction=i:5,waterlogged=b:false]",
        "minecraft:white_banner[rotation=5]",
    ),
    (
        "minecraft:red_wall_banner[facing=west]",
        "minecraft:wall_banner[facing_direction=i:4,waterlogged=b:false]",
        "minecraft:white_wall_banner[facing=west]",
    ),
    (
        "minecraft:cauldron",
        "minecraft:cauldron[cauldron_liquid=water,fill_level=i:0,waterlogged=b:false]",
        "minecraft:cauldron[]",
    ),
    (
        "minecraft:water_cauldron[level=2]",
        "minecraft:cauldron[cauldron_liquid=water,fill_level=i:4,waterlogged=b:false]",
        "minecraft:water_cauldron[level=2]",
    ),
    (
        "minecraft:lava_cauldron",
        "minecraft:cauldron[cauldron_liquid=lava,fill_level=i:6,waterlogged=b:false]",
        "minecraft:lava_cauldron[]",
    ),
    (
        "minecraft:cobblestone_wall[east=low,north=tall,south=none,up=true,waterlogged=false,west=none]",
        "minecraft:cobblestone_wall[wall_connection_type_east=short,wall_connection_type_north=tall,wall_connection_type_south=none,wall_connection_type_west=none,wall_post_bit=b:true,waterlogged=b:false]",
        "minecraft:cobblestone_wall[east=low,north=tall,south=none,up=true,waterlogged=false,west=none]",
    ),
    (
        "minecraft:oak_fence[east=true,north=false,south=true,waterlogged=false,west=false]",
        "minecraft:oak_fence[waterlogged=b:false]",
        "minecraft:oak_fence[east=false,north=false,south=false,waterlogged=false,west=false]",
    ),
    (
        "minecraft:snow[layers=3]",
        "minecraft:snow_layer[covered_bit=b:false,height=i:2,waterlogged=b:false]",
        "minecraft:snow[layers=3]",
    ),
    (
        "minecraft:farmland[moisture=7]",
        "minecraft:farmland[moisturized_amount=i:7,waterlogged=b:false]",
        "minecraft:farmland[moisture=7]",
    ),
    (
        "minecraft:end_rod[facing=down]",
        "minecraft:end_rod[facing_direction=i:0,waterlogged=b:false]",
        "minecraft:end_rod[facing=down]",
    ),
    ("minecraft:bookshelf", "minecraft:bookshelf[waterlogged=b:false]", "minecraft:bookshelf[]"),
    ("minecraft:crafting_table", "minecraft:crafting_table[waterlogged=b:false]", "minecraft:crafting_table[]"),
    (
        "minecraft:furnace[facing=north,lit=true]",
        "minecraft:lit_furnace[minecraft:cardinal_direction=north,waterlogged=b:false]",
        "minecraft:furnace[facing=north,lit=true]",
    ),
    (
        "minecraft:oak_pressure_plate[powered=true]",
        "minecraft:wooden_pressure_plate[redstone_signal=i:1,waterlogged=b:false]",
        "minecraft:oak_pressure_plate[powered=true]",
    ),
    (
        "minecraft:note_block[instrument=harp,note=4,powered=false]",
        "minecraft:noteblock[waterlogged=b:false]",
        "minecraft:note_block[instrument=harp,note=0,powered=false]",
    ),
    (
        "minecraft:tall_grass[half=upper]",
        "minecraft:tall_grass[upper_block_bit=b:true,waterlogged=b:false]",
        "minecraft:tall_grass[half=upper]",
    ),
    (
        "minecraft:sunflower[half=lower]",
        "minecraft:sunflower[upper_block_bit=b:false,waterlogged=b:false]",
        "minecraft:sunflower[half=lower]",
    ),
    (
        "minecraft:nether_portal[axis=x]",
        "minecraft:portal[portal_axis=x,waterlogged=b:false]",
        "minecraft:nether_portal[axis=x]",
    ),
    (
        "minecraft:bubble_column[drag=true]",
        "minecraft:bubble_column[drag_down=b:true,waterlogged=b:true]",
        "minecraft:bubble_column[drag=true]",
    ),
    ("minecraft:kelp[age=5]", "minecraft:kelp[kelp_age=i:5,waterlogged=b:true]", "minecraft:kelp[age=5]"),
    (
        "minecraft:sea_pickle[pickles=2,waterlogged=true]",
        "minecraft:sea_pickle[cluster_count=i:1,dead_bit=b:false,waterlogged=b:true]",
        "minecraft:sea_pickle[pickles=2,waterlogged=true]",
    ),
    ("minecraft:air", "minecraft:air[waterlogged=b:false]", "minecraft:air[]"),
    (
        "minecraft:barrier[waterlogged=false]",
        "minecraft:barrier[waterlogged=b:false]",
        "minecraft:barrier[waterlogged=false]",
    ),
];
