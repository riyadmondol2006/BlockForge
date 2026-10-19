// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The simple id tables: biomes, entity types, effects, enchantments, potions, banner patterns, trims, horns, map
//! decorations, paintings, legacy numeric ids and map colours. Each is a bidirectional map between a Chunker constant
//! name (`PLAINS`, `ALLAY`) and an edition value (a namespaced name or a numeric id), gated by version.

use std::collections::HashMap;
use std::sync::Arc;

use super::{Edition, StateValue, Version, data};

/// Table names, as produced by `Dump.java`.
pub mod names {
    pub const BIOME_ID: &str = "biome_id";
    pub const BIOME_NAME: &str = "biome_name";
    pub const ENTITY_TYPE: &str = "entity_type";
    pub const ENTITY_ID: &str = "entity_id";
    pub const PAINTING: &str = "painting";
    pub const EFFECT: &str = "effect";
    pub const EFFECT_ID: &str = "effect_id";
    pub const ENCHANTMENT: &str = "enchantment";
    pub const ENCHANTMENT_ID: &str = "enchantment_id";
    pub const HORN: &str = "horn";
    pub const POTION: &str = "potion";
    pub const POTION_ID: &str = "potion_id";
    pub const MAP_DECORATION: &str = "map_decoration";
    pub const MAP_DECORATION_ID: &str = "map_decoration_id";
    pub const TRIM_PATTERN: &str = "trim_pattern";
    pub const TRIM_MATERIAL: &str = "trim_material";
    pub const BANNER_PATTERN: &str = "banner_pattern";
    pub const BANNER_SHORT: &str = "banner_short";
    pub const LEGACY_BLOCK_ID: &str = "legacy_block_id";
    pub const LEGACY_ITEM_ID: &str = "legacy_item_id";
    pub const LEGACY_ENTITY_TYPE: &str = "legacy_entity_type";
    pub const LEGACY_ENTITY_ID: &str = "legacy_entity_id";
    pub const LEGACY_PAINTING: &str = "legacy_painting";
    pub const LEGACY_POTION_ID: &str = "legacy_potion_id";
    pub const MAP_COLORS: &str = "map_colors";
}

/// One packed row of a simple table.
#[derive(Debug)]
pub(crate) struct IdRow {
    pub table: Arc<str>,
    pub kind: u8,
    pub since: Option<Version>,
    pub until: Option<Version>,
    pub a: Arc<str>,
    pub b: Arc<str>,
}

/// A bidirectional name table in force for one version.
#[derive(Debug, Default)]
pub struct IdTable {
    fwd: HashMap<Arc<str>, StateValue>,
    inv: HashMap<StateValue, Arc<str>>,
}

impl IdTable {
    /// The table `name` of `edition` as it is at `version` (empty if the table does not exist).
    pub fn load(edition: Edition, name: &str, version: Version) -> IdTable {
        let mut t = IdTable::default();
        for row in data::id_rows(edition) {
            if &*row.table != name || !data::in_force(row.since, row.until, version) {
                continue;
            }
            let Some(value) = StateValue::parse_typed(&row.b) else { continue };
            // Keys are constant names, or `s:`-typed plain strings in the legacy id tables.
            let key: Arc<str> = row.a.strip_prefix("s:").map_or_else(|| row.a.clone(), Arc::from);
            match row.kind {
                b'B' | b'F' => {
                    t.fwd.insert(key.clone(), value.clone());
                    if row.kind == b'B' {
                        t.inv.insert(value, key);
                    }
                }
                b'I' => {
                    t.inv.insert(value, key);
                }
                _ => {}
            }
        }
        t
    }

    pub fn len(&self) -> usize {
        self.fwd.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fwd.is_empty()
    }

    /// Edition value -> pivot constant name.
    pub fn to_pivot(&self, value: &StateValue) -> Option<&str> {
        self.inv.get(value).map(|s| &**s)
    }

    /// Edition string -> pivot constant name. Like Chunker's name resolvers, a missing namespace means `minecraft:`
    /// (only meaningful for tables of namespaced names, and tried as given first).
    pub fn to_pivot_name(&self, input: &str) -> Option<&str> {
        if let Some(hit) = self.to_pivot(&StateValue::str(input)) {
            return Some(hit);
        }
        if input.contains(':') {
            return None;
        }
        self.to_pivot(&StateValue::Str(format!("minecraft:{input}").into()))
    }

    /// Pivot constant name -> edition value.
    pub fn from_pivot(&self, name: &str) -> Option<&StateValue> {
        self.fwd.get(name)
    }

    /// Pivot constant name -> edition string.
    pub fn from_pivot_str(&self, name: &str) -> Option<&str> {
        match self.fwd.get(name)? {
            StateValue::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Pivot constant name -> edition numeric id.
    pub fn from_pivot_int(&self, name: &str) -> Option<i32> {
        match self.fwd.get(name)? {
            StateValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// All (pivot name, edition value) pairs, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&Arc<str>, &StateValue)> {
        self.fwd.iter()
    }
}

/// The map colour palette (RGBA per base colour id) at `version`.
pub fn map_colors(edition: Edition, version: Version) -> Vec<[u8; 4]> {
    let mut rows: Vec<(usize, [u8; 4])> = Vec::new();
    for row in data::id_rows(edition) {
        if &*row.table != names::MAP_COLORS || row.kind != b'C' || !data::in_force(row.since, row.until, version) {
            continue;
        }
        let idx = row.a.parse().unwrap_or(0);
        let mut c = [0u8; 4];
        for (i, p) in row.b.split(',').take(4).enumerate() {
            c[i] = p.parse().unwrap_or(0);
        }
        rows.push((idx, c));
    }
    rows.sort_by_key(|r| r.0);
    rows.into_iter().map(|r| r.1).collect()
}
