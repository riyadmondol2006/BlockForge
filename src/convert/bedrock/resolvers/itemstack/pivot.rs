// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Conversions between the string based pivot of the mapping engine and the typed model.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::convert::mapping::{ItemKind, PivotBlock, PivotItem, StateMap, StateValue as MapValue};
use crate::convert::model::item::StewEffect;
use crate::convert::model::{
    BlockIdentifier, BlockType, EffectType, EntityType, ItemId, ItemProps, ItemType, PotionType, StateId, StateSet,
    VanillaEntityType,
};
use crate::convert::model::{ExplorerMap, HornInstrument, StateValue as ModelValue};

/// A pivot block as a model block (unknown types give `None`, unknown states are skipped).
pub(crate) fn block_from_pivot(block: &PivotBlock) -> Option<BlockIdentifier> {
    let ty = BlockType::from_name(&block.ty)?;
    Some(BlockIdentifier::with_states(ty, state_set(&block.states, "")))
}

/// The model states of pivot states; `prefix` is stripped from the keys (`S:` in item rows).
fn state_set(states: &BTreeMap<Arc<str>, Arc<str>>, prefix: &str) -> StateSet {
    let mut set = StateSet::new();
    for (key, value) in states {
        let Some(name) = key.strip_prefix(prefix) else { continue };
        let Some(id) = StateId::from_name(name) else { continue };
        let Some(ordinal) = id.value_ordinal(value) else { continue };
        set.set_raw(id, ordinal);
    }
    set
}

/// A pivot item as an item id plus the properties its mapping row fixes.
pub(crate) fn item_from_pivot(item: &PivotItem) -> Option<(ItemId, ItemProps)> {
    let id = match &item.kind {
        ItemKind::Block(name) => {
            let ty = BlockType::from_name(name)?;
            ItemId::Block(Arc::new(BlockIdentifier::with_states(ty, state_set(&item.props, "S:"))))
        }
        ItemKind::Item(name) => ItemId::Item(ItemType::from_name(name)?),
    };
    let mut props = ItemProps::default();
    for (key, value) in &item.props {
        let Some(name) = key.strip_prefix("P:") else { continue };
        match name {
            "potion" => props.potion = PotionType::from_name(value),
            "spawn_egg_mob" => props.spawn_egg_mob = VanillaEntityType::from_name(value).map(EntityType::Vanilla),
            "explorer_map" => props.explorer_map = ExplorerMap::from_name(value),
            "horn_instrument" => props.horn_instrument = HornInstrument::from_name(value),
            "ominous_bottle_amplifier" => {
                props.ominous_bottle_amplifier = value.strip_prefix("i:").and_then(|v| v.parse().ok());
            }
            "stew_effects" => props.stew_effects = parse_stew(value),
            _ => {}
        }
    }
    Some((id, props))
}

/// `?:ChunkerStewEffect{effects={BLINDNESS=160, POISON=240}}`.
fn parse_stew(text: &str) -> Option<StewEffect> {
    let inner = text.split_once("effects={")?.1.split_once('}')?.0;
    let mut effects = BTreeMap::new();
    for part in inner.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (name, duration) = part.split_once('=')?;
        effects.insert(EffectType::from_name(name)?, duration.trim().parse().ok()?);
    }
    Some(StewEffect { effects })
}

/// A model block as a pivot block.
pub(crate) fn block_to_pivot(block: &BlockIdentifier) -> Option<PivotBlock> {
    let ty = block.block_type()?;
    let mut states = BTreeMap::new();
    for (id, raw) in block.states().vanilla() {
        states.insert(Arc::from(id.name()), Arc::from(id.value_name(*raw)?));
    }
    Some(PivotBlock { ty: Arc::from(ty.name()), states })
}

/// The edition state value of a model state value.
pub(crate) fn map_value(value: &ModelValue) -> MapValue {
    match value {
        ModelValue::Bool(b) => MapValue::Bool(*b),
        ModelValue::Int(i) => MapValue::Int(*i),
        ModelValue::Str(s) => MapValue::Str(s.clone()),
    }
}

/// The model state value of an edition one.
pub(crate) fn model_value(value: &MapValue) -> ModelValue {
    match value {
        MapValue::Bool(b) => ModelValue::Bool(*b),
        MapValue::Int(i) => ModelValue::Int(*i),
        MapValue::Str(s) => ModelValue::Str(s.clone()),
    }
}

/// Edition states of a block palette compound (`states`): bytes become booleans, like `StateValue.fromNBT`.
pub(crate) fn states_of_compound(states: Option<&crate::convert::nbt::Compound>) -> StateMap {
    use crate::convert::nbt::Tag;
    let mut map = StateMap::new();
    if let Some(states) = states {
        for (key, tag) in states.iter() {
            let value = match tag {
                Tag::Byte(b) => MapValue::Bool(*b == 1),
                Tag::Int(i) => MapValue::Int(*i),
                Tag::String(s) => MapValue::Str(Arc::from(s.as_str())),
                _ => continue,
            };
            map.insert(Arc::from(key.as_str()), value);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stew_text() {
        let stew = parse_stew("?:ChunkerStewEffect{effects={BLINDNESS=160, POISON=240}}").unwrap();
        assert_eq!(stew.effects.get(&EffectType::BLINDNESS), Some(&160));
        assert_eq!(stew.effects.len(), 2);
    }
}
