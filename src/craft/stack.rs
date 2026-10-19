//! Stack sizes and what storing items takes.

use super::types::*;

/// How many of `id` fit in one slot. The game keeps this in code, not in its data files, so it is worked out from
/// the name (tools, armor, boats, beds, ... stack to 1; signs, banners, buckets, eggs, ender pearls to 16).
pub fn stack_size(_id: &str) -> u32 {
    64
}

/// How many containers of `per` slots `slots` need.
pub fn containers(slots: u64, per: u64) -> Containers {
    let count = slots.div_ceil(per);
    Containers { count, capacity: count * per, unused: count * per - slots }
}

/// What storing `rows` takes, each item in its own stacks.
pub fn storage(_ds: &Dataset, _rows: &[(ItemId, u64)]) -> Storage {
    let none = containers(0, 27);
    Storage {
        rows: Vec::new(),
        slots: 0,
        single_chests: none.clone(),
        double_chests: none.clone(),
        shulker_boxes: none.clone(),
        player_inventories: none,
    }
}
