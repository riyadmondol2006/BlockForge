//! Entities (write direction): a model [`Entity`] as the entity compound of a Java version.
//!
//! Item frames and paintings come from the model's classes. Bedrock actors carry their compound
//! ([`Entity::source_nbt`]) and are mapped field by field: the fields every entity has (`common`), those of mobs
//! (`mob`), then those of the type (`types`). Their items are read by the reader's resolvers
//! ([`EntitySource`]) and written through the same item writer and hooks as the items of container block entities,
//! so shulker box contents, enchantments and map links come along. Chunker writes no entity data; this is
//! BlockForge's own.
//!
//! Each version is written in its own format. The switches below were checked with the game's own DataFixer: an
//! entity written for 1.19.2 and upgraded by the game to each later version gives what is written for that version.
//! - 1.16: `UUID` int arrays (before: `UUIDMost`/`UUIDLeast` and `OwnerUUID` strings), `minecraft:generic.*`
//!   attribute names.
//! - 1.19: the cat `variant` (before: `CatType`), painting `variant` and `facing` (before: `Motive`, `Facing`).
//! - 1.20.5: item components, the `leash` int array, `body_armor_item` (horse armour, llama carpets), chested horse
//!   slots from 0, the bee `hive_pos`, `generic.jump_strength`.
//! - 1.21.2: `attributes: [{id, base}]` without `generic.`, one type per boat wood.
//! - 1.21.5: `equipment` and `drop_chances` (saddles too), names as text components, `block_pos` of paintings and
//!   frames, the turtle `home_pos`.
//! - 26.1: `VillagerDataFinalized`; 26.3: carried and falling blocks as `{id, properties}`.
//!
//! Left out: passengers (riders are written next to their mount), armor stand poses other than the default and the
//! zero pose, villager professions before 1.14.

mod common;
mod mob;
mod types;

#[cfg(test)]
mod tests;

use uuid::Uuid;

use super::itemstack::{ItemHooks, JavaItems};
use crate::convert::java::format::JavaFormat;
use crate::convert::model::entity::{ItemFrame, Painting};
use crate::convert::model::{BlockIdentifier, Entity, EntityData, EntitySource, EntityType, ItemStack};
use crate::convert::nbt::{Compound, Tag};
use crate::convert::util::Version;

pub use common::uuid_of_unique_id;

/// What the entity writer needs from the world writer.
pub struct EntityContext<'a> {
    pub items: &'a JavaItems,
    pub format: &'a JavaFormat,
    /// The world writer's item hooks (map ids, the block entity data of items), the same as for containers.
    pub hooks: &'a dyn ItemHooks,
    /// Reads the items and blocks of source compounds; without it, Bedrock actors are not written.
    pub source: Option<&'a dyn EntitySource>,
    /// The Java block state compound (`{Name, Properties}`) of a model block, `None` for air or no Java form.
    pub block_state: &'a dyn Fn(&BlockIdentifier) -> Option<Compound>,
    /// The Java account that owns the host's pets (`ConversionSettings::pet_owner_uuid`).
    pub pet_owner: Option<Uuid>,
}

impl<'a> EntityContext<'a> {
    pub(crate) fn version(&self) -> Version {
        self.format.version
    }

    /// `version >= major.minor.patch`.
    pub(crate) fn since(&self, major: i32, minor: i32, patch: i32) -> bool {
        self.format.version.is_greater_than_or_equal(major, minor, patch)
    }

    /// A model stack as an item compound, through the writer's hooks.
    pub(crate) fn item(&self, stack: &ItemStack) -> Option<Compound> {
        if stack.is_air() {
            return None;
        }
        self.items.write_with(stack, self.hooks)
    }

    /// A source item compound as a Java item compound (`None`: air, or no Java form).
    pub(crate) fn source_item(&self, nbt: &Compound) -> Option<Compound> {
        let stack = self.source?.item(nbt)?;
        self.item(&stack)
    }

    /// A source block compound as a Java block state, `None` for air.
    pub(crate) fn source_block(&self, nbt: &Compound) -> Option<Compound> {
        let block = self.source?.block(nbt)?;
        if block.is_air() {
            return None;
        }
        (self.block_state)(&block)
    }

    /// The Java id of a type for this version.
    pub(crate) fn entity_id(&self, ty: &EntityType) -> Option<String> {
        self.items.lookups().entity_type(ty)
    }
}

/// The entity compound of `entity` for the context's version, `None` when the version has no such entity (or a
/// Bedrock actor arrives without a source reader).
pub fn write_entity(ctx: &EntityContext<'_>, entity: &Entity) -> Option<Compound> {
    match &entity.data {
        EntityData::ItemFrame(frame) => write_item_frame(ctx, entity, frame),
        EntityData::Painting(painting) => write_painting(ctx, entity, painting),
        EntityData::Unknown(EntityType::Vanilla(ty)) => {
            let source = entity.source_nbt.as_ref()?;
            ctx.source?;
            types::write_actor(ctx, entity, *ty, source)
        }
        EntityData::Cushion { .. } | EntityData::Unknown(EntityType::Custom(_)) => None,
    }
}

/// The 3D facing id Java stores (`Direction.get3DDataValue`).
fn facing_3d(direction: crate::convert::model::FacingDirection) -> i8 {
    use crate::convert::model::FacingDirection as F;
    match direction {
        F::DOWN => 0,
        F::UP => 1,
        F::NORTH => 2,
        F::SOUTH => 3,
        F::WEST => 4,
        F::EAST => 5,
    }
}

/// `TileX/Y/Z` before 1.21.5, `block_pos` from then on.
fn put_tile(ctx: &EntityContext<'_>, out: &mut Compound, tile: crate::convert::model::entity::Tile) {
    if ctx.since(1, 21, 5) {
        out.put("block_pos", vec![tile.x, tile.y, tile.z]);
    } else {
        out.put("TileX", tile.x);
        out.put("TileY", tile.y);
        out.put("TileZ", tile.z);
    }
}

/// `ItemFrame` / `GlowItemFrame` (a Bedrock frame block and its block entity).
fn write_item_frame(ctx: &EntityContext<'_>, entity: &Entity, frame: &ItemFrame) -> Option<Compound> {
    let mut out = Compound::new();
    out.put("id", ctx.entity_id(&entity.entity_type())?);
    let direction = frame.direction.unwrap_or(crate::convert::model::FacingDirection::NORTH);
    let facing = facing_3d(direction);
    let (dx, dy, dz) = common::step_3d(facing);
    let t = frame.tile;
    // The centre of the frame, against the block it hangs on; the game places it again from the tile and facing.
    let pos = [
        t.x as f64 + 0.5 - 0.46875 * dx as f64,
        t.y as f64 + 0.5 - 0.46875 * dy as f64,
        t.z as f64 + 0.5 - 0.46875 * dz as f64,
    ];
    common::put_position(&mut out, pos, [0.0; 3], entity.yaw, entity.pitch);
    put_tile(ctx, &mut out, t);
    out.put("Facing", facing);
    if let Some(item) = frame.item.as_deref().and_then(|stack| ctx.item(stack)) {
        out.put("Item", item);
    }
    out.put("ItemRotation", frame.item_rotation.rem_euclid(8));
    out.put("ItemDropChance", 1.0f32);
    Some(out)
}

/// `Painting`: the motive, the 2D facing (0 south, 1 west, 2 north, 3 east) and the block it hangs from.
fn write_painting(ctx: &EntityContext<'_>, entity: &Entity, painting: &Painting) -> Option<Compound> {
    use crate::convert::mapping::{Edition, IdTable, tables::names};
    use crate::convert::model::FacingDirectionHorizontal as H;
    let motive = painting.motive?;
    let version = ctx.version();
    let map_version = crate::convert::mapping::Version::new(
        version.major.max(0) as u32,
        version.minor.max(0) as u32,
        version.patch.max(0) as u32,
    );
    let table = IdTable::load(Edition::Java, names::PAINTING, map_version);
    let variant = table.from_pivot_str(motive.name())?.to_string();
    let facing: i8 = match painting.direction {
        H::SOUTH => 0,
        H::WEST => 1,
        H::NORTH => 2,
        H::EAST => 3,
    };
    let mut out = Compound::new();
    out.put("id", ctx.entity_id(&entity.entity_type())?);
    common::put_position(&mut out, entity.position, [0.0; 3], entity.yaw, 0.0);
    if let Some(source) = &entity.source_nbt {
        common::put_identity(ctx, &mut out, source);
    }
    put_tile(ctx, &mut out, painting.tile);
    if ctx.since(1, 19, 0) {
        out.put("variant", variant);
        out.put("facing", facing);
    } else {
        out.put("Motive", variant);
        out.put("Facing", facing);
    }
    Some(out)
}

/// A list of `Tag::Compound` items, empty compounds for empty slots (`HandItems`, `ArmorItems` before 1.21.5).
pub(crate) fn slots(items: &[Option<Compound>]) -> Tag {
    Tag::List(items.iter().map(|i| Tag::Compound(i.clone().unwrap_or_default())).collect())
}
