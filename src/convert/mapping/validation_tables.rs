// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The simple id tables for Bedrock -> Java: biomes, entity types, enchantments, effects, potions, trims, banner
//! patterns and paintings. Chunker's data folder has no vanilla list for these, so the checks are internal and
//! cross-edition: every Bedrock value in force at a Bedrock version maps to a pivot name (`to_pivot`), the pivot name
//! is unique, and the Java table at the closest and latest Java release writes it (`to_java`; `newer:` when only a
//! later Java release has it, `gap:` when no Java release has it).

use std::path::Path;

use super::cross::closest_java;
use super::vdata::{self, VersionDir};
use super::{Tally, VerReport, par_map};
use crate::convert::mapping::tables::{IdTable, names};
use crate::convert::mapping::{Edition, StateValue};

/// (check name, Bedrock table, Java table).
const PAIRS: &[(&str, &str, &str)] = &[
    ("biome_id>biome_name", names::BIOME_ID, names::BIOME_NAME),
    ("biome_id>biome_id", names::BIOME_ID, names::BIOME_ID),
    ("entity_type", names::ENTITY_TYPE, names::ENTITY_TYPE),
    ("entity_id>entity_type", names::ENTITY_ID, names::ENTITY_TYPE),
    ("enchantment_id>enchantment", names::ENCHANTMENT_ID, names::ENCHANTMENT),
    ("effect_id>effect", names::EFFECT_ID, names::EFFECT),
    ("potion_id>potion", names::POTION_ID, names::POTION),
    ("trim_material", names::TRIM_MATERIAL, names::TRIM_MATERIAL),
    ("trim_pattern", names::TRIM_PATTERN, names::TRIM_PATTERN),
    ("banner_pattern", names::BANNER_PATTERN, names::BANNER_PATTERN),
    ("painting", names::PAINTING, names::PAINTING),
];

pub(crate) fn run(dir: &Path) -> Vec<VerReport> {
    let bedrock = vdata::version_dirs(dir, "bedrock");
    let java = vdata::version_dirs(dir, "java");
    let Some(latest) = java.last() else { return Vec::new() };
    let mut jobs: Vec<(&VersionDir, &VersionDir)> = Vec::new();
    for b in &bedrock {
        let c = closest_java(b.version, &java);
        jobs.push((b, c));
        if c.name != latest.name {
            jobs.push((b, latest));
        }
    }
    par_map(&jobs, |(b, j)| {
        let mut checks = Vec::new();
        let mut ids = 0;
        for (name, bt, jt) in PAIRS {
            let from = IdTable::load(Edition::Bedrock, bt, b.version);
            let to = IdTable::load(Edition::Java, jt, j.version);
            let newest = IdTable::load(Edition::Java, jt, latest.version);
            let mut t = Tally::default();
            if from.is_empty() || to.is_empty() {
                checks.push((*name, t));
                continue;
            }
            ids += from.len();
            for (pivot, value) in from.iter() {
                if from.to_pivot(value) != Some(&**pivot) {
                    t.fail(pivot, || format!("{pivot}: {value} maps back to {:?}", from.to_pivot(value)));
                    continue;
                }
                if to.from_pivot(pivot).is_some() {
                    t.pass();
                } else if newest.from_pivot(pivot).is_some() {
                    t.allow(&format!("newer:{pivot}"));
                } else {
                    t.allow(&format!("gap:{pivot}"));
                }
            }
            // The Java table must read its own values back.
            for (pivot, value) in to.iter() {
                let _: &StateValue = value;
                if to.to_pivot(value).is_none() {
                    t.fail(pivot, || format!("Java {pivot}: {value} does not map to the pivot"));
                }
            }
            checks.push((*name, t));
        }
        VerReport { ids, area: "tables", edition: "bedrock>java", version: format!("{}->{}", b.name, j.name), checks }
    })
}
