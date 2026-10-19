// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Entities (`intermediate.column.entity`). Chunker models five kinds; every other entity is an `Unknown` one that
//! keeps its position, motion, rotation and (when copying is on) the original NBT.

use std::sync::Arc;

use crate::convert::nbt::Compound;

use super::generated::{DyeColor, FacingDirection, FacingDirectionHorizontal, Motive, VanillaEntityType};
use super::item::ItemStack;

impl VanillaEntityType {
    /// Whether the type has a vanilla spawn egg.
    pub fn has_spawn_egg(self) -> bool {
        super::generated::ENTITY_SPAWN_EGG[self as usize]
    }
}

/// An entity type: a vanilla one or a custom one known by its identifier (`ChunkerCustomEntityType`; equality is by
/// identifier, which the Java class forgot to implement).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum EntityType {
    Vanilla(VanillaEntityType),
    Custom(Arc<str>),
}

impl From<VanillaEntityType> for EntityType {
    fn from(value: VanillaEntityType) -> Self {
        EntityType::Vanilla(value)
    }
}

/// Tile position of a hanging entity (`HangingEntity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Tile {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// `ItemFrameEntity` (and `GlowItemFrameEntity`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ItemFrame {
    pub tile: Tile,
    /// A glow item frame (`GlowItemFrameEntity`).
    pub glow: bool,
    pub item: Option<Box<ItemStack>>,
    pub direction: Option<FacingDirection>,
    pub item_rotation: i8,
    pub map: bool,
    pub photo: bool,
}

/// `PaintingEntity`.
#[derive(Clone, Debug, PartialEq)]
pub struct Painting {
    pub tile: Tile,
    pub motive: Option<Motive>,
    pub direction: FacingDirectionHorizontal,
}

impl Default for Painting {
    fn default() -> Self {
        Painting { tile: Tile::default(), motive: None, direction: FacingDirectionHorizontal::SOUTH }
    }
}

/// The class specific part of an entity.
#[derive(Clone, Debug, PartialEq)]
pub enum EntityData {
    ItemFrame(ItemFrame),
    Painting(Painting),
    /// `CushionEntity`.
    Cushion {
        color: DyeColor,
    },
    /// `UnknownEntity`: anything else, identified by its type.
    Unknown(EntityType),
}

/// An entity with absolute coordinates.
///
/// `PartialEq` compares floating point fields like `Double.compare` (bitwise total order: `NaN == NaN`, `-0.0 != 0.0`)
/// and ignores `original_nbt`.
#[derive(Clone, Debug)]
pub struct Entity {
    pub position: [f64; 3],
    pub motion: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub original_nbt: Option<Compound>,
    pub data: EntityData,
}

impl Entity {
    pub fn new(data: EntityData) -> Entity {
        Entity { position: [0.0; 3], motion: [0.0; 3], yaw: 0.0, pitch: 0.0, original_nbt: None, data }
    }

    /// An `Unknown` entity of the given type.
    pub fn unknown(ty: impl Into<EntityType>) -> Entity {
        Entity::new(EntityData::Unknown(ty.into()))
    }

    /// The entity type (`getEntityType`).
    pub fn entity_type(&self) -> EntityType {
        match &self.data {
            EntityData::ItemFrame(f) if f.glow => VanillaEntityType::GLOW_ITEM_FRAME.into(),
            EntityData::ItemFrame(_) => VanillaEntityType::ITEM_FRAME.into(),
            EntityData::Painting(_) => VanillaEntityType::PAINTING.into(),
            EntityData::Cushion { .. } => VanillaEntityType::CUSHION.into(),
            EntityData::Unknown(t) => t.clone(),
        }
    }
}

impl PartialEq for Entity {
    fn eq(&self, other: &Self) -> bool {
        let d = |a: f64, b: f64| a.total_cmp(&b).is_eq();
        self.position.iter().zip(&other.position).all(|(a, b)| d(*a, *b))
            && self.motion.iter().zip(&other.motion).all(|(a, b)| d(*a, *b))
            && self.yaw.total_cmp(&other.yaw).is_eq()
            && self.pitch.total_cmp(&other.pitch).is_eq()
            && self.data == other.data
    }
}
