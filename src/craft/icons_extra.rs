//! Icons that are not one plain texture: spawn eggs of jars that tint them in code, potions, and the few
//! blocks (chests, heads) that only have an entity texture, of which a front view is cut.

use super::{IconImage, tex_path};

/// The jar path of the spawn egg base texture that older jars share between all eggs.
pub const SPAWN_EGG: &str = "assets/minecraft/textures/item/spawn_egg.png";
/// The overlay (spots) laid over the tinted base of a spawn egg.
pub const SPAWN_EGG_OVERLAY: &str = "assets/minecraft/textures/item/spawn_egg_overlay.png";
/// The colour of the liquid of a potion without effects.
pub const POTION_TINT: u32 = 0x385DC6;

/// The (base, spots) colours of a spawn egg; mobs not listed get colours derived from their name.
pub fn egg_colors(mob: &str) -> (u32, u32) {
    match mob {
        "allay" => (0x00B1D1, 0x00F7FF),
        "armadillo" => (0xAD716D, 0x824848),
        "axolotl" => (0xFBC1E3, 0xA91D1D),
        "bat" => (0x4C3E30, 0x0F0F0F),
        "bee" => (0xEDC343, 0x43241B),
        "blaze" => (0xF6B201, 0xFFF87E),
        "bogged" => (0x8A9C72, 0x314D1C),
        "breeze" => (0xA3B3FF, 0x7F9BD0),
        "camel" => (0xFCC363, 0xA27D3F),
        "cat" => (0xEFC88E, 0x957C5E),
        "cave_spider" => (0x0C424E, 0xA80E0E),
        "chicken" => (0xA1A1A1, 0xFF0000),
        "cod" => (0xC1A25B, 0xE5B48E),
        "cow" => (0x443626, 0xA1A1A1),
        "creeper" => (0x0DA70B, 0x000000),
        "dolphin" => (0x223B4D, 0xF9F9F9),
        "donkey" => (0x534539, 0x867566),
        "drowned" => (0x8FF1D7, 0x799C65),
        "elder_guardian" => (0xCDCAAF, 0x747693),
        "ender_dragon" => (0x1C1C1C, 0xE079FA),
        "enderman" => (0x161616, 0x000000),
        "endermite" => (0x161616, 0x6E6E6E),
        "evoker" => (0x959B9B, 0x1E1C1A),
        "fox" => (0xD5B69F, 0xCC6920),
        "frog" => (0xD07644, 0xFFC77C),
        "ghast" => (0xF9F9F9, 0xBCBCBC),
        "glow_squid" => (0x095656, 0x83EDB4),
        "goat" => (0xA09B8B, 0x55493B),
        "guardian" => (0x5A8272, 0xF17D30),
        "hoglin" => (0xC66E55, 0x5F6464),
        "horse" => (0xC09E7D, 0xEEE500),
        "husk" => (0x797061, 0xE6CC94),
        "iron_golem" => (0xDBCDC6, 0x7A9E6A),
        "llama" => (0xC09E7D, 0x995F40),
        "magma_cube" => (0x340000, 0xFCFC00),
        "mooshroom" => (0xA00F10, 0xB7B7B7),
        "mule" => (0x1B0200, 0x51331D),
        "ocelot" => (0xEFDE7D, 0x564434),
        "panda" => (0xE7E7E7, 0x1B1C1B),
        "parrot" => (0x0DA70B, 0xFF0000),
        "phantom" => (0x43518D, 0x88FF00),
        "pig" => (0xF0A5A2, 0xDB635F),
        "piglin" => (0x995F40, 0xF5C98F),
        "piglin_brute" => (0x592A10, 0xE5B96E),
        "pillager" => (0x532F36, 0x959B9B),
        "polar_bear" => (0xF2F2F2, 0x959B9B),
        "pufferfish" => (0xF6B201, 0x37C3F2),
        "rabbit" => (0x995F40, 0x734831),
        "ravager" => (0x757470, 0x5B5049),
        "salmon" => (0xA00F10, 0x0E8472),
        "sheep" => (0xE7E7E7, 0xFFB5B5),
        "shulker" => (0x946794, 0x4D3852),
        "silverfish" => (0x6E6E6E, 0x303030),
        "skeleton" => (0xC1C1C1, 0x494949),
        "skeleton_horse" => (0x68684F, 0xD1D1C9),
        "slime" => (0x51A03E, 0x7EBF6E),
        "sniffer" => (0x87C627, 0x9B3E3E),
        "snow_golem" => (0xD9F2F2, 0xF29E1F),
        "spider" => (0x342D27, 0xA80E0E),
        "squid" => (0x223B4D, 0x708899),
        "stray" => (0x617677, 0xDDEAEA),
        "strider" => (0x9C3436, 0x4D494D),
        "tadpole" => (0x6D533C, 0x160A00),
        "trader_llama" => (0xEDC76E, 0x456296),
        "tropical_fish" => (0xEF6915, 0xFFF9EF),
        "turtle" => (0xE7E7E7, 0x00FF00),
        "vex" => (0x7A90A4, 0xE8EDF1),
        "villager" => (0x563C33, 0xBD8B72),
        "vindicator" => (0x959B9B, 0x275E61),
        "wandering_trader" => (0x456296, 0xE3A055),
        "warden" => (0x0F4649, 0x1CD785),
        "witch" => (0x340000, 0x51A03E),
        "wither" => (0x141414, 0x4F4F4F),
        "wither_skeleton" => (0x141414, 0x474D4D),
        "wolf" => (0xD7D3D3, 0xCEAF96),
        "zoglin" => (0xC66E55, 0xE6E6E6),
        "zombie" => (0x00AFAF, 0x799C65),
        "zombie_horse" => (0x3E5F3E, 0xA8E9B3),
        "zombie_villager" => (0x563C33, 0x799C65),
        "zombified_piglin" => (0xEA9393, 0x4C7129),
        _ => {
            // A mob added after this table: stable colours from its name.
            let h = mob.bytes().fold(0x811C_9DC5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
            (h & 0x00FF_FFFF | 0x0040_4040, h.rotate_left(13) & 0x00FF_FFFF | 0x0040_4040)
        }
    }
}

/// An entity texture that stands in for an item that has no flat texture: (id, texture under `textures/`).
pub fn entity_stand_in(name: &str) -> &'static [&'static str] {
    match name {
        "chest" => &["entity/chest/normal"],
        "trapped_chest" => &["entity/chest/trapped"],
        "ender_chest" => &["entity/chest/ender"],
        "decorated_pot" => &["entity/decorated_pot/decorated_pot_side"],
        "skeleton_skull" => &["entity/skeleton/skeleton"],
        "wither_skeleton_skull" => &["entity/skeleton/wither_skeleton"],
        "creeper_head" => &["entity/creeper/creeper"],
        "zombie_head" => &["entity/zombie/zombie"],
        "player_head" => &["entity/player/wide/steve", "entity/steve"],
        _ => &[],
    }
}

/// Where the stand-in textures of `entity_stand_in` live in a jar.
pub fn entity_path(rel: &str) -> String {
    format!("assets/minecraft/textures/{rel}.png")
}

/// The flat icon cut from an entity texture (`rgba`, `w` x `h`): a chest from its front, a head from its face.
pub fn entity_icon(path: &str, rgba: &[u8], w: usize, h: usize) -> Option<IconImage> {
    let rel = path.strip_prefix("assets/minecraft/textures/")?.strip_suffix(".png")?;
    let get = |x: usize, y: usize| -> [u8; 4] {
        let i = (y * w + x) * 4;
        rgba.get(i..i + 4).map_or([0; 4], |p| [p[0], p[1], p[2], p[3]])
    };
    let scale = (w / 64).max(1);
    if rel.starts_with("entity/chest/") {
        if w < 64 || h < 64 {
            return None;
        }
        let mut out = IconImage { width: 16, height: 16, rgba: vec![0; 16 * 16 * 4] };
        let mut put = |dx: usize, dy: usize, sx: usize, sy: usize, cw: usize, ch: usize| {
            for y in 0..ch {
                for x in 0..cw {
                    let p = get((sx + x) * scale, (sy + y) * scale);
                    let o = ((dy + y) * 16 + dx + x) * 4;
                    out.rgba[o..o + 4].copy_from_slice(&p);
                }
            }
        };
        put(1, 1, 14, 14, 14, 5); // lid front
        put(1, 6, 14, 33, 14, 10); // body front
        put(7, 3, 1, 1, 2, 4); // latch
        return Some(out);
    }
    if rel.contains("/skeleton/") || rel.contains("/creeper/") || rel.contains("/zombie/") || rel.contains("steve") {
        if w < 64 {
            return None;
        }
        // The face of the head is the 8x8 square at (8, 8); shown twice as big.
        let mut out = IconImage { width: 16, height: 16, rgba: vec![0; 16 * 16 * 4] };
        for y in 0..16 {
            for x in 0..16 {
                let p = get((8 + x / 2) * scale, (8 + y / 2) * scale);
                out.rgba[(y * 16 + x) * 4..][..4].copy_from_slice(&p);
            }
        }
        return Some(out);
    }
    None
}

/// Multiplies the RGB of every pixel by `tint` (0xRRGGBB), leaving the alpha alone.
pub fn apply_tint(rgba: &mut [u8], tint: u32) {
    let t = [(tint >> 16) & 255, (tint >> 8) & 255, tint & 255];
    for px in rgba.chunks_exact_mut(4) {
        for (c, m) in px.iter_mut().zip(t) {
            *c = (u32::from(*c) * m / 255) as u8;
        }
    }
}

/// Lays `top` over `bottom` (same size, straight alpha).
pub fn over(bottom: &mut [u8], top: &[u8]) {
    for (b, t) in bottom.chunks_exact_mut(4).zip(top.chunks_exact(4)) {
        let ta = u32::from(t[3]);
        if ta == 0 {
            continue;
        }
        let ba = u32::from(b[3]);
        let oa = ta + ba * (255 - ta) / 255;
        for c in 0..3 {
            let v = u32::from(t[c]) * ta + u32::from(b[c]) * ba * (255 - ta) / 255;
            b[c] = (v / oa.max(1)) as u8;
        }
        b[3] = oa as u8;
    }
}

/// The path of the shared overlay texture of potions.
pub fn potion_overlay() -> String {
    tex_path("item", "potion_overlay")
}
