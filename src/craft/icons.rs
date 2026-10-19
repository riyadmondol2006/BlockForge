//! Item pictures, read from the game's own textures.
//!
//! An item id is turned into the path of one flat texture inside the client jar: the item's own texture if it has
//! one, else the first usable texture of its model (following `parent` chains, and the item definitions of
//! 1.21.4 and later), else a block texture named like the item. Nothing of Mojang's is bundled.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::rc::Rc;

use serde_json::Value;

use super::source::{FileSource, ZipSource};

#[path = "icons_extra.rs"]
mod extra;

/// How many `parent` hops (and `#ref` hops) are followed before giving up.
const MAX_DEPTH: usize = 8;
/// The largest texture side that is decoded; anything bigger is not an icon.
const MAX_SIDE: u32 = 1024;
/// The texture keys of a model, best first. Keys not listed here are tried afterwards (except the ones in `SKIP`).
const PREFER: [&str; 13] =
    ["layer0", "all", "front", "north", "side", "east", "top", "up", "end", "texture", "cross", "plant", "particle"];
/// Keys that never make a good flat icon (the bottom face, the overlay of grass sides).
const SKIP: [&str; 4] = ["down", "bottom", "overlay", "layer1"];

/// An RGBA picture, 4 bytes a pixel, row by row.
pub struct IconImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// The parts of a model file that matter for a flat icon.
struct Model {
    parent: Option<String>,
    textures: BTreeMap<String, String>,
}

pub struct Icons {
    src: Box<dyn FileSource>,
    paths: HashMap<String, Option<String>>,
    models: HashMap<String, Option<Rc<Model>>>,
}

impl Icons {
    pub fn open(jar: &Path) -> Result<Icons, String> {
        Ok(Icons::new(Box::new(ZipSource::open(jar)?)))
    }
    pub fn new(src: Box<dyn FileSource>) -> Icons {
        Icons { src, paths: HashMap::new(), models: HashMap::new() }
    }

    /// The picture of `id` (`minecraft:oak_planks`): the first frame of an animated texture, tinted where the game
    /// tints it (leaves, grass), or `None` when the item has no usable texture.
    pub fn image(&mut self, id: &str) -> Option<IconImage> {
        let path = self.texture_path(id)?;
        let name = bare(id)?;
        let bytes = self.src.read(&path)?;
        let mut img = decode(&bytes)?;
        if path.contains("/textures/entity/") {
            let full = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()?.to_rgba8();
            let (w, h) = (full.width() as usize, full.height() as usize);
            return extra::entity_icon(&path, full.as_raw(), w, h).or(Some(img));
        }
        if path == extra::SPAWN_EGG
            && let Some(mob) = name.strip_suffix("_spawn_egg")
        {
            let (base, spots) = extra::egg_colors(mob);
            extra::apply_tint(&mut img.rgba, base);
            if let Some(mut over) = self.src.read(extra::SPAWN_EGG_OVERLAY).and_then(|b| decode(&b))
                && over.width == img.width
                && over.height == img.height
            {
                extra::apply_tint(&mut over.rgba, spots);
                extra::over(&mut img.rgba, &over.rgba);
            }
            return Some(img);
        }
        if matches!(name, "potion" | "splash_potion" | "lingering_potion")
            && let Some(mut liquid) = self.src.read(&extra::potion_overlay()).and_then(|b| decode(&b))
            && liquid.width == img.width
            && liquid.height == img.height
        {
            extra::apply_tint(&mut liquid.rgba, extra::POTION_TINT);
            extra::over(&mut liquid.rgba, &img.rgba);
            return Some(liquid);
        }
        if let Some(tint) = tint_for(name) {
            extra::apply_tint(&mut img.rgba, tint);
        }
        Some(img)
    }

    /// The jar path of the texture that stands for `id`, if there is one (the answer is remembered).
    pub fn texture_path(&mut self, id: &str) -> Option<String> {
        if let Some(known) = self.paths.get(id) {
            return known.clone();
        }
        let found = bare(id).and_then(|name| self.resolve(name));
        self.paths.insert(id.to_string(), found.clone());
        found
    }

    fn resolve(&mut self, name: &str) -> Option<String> {
        if name.is_empty() || name.contains(['/', '\\', '.']) {
            return None;
        }
        let item = tex_path("item", name);
        if self.src.exists(&item) {
            return Some(item);
        }
        let top_first = name == "grass_block";
        if let Some((model, special)) = self.model_of_item(name)
            && let Some(found) = self.texture_of_model(&model, top_first, special)
        {
            return Some(found);
        }
        for suffix in ["", "_front", "_side", "_top"] {
            let p = tex_path("block", &format!("{name}{suffix}"));
            if self.src.exists(&p) {
                return Some(p);
            }
        }
        extra::entity_stand_in(name).iter().map(|rel| extra::entity_path(rel)).find(|p| self.src.exists(p))
    }

    /// The model an item uses: through its definition (`items/<n>.json`, 1.21.4+) or `models/item/<n>.json`;
    /// the flag says the model is only the base of a special (entity) renderer, whose texture is not flat.
    fn model_of_item(&mut self, name: &str) -> Option<(String, bool)> {
        if let Some(bytes) = self.src.read(&format!("assets/minecraft/items/{name}.json")) {
            let v: Value = serde_json::from_slice(&bytes).ok()?;
            return definition_model(&v, 0);
        }
        let own = format!("item/{name}");
        self.model(&own).map(|_| (own, false))
    }

    fn model(&mut self, name: &str) -> Option<Rc<Model>> {
        if let Some(m) = self.models.get(name) {
            return m.clone();
        }
        let parsed = self
            .src
            .read(&format!("assets/minecraft/models/{name}.json"))
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .map(|v| Rc::new(parse_model(&v)));
        self.models.insert(name.to_string(), parsed.clone());
        parsed
    }

    /// The best texture of a model (given as `minecraft:block/x` or `block/x`), merging the `textures` of its parents.
    fn texture_of_model(&mut self, model: &str, top_first: bool, special: bool) -> Option<String> {
        let mut textures: BTreeMap<String, String> = BTreeMap::new();
        let mut entity = special;
        let mut cur = Some(bare(model)?.to_string());
        for _ in 0..MAX_DEPTH {
            let Some(name) = cur.take() else { break };
            if name.starts_with("builtin/") {
                entity = entity || name == "builtin/entity";
                break;
            }
            let Some(m) = self.model(&name) else { break };
            for (k, v) in &m.textures {
                textures.entry(k.clone()).or_insert_with(|| v.clone());
            }
            cur = m.parent.as_deref().and_then(bare).map(str::to_string);
        }
        let mut keys: Vec<&str> = Vec::new();
        if top_first {
            keys.extend(["top", "up"]);
        }
        keys.extend(PREFER.iter().copied().filter(|k| !(entity && *k == "particle")));
        let mut rest: Vec<&str> = textures
            .keys()
            .map(String::as_str)
            .filter(|k| !keys.contains(k) && !SKIP.contains(k) && !(entity && *k == "particle"))
            .collect();
        rest.sort_unstable();
        keys.extend(rest);
        for key in keys {
            let Some(v) = textures.get(key) else { continue };
            let Some(target) = follow_ref(&textures, v) else { continue };
            let Some(rel) = bare(target) else { continue };
            let p = format!("assets/minecraft/textures/{rel}.png");
            if self.src.exists(&p) {
                return Some(p);
            }
        }
        None
    }
}

/// `minecraft:x` and `x` are the game's own; any other namespace is not in the client jar.
fn bare(s: &str) -> Option<&str> {
    match s.split_once(':') {
        Some(("minecraft", rest)) => Some(rest),
        Some(_) => None,
        None => Some(s),
    }
}

fn tex_path(dir: &str, name: &str) -> String {
    format!("assets/minecraft/textures/{dir}/{name}.png")
}

fn parse_model(v: &Value) -> Model {
    let parent = v.get("parent").and_then(Value::as_str).map(str::to_string);
    let mut textures = BTreeMap::new();
    if let Some(Value::Object(t)) = v.get("textures") {
        for (k, val) in t {
            if let Some(s) = val.as_str() {
                textures.insert(k.clone(), s.to_string());
            }
        }
    }
    Model { parent, textures }
}

/// Resolves `#name` indirections in a model's texture map.
fn follow_ref<'a>(textures: &'a BTreeMap<String, String>, mut v: &'a str) -> Option<&'a str> {
    for _ in 0..MAX_DEPTH {
        match v.strip_prefix('#') {
            Some(name) => v = textures.get(name)?,
            None => return Some(v),
        }
    }
    None
}

/// The first model named in an item definition (`items/<n>.json`): the plain model, the base of a special model
/// (flagged `true`), the default of conditions, selections and range dispatches.
fn definition_model(v: &Value, depth: usize) -> Option<(String, bool)> {
    if depth > 16 {
        return None;
    }
    match v {
        Value::Object(o) => {
            if let Some(Value::String(s)) = o.get("base") {
                return Some((s.clone(), true));
            }
            if let Some(Value::String(s)) = o.get("model") {
                return Some((s.clone(), false));
            }
            for k in ["on_false", "fallback", "model", "models", "entries", "cases", "on_true"] {
                if let Some(found) = o.get(k).and_then(|c| definition_model(c, depth + 1)) {
                    return Some(found);
                }
            }
            o.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "tints" | "transformation"))
                .find_map(|(_, c)| definition_model(c, depth + 1))
        }
        Value::Array(a) => a.iter().find_map(|c| definition_model(c, depth + 1)),
        _ => None,
    }
}

/// Decodes a PNG into straight RGBA, keeping only the first square frame of an animated strip.
fn decode(bytes: &[u8]) -> Option<IconImage> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE * 64 {
        return None;
    }
    let side_h = if h > w && h % w == 0 { w } else { h };
    let mut rgba = img.into_raw();
    rgba.truncate(w as usize * side_h as usize * 4);
    Some(IconImage { width: w as usize, height: side_h as usize, rgba })
}

/// The colour the game multiplies onto the grayscale texture of `name` in the default (plains) biome.
fn tint_for(name: &str) -> Option<u32> {
    const GRASS: u32 = 0x91BD59;
    match name {
        "grass_block" | "short_grass" | "grass" | "tall_grass" | "fern" | "large_fern" | "sugar_cane" | "vine" => {
            Some(GRASS)
        }
        "lily_pad" => Some(0x208030),
        "birch_leaves" => Some(0x80A755),
        "spruce_leaves" => Some(0x619961),
        "cherry_leaves" | "pale_oak_leaves" | "azalea_leaves" | "flowering_azalea_leaves" => None,
        n if n == "leaves" || n.ends_with("_leaves") => Some(0x77AB2F),
        _ => None,
    }
}

#[cfg(test)]
#[path = "icons_tests.rs"]
mod tests;
