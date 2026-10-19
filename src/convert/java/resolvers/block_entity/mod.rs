// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Block entity handlers (write direction): a model [`BlockEntity`] to the compound a Java chunk stores.
//!
//! Chunker walks a class hierarchy of handlers (`JavaBlockEntityHandler` for the position, then the container
//! handlers, then the class' own); here [`write_block_entity`] does the same steps in a fixed order and then
//! matches on the variant. Which kinds exist depends on the game version, as in `JavaBlockEntityResolver`
//! (`is_supported`); a kind that the version does not know, or that Chunker never writes to Java, gives `None`.
//!
//! Not written, as in Chunker: the cauldron (not a Java block entity), the moving piston (made from the block, with
//! no model kind), `Test` and `TestInstance` (no Java handler) and, from 26.2, beds (no longer block entities).
//! Chunker writes no `keepPacked`, lock or beacon level either, and no `SpawnPotentials`.

mod banner;
mod containers;
mod ctx;
mod item_embed;
mod misc;
mod sign;
mod skull;
mod spawner;
mod stub_items;
mod tables;
#[cfg(test)]
mod tests;

use stub_items::JavaItems;

use crate::convert::java::format::JavaFormat;
use crate::convert::model::{BlockEntity, BlockEntityData, BlockEntityKind};
use crate::convert::nbt::Compound;
use ctx::Ctx;

pub use item_embed::write_block_entity_to_item;

/// The namespaced id Java stores for a kind (`minecraft:chest`), whatever the version; empty for `Unknown`, whose id is
/// the raw type kept in the block entity. The id does not depend on the version since 1.11; `format` is taken so the
/// call matches the other resolvers.
pub fn java_block_entity_id(kind: BlockEntityKind, _format: &JavaFormat) -> &'static str {
    use BlockEntityKind as K;
    match kind {
        K::Banner => "minecraft:banner",
        K::Barrel => "minecraft:barrel",
        K::Beacon => "minecraft:beacon",
        K::Bed => "minecraft:bed",
        K::Beehive => "minecraft:beehive",
        K::Bell => "minecraft:bell",
        K::BlastFurnace => "minecraft:blast_furnace",
        K::BrewingStand => "minecraft:brewing_stand",
        K::Brushable => "minecraft:brushable_block",
        K::CalibratedSculkSensor => "minecraft:calibrated_sculk_sensor",
        K::Campfire => "minecraft:campfire",
        K::Cauldron => "minecraft:cauldron",
        K::Chest => "minecraft:chest",
        K::ChiseledBookshelf => "minecraft:chiseled_bookshelf",
        K::CommandBlock => "minecraft:command_block",
        K::Comparator => "minecraft:comparator",
        K::Conduit => "minecraft:conduit",
        K::CopperGolemStatue => "minecraft:copper_golem_statue",
        K::Crafter => "minecraft:crafter",
        K::CreakingHeart => "minecraft:creaking_heart",
        K::DaylightDetector => "minecraft:daylight_detector",
        K::DecoratedPot => "minecraft:decorated_pot",
        K::Dispenser => "minecraft:dispenser",
        K::Dropper => "minecraft:dropper",
        K::EnchantmentTable => "minecraft:enchanting_table",
        K::EndGateway => "minecraft:end_gateway",
        K::EndPortal => "minecraft:end_portal",
        K::EnderChest => "minecraft:ender_chest",
        K::Furnace => "minecraft:furnace",
        K::HangingSign => "minecraft:hanging_sign",
        K::Hopper => "minecraft:hopper",
        K::Jigsaw => "minecraft:jigsaw",
        K::Jukebox => "minecraft:jukebox",
        K::Lectern => "minecraft:lectern",
        K::PotentSulfur => "minecraft:potent_sulfur",
        K::SculkCatalyst => "minecraft:sculk_catalyst",
        K::SculkSensor => "minecraft:sculk_sensor",
        K::SculkShrieker => "minecraft:sculk_shrieker",
        K::Shelf => "minecraft:shelf",
        K::ShulkerBox => "minecraft:shulker_box",
        K::Sign => "minecraft:sign",
        K::Skull => "minecraft:skull",
        K::Smoker => "minecraft:smoker",
        K::Spawner => "minecraft:mob_spawner",
        K::Structure => "minecraft:structure_block",
        K::Test => "minecraft:test_block",
        K::TestInstance => "minecraft:test_instance_block",
        K::TrappedChest => "minecraft:trapped_chest",
        K::TrialSpawner => "minecraft:trial_spawner",
        K::Vault => "minecraft:vault",
        K::Unknown => "",
    }
}

/// Kinds that never reach a Java chunk: the cauldron is removed before writing (it is not a Java block entity), and
/// Chunker has no Java handler for the test blocks.
pub const UNSUPPORTED_ON_JAVA: [BlockEntityKind; 3] =
    [BlockEntityKind::Cauldron, BlockEntityKind::Test, BlockEntityKind::TestInstance];

/// Whether `write_block_entity` writes the kind for the format: the handler registration of
/// `JavaBlockEntityResolver` (`Unknown` is always a pass-through).
pub fn is_supported(kind: BlockEntityKind, format: &JavaFormat) -> bool {
    use BlockEntityKind as K;
    let since = |major, minor, patch| format.version.is_greater_than_or_equal(major, minor, patch);
    match kind {
        K::Unknown => true,
        K::Cauldron | K::Test | K::TestInstance => false,
        K::Bed => format.bed_block_entity,
        K::Barrel | K::BlastFurnace | K::Smoker | K::Campfire | K::Jigsaw | K::Bell | K::Lectern => since(1, 14, 0),
        K::Beehive => since(1, 15, 0),
        K::SculkSensor => since(1, 17, 0),
        K::SculkCatalyst | K::SculkShrieker => since(1, 19, 0),
        K::ChiseledBookshelf => since(1, 19, 3),
        K::Brushable | K::DecoratedPot => since(1, 19, 4),
        K::HangingSign | K::CalibratedSculkSensor => since(1, 20, 0),
        K::Crafter | K::TrialSpawner => since(1, 20, 3),
        K::Vault => since(1, 20, 5),
        K::CreakingHeart => since(1, 21, 2),
        K::Shelf | K::CopperGolemStatue => since(1, 21, 9),
        K::PotentSulfur => since(26, 2, 0),
        _ => true,
    }
}

/// Write a block entity as the Java chunk compound of `format` (`id`, `x`, `y`, `z` and the kind's fields).
///
/// The original NBT, when the entity carries it, is copied first and the handlers overwrite what they write; the
/// `id` stays the one of the kind. `None` when the format has no such block entity or a text or item fails to convert.
pub fn write_block_entity(items: &JavaItems, format: &JavaFormat, be: &BlockEntity) -> Option<Compound> {
    let kind = be.kind();
    if !is_supported(kind, format) {
        return None;
    }
    let ctx = Ctx::new(items, format);
    let id = match &be.data {
        BlockEntityData::Unknown(raw) => raw.to_string(),
        _ => java_block_entity_id(kind, format).to_owned(),
    };
    let mut out = Compound::new();
    out.put("id", id);
    if let Some(original) = &be.original_nbt {
        for (key, value) in original {
            if key != "id" {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    out.put("x", be.x);
    out.put("y", be.y);
    out.put("z", be.z);
    write_fields(&ctx, &mut out, be)?;
    Some(out)
}

/// The container, loot table and class specific handlers, in the hierarchy order of Chunker.
fn write_fields(ctx: &Ctx, out: &mut Compound, be: &BlockEntity) -> Option<()> {
    use BlockEntityData as D;
    match &be.data {
        D::Barrel(r) | D::Chest(r) | D::Dispenser(r) | D::Dropper(r) | D::ShulkerBox(r) | D::TrappedChest(r) => {
            containers::write_container(ctx, out, be, &r.container)?;
            containers::write_loot_table(ctx, out, &r.loot_table);
        }
        D::EnderChest(c) | D::Hopper(c) => containers::write_container(ctx, out, be, c)?,
        D::BlastFurnace(f) | D::Furnace(f) | D::Smoker(f) => {
            containers::write_container(ctx, out, be, &f.container)?;
            containers::write_furnace(ctx, out, f);
        }
        D::BrewingStand(b) => {
            containers::write_container(ctx, out, be, &b.container)?;
            containers::write_brewing_stand(ctx, out, b);
        }
        D::Crafter(c) => {
            containers::write_container(ctx, out, be, &c.container.container)?;
            containers::write_loot_table(ctx, out, &c.container.loot_table);
            containers::write_crafter(out, c);
        }
        D::Banner(b) => banner::write_banner(ctx, out, be, b)?,
        D::Beacon(b) => banner::write_beacon(ctx, out, be, b)?,
        D::Brushable(b) => misc::write_brushable(ctx, out, b),
        D::Campfire(c) => misc::write_campfire(ctx, out, c),
        D::ChiseledBookshelf(books) => misc::write_slot_array(ctx, out, &books[..]),
        D::Shelf(items) => misc::write_slot_array(ctx, out, &items[..]),
        D::CommandBlock(c) => misc::write_command_block(ctx, out, be, c)?,
        D::Comparator { output_signal } => {
            out.put("OutputSignal", *output_signal);
        }
        D::DecoratedPot(p) => misc::write_decorated_pot(ctx, out, p),
        D::EnchantmentTable => misc::write_custom_name(ctx, out, be)?,
        D::EndGateway(g) => misc::write_end_gateway(out, g),
        D::HangingSign(s) | D::Sign(s) => sign::write(ctx, out, s)?,
        D::Jigsaw(j) => misc::write_jigsaw(ctx, out, j),
        D::Jukebox { record } => misc::write_jukebox(ctx, out, record.as_deref()),
        D::Lectern { book, page } => misc::write_lectern(ctx, out, book.as_deref(), *page),
        D::SculkSensor | D::CalibratedSculkSensor => misc::write_sculk_sensor(out),
        D::Skull(s) => skull::write(ctx, out, s),
        D::Spawner(s) => spawner::write(ctx, out, s),
        D::Structure(s) => misc::write_structure(ctx, out, s),
        // Empty handlers: the state lives in the block state.
        D::Bed
        | D::Beehive
        | D::Bell
        | D::Cauldron(_)
        | D::Conduit
        | D::CopperGolemStatue
        | D::CreakingHeart
        | D::DaylightDetector
        | D::EndPortal
        | D::PotentSulfur
        | D::SculkCatalyst
        | D::SculkShrieker
        | D::Test
        | D::TestInstance
        | D::TrialSpawner
        | D::Vault
        | D::Unknown(_) => {}
    }
    Some(())
}
