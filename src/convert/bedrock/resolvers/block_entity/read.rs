// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! The read side of the Bedrock block entity handlers, one function per Chunker handler.

use crate::convert::model::blockentity::{Container, Joint, SignFace, StructureMirror, StructureRotation};
use crate::convert::model::{
    BlockEntity, BlockEntityData, BlockEntityKind, Color, CopperGolemPose, DyeColor, FacingDirection, ItemId,
    ItemStack, PotionBottleType, StructureBlockMode,
};
use crate::convert::nbt::{Compound, Tag};
use crate::convert::util::json_text;

use super::font;
use super::{BedrockOnly, BedrockOnlyKind, Extra, Read, Target};
use crate::convert::bedrock::resolvers::itemstack::{BedrockItems, Missing};

/// Widest sign line before the text is split (pixels).
const SIGN_MAX_WIDTH: i32 = 88;
const HANGING_SIGN_MAX_WIDTH: i32 = 60;

/// Read `tag` as `target`.
pub(super) fn read_target(items: &BedrockItems, target: Target, tag: &Compound, keep: bool) -> Option<Read> {
    match target {
        Target::Model(kind) => read_model(items, kind, tag, keep),
        Target::PistonArm => Some(only(
            tag,
            BedrockOnlyKind::PistonArm {
                sticky: tag.get_i8_or("Sticky", 0) == 1,
                extended: tag.get_i8_or("State", 0) == 2,
            },
        )),
        Target::NoteBlock => {
            let note = u8::try_from(tag.get_i8_or("note", 0)).ok().filter(|n| *n <= 24);
            Some(only(tag, BedrockOnlyKind::NoteBlock { note }))
        }
        Target::ItemFrame { glow } => {
            let item = tag.get_compound("Item").and_then(|i| items.read(i));
            let rotation = match tag.get("ItemRotation") {
                Some(Tag::Float(f)) => java_round((f + 360.0).rem_euclid(360.0) / 45.0) as i8,
                Some(Tag::Byte(b)) => *b,
                _ => 0,
            };
            Some(only(tag, BedrockOnlyKind::ItemFrame { glow, item, rotation }))
        }
        Target::FlowerPot => {
            let plant = tag.get_compound("PlantBlock").and_then(|p| items.read_block_compound(p, false));
            Some(only(tag, BedrockOnlyKind::FlowerPot { plant }))
        }
        Target::Marker(name) => Some(only(tag, BedrockOnlyKind::Marker(name))),
    }
}

fn only(tag: &Compound, kind: BedrockOnlyKind) -> Read {
    Read::BedrockOnly(BedrockOnly {
        x: tag.get_i32_or("x", 0),
        y: tag.get_i32_or("y", 0),
        z: tag.get_i32_or("z", 0),
        kind,
    })
}

/// `Math.round` for floats (round half up).
fn java_round(v: f32) -> i32 {
    (v + 0.5).floor() as i32
}

/// The base handler: position, `isMovable`, `CustomName` and the original tag.
pub(super) fn read_common(_items: &BedrockItems, tag: &Compound, entity: &mut BlockEntity, keep: bool) {
    if keep {
        entity.original_nbt = Some(tag.clone());
    }
    entity.x = tag.get_i32_or("x", 0);
    entity.y = tag.get_i32_or("y", 0);
    entity.z = tag.get_i32_or("z", 0);
    entity.movable = tag.get_i8_or("isMovable", 0) == 1;
    entity.custom_name = tag.get_str("CustomName").map(json_text::from_text);
}

fn read_model(items: &BedrockItems, kind: BlockEntityKind, tag: &Compound, keep: bool) -> Option<Read> {
    let data = BlockEntityData::default_for(kind)?;
    let mut entity = BlockEntity::new(0, 0, 0, data);
    read_common(items, tag, &mut entity, keep);
    let mut extra = Extra::None;

    if kind.is_container() {
        let brewing = kind == BlockEntityKind::BrewingStand;
        if let Some(container) = entity.data.container_mut() {
            read_container(items, tag, container, brewing);
        }
    }
    if kind.is_randomizable() {
        let table = loot_table(items, tag);
        match &mut entity.data {
            BlockEntityData::Barrel(r)
            | BlockEntityData::Chest(r)
            | BlockEntityData::Dispenser(r)
            | BlockEntityData::Dropper(r)
            | BlockEntityData::ShulkerBox(r)
            | BlockEntityData::TrappedChest(r) => r.loot_table = table,
            BlockEntityData::Crafter(c) => c.container.loot_table = table,
            _ => {}
        }
    }

    match &mut entity.data {
        BlockEntityData::Chest(_) => {
            let pair = (tag.contains_key("pairx") || tag.contains_key("pairz"))
                .then(|| (tag.get_i32_or("pairx", entity.x), tag.get_i32_or("pairz", entity.z)));
            let lead = tag.get_i8_or("pairlead", 0) == 1;
            extra = Extra::Chest { pair, lead };
        }
        BlockEntityData::Furnace(f) | BlockEntityData::BlastFurnace(f) | BlockEntityData::Smoker(f) => {
            f.burn_time = tag.get_i16_or("BurnTime", 0);
            f.cook_time = tag.get_i16_or("CookTime", 0);
            f.cook_time_total = tag.get_i16_or("BurnDuration", 0);
        }
        BlockEntityData::Sign(sign) | BlockEntityData::HangingSign(sign) => {
            let hanging = kind == BlockEntityKind::HangingSign;
            sign.waxed = tag.get_i8_or("IsWaxed", 0) == 1;
            read_sign_face(tag.get_compound("FrontText").unwrap_or(tag), &mut sign.front, hanging);
            if let Some(back) = tag.get_compound("BackText") {
                read_sign_face(back, &mut sign.back, hanging);
            }
        }
        BlockEntityData::Spawner(s) => {
            s.delay = tag.get_i16_or("Delay", 20);
            s.spawn_range = tag.get_i16_or("SpawnRange", 4);
            s.max_spawn_delay = tag.get_i16_or("MaxSpawnDelay", 800);
            s.min_spawn_delay = tag.get_i16_or("MinSpawnDelay", 200);
            s.spawn_count = tag.get_i16_or("SpawnCount", 4);
            s.required_player_range = tag.get_i16_or("RequiredPlayerRange", 16);
            s.max_nearby_entities = tag.get_i16_or("MaxNearbyEntities", 6);
            if tag.contains_key("EntityIdentifier") {
                let id = tag.get_str("EntityIdentifier").unwrap_or_default();
                if !id.is_empty() && id != "minecraft:" {
                    s.entity_type = items.read_entity_type(id);
                    if s.entity_type.is_none() {
                        items.missing(Missing::EntityType, id);
                    }
                }
            }
        }
        BlockEntityData::Skull(_) => {
            let rotation = if tag.contains_key("Rotation") {
                java_round((tag.get_f32_or("Rotation", 0.0) + 360.0).rem_euclid(360.0) / 22.5) as i8
            } else {
                tag.get_i8_or("Rot", 0)
            };
            extra = Extra::Skull {
                skull_type: tag.get_i8_or("SkullType", 0),
                rotation,
                moving_mouth: tag.get_i8_or("DoingAnimation", 0) == 1,
            };
        }
        BlockEntityData::Beacon(b) => {
            b.primary_effect = read_effect(items, tag.get_i32_or("primary", 0));
            b.secondary_effect = read_effect(items, tag.get_i32_or("secondary", 0));
        }
        BlockEntityData::BrewingStand(b) => {
            b.fuel = tag.get_i16_or("FuelAmount", 0);
            b.fuel_total = tag.get_i16_or("FuelTotal", 0);
            b.brew_time = tag.get_i16_or("CookTime", 0);
        }
        BlockEntityData::Jukebox { record } => {
            *record = tag.get_compound("RecordItem").and_then(|r| items.read(r)).map(Box::new);
        }
        BlockEntityData::Comparator { output_signal } => *output_signal = tag.get_i32_or("OutputSignal", 0),
        BlockEntityData::Cauldron(c) => {
            c.potion_bottle_type = if tag.get_i8_or("IsSplash", 0) == 1 {
                PotionBottleType::SPLASH
            } else {
                PotionBottleType::from_id(tag.get_i16_or("PotionType", -1) as i32)
            };
            let potion = tag.get_i16_or("PotionId", -1) as i32;
            c.potion_type = items.tables.potion(potion).unwrap_or_else(|| {
                items.missing(Missing::Potion, &potion.to_string());
                crate::convert::model::PotionType::WATER
            });
            if tag.contains_key("CustomColor") {
                c.custom_color = Some(Color::from_argb(tag.get_i32_or("CustomColor", 0) as u32));
            }
        }
        BlockEntityData::Banner(b) => read_banner(items, tag, b),
        BlockEntityData::Structure(s) => read_structure(items, tag, s),
        BlockEntityData::Jigsaw(j) => {
            j.name = tag.get_str("name").unwrap_or("minecraft:empty").to_string();
            j.target = tag.get_str("target").unwrap_or("minecraft:empty").to_string();
            j.pool = tag.get_str("target_pool").unwrap_or("minecraft:empty").to_string();
            j.final_state = tag.get_str("final_state").unwrap_or("minecraft:air").to_string();
            j.joint = match tag.get_str("joint").unwrap_or("rollable") {
                "aligned" => Joint::Aligned,
                _ => Joint::Rollable,
            };
            j.selection_priority = tag.get_i32_or("selection_priority", 0);
            j.placement_priority = tag.get_i32_or("placement_priority", 0);
        }
        BlockEntityData::ShulkerBox(_) => {
            let facing = match tag.get_i8_or("facing", 0) {
                2 => FacingDirection::NORTH,
                5 => FacingDirection::EAST,
                3 => FacingDirection::SOUTH,
                4 => FacingDirection::WEST,
                1 => FacingDirection::UP,
                0 => FacingDirection::DOWN,
                _ => FacingDirection::NORTH,
            };
            extra = Extra::Shulker { facing };
        }
        BlockEntityData::CommandBlock(c) => {
            c.command = Some(tag.get_str("Command").unwrap_or_default().to_string());
            c.auto = tag.get_i8_or("auto", 0) == 1;
            c.track_output = tag.get_i8_or("TrackOutput", 1) == 1;
        }
        BlockEntityData::Bed => {
            let color = DyeColor::by_id(tag.get_i8_or("color", 0) as i32).unwrap_or(DyeColor::WHITE);
            extra = Extra::Bed { color };
        }
        BlockEntityData::Lectern { book, page } => {
            *page = tag.get_i32_or("page", 0).wrapping_mul(2);
            *book = tag.get_compound("book").and_then(|b| items.read(b)).map(Box::new);
        }
        BlockEntityData::Campfire(c) => {
            for i in 0..4 {
                if let Some(item) = tag.get_compound(&format!("Item{}", i + 1)) {
                    c.items[i] = items.read(item);
                }
                c.cooking_times[i] = tag.get_i32_or(&format!("ItemTime{}", i + 1), 0);
            }
        }
        BlockEntityData::ChiseledBookshelf(books) => read_slots(items, tag, &mut books[..]),
        BlockEntityData::Shelf(shelf) => read_slots(items, tag, &mut shelf[..]),
        BlockEntityData::Brushable(b) => {
            b.brush_direction = tag.get_i8_or("brush_direction", 0);
            b.brush_count = tag.get_i32_or("brush_count", 0);
            b.item = tag.get_compound("item").and_then(|i| items.read(i));
            b.loot_table = loot_table(items, tag);
        }
        BlockEntityData::DecoratedPot(pot) => read_decorated_pot(items, tag, pot),
        BlockEntityData::Crafter(c) => {
            c.disabled_slots = tag.get_i16_or("disabled_slots", 0);
            c.crafting_ticks_remaining = tag.get_i32_or("crafting_ticks_remaining", 0);
        }
        BlockEntityData::EndGateway(g) => {
            g.age = tag.get_i32_or("Age", 0) as i64;
            if let Some(Tag::List(exit)) = tag.get("ExitPortal")
                && let [Tag::Int(x), Tag::Int(y), Tag::Int(z)] = exit.as_slice()
            {
                (g.exit_x, g.exit_y, g.exit_z) = (*x, *y, *z);
            }
        }
        BlockEntityData::CopperGolemStatue => {
            let pose = match tag.get_i32_or("Pose", 0) {
                1 => CopperGolemPose::SITTING,
                2 => CopperGolemPose::RUNNING,
                3 => CopperGolemPose::STAR,
                _ => CopperGolemPose::STANDING,
            };
            extra = Extra::CopperGolem { pose };
        }
        _ => {}
    }
    Some(Read::Entity { entity, extra })
}

fn read_effect(items: &BedrockItems, id: i32) -> crate::convert::model::EffectType {
    items.tables.effect(id).unwrap_or_else(|| {
        items.missing(Missing::Effect, &id.to_string());
        crate::convert::model::EffectType::EMPTY
    })
}

/// The `Items` list of a container (`BedrockContainerBlockEntityHandler`); air and unreadable items are left out.
fn read_container(items: &BedrockItems, tag: &Compound, container: &mut Container, brewing: bool) {
    let Some(list) = tag.get_list("Items") else { return };
    let mut index: i8 = 0;
    for entry in list {
        if let Tag::Compound(item_tag) = entry {
            let slot = remap_slot(brewing, true, item_tag.get_i8_or("Slot", index));
            if let Some(item) = items.read(item_tag)
                && !item.is_air()
            {
                container.items.insert(slot, item);
            }
        }
        index = index.wrapping_add(1);
    }
}

/// Brewing stands are ordered bottles first, then the ingredient and the fuel, in Bedrock.
pub(super) fn remap_slot(brewing: bool, reading: bool, slot: i8) -> i8 {
    if !brewing {
        return slot;
    }
    match (reading, slot) {
        (true, 0) => 3,
        (true, 1) => 0,
        (true, 2) => 1,
        (true, 3) => 2,
        (false, 3) => 0,
        (false, 0) => 1,
        (false, 1) => 2,
        (false, 2) => 3,
        _ => slot,
    }
}

/// An ordered `Items` list into fixed slots.
fn read_slots(items: &BedrockItems, tag: &Compound, slots: &mut [Option<ItemStack>]) {
    let Some(list) = tag.get_list("Items") else { return };
    for (index, entry) in list.iter().enumerate() {
        if let (Tag::Compound(item_tag), Some(slot)) = (entry, slots.get_mut(index)) {
            *slot = items.read(item_tag);
        }
    }
}

/// `LootTable` as a Java loot table name.
fn loot_table(items: &BedrockItems, tag: &Compound) -> Option<String> {
    let table = tag.get_str("LootTable")?;
    (!table.is_empty() && items.loot_tables_enabled()).then(|| bedrock_loot_table_to_chunker(table))
}

/// `loot_tables/chests/x.json` to `minecraft:chests/x`, with the old names fixed.
pub fn bedrock_loot_table_to_chunker(loot_table: &str) -> String {
    let table = loot_table.replace("loot_tables/", "minecraft:").replace(".json", "");
    match table.as_str() {
        "minecraft:chests/buriedtreasure" => "minecraft:chests/buried_treasure".into(),
        "minecraft:chests/dispenser_trap" => "minecraft:chests/jungle_temple_dispenser".into(),
        "minecraft:chests/shipwreck" => "minecraft:chests/shipwreck_map".into(),
        "minecraft:chests/shipwrecksupply" => "minecraft:chests/shipwreck_supply".into(),
        "minecraft:chests/shipwrecktreasure" => "minecraft:chests/shipwreck_treasure".into(),
        _ => table,
    }
}

/// One side of a sign (`readSignFace`).
fn read_sign_face(input: &Compound, face: &mut SignFace, hanging: bool) {
    let empty = json_text::empty_text();
    if input.contains_key("Text") {
        let text = input.get_str("Text").unwrap_or_default();
        let lines = java_split_newlines(text);
        let lines = font::split(if hanging { HANGING_SIGN_MAX_WIDTH } else { SIGN_MAX_WIDTH }, 4, lines);
        face.lines = (0..4).map(|i| lines.get(i).map_or_else(|| empty.clone(), |l| json_text::from_text(l))).collect();
    } else {
        face.lines = (1..=4)
            .map(|i| input.get_str(&format!("Text{i}")).map_or_else(|| empty.clone(), json_text::from_text))
            .collect();
    }
    face.lit = input.get_i8_or("IgnoreLighting", 0) == 1;
    face.color = input.get_i32_or("SignTextColor", 0xff00_0000_u32 as i32);
}

/// Java's `String.split("\n")`: no match gives the input, otherwise trailing empty strings are removed.
fn java_split_newlines(text: &str) -> Vec<String> {
    if !text.contains('\n') {
        return vec![text.to_string()];
    }
    let mut parts: Vec<String> = text.split('\n').map(str::to_string).collect();
    while parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }
    parts
}

fn read_banner(items: &BedrockItems, tag: &Compound, banner: &mut crate::convert::model::blockentity::Banner) {
    banner.base = tag.get_i32("Base").and_then(DyeColor::by_reversed_id);
    let Some(list) = tag.get_list("Patterns") else { return };
    for entry in list {
        let Tag::Compound(pattern) = entry else { continue };
        let color = pattern.get_i32("Color").and_then(DyeColor::by_reversed_id);
        let kind = pattern.get_str("Pattern").and_then(|short| {
            let found = items.tables.banner_pattern(short);
            if found.is_none() {
                items.missing(Missing::BannerPattern, short);
            }
            found
        });
        if let (Some(color), Some(kind)) = (color, kind) {
            banner.patterns.push((color, kind));
        }
    }
}

fn read_structure(items: &BedrockItems, tag: &Compound, s: &mut crate::convert::model::blockentity::Structure) {
    let at = |game| items.version().at_least(game);
    s.name = tag.get_str("structureName").unwrap_or_default().to_string();
    s.metadata = tag.get_str("dataField").unwrap_or_default().to_string();
    s.pos_x = tag.get_i32_or("xStructureOffset", 0);
    s.pos_y = tag.get_i32_or("yStructureOffset", 0);
    s.pos_z = tag.get_i32_or("zStructureOffset", 0);
    s.size_x = tag.get_i32_or("xStructureSize", 0);
    s.size_y = tag.get_i32_or("yStructureSize", 0);
    s.size_z = tag.get_i32_or("zStructureSize", 0);
    s.mode = match tag.get_i32_or("data", 0) {
        1 => StructureBlockMode::SAVE,
        2 => StructureBlockMode::LOAD,
        3 => StructureBlockMode::CORNER,
        4 => StructureBlockMode::INVALID,
        5 => StructureBlockMode::EXPORT,
        _ => StructureBlockMode::DATA,
    };
    s.rotation = match tag.get_i8_or("rotation", 0) {
        1 => StructureRotation::Clockwise90,
        2 => StructureRotation::Clockwise180,
        3 => StructureRotation::CounterClockwise90,
        _ => StructureRotation::None,
    };
    s.mirror = match tag.get_i8_or("mirror", 0) {
        1 => StructureMirror::LeftRight,
        2 => StructureMirror::FrontBack,
        3 => StructureMirror::Both,
        _ => StructureMirror::None,
    };
    s.ignore_entities = tag.get_i8_or("ignoreEntities", 0) == 1;
    s.include_players = tag.get_i8_or("includePlayers", 0) == 1;
    s.remove_blocks = tag.get_i8_or("removeBlocks", 0) == 1;
    s.powered = tag.get_i8_or("isPowered", 0) == 1;
    s.show_bounding_box = tag.get_i8_or("showBoundingBox", 1) == 1;
    s.integrity = tag.get_f32_or("integrity", 100.0) / 100.0;
    s.seed = tag.get_i64_or("seed", 0);
    if at([1, 13, 0]) {
        s.redstone_save_mode = tag.get_i32_or("redstoneSaveMode", 0);
    }
    if at([1, 21, 20]) {
        s.last_touched_player_id = tag.get_i64_or("lastTouchedPlayerId", 0);
    }
    if at([1, 16, 230]) {
        s.animation_mode = tag.get_i8_or("animationMode", 0);
        s.animation_seconds = tag.get_f32_or("animationSeconds", 0.0);
    }
}

fn read_decorated_pot(
    items: &BedrockItems,
    tag: &Compound,
    pot: &mut crate::convert::model::blockentity::DecoratedPot,
) {
    let key = if tag.contains_key("sherds") { "sherds" } else { "shards" };
    let sherds: Option<Vec<&str>> = tag
        .get_list(key)
        .filter(|l| l.iter().all(|t| matches!(t, Tag::String(_))))
        .map(|l| l.iter().filter_map(Tag::as_str).collect());
    if let Some(sherds) = sherds.filter(|s| !s.is_empty()) {
        let ids: Vec<ItemId> = sherds
            .iter()
            .map(|name| {
                items
                    .read_identifier(name, &Default::default())
                    .map(|s| s.id)
                    .unwrap_or_else(|| ItemId::Block(crate::convert::model::air().clone()))
            })
            .collect();
        pot.back = ids[0].clone();
        if ids.len() == 1 {
            return;
        }
        pot.left = ids[1].clone();
        if ids.len() == 2 {
            return;
        }
        pot.right = ids[2].clone();
        if ids.len() == 3 {
            return;
        }
        pot.front = ids[3].clone();
    }
    pot.item = tag.get_compound("item").and_then(|i| items.read(i));
}
