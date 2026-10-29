//! The item hooks of a whole conversion: what an item needs from the rest of the world to be written in full.
//!
//! [`WorldItemHooks`] is made once by the [`JavaWorldWriter`](crate::convert::java::writer::JavaWorldWriter) (see
//! [`JavaWorldWriter::item_hooks`](crate::convert::java::writer::JavaWorldWriter::item_hooks)) and passed to every
//! item it writes: containers in chunks (through the block entity writer), the player's inventory, armor and ender
//! chest, the other players' profiles. Anything else that writes items (entities: mob equipment, item frames,
//! minecart chests ...) should take the same hooks and call [`JavaItems::write_with`] with them.
//!
//! - Maps: the model index of a map (`ItemProps::map_index`, set by the Bedrock reader from `map_uuid`) is the place
//!   of the map in the level's list, and the level writer numbers the map files from 0 in that order, so the Java id
//!   is the index itself (`minecraft:map_id` from 1.20.5, `tag.map` before).
//! - Block entity data of items (shulker box contents, banner patterns, player heads ...): the block entity writer,
//!   with these same hooks, so a map in a shulker box in a chest keeps its id too.
//! - Entity data of items (paintings): not handled yet, the default drops it.

use super::block_entity::write_block_entity_to_item_with;
use super::itemstack::{ItemHooks, JavaItems};
use crate::convert::java::format::JavaFormat;
use crate::convert::model::{BlockEntity, ItemStack};
use crate::convert::nbt::Compound;

/// The [`ItemHooks`] of one converted world. Cheap to clone; `Send` and `Sync`.
#[derive(Clone)]
pub struct WorldItemHooks {
    items: JavaItems,
    format: JavaFormat,
    /// How many maps the level writes (ids `0..maps`).
    maps: i32,
}

impl WorldItemHooks {
    /// Hooks for a world written with `items` in `format` that has `maps` maps.
    pub fn new(items: &JavaItems, format: &JavaFormat, maps: usize) -> WorldItemHooks {
        WorldItemHooks { items: items.clone(), format: format.clone(), maps: maps.min(i32::MAX as usize) as i32 }
    }

    /// The number of maps whose ids items may name.
    pub fn map_count(&self) -> i32 {
        self.maps
    }
}

impl ItemHooks for WorldItemHooks {
    fn block_entity(&self, _stack: &ItemStack, entity: &BlockEntity, item: &mut Compound) -> Option<Compound> {
        // Written straight into the item (components or `BlockEntityTag`), nothing left for the caller to add.
        write_block_entity_to_item_with(&self.items, &self.format, entity, item, self);
        None
    }

    fn map_id(&self, index: i32) -> Option<i32> {
        (0..self.maps).contains(&index).then_some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::model::{BlockEntityData, ItemType};
    use crate::convert::util::Version;

    #[test]
    fn maps_get_their_ids_and_shulker_boxes_their_contents() {
        for (version, components) in [(Version::new(26, 3, 0), true), (Version::new(1, 20, 4), false)] {
            let format = JavaFormat::for_version(version);
            let items = JavaItems::new(&format, format.version);
            let hooks = WorldItemHooks::new(&items, &format, 3);
            assert_eq!(
                (hooks.map_id(0), hooks.map_id(2), hooks.map_id(3), hooks.map_id(-1)),
                (Some(0), Some(2), None, None)
            );

            let mut map = ItemStack::new(ItemType::FILLED_MAP);
            map.props.map_index = Some(2);
            let mut contents = crate::convert::model::blockentity::RandomContainer::default();
            contents.container.items.insert(0, map.clone());
            let be = BlockEntity::new(0, 0, 0, BlockEntityData::ShulkerBox(contents));
            let shulker = ItemStack::new(ItemType::FILLED_MAP);
            let mut out = Compound::new().with("id", "minecraft:shulker_box");
            assert!(hooks.block_entity(&shulker, &be, &mut out).is_none());
            let inner = if components {
                let list = out.get_compound("components").unwrap().get_list("minecraft:container").unwrap();
                list[0].as_compound().unwrap().get_compound("item").unwrap().clone()
            } else {
                let tag = out.get_compound("tag").unwrap().get_compound("BlockEntityTag").unwrap();
                tag.get_list("Items").unwrap()[0].as_compound().unwrap().clone()
            };
            let id = if components {
                inner.get_compound("components").and_then(|c| c.get_integer("minecraft:map_id"))
            } else {
                inner.get_compound("tag").and_then(|c| c.get_integer("map"))
            };
            assert_eq!(id, Some(2), "{version}");
            // Without hooks the map loses its link.
            let plain = items.write(&map).unwrap();
            assert!(!plain.get_compound("components").is_some_and(|c| c.contains_key("minecraft:map_id")));
            assert!(!plain.get_compound("tag").is_some_and(|c| c.contains_key("map")));
        }
    }
}
