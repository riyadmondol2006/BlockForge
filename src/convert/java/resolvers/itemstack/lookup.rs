// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The identifier tables an item writer needs for one Java version, built once and shared.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use crate::convert::mapping::tables::names;
use crate::convert::mapping::{BlockResolver, Edition, IdTable, ItemResolver, Version};
use crate::convert::model::{BlockKind, BlockRef, EntityType, PotionType};
use crate::convert::model::{EffectType, EnchantmentType, ExplorerMap, HornInstrument, TrimMaterial, TrimPattern};

/// Everything resolved by name for one game version.
pub struct Lookups {
    pub version: Version,
    pub items: ItemResolver,
    blocks: OnceLock<BlockResolver>,
    enchantments: IdTable,
    potions: IdTable,
    effects: IdTable,
    trim_patterns: IdTable,
    trim_materials: IdTable,
    horns: IdTable,
    map_decorations: IdTable,
    map_decoration_ids: IdTable,
    entity_types: IdTable,
}

static CACHE: LazyLock<Mutex<HashMap<Version, Arc<Lookups>>>> = LazyLock::new(Mutex::default);

impl Lookups {
    /// The shared lookups for `version`.
    pub fn get(version: Version) -> Arc<Lookups> {
        if let Ok(cache) = CACHE.lock()
            && let Some(hit) = cache.get(&version)
        {
            return hit.clone();
        }
        let built = Arc::new(Lookups::build(version));
        if let Ok(mut cache) = CACHE.lock() {
            cache.insert(version, built.clone());
        }
        built
    }

    fn build(version: Version) -> Lookups {
        let table = |name: &str| IdTable::load(Edition::Java, name, version);
        Lookups {
            version,
            items: ItemResolver::new(Edition::Java, version),
            blocks: OnceLock::new(),
            enchantments: table(names::ENCHANTMENT),
            potions: table(names::POTION),
            effects: table(names::EFFECT),
            trim_patterns: table(names::TRIM_PATTERN),
            trim_materials: table(names::TRIM_MATERIAL),
            horns: table(names::HORN),
            map_decorations: table(names::MAP_DECORATION),
            map_decoration_ids: table(names::MAP_DECORATION_ID),
            entity_types: table(names::ENTITY_TYPE),
        }
    }

    /// The Java block table (built on first use, it is big).
    pub fn blocks(&self) -> &BlockResolver {
        self.blocks.get_or_init(|| BlockResolver::new(Edition::Java, self.version))
    }

    pub fn enchantment(&self, e: EnchantmentType) -> Option<&str> {
        self.enchantments.from_pivot_str(e.name())
    }

    /// The potion id; potions without a Java id become water (as `writePotionType` does).
    pub fn potion(&self, p: PotionType) -> &str {
        self.potions
            .from_pivot_str(p.name())
            .or_else(|| self.potions.from_pivot_str(PotionType::WATER.name()))
            .unwrap_or("minecraft:water")
    }

    /// The effect id; unknown effects become `minecraft:empty`.
    pub fn effect(&self, e: EffectType) -> &str {
        self.effects.from_pivot_str(e.name()).unwrap_or("minecraft:empty")
    }

    pub fn trim_pattern(&self, p: TrimPattern) -> Option<&str> {
        self.trim_patterns.from_pivot_str(p.name())
    }

    pub fn trim_material(&self, m: TrimMaterial) -> Option<&str> {
        self.trim_materials.from_pivot_str(m.name())
    }

    /// The instrument id; unknown ones become the ponder horn.
    pub fn horn(&self, h: HornInstrument) -> &str {
        self.horns
            .from_pivot_str(h.name())
            .or_else(|| self.horns.from_pivot_str(HornInstrument::PONDER_GOAT_HORN.name()))
            .unwrap_or("minecraft:ponder_goat_horn")
    }

    /// The map decoration type name (1.20.5+).
    pub fn map_decoration(&self, m: ExplorerMap) -> Option<&str> {
        self.map_decorations.from_pivot_str(m.name())
    }

    /// The numeric map decoration type (before 1.20.5).
    pub fn map_decoration_id(&self, m: ExplorerMap) -> Option<i32> {
        self.map_decoration_ids.from_pivot_int(m.name())
    }

    /// The entity id; custom entity types keep their identifier.
    pub fn entity_type(&self, t: &EntityType) -> Option<String> {
        match t {
            EntityType::Vanilla(v) => self.entity_types.from_pivot_str(v.name()).map(str::to_owned),
            EntityType::Custom(id) => Some(id.to_string()),
        }
    }

    /// The block id of a block item: the table id, a custom block's own identifier, or the preserved one.
    pub fn block_id(&self, block: &BlockRef) -> Option<String> {
        if let Some(p) = block.preserved()
            && p.from_reader
        {
            return Some(p.identifier.name.to_string());
        }
        match block.kind() {
            BlockKind::Custom(id) => Some(id.to_string()),
            BlockKind::Vanilla(ty) => {
                let mut pivot = crate::convert::mapping::PivotBlock::new(ty.name(), &[]);
                for (state, raw) in block.states().vanilla() {
                    if let Some(value) = state.value_name(*raw) {
                        pivot.states.insert(state.name().into(), value.into());
                    }
                }
                let blocks = self.blocks();
                if let Some((id, _)) = blocks.from_pivot(&pivot) {
                    return Some(id.to_string());
                }
                // States the table cannot write: try the plain type.
                pivot.states.clear();
                blocks.from_pivot(&pivot).map(|(id, _)| id.to_string())
            }
        }
    }
}
