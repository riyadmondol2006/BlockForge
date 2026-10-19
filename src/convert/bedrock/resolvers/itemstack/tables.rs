// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! The simple id tables of one Bedrock version (`BedrockEnchantmentIDResolver`, `BedrockPotionIDResolver`,
//! `BedrockEffectIDResolver`, the trim, banner pattern and entity type resolvers).

use std::sync::Arc;

use crate::convert::mapping::tables::names;
use crate::convert::mapping::{Edition, IdTable, StateValue, Version};
use crate::convert::model::{
    BannerPattern, EffectType, EnchantmentType, EntityType, PotionType, TrimMaterial, TrimPattern, VanillaEntityType,
};

/// Everything the item and block entity readers look up by id.
pub(crate) struct Tables {
    enchantments: IdTable,
    potions: IdTable,
    effects: IdTable,
    trim_patterns: IdTable,
    trim_materials: IdTable,
    banner_patterns: IdTable,
    entity_types: IdTable,
}

impl Tables {
    pub(crate) fn load(version: Version) -> Tables {
        let load = |name| IdTable::load(Edition::Bedrock, name, version);
        Tables {
            enchantments: load(names::ENCHANTMENT_ID),
            potions: load(names::POTION_ID),
            effects: load(names::EFFECT_ID),
            trim_patterns: load(names::TRIM_PATTERN),
            trim_materials: load(names::TRIM_MATERIAL),
            banner_patterns: load(names::BANNER_PATTERN),
            entity_types: load(names::ENTITY_TYPE),
        }
    }

    /// The enchantment with the legacy numeric id.
    pub(crate) fn enchantment(&self, id: i32) -> Option<EnchantmentType> {
        self.enchantments.to_pivot(&StateValue::Int(id)).and_then(EnchantmentType::from_name)
    }

    pub(crate) fn potion(&self, id: i32) -> Option<PotionType> {
        self.potions.to_pivot(&StateValue::Int(id)).and_then(PotionType::from_name)
    }

    pub(crate) fn effect(&self, id: i32) -> Option<EffectType> {
        self.effects.to_pivot(&StateValue::Int(id)).and_then(EffectType::from_name)
    }

    pub(crate) fn trim_pattern(&self, id: &str) -> Option<TrimPattern> {
        self.trim_patterns.to_pivot(&StateValue::str(id)).and_then(TrimPattern::from_name)
    }

    pub(crate) fn trim_material(&self, id: &str) -> Option<TrimMaterial> {
        self.trim_materials.to_pivot(&StateValue::str(id)).and_then(TrimMaterial::from_name)
    }

    /// The pattern with the short code (`bs`).
    pub(crate) fn banner_pattern(&self, short: &str) -> Option<BannerPattern> {
        self.banner_patterns.to_pivot(&StateValue::str(short)).and_then(BannerPattern::from_name)
    }

    /// A vanilla entity type from its identifier (`minecraft:` is assumed when there is no namespace).
    pub(crate) fn entity_type(&self, id: &str) -> Option<EntityType> {
        self.entity_types
            .to_pivot_name(id)
            .and_then(VanillaEntityType::from_name)
            .map(EntityType::Vanilla)
    }

    // The reverse lookups used when writing.

    pub(crate) fn enchantment_id(&self, e: EnchantmentType) -> Option<i32> {
        self.enchantments.from_pivot_int(e.name())
    }



    pub(crate) fn trim_pattern_id(&self, p: TrimPattern) -> Option<&str> {
        self.trim_patterns.from_pivot_str(p.name())
    }

    pub(crate) fn trim_material_id(&self, m: TrimMaterial) -> Option<&str> {
        self.trim_materials.from_pivot_str(m.name())
    }


    pub(crate) fn entity_type_id(&self, t: VanillaEntityType) -> Option<Arc<str>> {
        self.entity_types.from_pivot_str(t.name()).map(Arc::from)
    }
}
