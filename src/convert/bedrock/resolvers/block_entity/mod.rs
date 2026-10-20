// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Block entity handlers: a Bedrock block entity compound (`id`, `x`, `y`, `z`, ...) to the model
//! [`BlockEntity`] (`BedrockBlockEntityResolver` and its handlers).
//!
//! Chunker reads Bedrock specific subclasses and later fixes them against the column (`updateBeforeProcess`,
//! `shouldRemoveBeforeProcess`): chests learn their `ChestType` from the pair coordinates, beds, banners and skulls
//! move their colour and type into the block, piston arms, note blocks, flower pots and item frames are dropped after
//! updating a block or becoming an entity. [`read_block_entity`] gives the model entity alone;
//! [`read_into_column`] does the whole read and column step.

mod column;
mod font;
mod font_data;
mod read;

use crate::convert::bedrock::data_version::BedrockVersion;
use crate::convert::model::{
    BlockEntity, BlockEntityData, BlockEntityKind, BlockIdentifier, CopperGolemPose, DyeColor, FacingDirection,
    ItemStack,
};
use crate::convert::nbt::Compound;

pub use column::read_into_column;
pub use font::{char_width, split as split_sign_text, text_width};

use super::itemstack::BedrockItems;

/// A block entity that exists only on Bedrock; the column step turns it into block states or an entity.
#[derive(Clone, Debug, PartialEq)]
pub struct BedrockOnly {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub kind: BedrockOnlyKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BedrockOnlyKind {
    /// `PistonArm`: sticky, and extended when `State` is 2.
    PistonArm { sticky: bool, extended: bool },
    /// `Music` (note block): the note, when it is a valid one.
    NoteBlock { note: Option<u8> },
    /// `ItemFrame` / `GlowItemFrame`.
    ItemFrame { glow: bool, item: Option<ItemStack>, rotation: i8 },
    /// `FlowerPot` with the `PlantBlock`.
    FlowerPot { plant: Option<BlockIdentifier> },
    /// `Lodestone`, `SporeBlossom` and `MovingBlock`: nothing to keep.
    Marker(&'static str),
}

/// Bedrock fields of a block entity the model keeps in the block instead.
#[derive(Clone, Debug, PartialEq)]
pub enum Extra {
    None,
    /// `pairx`/`pairz` of a chest (only when both are present) and `pairlead`.
    Chest {
        pair: Option<(i32, i32)>,
        lead: bool,
    },
    Bed {
        color: DyeColor,
    },
    Skull {
        skull_type: i8,
        rotation: i8,
        moving_mouth: bool,
    },
    Shulker {
        facing: FacingDirection,
    },
    CopperGolem {
        pose: CopperGolemPose,
    },
}

/// The result of reading one compound.
#[derive(Clone, Debug, PartialEq)]
pub enum Read {
    /// A model block entity with the Bedrock fields the column step needs.
    Entity { entity: BlockEntity, extra: Extra },
    /// A Bedrock only block entity.
    BedrockOnly(BedrockOnly),
}

impl Read {
    /// The model entity, without any column step (`None` for Bedrock only block entities).
    pub fn into_entity(self) -> Option<BlockEntity> {
        match self {
            Read::Entity { entity, .. } => Some(entity),
            Read::BedrockOnly(_) => None,
        }
    }
}

/// What a Bedrock id stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Model(BlockEntityKind),
    PistonArm,
    NoteBlock,
    ItemFrame { glow: bool },
    FlowerPot,
    Marker(&'static str),
}

/// The handler registered for `id` at `version` (`BedrockBlockEntityResolver.registerTypeHandlers`).
fn target_of(version: &BedrockVersion, id: &str) -> Option<Target> {
    use BlockEntityKind as K;
    let model = |k| Some(Target::Model(k));
    let at = |game| version.at_least(game);
    match id {
        "Chest" => model(K::Chest),
        "Furnace" => model(K::Furnace),
        "Sign" => model(K::Sign),
        "MobSpawner" => model(K::Spawner),
        "EnchantTable" => model(K::EnchantmentTable),
        "Skull" => model(K::Skull),
        "FlowerPot" => Some(Target::FlowerPot),
        "BrewingStand" => model(K::BrewingStand),
        "DaylightDetector" => model(K::DaylightDetector),
        "Music" => Some(Target::NoteBlock),
        "Jukebox" => model(K::Jukebox),
        "Comparator" => model(K::Comparator),
        "Dispenser" => model(K::Dispenser),
        "Dropper" => model(K::Dropper),
        "Hopper" => model(K::Hopper),
        "Cauldron" => model(K::Cauldron),
        "ItemFrame" => Some(Target::ItemFrame { glow: false }),
        "PistonArm" => Some(Target::PistonArm),
        "Beacon" => model(K::Beacon),
        "MovingBlock" => Some(Target::Marker("MovingBlock")),
        "EndPortal" => model(K::EndPortal),
        "EnderChest" => model(K::EnderChest),
        "EndGateway" => model(K::EndGateway),
        "Banner" => model(K::Banner),
        "StructureBlock" => model(K::Structure),
        "JigsawBlock" => model(K::Jigsaw),
        "ShulkerBox" => model(K::ShulkerBox),
        "CommandBlock" => model(K::CommandBlock),
        "Bed" => model(K::Bed),
        "Conduit" => model(K::Conduit),
        "Lectern" => model(K::Lectern),
        "BlastFurnace" => model(K::BlastFurnace),
        "Bell" => model(K::Bell),
        "Smoker" => model(K::Smoker),
        "Campfire" => model(K::Campfire),
        "Barrel" => model(K::Barrel),
        "Lodestone" if at([1, 16, 0]) => Some(Target::Marker("Lodestone")),
        "Beehive" if at([1, 16, 0]) => model(K::Beehive),
        "GlowItemFrame" if at([1, 17, 0]) => Some(Target::ItemFrame { glow: true }),
        "SculkSensor" if at([1, 17, 0]) => model(K::SculkSensor),
        "SporeBlossom" if at([1, 17, 0]) => Some(Target::Marker("SporeBlossom")),
        "SculkCatalyst" if at([1, 17, 30]) => model(K::SculkCatalyst),
        "SculkShrieker" if at([1, 17, 30]) => model(K::SculkShrieker),
        "ChiseledBookshelf" if at([1, 19, 50]) => model(K::ChiseledBookshelf),
        "HangingSign" if at([1, 19, 50]) => model(K::HangingSign),
        "DecoratedPot" if at([1, 19, 70]) => model(K::DecoratedPot),
        "BrushableBlock" if at([1, 19, 70]) => model(K::Brushable),
        "CalibratedSculkSensor" if at([1, 19, 80]) => model(K::CalibratedSculkSensor),
        "Crafter" if at([1, 20, 50]) => model(K::Crafter),
        "TrialSpawner" if at([1, 20, 60]) => model(K::TrialSpawner),
        "Vault" if at([1, 20, 70]) => model(K::Vault),
        "CreakingHeart" if at([1, 21, 50]) => model(K::CreakingHeart),
        "CopperGolemStatue" if at([1, 21, 110]) => model(K::CopperGolemStatue),
        "Shelf" if at([1, 21, 110]) => model(K::Shelf),
        "PotentSulfurBlock" if at([1, 26, 20]) => model(K::PotentSulfur),
        _ => None,
    }
}

/// Whether Bedrock has a handler for the model kind at `version` (the kinds an item can carry).
fn kind_registered(version: &BedrockVersion, kind: BlockEntityKind) -> bool {
    use BlockEntityKind as K;
    let id = match kind {
        K::Chest | K::TrappedChest => "Chest",
        K::Furnace => "Furnace",
        K::Sign => "Sign",
        K::Spawner => "MobSpawner",
        K::EnchantmentTable => "EnchantTable",
        K::Skull => "Skull",
        K::BrewingStand => "BrewingStand",
        K::DaylightDetector => "DaylightDetector",
        K::Jukebox => "Jukebox",
        K::Comparator => "Comparator",
        K::Dispenser => "Dispenser",
        K::Dropper => "Dropper",
        K::Hopper => "Hopper",
        K::Cauldron => "Cauldron",
        K::Beacon => "Beacon",
        K::EndPortal => "EndPortal",
        K::EnderChest => "EnderChest",
        K::EndGateway => "EndGateway",
        K::Banner => "Banner",
        K::Structure => "StructureBlock",
        K::Jigsaw => "JigsawBlock",
        K::ShulkerBox => "ShulkerBox",
        K::CommandBlock => "CommandBlock",
        K::Bed => "Bed",
        K::Conduit => "Conduit",
        K::Lectern => "Lectern",
        K::BlastFurnace => "BlastFurnace",
        K::Bell => "Bell",
        K::Smoker => "Smoker",
        K::Campfire => "Campfire",
        K::Barrel => "Barrel",
        K::Beehive => "Beehive",
        K::SculkSensor => "SculkSensor",
        K::SculkCatalyst => "SculkCatalyst",
        K::SculkShrieker => "SculkShrieker",
        K::ChiseledBookshelf => "ChiseledBookshelf",
        K::HangingSign => "HangingSign",
        K::DecoratedPot => "DecoratedPot",
        K::Brushable => "BrushableBlock",
        K::CalibratedSculkSensor => "CalibratedSculkSensor",
        K::Crafter => "Crafter",
        K::TrialSpawner => "TrialSpawner",
        K::Vault => "Vault",
        K::CreakingHeart => "CreakingHeart",
        K::CopperGolemStatue => "CopperGolemStatue",
        K::Shelf => "Shelf",
        K::PotentSulfur => "PotentSulfurBlock",
        _ => return false,
    };
    target_of(version, id).is_some()
}

/// The block entity id of a compound (`id`).
pub fn block_entity_id(tag: &Compound) -> Option<&str> {
    tag.get_str("id")
}

/// Read one block entity compound with the Bedrock fields kept (see [`read_into_column`]).
///
/// An id without a handler gives `BlockEntityData::Unknown` when `keep_unknown` is set (the original tag is kept in
/// every entity then), otherwise `None`.
pub fn read_block_entity_full(items: &BedrockItems, tag: &Compound, keep_unknown: bool) -> Option<Read> {
    let id = block_entity_id(tag)?;
    match target_of(items.version(), id) {
        Some(target) => read::read_target(items, target, tag, keep_unknown),
        None if keep_unknown => {
            let data = BlockEntityData::Unknown(id.into());
            let mut entity = BlockEntity::new(0, 0, 0, data);
            read::read_common(items, tag, &mut entity, true);
            Some(Read::Entity { entity, extra: Extra::None })
        }
        None => None,
    }
}

/// A Bedrock block entity compound (`id`, `x`, `y`, `z`) as a model block entity.
///
/// Bedrock only block entities (piston arms, note blocks, item frames, flower pots, lodestones, spore blossoms and
/// moving blocks) are not part of the model and give `None`; so does an unknown id unless `keep_unknown` is set, in
/// which case an `Unknown` entity keeps the original NBT. Trapped chests are plain chests here, and the block state
/// changes Chunker derives from some entities are left to [`read_into_column`].
pub fn read_block_entity(items: &BedrockItems, tag: &Compound, keep_unknown: bool) -> Option<BlockEntity> {
    read_block_entity_full(items, tag, keep_unknown)?.into_entity()
}

/// The block entity carried in the `tag` of an item of a block (`BLOCK_ENTITY` item property): read as `kind` (the
/// id and position are not needed), with the item flavour of the column step (trapped chests, Bedrock types dropped).
pub(crate) fn read_item_block_entity(
    items: &BedrockItems,
    kind: BlockEntityKind,
    trapped: bool,
    tag: &Compound,
) -> Option<BlockEntity> {
    if !kind_registered(items.version(), kind) {
        return None;
    }
    let kind = if kind == BlockEntityKind::TrappedChest { BlockEntityKind::Chest } else { kind };
    let Some(Read::Entity { mut entity, .. }) =
        read::read_target(items, Target::Model(kind), tag, items.keep_original_nbt())
    else {
        return None;
    };
    if trapped && let BlockEntityData::Chest(chest) = entity.data {
        entity.data = BlockEntityData::TrappedChest(chest);
    }
    Some(entity)
}

#[cfg(test)]
mod tests;
