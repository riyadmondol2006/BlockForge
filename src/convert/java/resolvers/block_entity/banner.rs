// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Banners (`JavaBannerBlockEntityHandler`) and beacons (`JavaBeaconBlockEntityHandler`).

use super::ctx::Ctx;
use super::tables;
use crate::convert::model::blockentity::{Banner, Beacon};
use crate::convert::model::{BlockEntity, EffectType};
use crate::convert::nbt::{Compound, Tag};

pub(super) fn write_banner(ctx: &Ctx, out: &mut Compound, be: &BlockEntity, banner: &Banner) -> Option<()> {
    if let Some(base) = banner.base {
        out.put("Base", base.id() as i32);
    }
    let modern = ctx.since(1, 20, 5);
    let mut patterns = Vec::with_capacity(banner.patterns.len());
    for (color, pattern) in &banner.patterns {
        let mut entry = Compound::new();
        if modern {
            let Some(id) = tables::banner_pattern_id(*pattern, ctx.version) else { continue };
            entry.put("color", color.lower_name());
            entry.put("pattern", id);
        } else {
            let Some(short) = tables::banner_short_name(*pattern) else { continue };
            entry.put("Color", color.id() as i32);
            entry.put("Pattern", short);
        }
        patterns.push(Tag::Compound(entry));
    }
    out.put(if modern { "patterns" } else { "Patterns" }, Tag::List(patterns));
    if let Some(name) = &be.custom_name {
        out.put("CustomName", ctx.text(name)?);
    }
    Some(())
}

pub(super) fn write_beacon(ctx: &Ctx, out: &mut Compound, be: &BlockEntity, beacon: &Beacon) -> Option<()> {
    let modern = ctx.since(1, 20, 2);
    for (effect, name, legacy) in [
        (beacon.primary_effect, "primary_effect", "Primary"),
        (beacon.secondary_effect, "secondary_effect", "Secondary"),
    ] {
        if effect == EffectType::EMPTY {
            continue;
        }
        if modern {
            out.put(name, tables::effect_name(effect, ctx.version));
        } else {
            out.put(legacy, tables::effect_id(effect, ctx.version));
        }
    }
    if let Some(custom) = &be.custom_name {
        out.put("CustomName", ctx.text(custom)?);
    }
    Some(())
}
