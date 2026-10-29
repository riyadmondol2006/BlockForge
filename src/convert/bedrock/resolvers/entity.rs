//! Bedrock actors (the `actorprefix` records and the legacy chunk `Entity` record) as model entities: the type, the
//! position and rotation, the entity filters of the conversion settings, paintings (Bedrock stores their centre, the
//! model their block) and the leash knot a mob is tied to. Everything else stays in the actor's compound
//! ([`Entity::source_nbt`]) for the Java entity writer.
//!
//! Chunker reads no Bedrock entity; this is BlockForge's own.

use std::collections::HashMap;

use super::itemstack::BedrockItems;
use crate::convert::level::{ConversionSettings, EntityFilter};
use crate::convert::model::entity::{Painting, Tile};
use crate::convert::model::{
    Entity, EntityData, EntityType, FacingDirectionHorizontal, Motive, VanillaEntityType as V,
};
use crate::convert::nbt::{Compound, Tag};

/// The `UniqueID` of the world's host (`~local_player`); the pets of the host name it as `OwnerNew`.
pub const HOST_UNIQUE_ID: i64 = -4294967295;

/// The `UniqueID` of an actor.
pub fn unique_id(tag: &Compound) -> Option<i64> {
    tag.get_i64("UniqueID")
}

/// An actor id field (`OwnerNew`, `LeasherID`); -1 and 0 mean none.
pub fn actor_link(tag: &Compound, key: &str) -> Option<i64> {
    tag.get_i64(key).filter(|id| *id != -1 && *id != 0)
}

/// How the filters treat a type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A mob that is not hostile.
    Mob,
    /// A hostile mob (Java's `Enemy`).
    Hostile,
    /// Not a mob: paintings, armor stands, minecarts, boats, end crystals.
    Object,
    /// There for a moment only: dropped items, experience orbs, falling blocks, projectiles, lightning.
    Transient,
    /// Never written: Java makes the leash knots again from the mobs tied to them, and players live elsewhere.
    Never,
}

const HOSTILE: &[V] = &[
    V::BLAZE,
    V::BOGGED,
    V::BREEZE,
    V::CAVE_SPIDER,
    V::CREAKING,
    V::CREEPER,
    V::DROWNED,
    V::ELDER_GUARDIAN,
    V::ENDER_DRAGON,
    V::ENDERMAN,
    V::ENDERMITE,
    V::EVOKER,
    V::GHAST,
    V::GIANT,
    V::GUARDIAN,
    V::HOGLIN,
    V::HUSK,
    V::ILLUSIONER,
    V::MAGMA_CUBE,
    V::PARCHED,
    V::PHANTOM,
    V::PIGLIN,
    V::PIGLIN_BRUTE,
    V::PILLAGER,
    V::RAVAGER,
    V::SHULKER,
    V::SILVERFISH,
    V::SKELETON,
    V::SLIME,
    V::SPIDER,
    V::STRAY,
    V::VEX,
    V::VINDICATOR,
    V::WARDEN,
    V::WITCH,
    V::WITHER,
    V::WITHER_SKELETON,
    V::ZOGLIN,
    V::ZOMBIE,
    V::ZOMBIE_NAUTILUS,
    V::ZOMBIE_VILLAGER,
    V::ZOMBIFIED_PIGLIN,
];

const TRANSIENT: &[V] = &[
    V::AREA_EFFECT_CLOUD,
    V::ARROW,
    V::BREEZE_WIND_CHARGE_PROJECTILE,
    V::DRAGON_FIREBALL,
    V::EGG,
    V::ENDER_PEARL,
    V::EVOKER_FANGS,
    V::EXPERIENCE_BOTTLE,
    V::EXPERIENCE_ORB,
    V::EYE_OF_ENDER,
    V::FALLING_BLOCK,
    V::FIREBALL,
    V::FIREWORK_ROCKET,
    V::FISHING_BOBBER,
    V::ITEM,
    V::LIGHTNING_BOLT,
    V::LINGERING_POTION,
    V::LLAMA_SPIT,
    V::OMINOUS_ITEM_SPAWNER,
    V::POTION,
    V::SHULKER_BULLET,
    V::SMALL_FIREBALL,
    V::SNOWBALL,
    V::SPECTRAL_ARROW,
    V::TNT,
    V::TRIDENT,
    V::WIND_CHARGE,
    V::WITHER_SKULL,
    V::WITHER_SKULL_DANGEROUS,
];

const OBJECTS: &[V] = &[
    V::ARMOR_STAND,
    V::BLOCK_DISPLAY,
    V::CHEST_MINECART,
    V::COMMAND_BLOCK_MINECART,
    V::END_CRYSTAL,
    V::FURNACE_MINECART,
    V::GLOW_ITEM_FRAME,
    V::HOPPER_MINECART,
    V::INTERACTION,
    V::ITEM_DISPLAY,
    V::ITEM_FRAME,
    V::MARKER,
    V::MINECART,
    V::PAINTING,
    V::SPAWNER_MINECART,
    V::TEXT_DISPLAY,
    V::TNT_MINECART,
];

/// Mobs that "pets and named only" keeps whatever they carry.
const ALWAYS_KEPT: &[V] = &[V::VILLAGER, V::WANDERING_TRADER, V::IRON_GOLEM, V::SNOW_GOLEM, V::COPPER_GOLEM];

/// Boats by Bedrock `Variant`: (boat, chest boat).
const BOATS: [(V, V); 10] = [
    (V::OAK_BOAT, V::OAK_CHEST_BOAT),
    (V::SPRUCE_BOAT, V::SPRUCE_CHEST_BOAT),
    (V::BIRCH_BOAT, V::BIRCH_CHEST_BOAT),
    (V::JUNGLE_BOAT, V::JUNGLE_CHEST_BOAT),
    (V::ACACIA_BOAT, V::ACACIA_CHEST_BOAT),
    (V::DARK_OAK_BOAT, V::DARK_OAK_CHEST_BOAT),
    (V::MANGROVE_BOAT, V::MANGROVE_CHEST_BOAT),
    (V::BAMBOO_RAFT, V::BAMBOO_CHEST_RAFT),
    (V::CHERRY_BOAT, V::CHERRY_CHEST_BOAT),
    (V::PALE_OAK_BOAT, V::PALE_OAK_CHEST_BOAT),
];

/// Whether `t` is a boat (or raft) of any wood, with or without a chest.
pub fn is_boat(t: V) -> bool {
    BOATS.iter().any(|(b, c)| *b == t || *c == t) || matches!(t, V::POPLAR_BOAT | V::POPLAR_CHEST_BOAT)
}

/// How the filters treat a type.
pub fn kind(t: V) -> Kind {
    if matches!(t, V::LEASH_KNOT | V::PLAYER | V::MOVING_BLOCK | V::SHIELD | V::ELDER_GUARDIAN_GHOST) {
        Kind::Never
    } else if TRANSIENT.contains(&t) {
        Kind::Transient
    } else if OBJECTS.contains(&t) || is_boat(t) {
        Kind::Object
    } else if HOSTILE.contains(&t) {
        Kind::Hostile
    } else {
        Kind::Mob
    }
}

/// Whether an actor carries one of the component groups (`definitions`, `+minecraft:cat_calico`).
pub fn has_definition(tag: &Compound, group: &str) -> bool {
    tag.get_list("definitions").is_some_and(|list| {
        list.iter().filter_map(Tag::as_str).any(|d| d.strip_prefix('+').is_some_and(|g| g == group))
    })
}

/// The vanilla type of an actor: the identifier through the entity table, with the types Bedrock keeps in fields
/// (trader llamas are llamas with `MarkVariant` 1, boats keep their wood in `Variant`).
pub fn entity_type(items: &BedrockItems, tag: &Compound) -> Option<V> {
    let identifier = tag.get_str("identifier")?;
    let EntityType::Vanilla(t) = items.tables.entity_type(identifier)? else { return None };
    let variant = tag.get_i32_or("Variant", 0);
    Some(match t {
        V::LLAMA
            if tag.get_i32_or("MarkVariant", 0) == 1 || has_definition(tag, "minecraft:llama_wandering_trader") =>
        {
            V::TRADER_LLAMA
        }
        V::OAK_BOAT | V::OAK_CHEST_BOAT => {
            let (boat, chest) = BOATS.get(variant as usize).copied().unwrap_or(BOATS[0]);
            if t == V::OAK_CHEST_BOAT { chest } else { boat }
        }
        other => other,
    })
}

/// Whether the settings keep an actor of this type.
pub fn keep(settings: &ConversionSettings, t: V, tag: &Compound) -> bool {
    match kind(t) {
        Kind::Never => false,
        Kind::Transient => !settings.drop_transient_entities && settings.entity_filter != EntityFilter::None,
        Kind::Object => settings.entity_filter != EntityFilter::None,
        kind @ (Kind::Mob | Kind::Hostile) => match settings.entity_filter {
            EntityFilter::All => true,
            EntityFilter::NoHostile => kind == Kind::Mob,
            EntityFilter::PetsAndNamedOnly => is_cared_for(t, tag),
            EntityFilter::None => false,
        },
    }
}

/// A mob someone looks after: tamed or owned, named, leashed, or a villager, trader or golem.
pub fn is_cared_for(t: V, tag: &Compound) -> bool {
    ALWAYS_KEPT.contains(&t)
        || tag.get_i8_or("IsTamed", 0) != 0
        || actor_link(tag, "OwnerNew").is_some()
        || actor_link(tag, "LeasherID").is_some()
        || tag.get_str("CustomName").is_some_and(|n| !n.is_empty())
}

/// Three numbers of a list (`Pos`, `Motion`), any numeric tag type.
pub fn vec3(tag: &Compound, key: &str) -> Option<[f64; 3]> {
    let list = tag.get_list(key)?;
    let n = |i: usize| list.get(i).and_then(Tag::as_number);
    Some([n(0)?, n(1)?, n(2)?])
}

/// The chunk a position lies in.
pub fn chunk_of(pos: [f64; 3]) -> (i32, i32) {
    ((pos[0].floor() as i64 >> 4) as i32, (pos[2].floor() as i64 >> 4) as i32)
}

/// One actor as a model entity, `None` when it is filtered out or has no vanilla type. `knots` are the positions
/// of the leash knots of the dimension by `UniqueID`.
pub fn read_actor(
    items: &BedrockItems,
    settings: &ConversionSettings,
    tag: &Compound,
    knots: &HashMap<i64, [f64; 3]>,
) -> Option<Entity> {
    let t = entity_type(items, tag)?;
    if !keep(settings, t, tag) {
        return None;
    }
    let position = vec3(tag, "Pos")?;
    if position.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let data = match t {
        V::PAINTING => EntityData::Painting(painting(tag, position)?),
        other => EntityData::Unknown(EntityType::Vanilla(other)),
    };
    let mut entity = Entity::new(data);
    entity.position = position;
    if matches!(kind(t), Kind::Object) && t.name().ends_with("MINECART") {
        // Bedrock keeps a minecart half a block above its rail; Java sits it 1/16 above.
        let y = position[1];
        if (y - y.floor() - 0.5).abs() < 1e-3 {
            entity.position[1] = y.floor() + 0.0625;
        }
    }
    if let Some(rotation) = tag.get_list("Rotation") {
        entity.yaw = rotation.first().and_then(Tag::as_number).unwrap_or(0.0) as f32;
        entity.pitch = rotation.get(1).and_then(Tag::as_number).unwrap_or(0.0) as f32;
    }
    if let Some(knot) = actor_link(tag, "LeasherID").and_then(|id| knots.get(&id)) {
        entity.leash = Some(Tile { x: knot[0].floor() as i32, y: knot[1].floor() as i32, z: knot[2].floor() as i32 });
    }
    entity.source_nbt = Some(tag.clone());
    Some(entity)
}

/// A Bedrock motive name (`SkullAndRoses`, `Aztec2`) as the model motive.
pub fn motive(name: &str) -> Option<Motive> {
    let mut constant = String::with_capacity(name.len() + 4);
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            constant.push('_');
        }
        constant.push(c.to_ascii_uppercase());
    }
    Motive::from_name(&constant)
}

/// A painting: Bedrock stores its centre, 1/32 off the wall, and `Direction` (0 south, 1 west, 2 north, 3 east, as
/// Java's facing). The block is found as Java's `HangingEntity` places the centre: half a block towards the wall,
/// and half a block left and down for an even width or height.
pub fn painting(tag: &Compound, center: [f64; 3]) -> Option<Painting> {
    let motive = motive(tag.get_str("Motive")?)?;
    let direction = match tag.get_i8("Direction").map(i32::from).or_else(|| tag.get_i32("Direction")).unwrap_or(0) {
        0 => FacingDirectionHorizontal::SOUTH,
        1 => FacingDirectionHorizontal::WEST,
        2 => FacingDirectionHorizontal::NORTH,
        _ => FacingDirectionHorizontal::EAST,
    };
    let (dx, dz) = step(direction);
    // Counter-clockwise of the facing, seen from above.
    let (cx, cz) = (dz, -dx);
    let (width, height) = Motive::SIZES[motive as usize];
    let even = |n: u8| if n % 2 == 0 { 0.5 } else { 0.0 };
    let (ow, oh) = (even(width), even(height));
    let x = center[0] + 0.46875 * dx as f64 - ow * cx as f64 - 0.5;
    let y = center[1] - oh - 0.5;
    let z = center[2] + 0.46875 * dz as f64 - ow * cz as f64 - 0.5;
    let tile = Tile { x: x.round() as i32, y: y.round() as i32, z: z.round() as i32 };
    Some(Painting { tile, motive: Some(motive), direction })
}

/// The x and z step of a horizontal direction.
pub fn step(direction: FacingDirectionHorizontal) -> (i32, i32) {
    match direction {
        FacingDirectionHorizontal::NORTH => (0, -1),
        FacingDirectionHorizontal::SOUTH => (0, 1),
        FacingDirectionHorizontal::WEST => (-1, 0),
        FacingDirectionHorizontal::EAST => (1, 0),
    }
}
