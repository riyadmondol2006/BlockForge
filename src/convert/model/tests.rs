// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Tests over the generated tables and the identifier, state and item types.

use std::collections::HashSet;
use std::sync::Arc;

use super::block::{BlockIdentifier, BlockKind, PreservedIdentifier};
use super::blockentity::{BlockEntityData, BlockEntityKind};
use super::generated::*;
use super::identifier::{Identifier, StateValue};
use super::item::{ItemId, ItemProps, ItemStack, ItemStackType};
use super::state::{StateEnum, StateSet};

#[test]
fn counts_match_the_java_enums() {
    assert_eq!(BlockType::COUNT, 1292);
    assert_eq!(StateId::COUNT, 133);
    assert_eq!(STATE_DEFS.len(), 133);
    assert_eq!(BLOCK_INFO.len(), BlockType::COUNT);
    assert_eq!(ItemType::COUNT, 471);
    assert_eq!(ITEM_ENTITY.len(), ItemType::COUNT);
    assert_eq!(VanillaEntityType::COUNT, 166);
    assert_eq!(ENTITY_SPAWN_EGG.len(), 166);
    assert_eq!(VanillaBiome::COUNT, 94);
    assert_eq!(BIOME_INFO.len(), 94);
    assert_eq!(DyeColor::COUNT, 16);
    assert_eq!(BannerPattern::COUNT, 43);
    assert_eq!(EnchantmentType::COUNT, 43);
    assert_eq!(EffectType::COUNT, 41);
    assert_eq!(PotionType::COUNT, 49);
    assert_eq!(FireworkShape::COUNT, 5);
    assert_eq!(HornInstrument::COUNT, 8);
    assert_eq!(ExplorerMap::COUNT, 16);
    assert_eq!(TrimMaterial::COUNT, 11);
    assert_eq!(TrimPattern::COUNT, 18);
    assert_eq!(Motive::COUNT, 51);
    assert_eq!(block_groups_count(), 10);
}

fn block_groups_count() -> usize {
    let groups = [
        block_groups::DOORS,
        block_groups::FENCE_GATES,
        block_groups::BARS_AND_GLASS_PANES,
        block_groups::REDSTONE_CONNECTABLE,
        block_groups::SNOWY_BLOCKS,
        block_groups::SNOWY_GRASS_BLOCKS,
        block_groups::STAIRS,
        block_groups::WALLS,
        block_groups::WOODEN_FENCES,
        block_groups::DOUBLE_CHESTS,
    ];
    groups.iter().map(|g| g.0).collect::<HashSet<_>>().len()
}

#[test]
fn enum_name_round_trips() {
    fn check<T: Copy + PartialEq + std::fmt::Debug>(
        all: &[T],
        name: impl Fn(T) -> &'static str,
        from_name: impl Fn(&str) -> Option<T>,
        from_ordinal: impl Fn(usize) -> Option<T>,
        ordinal: impl Fn(T) -> usize,
    ) {
        let mut seen = HashSet::new();
        for (i, v) in all.iter().enumerate() {
            assert!(seen.insert(name(*v)), "duplicate name {}", name(*v));
            assert_eq!(from_name(name(*v)), Some(*v), "{v:?}");
            assert_eq!(from_ordinal(i), Some(*v));
            assert_eq!(ordinal(*v), i);
        }
        assert_eq!(from_name("no such constant"), None);
        assert_eq!(from_name(""), None);
        assert_eq!(from_ordinal(all.len()), None);
    }
    check(&BlockType::ALL, BlockType::name, BlockType::from_name, BlockType::from_ordinal, BlockType::ordinal);
    check(&StateId::ALL, StateId::name, StateId::from_name, StateId::from_ordinal, StateId::ordinal);
    check(&ItemType::ALL, ItemType::name, ItemType::from_name, ItemType::from_ordinal, ItemType::ordinal);
    check(
        &VanillaEntityType::ALL,
        VanillaEntityType::name,
        VanillaEntityType::from_name,
        VanillaEntityType::from_ordinal,
        VanillaEntityType::ordinal,
    );
    check(
        &VanillaBiome::ALL,
        VanillaBiome::name,
        VanillaBiome::from_name,
        VanillaBiome::from_ordinal,
        VanillaBiome::ordinal,
    );
    check(&DyeColor::ALL, DyeColor::name, DyeColor::from_name, DyeColor::from_ordinal, DyeColor::ordinal);
    check(&PotionType::ALL, PotionType::name, PotionType::from_name, PotionType::from_ordinal, PotionType::ordinal);
    check(&Motive::ALL, Motive::name, Motive::from_name, Motive::from_ordinal, Motive::ordinal);
}

#[test]
fn java_names_and_ordinals() {
    assert_eq!(BlockType::AIR.ordinal(), 0);
    assert_eq!(BlockType::STONE.ordinal(), 1);
    assert_eq!(BlockType::from_name("OAK_PLANKS"), Some(BlockType::OAK_PLANKS));
    assert_eq!(BlockType::OAK_PLANKS.name(), "OAK_PLANKS");
    assert_eq!(BlockType::from_name("oak_planks"), None);
    assert_eq!(StateId::WATERLOGGED.ordinal(), 0);
    assert_eq!(StateId::AGE_1.ordinal(), 1);
    assert_eq!(StateId::WEST.ordinal(), 132);
    assert_eq!(format!("{:?}", BlockType::STONE), "STONE");
    assert_eq!(Axis::ALL, [Axis::X, Axis::Y, Axis::Z]);
    assert_eq!(FacingDirection::ALL.map(FacingDirection::name), ["DOWN", "UP", "NORTH", "SOUTH", "WEST", "EAST"]);
    assert_eq!(FacingDirectionHorizontal::ALL.map(FacingDirectionHorizontal::name), ["NORTH", "EAST", "SOUTH", "WEST"]);
    assert_eq!(Bool::ALL, [Bool::FALSE, Bool::TRUE]);
    assert_eq!(Age_25::COUNT, 26);
    assert_eq!(Note::COUNT, 25);
    assert_eq!(NoteBlockInstrument::COUNT, 27);
    assert_eq!(Power::COUNT, 16);
    assert_eq!(Rotation::COUNT, 16);
    assert_eq!(Layers::NAMES[0], "_1");
    assert_eq!(Orientation::COUNT, 12);
    assert_eq!(RailShape::COUNT, 10);
}

#[test]
fn state_defs_are_consistent() {
    for id in StateId::ALL {
        let def = id.def();
        assert!(!def.name.is_empty());
        assert!(def.values.len() >= 2, "{}", id.name());
        assert!((def.default as usize) < def.values.len(), "{}", id.name());
        for (i, v) in def.values.iter().enumerate() {
            assert_eq!(id.value_ordinal(v), Some(i as u8));
            assert_eq!(id.value_name(i as u8), Some(*v));
        }
        assert_eq!(id.value_name(def.values.len() as u8), None);
    }
    // Names shared on purpose.
    assert_eq!(StateId::with_state_name("age").count(), 8);
    assert_eq!(StateId::with_state_name("facing").count(), 3);
    assert_eq!(StateId::with_state_name("level").count(), 4);
    assert_eq!(StateId::with_state_name("shape").count(), 3);
    assert_eq!(StateId::with_state_name("type").count(), 3);
    assert_eq!(StateId::with_state_name("mode").count(), 3);
    assert_eq!(StateId::with_state_name("axis").count(), 2);
    assert_eq!(StateId::with_state_name("distance").count(), 2);
    assert_eq!(StateId::with_state_name("east").count(), 3);
    // Spot checks of the default values and value lists.
    assert_eq!(states::FACING_HORIZONTAL.default_value(), FacingDirectionHorizontal::NORTH);
    assert_eq!(states::AXIS.default_value(), Axis::Y);
    assert_eq!(states::AXIS_HORIZONTAL.default_value(), AxisHorizontal::X);
    assert_eq!(states::WATERLOGGED.default_value(), Bool::FALSE);
    assert_eq!(states::HALF.default_value(), Half::BOTTOM);
    assert_eq!(states::CHEST_TYPE.default_value(), ChestType::SINGLE);
    assert_eq!(states::CAULDRON_LEVEL.default_value(), CauldronLevel::_1);
    assert_eq!(states::ATTACHMENT_TYPE.default_value(), AttachmentType::FLOOR);
    assert_eq!(StateId::AGE_25.value_count(), 26);
    assert_eq!(StateId::FACING_HORIZONTAL_DOWN.value_count(), 5);
    assert_eq!(StateId::WATERLOGGED.state_name(), "waterlogged");
    assert_eq!(StateId::FACING_ALL.state_name(), "facing");
}

#[test]
fn state_masks() {
    let mask = state_groups::STAIRS;
    assert_eq!(mask.len(), 3);
    assert!(
        mask.contains(StateId::STAIR_SHAPE)
            && mask.contains(StateId::HALF)
            && mask.contains(StateId::FACING_HORIZONTAL)
    );
    assert!(!mask.contains(StateId::WATERLOGGED));
    assert_eq!(mask.iter().collect::<Vec<_>>(), {
        let mut v = vec![StateId::FACING_HORIZONTAL, StateId::HALF, StateId::STAIR_SHAPE];
        v.sort();
        v
    });
    assert!(state_groups::BED.contains(StateId::OCCUPIED));
    assert_eq!(state_groups::CONNECTABLE_ALL.len(), 6);
    assert_eq!(state_groups::WALL.len(), 5);
}

#[test]
fn block_type_metadata() {
    assert_eq!(BlockType::AIR.rgb_color(), 0);
    assert!(!BlockType::AIR.is_all_faces_solid());
    assert!(BlockType::AIR.is_air());
    assert_eq!(BlockType::STONE.rgb_color(), 0x707070);
    assert!(BlockType::STONE.is_all_faces_solid());
    // Every block supports waterlogged.
    assert!(BlockType::STONE.supports(StateId::WATERLOGGED));
    assert_eq!(BlockType::STONE.states().len(), 1);
    assert!(BlockType::GRASS_BLOCK.supports(StateId::SNOWY));
    assert!(!BlockType::STONE.supports(StateId::SNOWY));
    assert_eq!(BlockType::GRASS_BLOCK.rgb_color(), 0x7fb238);
    assert!(BlockType::OAK_STAIRS.supports(StateId::STAIR_SHAPE));
    assert!(BlockType::OAK_STAIRS.in_group(block_groups::STAIRS));
    assert!(!BlockType::STONE.in_group(block_groups::STAIRS));
    assert!(BlockType::OAK_DOOR.in_group(block_groups::DOORS));
    assert!(BlockType::COBBLESTONE_WALL.in_group(block_groups::WALLS));
    // Every block's state list is sorted and starts from the default set.
    for ty in BlockType::ALL {
        assert!(ty.supports(StateId::WATERLOGGED), "{ty:?}");
        assert!((ty.info().set as usize) < STATE_SETS.len());
    }
    // Block entities.
    assert_eq!(BlockType::CHEST.block_entity(), Some(BlockEntityKind::Chest));
    assert_eq!(BlockType::TRAPPED_CHEST.block_entity(), Some(BlockEntityKind::TrappedChest));
    assert_eq!(BlockType::WHITE_BED.block_entity(), Some(BlockEntityKind::Bed));
    assert_eq!(BlockType::OAK_SIGN.block_entity(), Some(BlockEntityKind::Sign));
    assert_eq!(BlockType::OAK_HANGING_SIGN.block_entity(), Some(BlockEntityKind::HangingSign));
    assert_eq!(BlockType::SHULKER_BOX.block_entity(), Some(BlockEntityKind::ShulkerBox));
    assert_eq!(BlockType::STONE.block_entity(), None);
    // Counts of the block entity carrying types from the survey.
    let with_entity = BlockType::ALL.iter().filter(|t| t.block_entity().is_some()).count();
    assert_eq!(with_entity, 198);
    let banners = BlockType::ALL.iter().filter(|t| t.block_entity() == Some(BlockEntityKind::Banner)).count();
    assert_eq!(banners, 32);
}

#[test]
fn block_entity_counts_in_item_table() {
    let banners = ItemType::ALL.iter().filter(|i| i.block_entity() == Some(BlockEntityKind::Banner)).count();
    let signs = ItemType::ALL.iter().filter(|i| i.block_entity() == Some(BlockEntityKind::Sign)).count();
    let hanging = ItemType::ALL.iter().filter(|i| i.block_entity() == Some(BlockEntityKind::HangingSign)).count();
    assert_eq!((banners, signs, hanging), (17, 13, 13));
    assert_eq!(ItemType::from_name("BRICK"), Some(ItemType::BRICK));
}

#[test]
fn block_entity_kind_chain() {
    use BlockEntityKind::*;
    assert_eq!(BlockEntityKind::COUNT, 51);
    assert_eq!(Dropper.parent(), Some(Dispenser));
    assert_eq!(TrappedChest.parent(), Some(Chest));
    assert_eq!(BlastFurnace.parent(), Some(Furnace));
    assert_eq!(Smoker.parent(), Some(Furnace));
    assert_eq!(HangingSign.parent(), Some(Sign));
    assert_eq!(EndGateway.parent(), Some(EndPortal));
    assert_eq!(CalibratedSculkSensor.parent(), Some(SculkSensor));
    let with_parent: Vec<_> = BlockEntityKind::ALL.iter().filter(|k| k.parent().is_some()).collect();
    assert_eq!(with_parent.len(), 7);
    assert!(Dropper.is_a(Dispenser) && Dropper.is_a(Dropper) && !Dispenser.is_a(Dropper));
    assert!(TrappedChest.is_a(Chest) && !Chest.is_a(TrappedChest));
    // Parents never loop.
    for k in BlockEntityKind::ALL {
        let mut cur = k;
        for _ in 0..4 {
            match cur.parent() {
                Some(p) => cur = p,
                None => break,
            }
        }
        assert!(cur.parent().is_none(), "{k:?}");
    }
    // Containers: 6 plain + 7 randomizable.
    assert_eq!(BlockEntityKind::ALL.iter().filter(|k| k.is_container()).count(), 13);
    assert_eq!(BlockEntityKind::ALL.iter().filter(|k| k.is_randomizable()).count(), 7);
    assert!(Furnace.is_container() && !Furnace.is_randomizable());
    assert!(Crafter.is_randomizable());
}

#[test]
fn block_entity_data_kinds() {
    let mut containers = 0;
    for kind in BlockEntityKind::ALL {
        match BlockEntityData::default_for(kind) {
            Some(data) => {
                assert_eq!(data.kind(), kind);
                assert_eq!(data.container().is_some(), kind.is_container(), "{kind:?}");
                assert_eq!(data.loot_table().is_some(), kind.is_randomizable(), "{kind:?}");
                containers += data.container().is_some() as usize;
            }
            None => assert_eq!(kind, BlockEntityKind::Unknown),
        }
    }
    assert_eq!(containers, 13);
    let unknown = BlockEntityData::Unknown(Arc::from("minecraft:foo"));
    assert_eq!(unknown.kind(), BlockEntityKind::Unknown);
    let mut furnace = BlockEntityData::default_for(BlockEntityKind::Smoker).unwrap();
    furnace.container_mut().unwrap().items.insert(-1, ItemStack::new(ItemType::BRICK));
    furnace.container_mut().unwrap().items.insert(3, ItemStack::new(ItemType::BRICK));
    // Slots are ordered as signed bytes.
    assert_eq!(furnace.container().unwrap().items.keys().copied().collect::<Vec<_>>(), vec![-1, 3]);
}

#[test]
fn block_entity_defaults() {
    use super::blockentity::*;
    let sign = Sign::default();
    assert!(sign.allow_op_features && !sign.waxed);
    assert_eq!(sign.front.color as u32, 0xff00_0000);
    assert!(sign.front.lines.is_empty());
    assert_eq!(BrewingStand::default().fuel_total, 20);
    let jig = Jigsaw::default();
    assert_eq!(
        (jig.name.as_str(), jig.pool.as_str(), jig.final_state.as_str()),
        ("minecraft:empty", "minecraft:empty", "minecraft:air")
    );
    assert_eq!(jig.joint, Joint::Rollable);
    let st = Structure::default();
    assert!(st.show_bounding_box);
    assert_eq!(st.integrity, 1.0);
    assert_eq!(st.mode, StructureBlockMode::DATA);
    let pot = DecoratedPot::default();
    assert_eq!(pot.back, ItemId::Item(ItemType::BRICK));
    assert_eq!(Beacon::default().primary_effect, EffectType::EMPTY);
    let cauldron = Cauldron::default();
    assert_eq!((cauldron.potion_type, cauldron.potion_bottle_type), (PotionType::EMPTY, PotionBottleType::UNKNOWN));
    assert_eq!(Campfire::default().items.len(), 4);
    assert_eq!(BlockEntityData::default_for(BlockEntityKind::Shelf).map(|d| d.kind()), Some(BlockEntityKind::Shelf));
}

#[test]
fn block_entity_equality_ignores_original_nbt() {
    use super::blockentity::BlockEntity;
    use crate::convert::nbt::{Compound, Tag};
    let a = BlockEntity::new(1, 2, 3, BlockEntityData::Bed);
    let mut b = a.clone();
    let mut nbt = Compound::new();
    nbt.insert("x", Tag::Int(1));
    b.original_nbt = Some(nbt);
    assert_eq!(a, b);
    b.custom_name = Some(serde_json::json!("hi"));
    assert_ne!(a, b);
    let mut c = a.clone();
    c.y = 9;
    assert_ne!(a, c);
    assert_ne!(a, BlockEntity::new(1, 2, 3, BlockEntityData::Bell));
}

#[test]
fn state_set_is_order_independent() {
    let mut a = StateSet::new();
    a.set(states::FACING_HORIZONTAL, FacingDirectionHorizontal::SOUTH);
    a.set(states::WATERLOGGED, Bool::TRUE);
    a.set(states::HALF, Half::TOP);
    let mut b = StateSet::new();
    b.set(states::HALF, Half::TOP);
    b.set(states::WATERLOGGED, Bool::TRUE);
    b.set(states::FACING_HORIZONTAL, FacingDirectionHorizontal::SOUTH);
    assert_eq!(a, b);
    assert_eq!(
        a.vanilla().iter().map(|e| e.0).collect::<Vec<_>>(),
        vec![StateId::WATERLOGGED, StateId::FACING_HORIZONTAL, StateId::HALF]
    );
    assert_eq!(a.get(states::HALF), Some(Half::TOP));
    assert_eq!(a.get(states::OPEN), None);
    a.set(states::HALF, Half::BOTTOM);
    assert_ne!(a, b);
    assert_eq!(a.len(), 3);
    assert!(a.remove(StateId::HALF));
    assert!(!a.remove(StateId::HALF));
    assert_eq!(a.len(), 2);
    // Custom states keep sorted by name.
    a.set_custom("zeta", StateValue::Int(1));
    a.set_custom("alpha", StateValue::Bool(true));
    assert_eq!(a.custom()[0].0.as_ref(), "alpha");
    assert_eq!(a.get_custom("zeta"), Some(&StateValue::Int(1)));
    a.set_custom("zeta", StateValue::Int(2));
    assert_eq!(a.get_custom("zeta"), Some(&StateValue::Int(2)));
    assert!(a.remove_custom("alpha"));
    assert_eq!(a.custom().len(), 1);
}

#[test]
fn block_identifier_defaults_and_copies() {
    let stairs = BlockIdentifier::new(BlockType::OAK_STAIRS);
    // The default only comes back when the type supports the state.
    assert_eq!(stairs.get(states::HALF), Some(Half::BOTTOM));
    assert_eq!(stairs.get_present(states::HALF), None);
    assert_eq!(stairs.get(states::OPEN), None);
    assert_eq!(stairs.get(states::WATERLOGGED), Some(Bool::FALSE));
    assert!(!stairs.contains_state(StateId::HALF));
    let top = stairs.with(states::HALF, Half::TOP);
    assert_eq!(top.get_present(states::HALF), Some(Half::TOP));
    assert!(top.contains_state(StateId::HALF));
    assert_ne!(top, stairs);
    assert_eq!(top.without(states::HALF), stairs);
    assert_eq!(stairs.without(states::HALF), stairs);
    assert_eq!(top.get_with(states::HALF, false), Some(Half::TOP));
    // Equal states set in a different order are equal and hash alike.
    let a = stairs.with(states::HALF, Half::TOP).with(states::FACING_HORIZONTAL, FacingDirectionHorizontal::WEST);
    let b = stairs.with(states::FACING_HORIZONTAL, FacingDirectionHorizontal::WEST).with(states::HALF, Half::TOP);
    assert_eq!(a, b);
    let set: HashSet<_> = [a.clone(), b].into_iter().collect();
    assert_eq!(set.len(), 1);
    assert!(a.state_string().contains("half=TOP"));
    assert_eq!(BlockIdentifier::new(BlockType::STONE).state_string(), "[]");
    assert!(!BlockIdentifier::new(BlockType::STONE).is_air());
    assert!(BlockIdentifier::new(BlockType::AIR).is_air());
    assert_eq!(*super::block::air().as_ref(), BlockIdentifier::new(BlockType::AIR));
    assert!(BlockIdentifier::new(BlockType::STONE).has_rgb_color());
    assert!(!BlockIdentifier::new(BlockType::AIR).has_rgb_color());
}

#[test]
fn preserved_identifier_makes_blocks_distinct() {
    let plain = BlockIdentifier::new(BlockType::STONE);
    let preserved = plain.with_preserved(Some(Arc::new(PreservedIdentifier {
        from_reader: true,
        identifier: Identifier::new("mod:block"),
    })));
    assert_ne!(plain, preserved);
    assert_eq!(preserved.preserved().unwrap().identifier.name.as_ref(), "mod:block");
    // The copy keeps the preserved identifier.
    assert!(preserved.with(states::WATERLOGGED, Bool::TRUE).preserved().is_some());
}

#[test]
fn custom_blocks() {
    let b = BlockIdentifier::custom(
        "mod:thing",
        [("waterlogged", StateValue::string("true")), ("power", StateValue::Int(3)), ("mode", StateValue::string("a"))],
    );
    assert_eq!(b.kind(), &BlockKind::Custom(Arc::from("mod:thing")));
    assert_eq!(b.block_type(), None);
    assert_eq!(b.get_present(states::WATERLOGGED), Some(Bool::TRUE));
    assert_eq!(b.states().custom().len(), 2);
    assert_eq!(b.states().get_custom("power"), Some(&StateValue::Int(3)));
    assert!(!b.has_rgb_color());
    assert!(!b.kind().is_all_faces_solid());
    assert!(b.kind().block_entity().is_none());
    // waterlogged accepts true, "true" and 1; anything else is false.
    for (v, expect) in [
        (StateValue::Bool(true), Bool::TRUE),
        (StateValue::Int(1), Bool::TRUE),
        (StateValue::string("true"), Bool::TRUE),
        (StateValue::Bool(false), Bool::FALSE),
        (StateValue::Int(2), Bool::FALSE),
        (StateValue::string("yes"), Bool::FALSE),
    ] {
        let c = BlockIdentifier::custom("mod:x", [("waterlogged", v)]);
        assert_eq!(c.get_present(states::WATERLOGGED), Some(expect));
    }
    // A custom type only supports waterlogged, so a vanilla state has no default.
    assert_eq!(b.get(states::HALF), None);
    assert_eq!(
        BlockIdentifier::custom("mod:x", Vec::<(&str, StateValue)>::new()).get(states::WATERLOGGED),
        Some(Bool::FALSE)
    );
    // Custom block types are equal by id; the states are part of the identifier.
    let one = BlockIdentifier::custom("mod:x", [("a", StateValue::Int(1))]);
    let two = BlockIdentifier::custom("mod:x", [("a", StateValue::Int(2))]);
    assert_ne!(one, two);
    assert_eq!(one.kind(), two.kind());
}

#[test]
fn dye_colors() {
    assert_eq!(DyeColor::WHITE.id(), 0);
    assert_eq!(DyeColor::BLACK.id(), 15);
    assert_eq!(DyeColor::WHITE.reversed_id(), 15);
    assert_eq!(DyeColor::by_id(3), Some(DyeColor::LIGHT_BLUE));
    assert_eq!(DyeColor::by_reversed_id(0), Some(DyeColor::BLACK));
    assert_eq!(DyeColor::by_id(16), None);
    assert_eq!(DyeColor::by_id(-1), None);
    assert_eq!(DyeColor::by_lower_name("light_blue"), Some(DyeColor::LIGHT_BLUE));
    assert_eq!(DyeColor::LIGHT_BLUE.lower_name(), "light_blue");
    assert_eq!(DyeColor::RED.java_rgb(), 0xB3312C);
    assert_eq!(DyeColor::RED.bedrock_rgb(), 0xB02E26);
    use super::color::Color;
    assert_eq!(DyeColor::find_closest(Color::from_rgb(0xB3312C)), DyeColor::RED);
    assert_eq!(DyeColor::find_closest(Color::from_rgb(0xB02E26)), DyeColor::RED);
    assert_eq!(DyeColor::find_closest(Color::from_rgb(0x000000)), DyeColor::BLACK);
    assert_eq!(DyeColor::find_closest(Color::from_rgb(0xffffff)), DyeColor::WHITE);
    // Every dye is its own closest colour.
    for d in DyeColor::ALL {
        assert_eq!(DyeColor::find_closest(Color::from_rgb(d.java_rgb())), d);
        assert_eq!(DyeColor::find_closest(Color::from_rgb(d.bedrock_rgb())), d);
    }
}

#[test]
fn small_item_enums() {
    assert_eq!(PotionBottleType::from_id(-1), PotionBottleType::UNKNOWN);
    assert_eq!(PotionBottleType::from_id(0), PotionBottleType::REGULAR);
    assert_eq!(PotionBottleType::from_id(1), PotionBottleType::SPLASH);
    assert_eq!(PotionBottleType::from_id(2), PotionBottleType::LINGERING);
    assert_eq!(PotionBottleType::from_id(3), PotionBottleType::UNKNOWN);
    assert_eq!(PotionBottleType::from_id(-5), PotionBottleType::UNKNOWN);
    assert_eq!(PotionBottleType::UNKNOWN.id(), -1);
    assert_eq!(PotionBottleType::LINGERING.id(), 2);
    assert_eq!(PotionType::LONG_MUNDANE.identifier(), "long_mundane");
    assert_eq!(PotionType::by_identifier("strong_healing"), Some(PotionType::STRONG_HEALING));
    assert_eq!(PotionType::by_identifier("nope"), None);
    assert_eq!(FireworkShape::by_lower_name("small_ball"), Some(FireworkShape::SMALL_BALL));
    assert_eq!(FireworkShape::BURST.id(), 4);
    assert_eq!(FireworkShape::by_id(5), None);
    assert_eq!(Motive::KEBAB.name(), "KEBAB");
    assert_eq!(Motive::SIZES[Motive::KEBAB as usize], (1, 1));
    assert_eq!(Motive::SIZES[Motive::UNPACKED as usize], (4, 4));
    assert_eq!(Motive::SIZES[Motive::WANDERER as usize], (1, 2));
    assert_eq!(Motive::SIZES[Motive::SEA as usize], (2, 1));
    assert!(VanillaEntityType::ALLAY.has_spawn_egg());
    assert!(!VanillaEntityType::ARROW.has_spawn_egg());
    assert_eq!(VanillaEntityType::from_name("ITEM_FRAME"), Some(VanillaEntityType::ITEM_FRAME));
}

#[test]
fn item_property_defaults() {
    let stack = ItemStack::new(ItemType::BRICK);
    assert!(stack.props.is_empty());
    assert_eq!(stack.props, ItemProps::default());
    assert_eq!(ItemProps::NAMES.len(), 25);
    let props = ItemProps::default();
    for name in ItemProps::NAMES {
        assert_eq!(props.has(name), Some(false), "{name}");
    }
    assert_eq!(props.has("nope"), None);
    let with = ItemStack::with_amount(BlockType::STONE, 64);
    assert_eq!(with.props.amount, Some(64));
    assert!(!with.props.is_empty());
    assert_ne!(with, ItemStack::new(BlockType::STONE));
    assert_eq!(with.props.has("amount"), Some(true));
    assert!(ItemStack::new(BlockType::AIR).is_air());
    assert!(!with.is_air());
    // A block item keeps its states.
    let oriented = ItemStack::new(BlockIdentifier::new(BlockType::OAK_STAIRS).with(states::HALF, Half::TOP));
    assert_ne!(oriented, ItemStack::new(BlockType::OAK_STAIRS));
    assert_eq!(ItemId::from(ItemType::BRICK).item_stack_type(), ItemStackType::Item(ItemType::BRICK));
    assert_eq!(ItemId::from(BlockType::CHEST).item_stack_type().block_entity(), Some(BlockEntityKind::Chest));
    assert_eq!(ItemId::from(ItemType::BRICK).item_stack_type().block_entity(), None);
    // Nested stacks compare structurally.
    let mut bundle = ItemStack::new(ItemType::BRICK);
    bundle.props.bundle_contents = Some(vec![with.clone()]);
    let mut other = bundle.clone();
    assert_eq!(bundle, other);
    other.props.bundle_contents.as_mut().unwrap()[0].props.amount = Some(1);
    assert_ne!(bundle, other);
}

#[test]
fn entity_types_and_equality() {
    use super::entity::*;
    let frame = Entity::new(EntityData::ItemFrame(ItemFrame::default()));
    assert_eq!(frame.entity_type(), EntityType::Vanilla(VanillaEntityType::ITEM_FRAME));
    let glow = Entity::new(EntityData::ItemFrame(ItemFrame { glow: true, ..ItemFrame::default() }));
    assert_eq!(glow.entity_type(), EntityType::Vanilla(VanillaEntityType::GLOW_ITEM_FRAME));
    assert_eq!(
        Entity::new(EntityData::Painting(Painting::default())).entity_type(),
        VanillaEntityType::PAINTING.into()
    );
    assert_eq!(
        Entity::new(EntityData::Cushion { color: DyeColor::WHITE }).entity_type(),
        VanillaEntityType::CUSHION.into()
    );
    assert_eq!(Painting::default().direction, FacingDirectionHorizontal::SOUTH);
    let custom = EntityType::Custom(Arc::from("mod:thing"));
    assert_eq!(custom, EntityType::Custom(Arc::from("mod:thing")));
    assert_eq!(Entity::unknown(custom.clone()).entity_type(), custom);
    // Floats compare like Double.compare: NaN equals NaN, -0.0 differs from 0.0.
    let mut a = Entity::unknown(VanillaEntityType::COW);
    let mut b = a.clone();
    a.position[0] = f64::NAN;
    b.position[0] = f64::NAN;
    assert_eq!(a, b);
    a.position[0] = 0.0;
    b.position[0] = -0.0;
    assert_ne!(a, b);
    // The original NBT does not take part.
    let mut c = a.clone();
    c.original_nbt = Some(crate::convert::nbt::Compound::new());
    assert_eq!(a, c);
}

#[test]
fn state_values_through_the_trait() {
    assert_eq!(<Half as StateEnum>::VALUES.len(), 2);
    assert_eq!(Half::from_value_ordinal(1), Some(Half::BOTTOM));
    assert_eq!(Half::from_value_ordinal(2), None);
    assert_eq!(Half::TOP.value_ordinal(), 0);
    assert_eq!(<Half as StateEnum>::VALUE_NAMES, &["TOP", "BOTTOM"]);
}
