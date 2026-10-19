//! Reading the recipe book of one game version out of its client jar.
//!
//! Nothing of Mojang's is bundled: the recipes, item tags, English names and the list of real items are all read
//! from the player's own vanilla client jar when the page opens. Only the files that are needed are inflated (the
//! recipe and tag JSON files, the language file and nothing else); the item list comes from the file names alone.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny, MapAccess, Visitor};
use serde_json::Value;

use super::source::{FileSource, ZipSource};
use super::types::*;
use crate::paths::Dirs;

#[cfg(test)]
#[path = "load_tests.rs"]
mod tests;

const NS: &str = "minecraft:";

/// A game version whose client jar is on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledVersion {
    pub version: String,
    pub jar: PathBuf,
}

// ---------------------------------------------------------------------------------------------------------
// Installed versions

/// The vanilla client jars in the shared libraries folder (`net/minecraft/client/<v>/client-<v>.jar`, only the
/// plain ones, not the loader-patched `-srg`/`-slim`/`-extra` copies), newest first.
///
/// Order: releases (and their `-preN` / `-rcN` / `-snapshot-N` builds, just below the release they lead to) by
/// version number, then the weekly snapshots (`24w14a`) after all of them, newest week first. Versions older than
/// 1.13 (and snapshots before 17w43a) are left out: their jars have no recipe files, so there is nothing to read.
pub fn installed_versions(dirs: &Dirs) -> Vec<InstalledVersion> {
    let root = dirs.libraries.join("net").join("minecraft").join("client");
    let Ok(read) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut found: Vec<(VersionKey, InstalledVersion)> = Vec::new();
    for entry in read.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(key) = version_key(&name).filter(has_recipe_files) else {
            continue;
        };
        let jar = entry.path().join(format!("client-{name}.jar"));
        if jar.is_file() {
            found.push((key, InstalledVersion { version: name, jar }));
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.version.cmp(&b.1.version)));
    found.into_iter().map(|(_, v)| v).collect()
}

/// What orders two game versions: `group` 1 for releases and their pre-builds, 0 for weekly snapshots; the version
/// numbers without trailing zeros; and where in the run-up to the release the build is (the release itself is 0).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct VersionKey {
    group: u8,
    nums: Vec<u32>,
    stage: i32,
}

/// The sort key of a release or snapshot id, `None` for anything else (`1.20.1-20230612.114412`, `latest`, ...).
fn version_key(v: &str) -> Option<VersionKey> {
    let b = v.as_bytes();
    // a weekly snapshot: 24w14a
    if b.len() == 6
        && b[2] == b'w'
        && b[5].is_ascii_lowercase()
        && b[..2].iter().chain(&b[3..5]).all(u8::is_ascii_digit)
    {
        let year: u32 = v[..2].parse().ok()?;
        let week: u32 = v[3..5].parse().ok()?;
        return Some(VersionKey { group: 0, nums: vec![year, week, (b[5] - b'a') as u32], stage: 0 });
    }
    let (base, stage) = match v.split_once('-') {
        None => (v, 0),
        Some((base, tail)) => {
            // `pre1`, `rc2`, `snapshot-3`; a build stamp such as `20230612.114412` is not one of these
            let kind = tail.trim_end_matches(|c: char| c.is_ascii_digit());
            let n: i32 = tail[kind.len()..].parse().ok().filter(|n| (0..1000).contains(n))?;
            let stage = match kind.trim_end_matches('-') {
                "pre" => -2000 + n,
                "rc" => -1000 + n,
                "snapshot" => -3000 + n,
                _ => return None,
            };
            (base, stage)
        }
    };
    let mut nums = Vec::new();
    for part in base.split('.') {
        if part.is_empty() || part.len() > 4 || !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        nums.push(part.parse().ok()?);
    }
    if nums.len() < 2 {
        return None;
    }
    while nums.last() == Some(&0) {
        nums.pop();
    }
    Some(VersionKey { group: 1, nums, stage })
}

/// Recipes became data files in 1.13 (snapshot 17w43a).
fn has_recipe_files(key: &VersionKey) -> bool {
    if key.group == 0 {
        return (key.nums[0], key.nums[1]) >= (17, 43);
    }
    key.nums[0] > 1 || key.nums.get(1).copied().unwrap_or(0) >= 13
}

// ---------------------------------------------------------------------------------------------------------
// Loading

pub fn load_jar(version: &str, jar: &Path) -> Result<Dataset, String> {
    let mut src = ZipSource::open(jar)?;
    load(version, &mut src)
}

/// Read the recipes, tags, names and item list of the game in `src`.
pub fn load(version: &str, src: &mut dyn FileSource) -> Result<Dataset, String> {
    let names = src.names();
    let layout = Layout::scan(&names);
    if layout.recipe_files.is_empty() {
        return Err(format!(
            "Minecraft 1.13 or newer is needed for the crafting calculator: the game files of {version} have no \
             recipe data"
        ));
    }
    let lang = read_lang(src);
    let mut listed: HashSet<String> = layout
        .item_assets
        .iter()
        .filter(|id| lang.as_ref().is_none_or(|l| has_name(l, id)))
        .map(|id| format!("{NS}{id}"))
        .collect();
    listed.remove("minecraft:air");

    let mut skipped: Vec<(String, String)> = Vec::new();
    let mut docs: Vec<(String, Value)> = Vec::with_capacity(layout.recipe_files.len());
    for (id, path) in &layout.recipe_files {
        match src.read(path).map(|b| serde_json::from_slice::<Value>(&b)) {
            Some(Ok(v)) => docs.push((id.clone(), v)),
            Some(Err(e)) => skipped.push((id.clone(), format!("invalid JSON: {e}"))),
            None => skipped.push((id.clone(), "the file could not be read".into())),
        }
    }
    // everything a recipe makes is an item, even when its model or name is missing
    let mut valid = listed.clone();
    for (_, v) in &docs {
        if let Some((out, _)) = v.get("result").and_then(|r| result_of(r, v).ok()) {
            valid.insert(out);
        }
    }

    let mut cx = Ctx { src, tag_paths: layout.tag_files, tag_cache: HashMap::new(), valid, tag_items: BTreeSet::new() };
    let mut recipes = Vec::new();
    let mut declared: HashMap<String, Category> = HashMap::new();
    for (id, doc) in &docs {
        match cx.build(id, doc) {
            Ok((recipe, category)) => {
                if let Some(c) = category {
                    declared.entry(recipe.output.clone()).or_insert(c);
                }
                recipes.push(recipe);
            }
            Err(reason) => skipped.push((id.clone(), reason)),
        }
    }
    if recipes.is_empty() {
        return Err(format!("No usable crafting recipes were found in the game files of {version}"));
    }

    let mut raw_stops = raw_stops_of(&recipes);
    for tag in ["minecraft:wool", "minecraft:logs"] {
        for id in cx.resolve_tag(tag, &mut Vec::new()).unwrap_or_default() {
            let short = id.strip_prefix(NS).unwrap_or(&id);
            if !short.starts_with("stripped_") && !short.ends_with("_wood") && !short.ends_with("_hyphae") {
                raw_stops.insert(id);
            }
        }
    }

    let mut ids = listed;
    ids.extend(cx.tag_items);
    for r in &recipes {
        ids.insert(r.output.clone());
        ids.extend(r.ingredients.iter().flat_map(|i| i.alternatives.iter().cloned()));
    }
    ids.remove("minecraft:air");
    let items: BTreeMap<ItemId, Item> = ids
        .into_iter()
        .map(|id| {
            let name = id
                .strip_prefix(NS)
                .and_then(|s| {
                    lang.as_ref()?
                        .get(&format!("item.minecraft.{s}"))
                        .or(lang.as_ref()?.get(&format!("block.minecraft.{s}")))
                })
                .cloned()
                .unwrap_or_else(|| pretty_name(&id));
            let category = declared.get(&id).copied().unwrap_or_else(|| fallback_category(&id));
            let stack = super::stack::stack_size(&id);
            (id.clone(), Item { id, name, stack, category })
        })
        .collect();
    Dataset::new(version, items, recipes, raw_stops, skipped)
}

/// Whether the language file names the item or block with this short id.
fn has_name(lang: &HashMap<String, String>, short: &str) -> bool {
    lang.contains_key(&format!("item.minecraft.{short}")) || lang.contains_key(&format!("block.minecraft.{short}"))
}

/// Where the interesting files of a jar are.
struct Layout {
    /// (recipe id, path), sorted by id.
    recipe_files: Vec<(String, String)>,
    /// Item tag id (`minecraft:logs`) -> path.
    tag_files: HashMap<String, String>,
    /// Short ids that have an item model.
    item_assets: Vec<String>,
}

impl Layout {
    fn scan(names: &[String]) -> Layout {
        // 1.21 renamed `recipes` to `recipe` and `tags/items` to `tags/item`
        let dir = |new: &'static str, old: &'static str| {
            if names.iter().any(|n| n.starts_with(new)) { new } else { old }
        };
        let recipes = dir("data/minecraft/recipe/", "data/minecraft/recipes/");
        let tags = dir("data/minecraft/tags/item/", "data/minecraft/tags/items/");
        // 1.21.4 moved the item models to `assets/minecraft/items/`
        let assets = dir("assets/minecraft/items/", "assets/minecraft/models/item/");
        let mut layout = Layout { recipe_files: Vec::new(), tag_files: HashMap::new(), item_assets: Vec::new() };
        for n in names {
            let Some(stem) = n.strip_suffix(".json") else {
                continue;
            };
            if let Some(rest) = stem.strip_prefix(recipes) {
                layout.recipe_files.push((rest.to_string(), n.clone()));
            } else if let Some(rest) = stem.strip_prefix(tags) {
                layout.tag_files.insert(format!("{NS}{rest}"), n.clone());
            } else if let Some(rest) = stem.strip_prefix(assets)
                && !rest.contains('/')
            {
                layout.item_assets.push(rest.to_string());
            }
        }
        layout.recipe_files.sort();
        layout
    }
}

/// The English names of items and blocks (`item.minecraft.x` and `block.minecraft.x` only: the file has some
/// 30 000 other lines), by full language key.
fn read_lang(src: &mut dyn FileSource) -> Option<HashMap<String, String>> {
    #[derive(Deserialize)]
    struct Key<'a>(#[serde(borrow)] Cow<'a, str>);
    struct Lang(HashMap<String, String>);
    struct LangVisitor;
    impl<'de> Visitor<'de> for LangVisitor {
        type Value = Lang;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a language file")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Lang, A::Error> {
            let mut out = HashMap::new();
            while let Some(Key(key)) = map.next_key::<Key>()? {
                if key.starts_with("item.minecraft.") || key.starts_with("block.minecraft.") {
                    if let Ok(v) = map.next_value::<String>() {
                        out.insert(key.into_owned(), v);
                    }
                } else {
                    map.next_value::<IgnoredAny>()?;
                }
            }
            Ok(Lang(out))
        }
    }
    impl<'de> Deserialize<'de> for Lang {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Lang, D::Error> {
            d.deserialize_map(LangVisitor)
        }
    }
    let bytes = src.read("assets/minecraft/lang/en_us.json")?;
    let Lang(map) = serde_json::from_slice(&bytes).ok()?;
    Some(map).filter(|m| !m.is_empty())
}

// ---------------------------------------------------------------------------------------------------------
// Recipes

/// `oak_planks` -> `minecraft:oak_planks`.
fn norm(id: &str) -> String {
    if id.contains(':') { id.to_string() } else { format!("{NS}{id}") }
}

/// A count field: absent means `default`, otherwise a whole number from 1 to `max`.
fn count_of(v: Option<&Value>, default: u32, max: u32, what: &str) -> Result<u32, String> {
    match v {
        None => Ok(default),
        Some(c) => c.as_u64().filter(|&c| (1..=max as u64).contains(&c)).map(|c| c as u32).ok_or(format!("bad {what}")),
    }
}

/// The item a result makes and how many, from the `result` of a recipe (a string, `{item,count}` or `{id,count}`)
/// and the recipe itself (old stonecutting keeps the count at the top).
fn result_of(result: &Value, recipe: &Value) -> Result<(ItemId, u32), String> {
    let (id, count) = match result {
        Value::String(s) => (s.as_str(), recipe.get("count")),
        Value::Object(o) => {
            let id = o
                .get("id")
                .or_else(|| o.get("item"))
                .and_then(Value::as_str)
                .ok_or("the result is not a fixed item")?;
            (id, o.get("count"))
        }
        _ => return Err("the result is not an item".into()),
    };
    Ok((norm(id), count_of(count, 1, 1 << 20, "result count")?))
}

struct Ctx<'a> {
    src: &'a mut dyn FileSource,
    tag_paths: HashMap<String, String>,
    tag_cache: HashMap<String, BTreeSet<String>>,
    /// Ids that are real items: those with a model and a name, and everything a recipe makes.
    valid: HashSet<String>,
    /// Members of the tags the recipes use, so the book lists them.
    tag_items: BTreeSet<String>,
}

impl Ctx<'_> {
    /// The items of item tag `id` (`minecraft:logs`), nested tags included. Optional entries that do not exist are
    /// left out; a required tag that does not exist is an error. A tag that contains itself adds nothing new.
    fn resolve_tag(&mut self, id: &str, stack: &mut Vec<String>) -> Result<BTreeSet<String>, String> {
        if let Some(done) = self.tag_cache.get(id) {
            return Ok(done.clone());
        }
        if stack.iter().any(|s| s == id) {
            return Ok(BTreeSet::new());
        }
        let path = self.tag_paths.get(id).cloned().ok_or_else(|| format!("the tag #{id} does not exist"))?;
        let bytes = self.src.read(&path).ok_or_else(|| format!("the tag #{id} could not be read"))?;
        let doc: Value = serde_json::from_slice(&bytes).map_err(|e| format!("the tag #{id} is invalid: {e}"))?;
        let mut out = BTreeSet::new();
        stack.push(id.to_string());
        let mut failed = None;
        for entry in doc.get("values").and_then(Value::as_array).into_iter().flatten() {
            let (name, required) = match entry {
                Value::String(s) => (s.as_str(), true),
                Value::Object(o) => match o.get("id").and_then(Value::as_str) {
                    Some(n) => (n, o.get("required").and_then(Value::as_bool).unwrap_or(true)),
                    None => continue,
                },
                _ => continue,
            };
            if let Some(nested) = name.strip_prefix('#') {
                match self.resolve_tag(&norm(nested), stack) {
                    Ok(set) => out.extend(set),
                    Err(e) if required => {
                        failed = Some(e);
                        break;
                    }
                    Err(_) => {}
                }
            } else {
                let item = norm(name);
                if item != "minecraft:air" && (required || self.valid.contains(&item)) {
                    out.insert(item);
                }
            }
        }
        stack.pop();
        if let Some(e) = failed {
            return Err(e);
        }
        self.tag_cache.insert(id.to_string(), out.clone());
        Ok(out)
    }

    /// The alternatives of an ingredient in any of its forms, sorted and without duplicates.
    fn alternatives(&mut self, v: &Value) -> Result<Vec<ItemId>, String> {
        let mut out = BTreeSet::new();
        self.collect(v, &mut out)?;
        if out.is_empty() {
            return Err("an ingredient matches no item".into());
        }
        Ok(out.into_iter().collect())
    }

    fn collect(&mut self, v: &Value, out: &mut BTreeSet<String>) -> Result<(), String> {
        match v {
            Value::String(s) => match s.strip_prefix('#') {
                Some(tag) => self.add_tag(tag, out),
                None => self.add_item(s, out),
            },
            Value::Array(list) => list.iter().try_for_each(|e| self.collect(e, out)),
            Value::Object(o) => match (o.get("item").and_then(Value::as_str), o.get("tag").and_then(Value::as_str)) {
                (Some(item), _) => self.add_item(item, out),
                (None, Some(tag)) => self.add_tag(tag.trim_start_matches('#'), out),
                _ => Err("an ingredient has neither an item nor a tag".into()),
            },
            _ => Err("an ingredient is not an item, a tag or a list".into()),
        }
    }

    fn add_item(&mut self, id: &str, out: &mut BTreeSet<String>) -> Result<(), String> {
        let id = norm(id);
        if !self.valid.contains(&id) {
            return Err(format!("{id} is not an item"));
        }
        out.insert(id);
        Ok(())
    }

    fn add_tag(&mut self, tag: &str, out: &mut BTreeSet<String>) -> Result<(), String> {
        let tag = norm(tag);
        let members = self.resolve_tag(&tag, &mut Vec::new())?;
        if members.is_empty() {
            return Err(format!("the tag #{tag} has no items"));
        }
        self.tag_items.extend(members.iter().cloned());
        out.extend(members);
        Ok(())
    }

    /// The ingredient in field `name` of the recipe, as one slot of the same name.
    fn field(&mut self, doc: &Value, name: &str, key: &str) -> Result<Ingredient, String> {
        let v = doc.get(name).ok_or_else(|| format!("no `{name}`"))?;
        Ok(Ingredient { key: key.to_string(), count: 1, alternatives: self.alternatives(v)? })
    }

    /// One recipe and the category it declares. `Err` is the reason the recipe is left out.
    fn build(&mut self, id: &str, doc: &Value) -> Result<(Recipe, Option<Category>), String> {
        let ty = doc.get("type").and_then(Value::as_str).ok_or("the recipe has no type")?;
        let ty = ty.strip_prefix(NS).unwrap_or(ty);
        let supported = matches!(
            ty,
            "crafting_shaped"
                | "crafting_shapeless"
                | "crafting_transmute"
                | "smelting"
                | "blasting"
                | "smoking"
                | "campfire_cooking"
                | "stonecutting"
                | "smithing_transform"
        );
        if !supported {
            return Err(match ty {
                t if t.starts_with("crafting_special_") => {
                    format!("dynamic recipe ({t}): it depends on the items used")
                }
                "crafting_dye" | "crafting_imbue" | "crafting_decorated_pot" => {
                    format!("dynamic recipe ({ty}): it depends on the items used")
                }
                "brewing" => "brewing recipe: potions are not crafted".into(),
                "smithing_trim" => "armor trims are put on an item, they do not make a new one".into(),
                t => format!("unsupported recipe type {t}"),
            });
        }
        self.build_supported(id, ty, doc).map_err(|e| format!("malformed {ty} recipe: {e}"))
    }

    fn build_supported(&mut self, id: &str, ty: &str, doc: &Value) -> Result<(Recipe, Option<Category>), String> {
        let (output, mut count) = result_of(doc.get("result").ok_or("no result")?, doc)?;
        let category = doc.get("category").and_then(Value::as_str).and_then(|c| match c {
            "building" | "blocks" => Some(Category::Building),
            "redstone" => Some(Category::Redstone),
            "equipment" => Some(Category::Equipment),
            "misc" => Some(Category::Misc),
            "food" => Some(Category::Food),
            _ => None,
        });
        let mut manual = None;
        let mut ticks = None;
        let mut small_grid = false;
        let kind;
        let ingredients;
        match ty {
            "crafting_shaped" => {
                kind = Kind::Shaped;
                (ingredients, small_grid) = self.shaped(doc)?;
            }
            "crafting_shapeless" => {
                kind = Kind::Shapeless;
                (ingredients, small_grid) = self.shapeless(doc)?;
            }
            "smelting" | "blasting" | "smoking" | "campfire_cooking" => {
                let (k, default) = match ty {
                    "smelting" => (Kind::Smelting, 200),
                    "blasting" => (Kind::Blasting, 100),
                    "smoking" => (Kind::Smoking, 100),
                    _ => (Kind::Campfire, 600),
                };
                kind = k;
                ticks = Some(count_of(doc.get("cookingtime"), default, u32::MAX, "cooking time")?);
                ingredients = vec![self.field(doc, "ingredient", "input")?];
            }
            "stonecutting" => {
                kind = Kind::Stonecutting;
                ingredients = vec![self.field(doc, "ingredient", "input")?];
            }
            "smithing_transform" => {
                kind = Kind::Smithing;
                manual = Some("Smithing keeps the item's enchantments and name, do this one by hand".to_string());
                ingredients = vec![
                    self.field(doc, "template", "template")?,
                    self.field(doc, "base", "base")?,
                    self.field(doc, "addition", "addition")?,
                ];
            }
            _ => {
                kind = Kind::Transmute;
                let input = self.field(doc, "input", "input")?;
                let mut material = self.field(doc, "material", "material")?;
                material.count = count_of(doc.get("material_count"), 1, 1 << 20, "material count")?;
                count = 1;
                if doc.get("add_material_count_to_result").and_then(Value::as_bool).unwrap_or(false) {
                    count += material.count;
                }
                ingredients = vec![input, material];
            }
        }
        let recipe = Recipe { id: id.to_string(), kind, output, count, ingredients, manual, ticks, small_grid };
        Ok((recipe, category))
    }

    /// One ingredient per key symbol, counted over the pattern, in order of first appearance.
    fn shaped(&mut self, doc: &Value) -> Result<(Vec<Ingredient>, bool), String> {
        let rows: Vec<&str> = doc
            .get("pattern")
            .and_then(Value::as_array)
            .ok_or("no pattern")?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let key = doc.get("key").and_then(Value::as_object).ok_or("no key")?;
        let mut order: Vec<char> = Vec::new();
        let mut counts: HashMap<char, u32> = HashMap::new();
        for c in rows.iter().flat_map(|r| r.chars()).filter(|&c| c != ' ') {
            let n = counts.entry(c).or_insert(0);
            if *n == 0 {
                order.push(c);
            }
            *n += 1;
        }
        if order.is_empty() {
            return Err("the pattern is empty".into());
        }
        let mut ingredients = Vec::new();
        for c in order {
            let symbol = c.to_string();
            let v = key.get(&symbol).ok_or_else(|| format!("the pattern uses `{c}`, which has no key"))?;
            ingredients.push(Ingredient { key: symbol, count: counts[&c], alternatives: self.alternatives(v)? });
        }
        let small = rows.len() <= 2 && rows.iter().all(|r| r.chars().count() <= 2);
        Ok((ingredients, small))
    }

    /// Entries with the same alternatives become one ingredient with a count.
    fn shapeless(&mut self, doc: &Value) -> Result<(Vec<Ingredient>, bool), String> {
        let entries = doc.get("ingredients").and_then(Value::as_array).ok_or("no ingredients")?;
        if entries.is_empty() {
            return Err("no ingredients".into());
        }
        let mut ingredients: Vec<Ingredient> = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            let alternatives = self.alternatives(entry)?;
            match ingredients.iter_mut().find(|g| g.alternatives == alternatives) {
                Some(g) => g.count += 1,
                None => ingredients.push(Ingredient { key: format!("#{i}"), count: 1, alternatives }),
            }
        }
        Ok((ingredients, entries.len() <= 4))
    }
}

// ---------------------------------------------------------------------------------------------------------
// Raw stops

/// Items that are gathered, not made, even though a recipe unpacks them from a block (or they are only in the book
/// because of a recipe): the ores and mob drops of p1's list, plus crops and drops of newer versions.
const RAW: &[&str] = &[
    "acacia_log",
    "ancient_debris",
    "bamboo",
    "beef",
    "birch_log",
    "black_wool",
    "blaze_rod",
    "blue_wool",
    "bone",
    "brown_wool",
    "cactus",
    "cherry_log",
    "chicken",
    "clay_ball",
    "coal",
    "coal_ore",
    "cobbled_deepslate",
    "cobblestone",
    "cod",
    "copper_ore",
    "crimson_stem",
    "cyan_wool",
    "dark_oak_log",
    "deepslate_coal_ore",
    "deepslate_copper_ore",
    "deepslate_diamond_ore",
    "deepslate_emerald_ore",
    "deepslate_gold_ore",
    "deepslate_iron_ore",
    "deepslate_lapis_ore",
    "deepslate_redstone_ore",
    "diamond",
    "diamond_ore",
    "dirt",
    "egg",
    "emerald",
    "emerald_ore",
    "ender_pearl",
    "feather",
    "flint",
    "gold_ore",
    "gravel",
    "gray_wool",
    "green_wool",
    "gunpowder",
    "honey_bottle",
    "iron_ore",
    "jungle_log",
    "kelp",
    "lapis_lazuli",
    "lapis_ore",
    "leather",
    "light_blue_wool",
    "light_gray_wool",
    "lime_wool",
    "magenta_wool",
    "mangrove_log",
    "mushroom_stem",
    "mutton",
    "nether_gold_ore",
    "nether_quartz_ore",
    "oak_log",
    "obsidian",
    "orange_wool",
    "pink_wool",
    "porkchop",
    "purple_wool",
    "quartz",
    "rabbit",
    "raw_copper",
    "raw_gold",
    "raw_iron",
    "red_sand",
    "red_wool",
    "redstone",
    "redstone_ore",
    "salmon",
    "sand",
    "slime_ball",
    "spruce_log",
    "string",
    "sugar_cane",
    "warped_stem",
    "wheat",
    "white_wool",
    "yellow_wool",
    // newer or other gatherables
    "resin_clump",
    "glowstone_dust",
    "nether_wart",
    "melon_slice",
    "honeycomb",
    "amethyst_shard",
    "echo_shard",
    "prismarine_shard",
    "prismarine_crystals",
    "nether_star",
    "shulker_shell",
    "snowball",
    "ice",
    "pumpkin",
    "carrot",
    "potato",
    "beetroot",
    "apple",
    "sweet_berries",
    "cocoa_beans",
    "ink_sac",
    "glow_ink_sac",
    "rabbit_hide",
    "turtle_scute",
    "scute",
    "phantom_membrane",
    "spider_eye",
    "rotten_flesh",
];

/// The one item every ingredient of the recipe can be, if there is only one.
fn single(r: &Recipe) -> Option<&str> {
    let mut all = r.ingredients.iter().flat_map(|i| i.alternatives.iter());
    let first = all.next()?;
    all.all(|a| a == first).then_some(first.as_str())
}

/// The listed raw items plus the ones the recipes give away (see `pack_rule`).
fn raw_stops_of(recipes: &[Recipe]) -> HashSet<ItemId> {
    let mut raw: HashSet<ItemId> = RAW.iter().map(|s| format!("{NS}{s}")).collect();
    raw.extend(pack_rule(recipes));
    raw
}

/// Items whose every crafting recipe takes just one other item (a block) that is itself made from at least two of
/// the item (diamond <-> diamond block): the block is made of the item, so the item is what you gather. Items that
/// also come out of a furnace (ingots from ore, nuggets from tools) are crafted, not gathered.
fn pack_rule(recipes: &[Recipe]) -> HashSet<ItemId> {
    let mut raw = HashSet::new();
    let mut crafted: HashMap<&str, Vec<&Recipe>> = HashMap::new();
    let mut cooked: HashSet<&str> = HashSet::new();
    for r in recipes {
        match r.kind {
            Kind::Shaped | Kind::Shapeless => crafted.entry(&r.output).or_default().push(r),
            Kind::Smelting | Kind::Blasting | Kind::Smoking | Kind::Campfire => {
                cooked.insert(&r.output);
            }
            _ => {}
        }
    }
    for (&x, made) in &crafted {
        if cooked.contains(x) || x.ends_with("_nugget") {
            continue;
        }
        let packs = |y: &str| {
            crafted.get(y).is_some_and(|rs| {
                rs.iter().any(|r| single(r) == Some(x) && r.ingredients.iter().map(|i| i.count).sum::<u32>() >= 2)
            })
        };
        if made.iter().all(|r| single(r).is_some_and(packs)) {
            raw.insert(x.to_string());
        }
    }
    raw
}

// ---------------------------------------------------------------------------------------------------------
// Categories

/// The category of an item no recipe declares one for, guessed from its name.
fn fallback_category(id: &str) -> Category {
    let s = id.strip_prefix(NS).unwrap_or(id);
    let ends = |list: &[&str]| list.iter().any(|x| s.ends_with(x));
    let is = |list: &[&str]| list.contains(&s);
    if ends(&[
        "_sword",
        "_pickaxe",
        "_axe",
        "_shovel",
        "_hoe",
        "_helmet",
        "_chestplate",
        "_leggings",
        "_boots",
        "_horse_armor",
        "_nautilus_armor",
        "_spear",
    ]) || is(&[
        "bow",
        "crossbow",
        "shield",
        "trident",
        "mace",
        "fishing_rod",
        "shears",
        "flint_and_steel",
        "elytra",
        "wolf_armor",
        "carrot_on_a_stick",
        "warped_fungus_on_a_stick",
        "brush",
        "spyglass",
        "compass",
    ]) {
        return Category::Equipment;
    }
    if ends(&["_button", "_pressure_plate", "_bulb", "_rail"])
        || is(&[
            "redstone",
            "repeater",
            "comparator",
            "piston",
            "sticky_piston",
            "observer",
            "hopper",
            "dropper",
            "dispenser",
            "lever",
            "tnt",
            "redstone_lamp",
            "daylight_detector",
            "target",
            "note_block",
            "tripwire_hook",
            "redstone_torch",
            "slime_block",
            "honey_block",
            "redstone_block",
            "rail",
            "lightning_rod",
            "crafter",
            "jukebox",
            "sculk_sensor",
            "calibrated_sculk_sensor",
            "trapped_chest",
            "tripwire",
        ])
    {
        return Category::Redstone;
    }
    if ends(&["_stew", "_soup", "_pie", "_berries"])
        || s.starts_with("cooked_")
        || is(&[
            "bread",
            "cake",
            "cookie",
            "baked_potato",
            "apple",
            "golden_apple",
            "enchanted_golden_apple",
            "golden_carrot",
            "carrot",
            "potato",
            "poisonous_potato",
            "beetroot",
            "melon_slice",
            "glistering_melon_slice",
            "dried_kelp",
            "honey_bottle",
            "chorus_fruit",
            "popped_chorus_fruit",
            "beef",
            "porkchop",
            "chicken",
            "mutton",
            "rabbit",
            "cod",
            "salmon",
            "tropical_fish",
            "pufferfish",
            "rotten_flesh",
            "spider_eye",
        ])
    {
        return Category::Food;
    }
    if ends(&[
        "planks",
        "_stairs",
        "_slab",
        "_wall",
        "_fence",
        "_fence_gate",
        "_door",
        "_trapdoor",
        "bricks",
        "_brick",
        "concrete",
        "concrete_powder",
        "terracotta",
        "glass",
        "glass_pane",
        "_wool",
        "_carpet",
        "_log",
        "_wood",
        "_stem",
        "_hyphae",
        "_stone",
        "sandstone",
        "_block",
        "_pillar",
    ]) || is(&[
        "stone",
        "cobblestone",
        "dirt",
        "sand",
        "red_sand",
        "gravel",
        "clay",
        "brick",
        "granite",
        "diorite",
        "andesite",
        "deepslate",
        "cobbled_deepslate",
        "obsidian",
        "netherrack",
        "end_stone",
        "glowstone",
    ]) {
        return Category::Building;
    }
    Category::Misc
}
