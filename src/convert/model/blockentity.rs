// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Block entities (`intermediate.column.blockentity`): one struct for the position and common fields and one enum
//! with a variant per Chunker class, so a `match` replaces the class walk of the Java handler dispatch.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value as Json;

use crate::convert::nbt::Compound;

use super::color::Color;
use super::entity::EntityType;
use super::generated::{BannerPattern, DyeColor, EffectType, ItemType, PotionBottleType, PotionType};
use super::item::{ItemId, ItemStack};

named_enum! {
    /// The concrete block entity classes of Chunker (51 of them, `UnknownBlockEntity` included).
    enum BlockEntityKind: u8 {
        Banner, Barrel, Beacon, Bed, Beehive, Bell, BlastFurnace, BrewingStand, Brushable, CalibratedSculkSensor,
        Campfire, Cauldron, Chest, ChiseledBookshelf, CommandBlock, Comparator, Conduit, CopperGolemStatue, Crafter,
        CreakingHeart, DaylightDetector, DecoratedPot, Dispenser, Dropper, EnchantmentTable, EndGateway, EndPortal,
        EnderChest, Furnace, HangingSign, Hopper, Jigsaw, Jukebox, Lectern, PotentSulfur, SculkCatalyst, SculkSensor,
        SculkShrieker, Shelf, ShulkerBox, Sign, Skull, Smoker, Spawner, Structure, Test, TestInstance, TrappedChest,
        TrialSpawner, Vault, Unknown
    }
}

impl BlockEntityKind {
    /// The concrete superclass in Chunker's hierarchy (`Dropper` extends `Dispenser`, ...), if there is one. Handler
    /// dispatch walks this chain.
    pub const fn parent(self) -> Option<BlockEntityKind> {
        use BlockEntityKind::*;
        match self {
            Dropper => Some(Dispenser),
            TrappedChest => Some(Chest),
            BlastFurnace | Smoker => Some(Furnace),
            HangingSign => Some(Sign),
            EndGateway => Some(EndPortal),
            CalibratedSculkSensor => Some(SculkSensor),
            _ => None,
        }
    }

    /// Whether `self` is `other` or inherits from it.
    pub fn is_a(self, other: BlockEntityKind) -> bool {
        let mut kind = Some(self);
        while let Some(k) = kind {
            if k == other {
                return true;
            }
            kind = k.parent();
        }
        false
    }

    /// Whether the class extends `ContainerBlockEntity` (has an item map).
    pub const fn is_container(self) -> bool {
        use BlockEntityKind::*;
        matches!(
            self,
            BlastFurnace
                | BrewingStand
                | EnderChest
                | Furnace
                | Hopper
                | Smoker
                | Barrel
                | Chest
                | Crafter
                | Dispenser
                | Dropper
                | ShulkerBox
                | TrappedChest
        )
    }

    /// Whether the class extends `RandomizableContainerBlockEntity` (has a loot table too).
    pub const fn is_randomizable(self) -> bool {
        use BlockEntityKind::*;
        matches!(self, Barrel | Chest | Crafter | Dispenser | Dropper | ShulkerBox | TrappedChest)
    }
}

/// Item slots of a container: slot to stack, sparse and ordered by the signed slot (`ContainerBlockEntity`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Container {
    pub items: BTreeMap<i8, ItemStack>,
}

/// A container with a loot table (`RandomizableContainerBlockEntity`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RandomContainer {
    pub container: Container,
    pub loot_table: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Banner {
    pub patterns: Vec<(DyeColor, BannerPattern)>,
    /// Java only uses the base for shields.
    pub base: Option<DyeColor>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Beacon {
    pub primary_effect: EffectType,
    pub secondary_effect: EffectType,
}

impl Default for Beacon {
    fn default() -> Self {
        Beacon { primary_effect: EffectType::EMPTY, secondary_effect: EffectType::EMPTY }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Brushable {
    pub brush_direction: i8,
    pub brush_count: i32,
    pub item: Option<ItemStack>,
    pub loot_table: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Campfire {
    pub items: [Option<ItemStack>; 4],
    pub cooking_times: [i32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cauldron {
    pub potion_type: PotionType,
    pub potion_bottle_type: PotionBottleType,
    pub custom_color: Option<Color>,
}

impl Default for Cauldron {
    fn default() -> Self {
        Cauldron {
            potion_type: PotionType::EMPTY,
            potion_bottle_type: PotionBottleType::UNKNOWN,
            custom_color: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CommandBlock {
    pub command: Option<String>,
    pub auto: bool,
    pub track_output: bool,
}

/// Default of [`BrewingStand::fuel_total`].
pub const DEFAULT_FUEL_TOTAL: i16 = 20;

#[derive(Clone, Debug, PartialEq)]
pub struct BrewingStand {
    pub container: Container,
    pub brew_time: i16,
    pub fuel: i16,
    pub fuel_total: i16,
}

impl Default for BrewingStand {
    fn default() -> Self {
        BrewingStand { container: Container::default(), brew_time: 0, fuel: 0, fuel_total: DEFAULT_FUEL_TOTAL }
    }
}

/// `FurnaceBlockEntity` (also `BlastFurnace` and `Smoker`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Furnace {
    pub container: Container,
    pub burn_time: i16,
    pub cook_time: i16,
    pub cook_time_total: i16,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Crafter {
    pub container: RandomContainer,
    pub disabled_slots: i16,
    pub crafting_ticks_remaining: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecoratedPot {
    pub back: ItemId,
    pub left: ItemId,
    pub right: ItemId,
    pub front: ItemId,
    pub item: Option<ItemStack>,
}

impl Default for DecoratedPot {
    fn default() -> Self {
        let brick = || ItemId::Item(ItemType::BRICK);
        DecoratedPot { back: brick(), left: brick(), right: brick(), front: brick(), item: None }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct EndGateway {
    pub age: i64,
    pub exit_x: i32,
    pub exit_y: i32,
    pub exit_z: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Joint {
    #[default]
    Rollable,
    Aligned,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Jigsaw {
    pub name: String,
    pub target: String,
    pub pool: String,
    pub final_state: String,
    pub joint: Joint,
    pub selection_priority: i32,
    pub placement_priority: i32,
}

impl Default for Jigsaw {
    fn default() -> Self {
        Jigsaw {
            name: "minecraft:empty".into(),
            target: "minecraft:empty".into(),
            pool: "minecraft:empty".into(),
            final_state: "minecraft:air".into(),
            joint: Joint::Rollable,
            selection_priority: 0,
            placement_priority: 0,
        }
    }
}

/// One side of a sign (`SignBlockEntity.SignFace`).
#[derive(Clone, Debug, PartialEq)]
pub struct SignFace {
    /// Text components, one per line.
    pub lines: Vec<Json>,
    pub lit: bool,
    /// ARGB, opaque black by default.
    pub color: i32,
}

impl Default for SignFace {
    fn default() -> Self {
        SignFace { lines: Vec::new(), lit: false, color: 0xff00_0000_u32 as i32 }
    }
}

/// `SignBlockEntity` (also used for hanging signs).
#[derive(Clone, Debug, PartialEq)]
pub struct Sign {
    pub front: SignFace,
    pub back: SignFace,
    pub waxed: bool,
    pub allow_op_features: bool,
}

impl Default for Sign {
    fn default() -> Self {
        Sign { front: SignFace::default(), back: SignFace::default(), waxed: false, allow_op_features: true }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Skull {
    pub owner_id: Option<String>,
    pub owner_name: Option<String>,
    pub texture: Option<String>,
    pub texture_signature: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Spawner {
    pub delay: i16,
    pub spawn_range: i16,
    pub max_spawn_delay: i16,
    pub min_spawn_delay: i16,
    pub spawn_count: i16,
    pub required_player_range: i16,
    pub max_nearby_entities: i16,
    pub entity_type: Option<EntityType>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum StructureMirror {
    #[default]
    None,
    LeftRight,
    FrontBack,
    /// Bedrock only.
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum StructureRotation {
    #[default]
    None,
    Clockwise90,
    Clockwise180,
    CounterClockwise90,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Structure {
    pub name: String,
    pub author: String,
    pub metadata: String,
    pub pos_x: i32,
    pub pos_y: i32,
    pub pos_z: i32,
    pub size_x: i32,
    pub size_y: i32,
    pub size_z: i32,
    pub rotation: StructureRotation,
    pub mirror: StructureMirror,
    pub mode: super::generated::StructureBlockMode,
    pub ignore_entities: bool,
    pub include_players: bool,
    pub remove_blocks: bool,
    pub strict: bool,
    pub powered: bool,
    pub show_air: bool,
    pub show_bounding_box: bool,
    pub integrity: f32,
    pub seed: i64,
    pub last_touched_player_id: i64,
    pub animation_mode: i8,
    pub animation_seconds: f32,
    pub redstone_save_mode: i32,
}

impl Default for Structure {
    fn default() -> Self {
        Structure {
            name: String::new(),
            author: String::new(),
            metadata: String::new(),
            pos_x: 0,
            pos_y: 0,
            pos_z: 0,
            size_x: 0,
            size_y: 0,
            size_z: 0,
            rotation: StructureRotation::None,
            mirror: StructureMirror::None,
            mode: super::generated::StructureBlockMode::DATA,
            ignore_entities: false,
            include_players: false,
            remove_blocks: false,
            strict: false,
            powered: false,
            show_air: false,
            show_bounding_box: true,
            integrity: 1.0,
            seed: 0,
            last_touched_player_id: 0,
            animation_mode: 0,
            animation_seconds: 0.0,
            redstone_save_mode: 0,
        }
    }
}

/// The class specific part of a block entity: one variant per [`BlockEntityKind`].
#[derive(Clone, Debug, PartialEq)]
pub enum BlockEntityData {
    Banner(Banner),
    Barrel(RandomContainer),
    Beacon(Beacon),
    Bed,
    Beehive,
    Bell,
    BlastFurnace(Furnace),
    BrewingStand(BrewingStand),
    Brushable(Box<Brushable>),
    CalibratedSculkSensor,
    Campfire(Box<Campfire>),
    Cauldron(Cauldron),
    Chest(RandomContainer),
    ChiseledBookshelf(Box<[Option<ItemStack>; 6]>),
    CommandBlock(CommandBlock),
    Comparator { output_signal: i32 },
    Conduit,
    CopperGolemStatue,
    Crafter(Box<Crafter>),
    CreakingHeart,
    DaylightDetector,
    DecoratedPot(Box<DecoratedPot>),
    Dispenser(RandomContainer),
    Dropper(RandomContainer),
    EnchantmentTable,
    EndGateway(EndGateway),
    EndPortal,
    EnderChest(Container),
    Furnace(Furnace),
    HangingSign(Box<Sign>),
    Hopper(Container),
    Jigsaw(Box<Jigsaw>),
    Jukebox { record: Option<Box<ItemStack>> },
    Lectern { book: Option<Box<ItemStack>>, page: i32 },
    PotentSulfur,
    SculkCatalyst,
    SculkSensor,
    SculkShrieker,
    Shelf(Box<[Option<ItemStack>; 3]>),
    ShulkerBox(RandomContainer),
    Sign(Box<Sign>),
    Skull(Skull),
    Smoker(Furnace),
    Spawner(Box<Spawner>),
    Structure(Box<Structure>),
    Test,
    TestInstance,
    TrappedChest(RandomContainer),
    TrialSpawner,
    Vault,
    /// `UnknownBlockEntity`: only the raw id is known; the rest survives in `original_nbt`.
    Unknown(Arc<str>),
}

impl BlockEntityData {
    /// The kind (Chunker class) of the variant.
    pub fn kind(&self) -> BlockEntityKind {
        use BlockEntityKind as K;
        match self {
            BlockEntityData::Banner(_) => K::Banner,
            BlockEntityData::Barrel(_) => K::Barrel,
            BlockEntityData::Beacon(_) => K::Beacon,
            BlockEntityData::Bed => K::Bed,
            BlockEntityData::Beehive => K::Beehive,
            BlockEntityData::Bell => K::Bell,
            BlockEntityData::BlastFurnace(_) => K::BlastFurnace,
            BlockEntityData::BrewingStand(_) => K::BrewingStand,
            BlockEntityData::Brushable(_) => K::Brushable,
            BlockEntityData::CalibratedSculkSensor => K::CalibratedSculkSensor,
            BlockEntityData::Campfire(_) => K::Campfire,
            BlockEntityData::Cauldron(_) => K::Cauldron,
            BlockEntityData::Chest(_) => K::Chest,
            BlockEntityData::ChiseledBookshelf(_) => K::ChiseledBookshelf,
            BlockEntityData::CommandBlock(_) => K::CommandBlock,
            BlockEntityData::Comparator { .. } => K::Comparator,
            BlockEntityData::Conduit => K::Conduit,
            BlockEntityData::CopperGolemStatue => K::CopperGolemStatue,
            BlockEntityData::Crafter(_) => K::Crafter,
            BlockEntityData::CreakingHeart => K::CreakingHeart,
            BlockEntityData::DaylightDetector => K::DaylightDetector,
            BlockEntityData::DecoratedPot(_) => K::DecoratedPot,
            BlockEntityData::Dispenser(_) => K::Dispenser,
            BlockEntityData::Dropper(_) => K::Dropper,
            BlockEntityData::EnchantmentTable => K::EnchantmentTable,
            BlockEntityData::EndGateway(_) => K::EndGateway,
            BlockEntityData::EndPortal => K::EndPortal,
            BlockEntityData::EnderChest(_) => K::EnderChest,
            BlockEntityData::Furnace(_) => K::Furnace,
            BlockEntityData::HangingSign(_) => K::HangingSign,
            BlockEntityData::Hopper(_) => K::Hopper,
            BlockEntityData::Jigsaw(_) => K::Jigsaw,
            BlockEntityData::Jukebox { .. } => K::Jukebox,
            BlockEntityData::Lectern { .. } => K::Lectern,
            BlockEntityData::PotentSulfur => K::PotentSulfur,
            BlockEntityData::SculkCatalyst => K::SculkCatalyst,
            BlockEntityData::SculkSensor => K::SculkSensor,
            BlockEntityData::SculkShrieker => K::SculkShrieker,
            BlockEntityData::Shelf(_) => K::Shelf,
            BlockEntityData::ShulkerBox(_) => K::ShulkerBox,
            BlockEntityData::Sign(_) => K::Sign,
            BlockEntityData::Skull(_) => K::Skull,
            BlockEntityData::Smoker(_) => K::Smoker,
            BlockEntityData::Spawner(_) => K::Spawner,
            BlockEntityData::Structure(_) => K::Structure,
            BlockEntityData::Test => K::Test,
            BlockEntityData::TestInstance => K::TestInstance,
            BlockEntityData::TrappedChest(_) => K::TrappedChest,
            BlockEntityData::TrialSpawner => K::TrialSpawner,
            BlockEntityData::Vault => K::Vault,
            BlockEntityData::Unknown(_) => K::Unknown,
        }
    }

    /// A default constructed value of the kind, like `new XBlockEntity()` (`None` for `Unknown`, which needs an id).
    pub fn default_for(kind: BlockEntityKind) -> Option<BlockEntityData> {
        use BlockEntityKind as K;
        Some(match kind {
            K::Banner => BlockEntityData::Banner(Banner::default()),
            K::Barrel => BlockEntityData::Barrel(RandomContainer::default()),
            K::Beacon => BlockEntityData::Beacon(Beacon::default()),
            K::Bed => BlockEntityData::Bed,
            K::Beehive => BlockEntityData::Beehive,
            K::Bell => BlockEntityData::Bell,
            K::BlastFurnace => BlockEntityData::BlastFurnace(Furnace::default()),
            K::BrewingStand => BlockEntityData::BrewingStand(BrewingStand::default()),
            K::Brushable => BlockEntityData::Brushable(Box::default()),
            K::CalibratedSculkSensor => BlockEntityData::CalibratedSculkSensor,
            K::Campfire => BlockEntityData::Campfire(Box::default()),
            K::Cauldron => BlockEntityData::Cauldron(Cauldron::default()),
            K::Chest => BlockEntityData::Chest(RandomContainer::default()),
            K::ChiseledBookshelf => BlockEntityData::ChiseledBookshelf(Box::default()),
            K::CommandBlock => BlockEntityData::CommandBlock(CommandBlock::default()),
            K::Comparator => BlockEntityData::Comparator { output_signal: 0 },
            K::Conduit => BlockEntityData::Conduit,
            K::CopperGolemStatue => BlockEntityData::CopperGolemStatue,
            K::Crafter => BlockEntityData::Crafter(Box::default()),
            K::CreakingHeart => BlockEntityData::CreakingHeart,
            K::DaylightDetector => BlockEntityData::DaylightDetector,
            K::DecoratedPot => BlockEntityData::DecoratedPot(Box::default()),
            K::Dispenser => BlockEntityData::Dispenser(RandomContainer::default()),
            K::Dropper => BlockEntityData::Dropper(RandomContainer::default()),
            K::EnchantmentTable => BlockEntityData::EnchantmentTable,
            K::EndGateway => BlockEntityData::EndGateway(EndGateway::default()),
            K::EndPortal => BlockEntityData::EndPortal,
            K::EnderChest => BlockEntityData::EnderChest(Container::default()),
            K::Furnace => BlockEntityData::Furnace(Furnace::default()),
            K::HangingSign => BlockEntityData::HangingSign(Box::default()),
            K::Hopper => BlockEntityData::Hopper(Container::default()),
            K::Jigsaw => BlockEntityData::Jigsaw(Box::default()),
            K::Jukebox => BlockEntityData::Jukebox { record: None },
            K::Lectern => BlockEntityData::Lectern { book: None, page: 0 },
            K::PotentSulfur => BlockEntityData::PotentSulfur,
            K::SculkCatalyst => BlockEntityData::SculkCatalyst,
            K::SculkSensor => BlockEntityData::SculkSensor,
            K::SculkShrieker => BlockEntityData::SculkShrieker,
            K::Shelf => BlockEntityData::Shelf(Box::default()),
            K::ShulkerBox => BlockEntityData::ShulkerBox(RandomContainer::default()),
            K::Sign => BlockEntityData::Sign(Box::default()),
            K::Skull => BlockEntityData::Skull(Skull::default()),
            K::Smoker => BlockEntityData::Smoker(Furnace::default()),
            K::Spawner => BlockEntityData::Spawner(Box::default()),
            K::Structure => BlockEntityData::Structure(Box::default()),
            K::Test => BlockEntityData::Test,
            K::TestInstance => BlockEntityData::TestInstance,
            K::TrappedChest => BlockEntityData::TrappedChest(RandomContainer::default()),
            K::TrialSpawner => BlockEntityData::TrialSpawner,
            K::Vault => BlockEntityData::Vault,
            K::Unknown => return None,
        })
    }

    /// The item container of the variant, for every class that extends `ContainerBlockEntity`.
    pub fn container(&self) -> Option<&Container> {
        use BlockEntityData::*;
        match self {
            BlastFurnace(f) | Furnace(f) | Smoker(f) => Some(&f.container),
            BrewingStand(b) => Some(&b.container),
            EnderChest(c) | Hopper(c) => Some(c),
            Barrel(r) | Chest(r) | Dispenser(r) | Dropper(r) | ShulkerBox(r) | TrappedChest(r) => Some(&r.container),
            Crafter(c) => Some(&c.container.container),
            _ => None,
        }
    }

    /// Mutable form of [`BlockEntityData::container`].
    pub fn container_mut(&mut self) -> Option<&mut Container> {
        use BlockEntityData::*;
        match self {
            BlastFurnace(f) | Furnace(f) | Smoker(f) => Some(&mut f.container),
            BrewingStand(b) => Some(&mut b.container),
            EnderChest(c) | Hopper(c) => Some(c),
            Barrel(r) | Chest(r) | Dispenser(r) | Dropper(r) | ShulkerBox(r) | TrappedChest(r) => {
                Some(&mut r.container)
            }
            Crafter(c) => Some(&mut c.container.container),
            _ => None,
        }
    }

    /// The loot table, for classes that extend `RandomizableContainerBlockEntity`.
    pub fn loot_table(&self) -> Option<&Option<String>> {
        use BlockEntityData::*;
        match self {
            Barrel(r) | Chest(r) | Dispenser(r) | Dropper(r) | ShulkerBox(r) | TrappedChest(r) => Some(&r.loot_table),
            Crafter(c) => Some(&c.container.loot_table),
            _ => None,
        }
    }

    /// The sign data of `Sign` and `HangingSign`.
    pub fn sign(&self) -> Option<&Sign> {
        match self {
            BlockEntityData::Sign(s) | BlockEntityData::HangingSign(s) => Some(s),
            _ => None,
        }
    }
}

/// A block entity (`BlockEntity`): absolute position, common fields and the class specific data.
///
/// `PartialEq` ignores `original_nbt`, like Chunker's `equals`.
#[derive(Clone, Debug)]
pub struct BlockEntity {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// Bedrock `isMovable`.
    pub movable: bool,
    /// Text component of the custom name.
    pub custom_name: Option<Json>,
    /// The tag the entity was read from; only kept when NBT copying is on.
    pub original_nbt: Option<Compound>,
    pub data: BlockEntityData,
}

impl BlockEntity {
    pub fn new(x: i32, y: i32, z: i32, data: BlockEntityData) -> BlockEntity {
        BlockEntity { x, y, z, movable: false, custom_name: None, original_nbt: None, data }
    }

    pub fn kind(&self) -> BlockEntityKind {
        self.data.kind()
    }
}

impl PartialEq for BlockEntity {
    fn eq(&self, other: &Self) -> bool {
        self.x == other.x
            && self.y == other.y
            && self.z == other.z
            && self.movable == other.movable
            && self.custom_name == other.custom_name
            && self.data == other.data
    }
}
