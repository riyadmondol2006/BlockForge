// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Block checks per edition and version: the Rust side of `JavaBlockIdentifierValidationTests` and
//! `BedrockBlockIdentifierValidationTests`.
//!
//! Per version: every vanilla block state maps to a pivot block (`to_pivot`), the pivot block is well formed
//! (`pivot_valid`), an identifier without states equals its default state (`defaults`, Java), and the pivot block maps
//! back to the same block (`roundtrip`). In the other direction every state combination of every pivot type the
//! version can write maps to an edition block (`from_pivot_maps`) that exists in the version's data
//! (`from_pivot_valid`).

use std::path::Path;
use std::sync::Arc;

use super::lossy;
use super::vdata::{self, BlockData, VersionDir};
use super::{Tally, VerReport, par_map, pivot_combos, pivot_valid, show};
use crate::convert::mapping::engine::PivotBlock;
use crate::convert::mapping::{BlockResolver, Edition, StateMap, StateValue, data};

/// Bedrock blocks to the pivot (the input side of a Bedrock -> Java conversion).
pub(crate) fn run_bedrock(dir: &Path) -> Vec<VerReport> {
    par_map(&vdata::version_dirs(dir, "bedrock"), check_bedrock_input)
}

/// The pivot to Java blocks (the output side), for every Java version.
pub(crate) fn run_java(dir: &Path) -> Vec<VerReport> {
    par_map(&vdata::version_dirs(dir, "java"), check_java_output)
}

fn failed_load(edition: &'static str, d: &VersionDir, err: String) -> VerReport {
    let mut t = Tally::default();
    t.fail("data", || err);
    VerReport { ids: 0, area: "blocks", edition, version: d.name.clone(), checks: vec![("load", t)] }
}

pub(crate) fn load(edition: Edition, d: &VersionDir) -> Result<BlockData, String> {
    match edition {
        Edition::Java => vdata::load_java_blocks(d),
        Edition::Bedrock => vdata::load_bedrock_blocks(d),
    }
}

fn unsupported(edition: Edition, id: &str) -> bool {
    edition == Edition::Bedrock && lossy::bedrock_unsupported_block(id)
}

fn normalise(edition: Edition, id: &str, states: &StateMap) -> (String, StateMap) {
    match edition {
        Edition::Java => (id.to_string(), lossy::java_remove_lossy(id, states)),
        Edition::Bedrock => lossy::bedrock_remove_lossy(id, states),
    }
}

fn check_bedrock_input(d: &VersionDir) -> VerReport {
    let edition = Edition::Bedrock;
    let blocks = match load(edition, d) {
        Ok(b) => b,
        Err(e) => return failed_load("bedrock", d, e),
    };
    let resolver = BlockResolver::new(edition, d.version);
    let pivot = data::pivot();
    let mut to_pivot = Tally::default();
    let mut valid = Tally::default();
    let mut roundtrip = Tally::default();
    for (id, list) in &blocks.states {
        for st in list {
            let p = resolver.to_pivot(id, st);
            let skip = unsupported(edition, id);
            match (&p, skip) {
                (Some(_), false) => to_pivot.pass(),
                (None, true) => to_pivot.allow(id),
                (None, false) => to_pivot.fail(id, || format!("{} is not mapped", show(id, st))),
                (Some(p), true) => {
                    to_pivot.fail(id, || format!("{} is mapped to {} but is unsupported", show(id, st), p.ty))
                }
            }
            let Some(p) = p else { continue };
            if skip {
                continue;
            }
            match pivot_valid(&pivot, &p) {
                Ok(()) => valid.pass(),
                Err(e) => valid.fail(id, || format!("{} -> {}: {e}", show(id, st), p.ty)),
            }
            if lossy::bedrock_lossy_block(id, st) {
                roundtrip.allow(id);
                continue;
            }
            match resolver.from_pivot(&p) {
                None => roundtrip.fail(id, || format!("{} -> {} could not map back", show(id, st), show_pivot(&p))),
                Some((back_id, back)) => {
                    if normalise(edition, id, st) == normalise(edition, &back_id, &back) {
                        roundtrip.pass();
                    } else {
                        roundtrip.fail(id, || {
                            format!("{} -> {} -> {}", show(id, st), show_pivot(&p), show(&back_id, &back))
                        });
                    }
                }
            }
        }
    }
    VerReport {
        ids: blocks.states.len(),
        area: "blocks",
        edition: "bedrock",
        version: d.name.clone(),
        checks: vec![("to_pivot", to_pivot), ("pivot_valid", valid), ("roundtrip", roundtrip)],
    }
}

fn check_java_output(d: &VersionDir) -> VerReport {
    let blocks = match load(Edition::Java, d) {
        Ok(b) => b,
        Err(e) => return failed_load("java", d, e),
    };
    let resolver = BlockResolver::new(Edition::Java, d.version);
    let (maps, valid) = check_from_pivot(Edition::Java, &resolver, &blocks);
    VerReport {
        ids: blocks.states.len(),
        area: "blocks",
        edition: "java",
        version: d.name.clone(),
        checks: vec![("from_pivot_maps", maps), ("from_pivot_valid", valid)],
    }
}

/// Every state combination of every pivot type: it must map when the version supports the type, and whatever it maps
/// to must be a block that exists in the version.
fn check_from_pivot(edition: Edition, resolver: &BlockResolver, blocks: &BlockData) -> (Tally, Tally) {
    let info = data::pivot();
    let mut maps = Tally::default();
    let mut valid = Tally::default();
    let mut types: Vec<&Arc<str>> = info.types.keys().collect();
    types.sort();
    for ty in types {
        let supported = resolver.is_supported_type(ty);
        for combo in pivot_combos(&info, ty) {
            let block = PivotBlock { ty: ty.clone(), states: combo.into_iter().collect() };
            let out = resolver.from_pivot(&block);
            match (&out, supported) {
                (None, true) => maps.fail(ty, || format!("{} is not mapped", show_pivot(&block))),
                (Some(_), true) => maps.pass(),
                _ => {}
            }
            let Some((id, mut states)) = out else { continue };
            if edition == Edition::Bedrock {
                states.entry("waterlogged".into()).or_insert(StateValue::Bool(false));
            }
            match blocks.allowed.get(&*id) {
                None => valid.fail(ty, || format!("{} -> {id} is not a block of this version", show_pivot(&block))),
                Some(set) if set.contains(&states) => valid.pass(),
                Some(_) => valid.fail(ty, || {
                    format!("{} -> {} has no such state combination", show_pivot(&block), show(&id, &states))
                }),
            }
        }
    }
    (maps, valid)
}

pub(crate) fn show_pivot(p: &PivotBlock) -> String {
    let s: Vec<String> = p.states.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{}[{}]", p.ty, s.join(","))
}
