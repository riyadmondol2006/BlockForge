// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Item checks, the Rust side of `JavaItemIdentifierValidationTests` and `BedrockItemIdentifierValidationTests`.
//!
//! Per version: every vanilla item id (block items excluded, they go through the block tables) maps to a pivot item
//! (`to_pivot`) and back to the same id without states (`roundtrip`); every pivot item type the version can write maps
//! (`from_pivot_maps`) to an id that exists in the version (`from_pivot_valid`). Like Chunker, the latest version of an
//! edition must support every pivot item type.

use std::collections::BTreeSet;
use std::path::Path;

use super::lossy;
use super::vdata::{self, VersionDir};
use super::{Tally, VerReport, par_map};
use crate::convert::mapping::items::{ItemKind, PivotItem};
use crate::convert::mapping::{Edition, ItemResolver, StateMap, Version, data};

/// Every item-only pivot type that any edition/version table mentions (`ChunkerVanillaItemType.values()`).
fn item_kinds(all_versions: &[Version]) -> BTreeSet<String> {
    let mut kinds = BTreeSet::new();
    for v in all_versions {
        for row in data::java_items(*v).into_iter().chain(data::bedrock_items(*v)) {
            if let ItemKind::Item(n) = &row.item {
                kinds.insert(n.to_string());
            }
        }
    }
    kinds
}

fn all_versions(dir: &Path) -> Vec<Version> {
    let mut v: Vec<Version> = vdata::version_dirs(dir, "java")
        .into_iter()
        .chain(vdata::version_dirs(dir, "bedrock"))
        .map(|d| d.version)
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Java output side: every pivot item type must map to an item of the version.
pub(crate) fn run_java(dir: &Path) -> Vec<VerReport> {
    let dirs = vdata::version_dirs(dir, "java");
    let latest = dirs.last().map(|d| d.version);
    let kinds = item_kinds(&all_versions(dir));
    par_map(&dirs, |d| {
        let resolver = ItemResolver::new(Edition::Java, d.version);
        match vdata::load_java_all_items(d) {
            Err(e) => load_failed("java", d, e),
            Ok(valid_ids) => {
                let (maps, valid) = check_from_pivot(&resolver, &valid_ids, &kinds, Some(d.version) == latest);
                VerReport {
                    ids: valid_ids.len(),
                    area: "items",
                    edition: "java",
                    version: d.name.clone(),
                    checks: vec![("from_pivot_maps", maps), ("from_pivot_valid", valid)],
                }
            }
        }
    })
}

/// Bedrock input side: every item id maps to the pivot and back.
pub(crate) fn run_bedrock(dir: &Path) -> Vec<VerReport> {
    let dirs = vdata::version_dirs(dir, "bedrock");
    par_map(&dirs, |d| {
        let resolver = ItemResolver::new(Edition::Bedrock, d.version);
        let items = match vdata::load_bedrock_items(d) {
            Ok(i) => i,
            Err(e) => return load_failed("bedrock", d, e),
        };
        let none = StateMap::new();
        let mut to_pivot = Tally::default();
        let mut roundtrip = Tally::default();
        for id in &items {
            let unsupported = lossy::bedrock_unsupported_item(id);
            let p = resolver.to_pivot(id, &none);
            match (&p, unsupported) {
                (Some(_), false) => to_pivot.pass(),
                (None, true) => to_pivot.allow(id),
                (None, false) => to_pivot.fail(id, || format!("{id} is not mapped")),
                (Some(p), true) => to_pivot.fail(id, || format!("{id} is mapped to {:?} but is unsupported", p.kind)),
            }
            let Some(p) = p else { continue };
            if unsupported {
                continue;
            }
            if lossy::bedrock_lossy_item(id) {
                roundtrip.allow(id);
                continue;
            }
            match resolver.from_pivot(&p) {
                None => roundtrip.fail(id, || format!("{id} -> {:?} could not map back", p.kind)),
                Some((back, states)) if &*back == id.as_str() && states.is_empty() => roundtrip.pass(),
                Some((back, states)) => roundtrip
                    .fail(id, || format!("{id} -> {:?} {:?} -> {}", p.kind, p.props, super::show(&back, &states))),
            }
        }
        VerReport {
            ids: items.len(),
            area: "items",
            edition: "bedrock",
            version: d.name.clone(),
            checks: vec![("to_pivot", to_pivot), ("roundtrip", roundtrip)],
        }
    })
}

fn load_failed(edition: &'static str, d: &VersionDir, e: String) -> VerReport {
    let mut t = Tally::default();
    t.fail("data", || e);
    VerReport { ids: 0, area: "items", edition, version: d.name.clone(), checks: vec![("load", t)] }
}

fn check_from_pivot(
    resolver: &ItemResolver,
    valid_ids: &BTreeSet<String>,
    kinds: &BTreeSet<String>,
    is_latest: bool,
) -> (Tally, Tally) {
    let mut maps = Tally::default();
    let mut valid = Tally::default();
    for kind in kinds {
        let k = ItemKind::Item(kind.as_str().into());
        if !(resolver.is_supported_kind(&k) || is_latest) {
            continue;
        }
        match resolver.from_pivot(&PivotItem { kind: k, props: Default::default() }) {
            None => maps.fail(kind, || format!("pivot item {kind} is not mapped")),
            Some((id, _)) => {
                maps.pass();
                if valid_ids.contains(&*id) {
                    valid.pass();
                } else {
                    valid.fail(kind, || format!("pivot item {kind} -> {id} is not an item of this version"));
                }
            }
        }
    }
    (maps, valid)
}
