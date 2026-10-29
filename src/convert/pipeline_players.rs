//! The players of a conversion ([`ConversionSettings::player`]): whose inventory the owner gets, what of the host
//! is kept, the items as model stacks for the writer and the other players' profiles.

use std::collections::BTreeMap;

use crate::convert::bedrock::reader::{BedrockWorldReader, PlayerProfile};
use crate::convert::java::writer::GuestPlayer;
use crate::convert::level::player::PlayerItem;
use crate::convert::level::{ConversionSettings, ConvertError, InventorySource, Level, LevelPlayer, PlayerStats};
use crate::convert::model::ItemStack;

/// What the writer gets for the players.
#[derive(Default)]
pub(super) struct Players {
    /// The owner's inventory and ender chest (the level's player).
    pub items: BTreeMap<i8, ItemStack>,
    pub ender_items: BTreeMap<i8, ItemStack>,
    pub guests: Vec<GuestPlayer>,
    pub warnings: Vec<String>,
}

/// Apply the player options to the level's player and convert the items of every player that is written.
pub(super) fn prepare(
    reader: &BedrockWorldReader,
    level: &mut Level,
    settings: &ConversionSettings,
) -> Result<Players, ConvertError> {
    let options = &settings.player;
    let mut out = Players::default();
    let wants_profiles = options.export_guest_profiles || options.inventory_source != InventorySource::Host;
    let profiles = if wants_profiles { reader.player_profiles()? } else { Vec::new() };

    if let Some(id) = options.inventory_source.profile_id() {
        match profiles.iter().find(|p| p.id.eq_ignore_ascii_case(id)) {
            Some(profile) => {
                let left_out = take_inventory(level, profile, &options.inventory_source);
                if left_out > 0 {
                    out.warnings.push(format!(
                        "{left_out} item stacks of player profile {id} did not fit in the inventory and were left out"
                    ));
                }
            }
            None => out.warnings.push(format!(
                "The player profile {id} chosen for the inventory is not in this world; the host's inventory is kept"
            )),
        }
    }

    if let Some(player) = &mut level.player {
        keep_only_chosen(player, settings);
        out.items = reader.read_items(&player.inventory);
        out.ender_items = reader.read_items(&player.ender_items);
    }

    if options.export_guest_profiles {
        for profile in &profiles {
            let Some(uuid) = options.guest_uuid(&profile.id) else {
                out.warnings.push(format!("Player profile {} has no uuid to name its file after; skipped", profile.id));
                continue;
            };
            if Some(uuid) == options.owner_uuid {
                continue;
            }
            let mut player = profile.player.clone();
            keep_only_chosen(&mut player, settings);
            out.guests.push(GuestPlayer {
                uuid,
                items: reader.read_items(&player.inventory),
                ender_items: reader.read_items(&player.ender_items),
                player,
            });
        }
    }
    Ok(out)
}

/// Clear what the options leave out: the ender chest and the stats.
fn keep_only_chosen(player: &mut LevelPlayer, settings: &ConversionSettings) {
    if !settings.player.include_ender_chest {
        player.ender_items.clear();
    }
    if !settings.player.include_stats {
        player.stats = PlayerStats::default();
    }
}

/// Give the level's player the inventory of `profile` the way `source` says; a world without a host player gets the
/// whole profile. Returns how many stacks did not fit (merging only).
fn take_inventory(level: &mut Level, profile: &PlayerProfile, source: &InventorySource) -> usize {
    let Some(host) = &mut level.player else {
        level.player = Some(profile.player.clone());
        return 0;
    };
    match source {
        InventorySource::Host => 0,
        InventorySource::Profile(_) => {
            host.inventory = profile.player.inventory.clone();
            host.ender_items = profile.player.ender_items.clone();
            host.stats.xp_level = profile.player.stats.xp_level;
            host.stats.xp_progress = profile.player.stats.xp_progress;
            0
        }
        InventorySource::MergeHostAnd(_) => merge_inventory(&mut host.inventory, &profile.player.inventory),
    }
}

/// Fill the empty slots of `host` from `other`: each stack in its own slot when that is free, else (main inventory
/// stacks only) in the first free slot of the 36. Returns how many stacks found no place.
fn merge_inventory(host: &mut BTreeMap<i8, PlayerItem>, other: &BTreeMap<i8, PlayerItem>) -> usize {
    const MAIN: std::ops::Range<i8> = 0..36;
    let mut moved = Vec::new();
    for (slot, item) in other {
        if host.contains_key(slot) {
            moved.push((*slot, item));
        } else {
            host.insert(*slot, item.clone());
        }
    }
    let mut left_out = 0;
    for (slot, item) in moved {
        match MAIN.clone().find(|s| !host.contains_key(s)).filter(|_| MAIN.contains(&slot)) {
            Some(free) => {
                host.insert(free, item.clone());
            }
            None => left_out += 1,
        }
    }
    left_out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::nbt::{Compound, Tag};

    fn item(name: &str) -> PlayerItem {
        Compound::new().with("Name", name).with("Count", Tag::Byte(1))
    }

    #[test]
    fn merging_fills_free_slots_first() {
        let mut host = BTreeMap::from([(0i8, item("a")), (1, item("b")), (103, item("helmet"))]);
        let other = BTreeMap::from([(0i8, item("c")), (5, item("d")), (103, item("cap")), (-106, item("shield"))]);
        assert_eq!(merge_inventory(&mut host, &other), 1);
        let names: Vec<(i8, &str)> = host.iter().map(|(s, i)| (*s, i.get_str("Name").unwrap())).collect();
        // c moves to the first free slot (2), d keeps 5, the shield takes the free off hand, the cap has no place.
        assert_eq!(names, [(-106, "shield"), (0, "a"), (1, "b"), (2, "c"), (5, "d"), (103, "helmet")]);
    }
}
