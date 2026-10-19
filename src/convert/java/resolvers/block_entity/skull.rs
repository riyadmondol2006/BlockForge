// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Skulls (`JavaSkullBlockEntityHandler`): the profile of a player head.

use super::ctx::Ctx;
use crate::convert::model::blockentity::Skull;
use crate::convert::nbt::{Compound, Tag};

/// A UUID string as the four ints Minecraft stores (`None` when it is not a UUID).
pub(super) fn uuid_ints(text: &str) -> Option<Vec<i32>> {
    let hex: String = text.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 || !hex.is_ascii() {
        return None;
    }
    (0..4).map(|i| u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16).ok().map(|v| v as i32)).collect()
}

/// The profile compound of a skull: `id`, `name` and the texture property (names differ before 1.20.5).
pub(super) fn profile(ctx: &Ctx, skull: &Skull) -> Compound {
    let modern = ctx.since(1, 20, 5);
    let key = |modern_name: &'static str, old: &'static str| if modern { modern_name } else { old };
    let mut owner = Compound::new();
    if let Some(id) = &skull.owner_id {
        if ctx.since(1, 16, 0) {
            if let Some(ints) = uuid_ints(id) {
                owner.put(key("id", "Id"), ints);
            }
        } else {
            owner.put("Id", id.as_str());
        }
    }
    if let Some(name) = &skull.owner_name {
        owner.put(key("name", "Name"), name.as_str());
    }
    if skull.texture.is_some() || skull.texture_signature.is_some() {
        let mut entry = Compound::new();
        if modern {
            entry.put("name", "textures");
        }
        if let Some(texture) = &skull.texture {
            entry.put(key("value", "Value"), texture.as_str());
        }
        if let Some(signature) = &skull.texture_signature {
            entry.put(key("signature", "Signature"), signature.as_str());
        }
        let textures = Tag::List(vec![Tag::Compound(entry)]);
        if modern {
            owner.put("properties", textures);
        } else {
            owner.put("Properties", Compound::new().with("textures", textures));
        }
    }
    owner
}

pub(super) fn write(ctx: &Ctx, out: &mut Compound, skull: &Skull) {
    if skull.owner_id.is_none()
        && skull.texture.is_none()
        && skull.texture_signature.is_none()
        && skull.owner_name.is_none()
    {
        return;
    }
    let owner = profile(ctx, skull);
    if ctx.since(1, 20, 5) {
        out.put("profile", owner);
    } else {
        out.put(if ctx.since(1, 16, 0) { "SkullOwner" } else { "Owner" }, owner);
    }
}
