use std::path::PathBuf;
use std::time::Instant;

use super::*;
use crate::craft::source::MemSource;

fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(rgba));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn tex(src: &mut MemSource, path: &str, rgba: [u8; 4]) {
    src.0.insert(format!("assets/minecraft/textures/{path}.png"), png(16, 16, rgba));
}

fn json(src: &mut MemSource, path: &str, content: &str) {
    src.0.insert(format!("assets/minecraft/{path}.json"), content.as_bytes().to_vec());
}

fn icons(src: MemSource) -> Icons {
    Icons::new(Box::new(src))
}

#[test]
fn item_texture_wins() {
    let mut s = MemSource::default();
    tex(&mut s, "item/diamond_pickaxe", [1, 2, 3, 255]);
    json(
        &mut s,
        "models/item/diamond_pickaxe",
        r#"{"parent":"minecraft:item/handheld","textures":{"layer0":"minecraft:item/other"}}"#,
    );
    let mut i = icons(s);
    assert_eq!(
        i.texture_path("minecraft:diamond_pickaxe").unwrap(),
        "assets/minecraft/textures/item/diamond_pickaxe.png"
    );
    let img = i.image("minecraft:diamond_pickaxe").unwrap();
    assert_eq!((img.width, img.height), (16, 16));
    assert_eq!(&img.rgba[..4], &[1, 2, 3, 255]);
}

#[test]
fn model_parent_chain_with_refs() {
    let mut s = MemSource::default();
    tex(&mut s, "block/oak_planks", [200, 160, 90, 255]);
    json(&mut s, "models/item/oak_planks", r#"{"parent":"minecraft:block/oak_planks"}"#);
    json(
        &mut s,
        "models/block/oak_planks",
        r#"{"parent":"minecraft:block/cube_all","textures":{"all":"minecraft:block/oak_planks"}}"#,
    );
    json(
        &mut s,
        "models/block/cube_all",
        r##"{"parent":"block/cube","textures":{"particle":"#all","down":"#all","up":"#all"}}"##,
    );
    json(&mut s, "models/block/cube", r#"{"parent":"block/block","elements":[]}"#);
    let mut i = icons(s);
    assert_eq!(i.texture_path("minecraft:oak_planks").unwrap(), "assets/minecraft/textures/block/oak_planks.png");
    // Asking again is answered from memory.
    assert!(i.texture_path("minecraft:oak_planks").is_some());
    assert!(i.image("oak_planks").is_some());
}

#[test]
fn child_textures_win_and_faces_are_ordered() {
    let mut s = MemSource::default();
    for n in ["crafting_table_front", "crafting_table_side", "crafting_table_top", "oak_planks"] {
        tex(&mut s, &format!("block/{n}"), [9, 9, 9, 255]);
    }
    json(&mut s, "models/item/crafting_table", r#"{"parent":"minecraft:block/crafting_table"}"#);
    json(
        &mut s,
        "models/block/crafting_table",
        r#"{"parent":"minecraft:block/cube","textures":{"down":"minecraft:block/oak_planks","east":"minecraft:block/crafting_table_side","north":"minecraft:block/crafting_table_front","up":"minecraft:block/crafting_table_top"}}"#,
    );
    json(&mut s, "models/block/cube", r##"{"textures":{"north":"#wrong"}}"##);
    let mut i = icons(s);
    assert!(i.texture_path("crafting_table").unwrap().ends_with("crafting_table_front.png"));
}

#[test]
fn front_side_top_preference() {
    let mut s = MemSource::default();
    for n in ["furnace_front", "furnace_side", "furnace_top", "barrel_side", "barrel_top", "barrel_bottom"] {
        tex(&mut s, &format!("block/{n}"), [9, 9, 9, 255]);
    }
    json(&mut s, "models/item/furnace", r#"{"parent":"minecraft:block/furnace"}"#);
    json(
        &mut s,
        "models/block/furnace",
        r#"{"textures":{"front":"minecraft:block/furnace_front","side":"minecraft:block/furnace_side","top":"minecraft:block/furnace_top"}}"#,
    );
    json(
        &mut s,
        "models/item/barrel",
        r#"{"textures":{"bottom":"block/barrel_bottom","top":"block/barrel_top","side":"block/barrel_side"}}"#,
    );
    let mut i = icons(s);
    assert!(i.texture_path("furnace").unwrap().ends_with("furnace_front.png"));
    assert!(i.texture_path("barrel").unwrap().ends_with("barrel_side.png"));
}

#[test]
fn item_definition_models() {
    let mut s = MemSource::default();
    tex(&mut s, "block/stone", [120, 120, 120, 255]);
    tex(&mut s, "item/bow", [1, 1, 1, 255]);
    tex(&mut s, "block/hay_block_side", [5, 5, 5, 255]);
    json(&mut s, "items/stone", r#"{"model":{"type":"minecraft:model","model":"minecraft:block/stone"}}"#);
    json(
        &mut s,
        "models/block/stone",
        r#"{"parent":"minecraft:block/cube_all","textures":{"all":"minecraft:block/stone"}}"#,
    );
    json(&mut s, "models/block/cube_all", r##"{"textures":{"particle":"#all"}}"##);
    // The bow has its own texture, but a dispatch definition is read too.
    json(
        &mut s,
        "items/hay_block",
        r#"{"model":{"type":"minecraft:condition","on_true":{"type":"minecraft:model","model":"minecraft:item/pull"},"on_false":{"type":"minecraft:select","cases":[{"when":"x","model":{"type":"minecraft:model","model":"minecraft:block/hay"}}],"property":"p"}}}"#,
    );
    json(&mut s, "models/block/hay", r#"{"textures":{"side":"minecraft:block/hay_block_side"}}"#);
    let mut i = icons(s);
    assert!(i.texture_path("minecraft:stone").unwrap().ends_with("block/stone.png"));
    assert!(i.texture_path("minecraft:hay_block").unwrap().ends_with("hay_block_side.png"));
}

#[test]
fn definition_search_prefers_defaults() {
    let v: Value = serde_json::from_str(
        r#"{"model":{"type":"minecraft:special","base":"minecraft:item/chest","model":{"type":"minecraft:chest","texture":"minecraft:normal"}}}"#,
    )
    .unwrap();
    assert_eq!(definition_model(&v, 0).unwrap(), ("minecraft:item/chest".to_string(), true));
    let v: Value = serde_json::from_str(
        r#"{"model":{"type":"minecraft:range_dispatch","entries":[{"model":{"type":"minecraft:model","model":"a"},"threshold":1}],"fallback":{"type":"minecraft:model","model":"b"}}}"#,
    )
    .unwrap();
    assert_eq!(definition_model(&v, 0).unwrap(), ("b".to_string(), false));
    let v: Value = serde_json::from_str(r#"{"model":{"type":"minecraft:empty"}}"#).unwrap();
    assert!(definition_model(&v, 0).is_none());
}

#[test]
fn block_fallbacks() {
    let mut s = MemSource::default();
    tex(&mut s, "block/cactus_side", [0, 90, 0, 255]);
    tex(&mut s, "block/bookshelf_top", [90, 60, 0, 255]);
    tex(&mut s, "block/loom_front", [90, 90, 0, 255]);
    tex(&mut s, "block/loom_side", [1, 1, 1, 255]);
    let mut i = icons(s);
    assert!(i.texture_path("cactus").unwrap().ends_with("cactus_side.png"));
    assert!(i.texture_path("bookshelf").unwrap().ends_with("bookshelf_top.png"));
    assert!(i.texture_path("loom").unwrap().ends_with("loom_front.png"));
}

#[test]
fn builtin_entity_has_no_flat_texture() {
    let mut s = MemSource::default();
    tex(&mut s, "block/oak_planks", [200, 160, 90, 255]);
    json(&mut s, "models/item/chest", r#"{"parent":"builtin/entity","textures":{"particle":"block/oak_planks"}}"#);
    let mut i = icons(s);
    assert!(i.texture_path("minecraft:chest").is_none());
}

#[test]
fn animated_texture_keeps_first_frame() {
    let mut s = MemSource::default();
    let mut strip = image::RgbaImage::new(16, 64);
    for (_, y, px) in strip.enumerate_pixels_mut() {
        *px = image::Rgba([(y / 16) as u8 * 10 + 1, 0, 0, 255]);
    }
    let mut bytes = Vec::new();
    strip.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    s.0.insert("assets/minecraft/textures/block/magma.png".into(), bytes);
    s.0.insert("assets/minecraft/textures/item/wide.png".into(), png(32, 32, [7, 7, 7, 255]));
    let mut i = icons(s);
    let img = i.image("magma").unwrap();
    assert_eq!((img.width, img.height), (16, 16));
    assert_eq!(img.rgba.len(), 16 * 16 * 4);
    assert!(img.rgba.chunks(4).all(|p| p[0] == 1));
    let wide = i.image("wide").unwrap();
    assert_eq!((wide.width, wide.height), (32, 32));
}

#[test]
fn tint_multiplies_rgb() {
    let mut s = MemSource::default();
    tex(&mut s, "block/oak_leaves", [255, 255, 255, 200]);
    tex(&mut s, "block/cherry_leaves", [255, 255, 255, 200]);
    tex(&mut s, "block/grass_block_top", [128, 128, 128, 255]);
    tex(&mut s, "block/grass_block_side", [1, 1, 1, 255]);
    tex(&mut s, "block/dirt", [1, 1, 1, 255]);
    json(&mut s, "models/item/oak_leaves", r#"{"parent":"minecraft:block/oak_leaves"}"#);
    json(&mut s, "models/block/oak_leaves", r#"{"textures":{"all":"minecraft:block/oak_leaves"}}"#);
    json(&mut s, "models/item/cherry_leaves", r#"{"textures":{"all":"minecraft:block/cherry_leaves"}}"#);
    json(&mut s, "models/item/grass_block", r#"{"parent":"minecraft:block/grass_block"}"#);
    json(
        &mut s,
        "models/block/grass_block",
        r#"{"textures":{"particle":"block/dirt","bottom":"block/dirt","top":"block/grass_block_top","side":"block/grass_block_side","overlay":"block/grass_block_side_overlay"}}"#,
    );
    let mut i = icons(s);
    let leaves = i.image("minecraft:oak_leaves").unwrap();
    assert_eq!(&leaves.rgba[..4], &[0x77, 0xAB, 0x2F, 200]);
    let cherry = i.image("minecraft:cherry_leaves").unwrap();
    assert_eq!(&cherry.rgba[..4], &[255, 255, 255, 200]);
    let grass = i.image("minecraft:grass_block").unwrap();
    assert!(grass.rgba[1] > grass.rgba[0] && grass.rgba[1] > grass.rgba[2]);
    assert_eq!(grass.rgba[0] as u32, 128 * 0x91 / 255);
}

#[test]
fn missing_and_corrupt_give_none() {
    let mut s = MemSource::default();
    s.0.insert("assets/minecraft/textures/item/broken.png".into(), b"not a png".to_vec());
    json(&mut s, "models/item/badjson", "{ nope");
    json(&mut s, "items/badjson2", "[[[");
    json(&mut s, "models/item/loop", r#"{"parent":"item/loop"}"#);
    json(&mut s, "models/item/refloop", r##"{"textures":{"layer0":"#a","a":"#layer0"}}"##);
    let mut i = icons(s);
    for id in [
        "minecraft:broken",
        "minecraft:nothing",
        "minecraft:badjson",
        "minecraft:badjson2",
        "minecraft:loop",
        "minecraft:refloop",
        "other:thing",
        "",
        "minecraft:a/../b",
    ] {
        assert!(i.image(id).is_none(), "{id}");
    }
}

#[test]
fn spawn_egg_is_tinted_with_overlay() {
    let mut s = MemSource::default();
    tex(&mut s, "item/spawn_egg", [255, 255, 255, 255]);
    let mut spots = image::RgbaImage::new(16, 16);
    spots.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
    let mut bytes = Vec::new();
    spots.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    s.0.insert("assets/minecraft/textures/item/spawn_egg_overlay.png".into(), bytes);
    json(&mut s, "models/item/creeper_spawn_egg", r#"{"parent":"minecraft:item/template_spawn_egg"}"#);
    json(
        &mut s,
        "models/item/template_spawn_egg",
        r#"{"parent":"item/generated","textures":{"layer0":"item/spawn_egg","layer1":"item/spawn_egg_overlay"}}"#,
    );
    json(&mut s, "models/item/newmob_spawn_egg", r#"{"parent":"minecraft:item/template_spawn_egg"}"#);
    let mut i = icons(s);
    let egg = i.image("minecraft:creeper_spawn_egg").unwrap();
    assert_eq!(&egg.rgba[..4], &[0, 0, 0, 255]); // the spot, black for a creeper
    assert_eq!(&egg.rgba[4..8], &[0x0D, 0xA7, 0x0B, 255]);
    assert!(i.image("minecraft:newmob_spawn_egg").is_some());
}

#[test]
fn potion_gets_a_liquid() {
    let mut s = MemSource::default();
    tex(&mut s, "item/potion_overlay", [255, 255, 255, 255]);
    let mut bottle = image::RgbaImage::new(16, 16);
    bottle.put_pixel(1, 0, image::Rgba([10, 20, 30, 255]));
    let mut bytes = Vec::new();
    bottle.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    s.0.insert("assets/minecraft/textures/item/potion.png".into(), bytes);
    let mut i = icons(s);
    let p = i.image("minecraft:potion").unwrap();
    assert_eq!(&p.rgba[..4], &[0x38, 0x5D, 0xC6, 255]);
    assert_eq!(&p.rgba[4..8], &[10, 20, 30, 255]);
}

#[test]
fn chest_is_cut_from_the_entity_texture() {
    let mut s = MemSource::default();
    s.0.insert("assets/minecraft/textures/entity/chest/normal.png".into(), png(64, 64, [150, 100, 40, 255]));
    json(&mut s, "models/item/chest", r#"{"parent":"builtin/entity","textures":{"particle":"block/oak_planks"}}"#);
    let mut i = icons(s);
    let c = i.image("minecraft:chest").unwrap();
    assert_eq!((c.width, c.height), (16, 16));
    assert_eq!(&c.rgba[(8 * 16 + 8) * 4..][..4], &[150, 100, 40, 255]);
    assert_eq!(c.rgba[3], 0);
}

// ---- real jars ----

fn jars() -> Vec<PathBuf> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let mut dirs = vec![home.join("bf-craft-work/jars")];
    if let Ok(d) = std::env::var("BF_CRAFT_JARS") {
        dirs.insert(0, PathBuf::from(d));
    }
    let mut out = Vec::new();
    for d in dirs {
        for n in ["client-1.20.4.jar", "client-1.21.1.jar"] {
            out.push(d.join(n));
        }
    }
    let lib = home.join(".local/share/blockforge/libraries/net/minecraft/client");
    out.push(lib.join("1.20.1/client-1.20.1.jar"));
    out.push(lib.join("26.3/client-26.3.jar"));
    let mut found: Vec<PathBuf> = Vec::new();
    for p in out {
        if p.exists() && !found.iter().any(|f| f.file_name() == p.file_name()) {
            found.push(p);
        }
    }
    found
}

/// Ids of the items a jar describes: `items/*.json` (1.21.4+) or `models/item/*.json`, without helper models.
fn item_ids(src: &dyn FileSource) -> Vec<String> {
    let names = src.names();
    let list = |prefix: &str| -> Vec<String> {
        let mut v: Vec<String> = names
            .iter()
            .filter_map(|n| n.strip_prefix(prefix)?.strip_suffix(".json"))
            .filter(|n| !n.contains('/'))
            .map(str::to_string)
            .collect();
        v.sort();
        v
    };
    let new = list("assets/minecraft/items/");
    let all = if new.is_empty() { list("assets/minecraft/models/item/") } else { new };
    all.into_iter().filter(|n| !is_helper_model(n)).collect()
}

fn is_helper_model(n: &str) -> bool {
    let numbered = n.rsplit_once('_').is_some_and(|(_, d)| d.len() == 2 && d.bytes().all(|b| b.is_ascii_digit()));
    n.starts_with("template_")
        || n.contains("_pulling_")
        || n.contains("_charged")
        || matches!(
            n,
            "generated" | "handheld" | "handheld_rod" | "handheld_mace" | "bundle_open_front" | "bundle_open_back"
        )
        || n.ends_with("_in_hand")
        || n.ends_with("_throwing")
        || n.ends_with("_blocking")
        || n.ends_with("_cast")
        || n.ends_with("_open")
        || n.starts_with("crossbow_")
        || n.starts_with("bow_")
        || numbered
}

fn open_jar(p: &Path) -> Icons {
    Icons::open(p).unwrap()
}

#[test]
fn real_jars_resolve_most_items() {
    for jar in jars() {
        let mut i = open_jar(&jar);
        let ids = item_ids(&*i.src);
        assert!(ids.len() > 500, "{}: {} items", jar.display(), ids.len());
        let t = Instant::now();
        let mut bad = Vec::new();
        for id in &ids {
            if i.image(&format!("minecraft:{id}")).is_none() {
                bad.push(id.clone());
            }
        }
        let took = t.elapsed();
        let ok = ids.len() - bad.len();
        let pct = ok as f64 * 100.0 / ids.len() as f64;
        println!("{}: {ok}/{} resolved ({pct:.1}%) in {took:?}", jar.display(), ids.len());
        println!("  first unresolved: {:?}", &bad[..bad.len().min(40)]);
        assert!(pct >= 85.0, "{}: only {pct:.1}%", jar.display());
    }
}

#[test]
fn real_jars_spot_checks() {
    for jar in jars() {
        let name = jar.display().to_string();
        let mut i = open_jar(&jar);
        for id in [
            "oak_planks",
            "stone",
            "diamond_pickaxe",
            "crafting_table",
            "furnace",
            "water_bucket",
            "redstone",
            "grass_block",
            "oak_leaves",
        ] {
            assert!(i.image(&format!("minecraft:{id}")).is_some(), "{name}: {id}");
        }
        assert!(i.texture_path("minecraft:crafting_table").unwrap().contains("crafting_table_front"), "{name}");
        assert!(i.texture_path("minecraft:furnace").unwrap().contains("furnace_front"), "{name}");
        assert!(i.texture_path("minecraft:grass_block").unwrap().contains("grass_block_top"), "{name}");
        let mean = |img: &IconImage| {
            let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
            for p in img.rgba.chunks(4).filter(|p| p[3] > 0) {
                r += u64::from(p[0]);
                g += u64::from(p[1]);
                b += u64::from(p[2]);
                n += 1;
            }
            (r / n.max(1), g / n.max(1), b / n.max(1))
        };
        for id in ["oak_leaves", "grass_block"] {
            let (r, g, b) = mean(&i.image(&format!("minecraft:{id}")).unwrap());
            assert!(g > r && g > b, "{name}: {id} is not green: {r},{g},{b}");
        }
        let planks = i.image("minecraft:oak_planks").unwrap();
        assert_eq!((planks.width, planks.height), (16, 16), "{name}");
    }
}

/// Repeated lookups cost one zip read and one decode.
#[test]
fn real_jar_repeat_lookups_are_fast() {
    let Some(jar) = jars().into_iter().next() else { return };
    let mut i = open_jar(&jar);
    assert!(i.image("minecraft:oak_planks").is_some());
    let t = Instant::now();
    for _ in 0..1000 {
        assert!(i.image("minecraft:oak_planks").is_some());
    }
    println!("1000 repeated lookups: {:?}", t.elapsed());
}

/// Dumps a contact sheet of every resolved icon of a jar (`BF_ICON_SHEET=1`, jar substring in `BF_ICON_JAR`).
#[test]
fn contact_sheet() {
    if std::env::var("BF_ICON_SHEET").is_err() {
        return;
    }
    let want = std::env::var("BF_ICON_JAR").unwrap_or_else(|_| "1.21.1".into());
    let Some(jar) = jars().into_iter().find(|j| j.display().to_string().contains(&want)) else { return };
    let mut i = open_jar(&jar);
    let ids = item_ids(&*i.src);
    let (cols, cell) = (40usize, 32usize);
    let shown: Vec<IconImage> = ids.iter().filter_map(|id| i.image(&format!("minecraft:{id}"))).collect();
    let rows = shown.len().div_ceil(cols);
    let mut sheet =
        image::RgbaImage::from_pixel((cols * cell) as u32, (rows * cell) as u32, image::Rgba([60, 60, 70, 255]));
    for (n, img) in shown.iter().enumerate() {
        let (ox, oy) = ((n % cols) * cell, (n / cols) * cell);
        for y in 0..cell {
            for x in 0..cell {
                let p = &img.rgba[((y * img.height / cell) * img.width + x * img.width / cell) * 4..][..4];
                if p[3] > 0 {
                    sheet.put_pixel((ox + x) as u32, (oy + y) as u32, image::Rgba([p[0], p[1], p[2], 255]));
                }
            }
        }
    }
    let home = std::env::var("HOME").unwrap();
    let out = format!("{home}/bf-craft-work/contact-{want}.png");
    sheet.save(&out).unwrap();
    println!("wrote {out} with {} icons", shown.len());
    let names: Vec<&String> = ids.iter().filter(|id| i.image(&format!("minecraft:{id}")).is_some()).collect();
    println!("order: {:?}", &names[..names.len().min(5)]);
}

#[test]
fn zz_debug_paths() {
    let Ok(ids) = std::env::var("BF_ICON_IDS") else { return };
    let want = std::env::var("BF_ICON_JAR").unwrap_or_else(|_| "1.21.1".into());
    let Some(jar) = jars().into_iter().find(|j| j.display().to_string().contains(&want)) else { return };
    let mut i = open_jar(&jar);
    for id in ids.split(',') {
        println!("{id} -> {:?}", i.texture_path(id));
    }
}
