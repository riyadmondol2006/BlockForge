// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Unit tests of the engine on tiny hand-made tables: every register flavour, grouping order, state groups.

use std::sync::Arc;

use crate::convert::mapping::engine::*;
use crate::convert::mapping::items::{ItemKind, ItemMapping, ItemResolver, PivotItem};
use crate::convert::mapping::{StateMap, StateValue, states};

fn a(s: &str) -> Arc<str> {
    Arc::from(s)
}

fn st(s: &str) -> StateValue {
    StateValue::str(s)
}

/// Pivot with FACING (name `facing`), HALF and WATERLOGGED; types STAIRS, SLAB-like A/B/C and PLAIN.
fn pivot() -> Arc<PivotInfo> {
    let mut p = PivotInfo::default();
    let mut state = |field: &str, name: &str, default: &str, values: &[&str]| {
        p.states.insert(
            a(field),
            StateInfo { name: a(name), default: a(default), values: values.iter().map(|v| a(v)).collect() },
        );
    };
    state("FACING", "facing", "NORTH", &["NORTH", "EAST", "SOUTH", "WEST"]);
    state("HALF", "half", "BOTTOM", &["BOTTOM", "TOP"]);
    state("WATERLOGGED", "waterlogged", "FALSE", &["FALSE", "TRUE"]);
    p.types.insert(a("STAIRS"), vec![a("FACING"), a("HALF"), a("WATERLOGGED")]);
    p.types.insert(a("PLAIN"), vec![a("WATERLOGGED")]);
    p.types.insert(a("A"), vec![a("HALF")]);
    p.types.insert(a("B"), vec![a("HALF")]);
    Arc::new(p)
}

fn tm(rows: &[(&str, &str)]) -> TypeMapping {
    let mut t = TypeMapping::new();
    for (i, o) in rows {
        t.mapping(vec![st(i)], vec![a(o)]).unwrap();
    }
    t
}

fn group(maps: &[(&str, &str, TypeMapping)]) -> StateGroup {
    StateGroup {
        states: maps
            .iter()
            .map(|(i, o, t)| StateMapping {
                input_names: vec![a(i)],
                output_states: vec![a(o)],
                ty: Arc::new(t.clone()),
            })
            .collect(),
        ..StateGroup::default()
    }
}

fn pb(ty: &str, s: &[(&str, &str)]) -> PivotBlock {
    PivotBlock::new(ty, s)
}

#[test]
fn register_inserts_both_directions() {
    let mut r = BlockResolver::empty(pivot());
    r.register(Flag::Register, BlockMapping::of("minecraft:plain", "PLAIN")).unwrap();
    assert_eq!(r.to_pivot("minecraft:plain", &StateMap::new()), Some(pb("PLAIN", &[])));
    assert_eq!(r.from_pivot(&pb("PLAIN", &[])), Some((a("minecraft:plain"), StateMap::new())));
    assert!(r.is_supported_id("minecraft:plain") && r.is_supported_type("PLAIN"));
    assert_eq!(r.to_pivot("minecraft:other", &StateMap::new()), None);
    assert_eq!(r.from_pivot(&pb("STAIRS", &[])), None);
    // A second register of the same key is an error in both directions.
    let err = r.register(Flag::Register, BlockMapping::of("minecraft:plain", "PLAIN")).unwrap_err();
    assert!(matches!(err, MappingError::Duplicate(_)));
}

#[test]
fn duplicate_input_only_writes_pivot_to_edition() {
    // Two pivot types write to one edition id (AIR / CAVE_AIR -> air); reading gives the first.
    let mut r = BlockResolver::empty(pivot());
    r.register(Flag::Register, BlockMapping::of("minecraft:air", "A")).unwrap();
    r.register(Flag::DuplicateInput, BlockMapping::of("minecraft:air", "B")).unwrap();
    assert_eq!(r.to_pivot("minecraft:air", &StateMap::new()).unwrap().ty, a("A"));
    assert_eq!(r.from_pivot(&pb("B", &[])).unwrap().0, a("minecraft:air"));
    assert_eq!(r.from_pivot(&pb("A", &[])).unwrap().0, a("minecraft:air"));
}

#[test]
fn duplicate_output_only_reads_edition_to_pivot() {
    // An alias accepted on read (grass -> SHORT_GRASS); writing uses the registered name.
    let mut r = BlockResolver::empty(pivot());
    r.register(Flag::Register, BlockMapping::of("minecraft:short_grass", "A")).unwrap();
    r.register(Flag::DuplicateOutput, BlockMapping::of("minecraft:grass", "A")).unwrap();
    assert_eq!(r.to_pivot("minecraft:grass", &StateMap::new()).unwrap().ty, a("A"));
    assert_eq!(r.from_pivot(&pb("A", &[])).unwrap().0, a("minecraft:short_grass"));
}

#[test]
fn override_output_replaces_the_written_name() {
    let mut r = BlockResolver::empty(pivot());
    r.register(Flag::Register, BlockMapping::of("minecraft:old", "A")).unwrap();
    r.register(Flag::OverrideOutput, BlockMapping::of("minecraft:new", "A")).unwrap();
    // both ids are read, the type is written as the new one
    assert_eq!(r.to_pivot("minecraft:old", &StateMap::new()).unwrap().ty, a("A"));
    assert_eq!(r.to_pivot("minecraft:new", &StateMap::new()).unwrap().ty, a("A"));
    assert_eq!(r.from_pivot(&pb("A", &[])).unwrap().0, a("minecraft:new"));
}

#[test]
fn override_input_replaces_the_read_type() {
    let mut r = BlockResolver::empty(pivot());
    r.register(Flag::Register, BlockMapping::of("minecraft:x", "A")).unwrap();
    r.register(Flag::OverrideInput, BlockMapping::of("minecraft:x", "B")).unwrap();
    assert_eq!(r.to_pivot("minecraft:x", &StateMap::new()).unwrap().ty, a("B"));
    assert_eq!(r.from_pivot(&pb("A", &[])).unwrap().0, a("minecraft:x"));
    assert_eq!(r.from_pivot(&pb("B", &[])).unwrap().0, a("minecraft:x"));
}

#[test]
fn override_input_output_replaces_both() {
    let mut r = BlockResolver::empty(pivot());
    r.register(Flag::Register, BlockMapping::of("minecraft:x", "A")).unwrap();
    r.register(Flag::Register, BlockMapping::of("minecraft:y", "B")).unwrap();
    // Same keys on both sides (id x with no pinned states, type A with none): the rows are replaced.
    let mut m = BlockMapping::of("minecraft:x", "A");
    m.states.clear();
    r.register(Flag::OverrideInputOutput, m).unwrap();
    assert_eq!(r.to_pivot("minecraft:x", &StateMap::new()).unwrap().ty, a("A"));
    // Pinning a different set of pivot states has no row to override on the way back.
    let err =
        r.register(Flag::OverrideInputOutput, BlockMapping::of("minecraft:y", "B").with("HALF", "TOP")).unwrap_err();
    assert!(matches!(err, MappingError::NothingToOverride(_)));
}

#[test]
fn duplicate_override_flavours_touch_one_direction() {
    let mut r = BlockResolver::empty(pivot());
    r.register(Flag::Register, BlockMapping::of("minecraft:x", "A")).unwrap();
    r.register(Flag::DuplicateOverrideInput, BlockMapping::of("minecraft:x", "B")).unwrap();
    assert_eq!(r.to_pivot("minecraft:x", &StateMap::new()).unwrap().ty, a("B"));
    assert_eq!(r.from_pivot(&pb("A", &[])).unwrap().0, a("minecraft:x"));
    assert_eq!(r.from_pivot(&pb("B", &[])), None); // pivot -> edition was untouched

    let mut r = BlockResolver::empty(pivot());
    r.register(Flag::Register, BlockMapping::of("minecraft:x", "A")).unwrap();
    r.register(Flag::DuplicateOverrideOutput, BlockMapping::of("minecraft:y", "A")).unwrap();
    assert_eq!(r.to_pivot("minecraft:x", &StateMap::new()).unwrap().ty, a("A"));
    assert_eq!(r.to_pivot("minecraft:y", &StateMap::new()), None);
    assert_eq!(r.from_pivot(&pb("A", &[])).unwrap().0, a("minecraft:y"));
}

#[test]
fn override_without_existing_entry_is_an_error() {
    for flag in [
        Flag::OverrideOutput,
        Flag::OverrideInput,
        Flag::OverrideInputOutput,
        Flag::DuplicateOverrideInput,
        Flag::DuplicateOverrideOutput,
    ] {
        let mut r = BlockResolver::empty(pivot());
        let err = r.register(flag, BlockMapping::of("minecraft:none", "A")).unwrap_err();
        assert!(matches!(err, MappingError::NothingToOverride(_)), "{flag:?}");
        // A failed register leaves the tables untouched.
        assert_eq!(r.row_counts(), (0, 0), "{flag:?}");
    }
}

#[test]
fn flag_modes_match_chunker() {
    use Mode::{Insert, Override, Skip};
    assert_eq!(Flag::Register.modes(), (Insert, Insert));
    assert_eq!(Flag::DuplicateInput.modes(), (Skip, Insert));
    assert_eq!(Flag::DuplicateOutput.modes(), (Insert, Skip));
    assert_eq!(Flag::OverrideOutput.modes(), (Insert, Override));
    assert_eq!(Flag::OverrideInput.modes(), (Override, Insert));
    assert_eq!(Flag::OverrideInputOutput.modes(), (Override, Override));
    assert_eq!(Flag::DuplicateOverrideInput.modes(), (Override, Skip));
    assert_eq!(Flag::DuplicateOverrideOutput.modes(), (Skip, Override));
}

#[test]
fn largest_pinned_set_wins() {
    // flatten-style rows: one id, different pinned sets.
    let mut r = BlockResolver::empty(pivot());
    r.register(Flag::Register, BlockMapping::of("minecraft:m", "PLAIN")).unwrap();
    r.register(Flag::DuplicateOutput, BlockMapping::of("minecraft:m", "A").when("kind", st("one"))).unwrap();
    r.register(
        Flag::DuplicateOutput,
        BlockMapping::of("minecraft:m", "B").when("kind", st("one")).when("size", StateValue::Int(2)),
    )
    .unwrap();
    let both = states(&[("kind", st("one")), ("size", StateValue::Int(2))]);
    assert_eq!(r.to_pivot("minecraft:m", &both).unwrap().ty, a("B"));
    let kind = states(&[("kind", st("one")), ("size", StateValue::Int(3))]);
    assert_eq!(r.to_pivot("minecraft:m", &kind).unwrap().ty, a("A"));
    let none = states(&[("kind", st("two"))]);
    assert_eq!(r.to_pivot("minecraft:m", &none).unwrap().ty, a("PLAIN"));
    // Pivot -> edition: pinned pivot states choose among rows too.
    let mut r = BlockResolver::empty(pivot());
    r.register(Flag::Register, BlockMapping::of("minecraft:bottom", "A").with("HALF", "BOTTOM")).unwrap();
    r.register(Flag::Register, BlockMapping::of("minecraft:top", "A").with("HALF", "TOP")).unwrap();
    assert_eq!(r.from_pivot(&pb("A", &[("HALF", "TOP")])).unwrap().0, a("minecraft:top"));
    assert_eq!(r.from_pivot(&pb("A", &[("HALF", "BOTTOM")])).unwrap().0, a("minecraft:bottom"));
    // A missing pinned state falls back to the type's default (HALF defaults to BOTTOM).
    assert_eq!(r.from_pivot(&pb("A", &[])).unwrap().0, a("minecraft:bottom"));
    // A pivot value no row pins is a miss.
    assert_eq!(r.from_pivot(&pb("A", &[("HALF", "MIDDLE")])), None);
}

#[test]
fn state_group_converts_states_both_ways() {
    let facing = tm(&[("north", "NORTH"), ("east", "EAST"), ("south", "SOUTH"), ("west", "WEST")]);
    let half = tm(&[("bottom", "BOTTOM"), ("top", "TOP")]);
    let g = Arc::new(group(&[("facing", "FACING", facing), ("half", "HALF", half)]));
    let mut r = BlockResolver::empty(pivot());
    r.register(Flag::Register, BlockMapping::of("minecraft:stairs", "STAIRS").group(&g)).unwrap();
    let java = states(&[("facing", st("south")), ("half", st("top"))]);
    let p = r.to_pivot("minecraft:stairs", &java).unwrap();
    assert_eq!(p, pb("STAIRS", &[("FACING", "SOUTH"), ("HALF", "TOP")]));
    assert_eq!(r.from_pivot(&p), Some((a("minecraft:stairs"), java)));
    // A missing edition state stays missing; a missing pivot state is written as the pivot default.
    assert_eq!(
        r.to_pivot("minecraft:stairs", &states(&[("half", st("top"))])).unwrap(),
        pb("STAIRS", &[("HALF", "TOP")])
    );
    let (_, written) = r.from_pivot(&pb("STAIRS", &[("HALF", "TOP")])).unwrap();
    assert_eq!(written, states(&[("facing", st("north")), ("half", st("top"))]));
}

#[test]
fn type_mapping_defaults_and_lossy_values() {
    // Three edition values collapse to one pivot value; the reverse row picks the written one. Unknown values use the default.
    let mut t = TypeMapping::new();
    t.mapping(vec![st("a")], vec![a("NORTH")]).unwrap();
    t.input_to_output.insert(vec![st("b")], vec![a("NORTH")]);
    t.mapping(vec![st("c")], vec![a("EAST")]).unwrap();
    t.defaults(vec![st("a")], vec![a("WEST")]);
    let g = Arc::new(group(&[("dir", "FACING", t)]));
    let mut r = BlockResolver::empty(pivot());
    r.register(Flag::Register, BlockMapping::of("minecraft:d", "STAIRS").group(&g)).unwrap();
    let get = |v: &str| r.to_pivot("minecraft:d", &states(&[("dir", st(v))])).unwrap().states.get("FACING").cloned();
    assert_eq!(get("a"), Some(a("NORTH")));
    assert_eq!(get("b"), Some(a("NORTH")));
    assert_eq!(get("c"), Some(a("EAST")));
    assert_eq!(get("zzz"), Some(a("WEST"))); // default output
    let w = r.from_pivot(&pb("STAIRS", &[("FACING", "NORTH")])).unwrap().1;
    assert_eq!(w.get("dir"), Some(&st("a")));
    let w = r.from_pivot(&pb("STAIRS", &[("FACING", "SOUTH")])).unwrap().1; // not mapped: default input
    assert_eq!(w.get("dir"), Some(&st("a")));
}

#[test]
fn duplicate_type_mapping_rows_are_rejected() {
    let mut t = TypeMapping::new();
    t.mapping(vec![st("a")], vec![a("NORTH")]).unwrap();
    assert!(t.mapping(vec![st("a")], vec![a("NORTH")]).is_err());
    // Same input with a different output is allowed (put-if-absent keeps the first).
    t.mapping(vec![st("a")], vec![a("EAST")]).unwrap();
    assert_eq!(t.input_to_output[&vec![st("a")]], vec![a("NORTH")]);
}

#[test]
fn default_outputs_and_inputs_fill_gaps() {
    let mut g = group(&[("half", "HALF", tm(&[("bottom", "BOTTOM"), ("top", "TOP")]))]);
    g.default_outputs.insert(a("FACING"), a("EAST"));
    g.default_inputs.insert(a("bit"), StateValue::Bool(false));
    let g = Arc::new(g);
    let mut r = BlockResolver::empty(pivot());
    r.register(Flag::Register, BlockMapping::of("minecraft:s", "STAIRS").group(&g)).unwrap();
    let p = r.to_pivot("minecraft:s", &states(&[("half", st("top"))])).unwrap();
    assert_eq!(p, pb("STAIRS", &[("FACING", "EAST"), ("HALF", "TOP")]));
    let (_, w) = r.from_pivot(&p).unwrap();
    assert_eq!(w, states(&[("bit", StateValue::Bool(false)), ("half", st("top"))]));
    // A default output is not overridden by a state that is present...
    let mut g2 = group(&[("facing", "FACING", tm(&[("e", "EAST")]))]);
    g2.default_outputs.insert(a("FACING"), a("WEST"));
    let g2 = Arc::new(g2);
    let mut r = BlockResolver::empty(pivot());
    r.register(Flag::Register, BlockMapping::of("minecraft:s", "STAIRS").group(&g2)).unwrap();
    let p = r.to_pivot("minecraft:s", &states(&[("facing", st("e"))])).unwrap();
    assert_eq!(p.states.get("FACING"), Some(&a("EAST")));
    // ...and with the state absent the default output applies instead of the type mapping default.
    let p = r.to_pivot("minecraft:s", &StateMap::new()).unwrap();
    assert_eq!(p.states.get("FACING"), Some(&a("WEST")));
}

#[test]
fn extra_group_waterlogged_like_bedrock() {
    let bools = {
        let mut t = TypeMapping::new();
        t.mapping(vec![StateValue::Bool(false)], vec![a("FALSE")]).unwrap();
        t.mapping(vec![StateValue::Bool(true)], vec![a("TRUE")]).unwrap();
        t
    };
    let water = group(&[("waterlogged", "WATERLOGGED", bools)]);
    let mut r = BlockResolver::empty(pivot());
    r.extra_group(Arc::new(water));
    r.register(Flag::Register, BlockMapping::of("minecraft:plain", "PLAIN")).unwrap();
    let p = r.to_pivot("minecraft:plain", &states(&[("waterlogged", StateValue::Bool(true))])).unwrap();
    assert_eq!(p, pb("PLAIN", &[("WATERLOGGED", "TRUE")]));
    let (_, w) = r.from_pivot(&p).unwrap();
    assert_eq!(w, states(&[("waterlogged", StateValue::Bool(true))]));
    // Absent -> pivot default FALSE is written for types that have the state.
    let (_, w) = r.from_pivot(&pb("PLAIN", &[])).unwrap();
    assert_eq!(w, states(&[("waterlogged", StateValue::Bool(false))]));
}

#[test]
fn extra_group_default_output_like_java() {
    let mut g = StateGroup::default();
    g.default_outputs.insert(a("WATERLOGGED"), a("FALSE"));
    let mut r = BlockResolver::empty(pivot());
    r.extra_group(Arc::new(g));
    r.register(Flag::Register, BlockMapping::of("minecraft:plain", "PLAIN")).unwrap();
    assert_eq!(r.to_pivot("minecraft:plain", &StateMap::new()).unwrap(), pb("PLAIN", &[("WATERLOGGED", "FALSE")]));
}

#[test]
fn pinned_states_on_both_sides() {
    // minecraft:double_slab -> A with HALF=TOP pinned; an edition state pinned on the way back.
    let mut r = BlockResolver::empty(pivot());
    r.register(
        Flag::Register,
        BlockMapping::of("minecraft:slab_top", "A").with("HALF", "TOP").when("kind", st("slab")),
    )
    .unwrap();
    let p = r.to_pivot("minecraft:slab_top", &states(&[("kind", st("slab"))])).unwrap();
    assert_eq!(p, pb("A", &[("HALF", "TOP")]));
    assert_eq!(r.to_pivot("minecraft:slab_top", &states(&[("kind", st("other"))])), None);
    assert_eq!(r.to_pivot("minecraft:slab_top", &StateMap::new()), None);
    let (id, w) = r.from_pivot(&p).unwrap();
    assert_eq!((&*id, w), ("minecraft:slab_top", states(&[("kind", st("slab"))])));
}

#[test]
fn lookups_are_cached_and_consistent() {
    let mut r = BlockResolver::empty(pivot());
    r.register(Flag::Register, BlockMapping::of("minecraft:plain", "PLAIN")).unwrap();
    for _ in 0..3 {
        assert_eq!(r.to_pivot("minecraft:plain", &StateMap::new()), Some(pb("PLAIN", &[])));
        assert_eq!(r.to_pivot("minecraft:nope", &StateMap::new()), None); // misses are cached as misses
    }
    assert_eq!(r.row_counts(), (1, 1));
    r.remove_output_mapping("PLAIN");
    assert!(!r.is_supported_type("PLAIN"));
}

#[test]
fn items_with_data_values_and_properties() {
    let mut r = ItemResolver::empty(pivot(), false);
    r.register(Flag::Register, ItemMapping::of("minecraft:banner", ItemKind::Item("BLACK_BANNER".into()))).unwrap();
    r.register(Flag::Register, ItemMapping::of("minecraft:banner", ItemKind::Item("RED_BANNER".into())).data(1))
        .unwrap();
    r.register(
        Flag::Register,
        ItemMapping::of("minecraft:spawn_egg", ItemKind::Item("SPAWN_EGG".into()))
            .data(10)
            .prop("P:spawn_egg_mob", "ALLAY"),
    )
    .unwrap();
    r.register(
        Flag::Register,
        ItemMapping::of("minecraft:bed", ItemKind::Block("A".into())).data(3).prop("S:HALF", "TOP"),
    )
    .unwrap();
    let get = |id: &str, data: Option<i32>| {
        let s = data.map_or_else(StateMap::new, |d| states(&[("data", StateValue::Int(d))]));
        r.to_pivot(id, &s)
    };
    assert_eq!(get("minecraft:banner", Some(1)).unwrap().kind, ItemKind::Item("RED_BANNER".into()));
    assert_eq!(get("minecraft:banner", Some(9)).unwrap().kind, ItemKind::Item("BLACK_BANNER".into())); // fallback row
    assert_eq!(get("minecraft:banner", None).unwrap().kind, ItemKind::Item("BLACK_BANNER".into()));
    assert_eq!(get("minecraft:banner", Some(7)).unwrap().props.len(), 0);
    let egg = get("minecraft:spawn_egg", Some(10)).unwrap();
    assert_eq!(egg, PivotItem::new(ItemKind::Item("SPAWN_EGG".into()), &[("P:spawn_egg_mob", "ALLAY")]));
    let (id, w) = r.from_pivot(&egg).unwrap();
    assert_eq!((&*id, w), ("minecraft:spawn_egg", states(&[("data", StateValue::Int(10))])));
    // Block items: a missing block state property uses the type's default (HALF defaults to BOTTOM -> no row).
    assert!(r.from_pivot(&PivotItem::new(ItemKind::Block("A".into()), &[])).is_none());
    assert!(r.from_pivot(&PivotItem::new(ItemKind::Block("A".into()), &[("S:HALF", "TOP")])).is_some());
    assert!(!r.is_supported_id("minecraft:stick"));
}

#[test]
fn item_default_data_is_added_for_legacy() {
    let mut r = ItemResolver::empty(pivot(), true);
    r.register(Flag::Register, ItemMapping::of("minecraft:stick", ItemKind::Item("STICK".into()))).unwrap();
    let (_, w) = r.from_pivot(&PivotItem::new(ItemKind::Item("STICK".into()), &[])).unwrap();
    assert_eq!(w, states(&[("data", StateValue::Int(0))]));
}
