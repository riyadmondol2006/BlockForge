//! Saved plans, the player's stock, and writing a plan out as text or JSON.
//!
//! On disk, under `<configs>/craft/`:
//! - `plans.json`: `{"schema":1,"plans":[...]}`, the saved plans (request, ticked-off keys, timestamps).
//! - `stock.json`: `{"schema":1,"stock":{"<game version>":{"<item id>":quantity}}}`.
//!
//! A file that cannot be read back (broken JSON, a newer schema) is renamed to `<name>.bad-<timestamp>` and the
//! store starts empty, so nothing the player saved is ever overwritten. Single entries that fail validation are
//! dropped, and the original file is then copied to `.bad-<timestamp>` first. Saves go through a temp file in the
//! same folder and a rename.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::types::*;
use crate::paths::Dirs;

/// The version of the files written here. A file with a bigger number is left alone.
pub const SCHEMA: u32 = 1;
/// The biggest plan file [`import_plan`] reads.
pub const MAX_PLAN_FILE: usize = 5 * 1024 * 1024;
/// The biggest store file [`load`] reads.
const MAX_STORE_FILE: u64 = 64 * 1024 * 1024;
/// The longest plan name, in characters.
pub const MAX_NAME: usize = 80;
const MAX_ENTRIES: usize = 20_000;
const FALLBACK_NAME: &str = "Untitled plan";

/// One saved plan: the request, and which gather/craft items were ticked off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedPlan {
    pub id: u64,
    pub name: String,
    pub version: String,
    pub request: Request,
    /// `gather:<item>` and `step:<recipe id>` keys that were ticked.
    pub done: BTreeSet<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Store {
    pub plans: Vec<SavedPlan>,
    /// Per game version, the items the player says they own.
    pub stock: BTreeMap<String, BTreeMap<ItemId, u64>>,
}

// ---------------------------------------------------------------------------------------------------------
// Store helpers

static NO_STOCK: BTreeMap<ItemId, u64> = BTreeMap::new();

impl Store {
    pub fn get(&self, id: u64) -> Option<&SavedPlan> {
        self.plans.iter().find(|p| p.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut SavedPlan> {
        self.plans.iter_mut().find(|p| p.id == id)
    }

    /// Add a plan: it gets a fresh unique id (its creation time in milliseconds, bumped when taken) and a unique
    /// trimmed name. Returns the id.
    pub fn add(&mut self, mut plan: SavedPlan) -> u64 {
        plan.name = unique_name(self, &plan.name);
        let mut id = if plan.created_ms > 0 { plan.created_ms as u64 } else { now_ms().max(1) as u64 };
        while self.plans.iter().any(|p| p.id == id) {
            id += 1;
        }
        plan.id = id;
        self.plans.push(plan);
        id
    }

    /// Give a plan a new name (trimmed, made unique among the others). Returns the name it got.
    pub fn rename(&mut self, id: u64, name: &str, now_ms: i64) -> Option<String> {
        let name = unique_name_except(self, name, Some(id));
        let plan = self.get_mut(id)?;
        plan.name = name.clone();
        plan.updated_ms = now_ms;
        Some(name)
    }

    /// Copy a plan under a unique name, without its ticked-off items. Returns the id of the copy.
    pub fn duplicate(&mut self, id: u64, now_ms: i64) -> Option<u64> {
        let mut copy = self.get(id)?.clone();
        copy.done.clear();
        copy.created_ms = now_ms;
        copy.updated_ms = now_ms;
        Some(self.add(copy))
    }

    pub fn delete(&mut self, id: u64) -> bool {
        let before = self.plans.len();
        self.plans.retain(|p| p.id != id);
        self.plans.len() != before
    }

    /// Say how many of an item the player owns in a game version. Zero removes the entry.
    pub fn set_stock(&mut self, version: &str, item: &str, qty: u64) {
        if qty == 0 {
            if let Some(map) = self.stock.get_mut(version) {
                map.remove(item);
                if map.is_empty() {
                    self.stock.remove(version);
                }
            }
        } else {
            self.stock.entry(version.to_string()).or_default().insert(item.to_string(), qty.min(MAX_QUANTITY));
        }
    }

    pub fn stock_for(&self, version: &str) -> &BTreeMap<ItemId, u64> {
        self.stock.get(version).unwrap_or(&NO_STOCK)
    }
}

/// A name for a new plan: trimmed, cut to [`MAX_NAME`] characters, and numbered ("Name (2)") when another plan
/// already has it (letters are compared without case).
pub fn unique_name(store: &Store, wanted: &str) -> String {
    unique_name_except(store, wanted, None)
}

fn unique_name_except(store: &Store, wanted: &str, except: Option<u64>) -> String {
    let mut base: String = wanted.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_NAME).collect();
    if base.is_empty() {
        base = FALLBACK_NAME.to_string();
    }
    let taken = |n: &str| store.plans.iter().any(|p| Some(p.id) != except && p.name.to_lowercase() == n.to_lowercase());
    if !taken(&base) {
        return base;
    }
    // "Name (2)" is numbered again from "Name", not turned into "Name (2) (2)"
    let stem = strip_number(&base).trim_end().to_string();
    for n in 2u32.. {
        let tail = format!(" ({n})");
        let keep = MAX_NAME.saturating_sub(tail.chars().count());
        let cand = format!("{}{}", stem.chars().take(keep).collect::<String>().trim_end(), tail);
        if !taken(&cand) {
            return cand;
        }
    }
    unreachable!()
}

/// `Name (12)` -> `Name`; anything else comes back as it is.
fn strip_number(name: &str) -> &str {
    if let Some(open) = name.rfind(" (")
        && let Some(num) = name[open + 2..].strip_suffix(')')
        && !num.is_empty()
        && num.bytes().all(|b| b.is_ascii_digit())
        && open > 0
    {
        return &name[..open];
    }
    name
}

// ---------------------------------------------------------------------------------------------------------
// Validation

/// `namespace:path` in lowercase, like `minecraft:oak_planks`.
pub fn valid_item_id(id: &str) -> bool {
    let Some((ns, path)) = id.split_once(':') else { return false };
    id.len() <= 128
        && !ns.is_empty()
        && !path.is_empty()
        && ns.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-'))
        && path.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'/'))
}

fn valid_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 40
        && v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b' '))
}

/// A recipe id or an ingredient key: free text, but short and printable.
fn valid_key(s: &str, max: usize) -> bool {
    !s.is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)
}

fn check_quantity(what: &str, q: u64) -> Result<(), String> {
    if q == 0 || q > MAX_QUANTITY {
        return Err(format!("{what} must be between 1 and {}", thousands(MAX_QUANTITY)));
    }
    Ok(())
}

fn check_item(id: &str) -> Result<(), String> {
    if valid_item_id(id) { Ok(()) } else { Err(format!("\"{}\" is not an item id", shorten(id))) }
}

fn shorten(s: &str) -> String {
    let t: String = s.chars().filter(|c| !c.is_control()).take(40).collect();
    if s.chars().count() > 40 { format!("{t}...") } else { t }
}

/// Check a plan from outside and tidy its name. Does not touch ids or timestamps.
fn validate(mut plan: SavedPlan) -> Result<SavedPlan, String> {
    if !valid_version(&plan.version) {
        return Err("The game version is missing or not valid".into());
    }
    check_item(&plan.request.item)?;
    check_quantity("The quantity", plan.request.quantity)?;
    let r = &plan.request;
    if r.inventory.len() + r.recipe_choices.len() + r.ingredient_choices.len() + r.supplied.len() + plan.done.len()
        > MAX_ENTRIES
    {
        return Err("The plan has too many entries".into());
    }
    for (id, q) in &r.inventory {
        check_item(id)?;
        check_quantity("An inventory quantity", *q)?;
    }
    for (item, recipe) in &r.recipe_choices {
        check_item(item)?;
        if !valid_key(recipe, 200) {
            return Err("A recipe choice is not valid".into());
        }
    }
    for (key, item) in &r.ingredient_choices {
        if !valid_key(key, 300) {
            return Err("An ingredient choice is not valid".into());
        }
        check_item(item)?;
    }
    for id in &r.supplied {
        check_item(id)?;
    }
    if plan.done.iter().any(|k| !valid_key(k, 300)) {
        return Err("A ticked-off entry is not valid".into());
    }
    plan.name = unique_name(&Store::default(), &plan.name);
    Ok(plan)
}

// ---------------------------------------------------------------------------------------------------------
// File formats

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RequestDto {
    item: String,
    quantity: u64,
    inventory: BTreeMap<String, u64>,
    recipe_choices: BTreeMap<String, String>,
    ingredient_choices: BTreeMap<String, String>,
    supplied: Vec<String>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct PlanDto {
    id: u64,
    name: String,
    version: String,
    request: RequestDto,
    done: Vec<String>,
    created_ms: i64,
    updated_ms: i64,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct PlansFile {
    schema: u32,
    plans: Vec<Value>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct StockFile {
    schema: u32,
    stock: BTreeMap<String, BTreeMap<String, u64>>,
}

/// A plan file written by [`export_plan`].
#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct PlanExport {
    blockforge_craft_plan: u32,
    name: String,
    version: String,
    request: RequestDto,
    done: Vec<String>,
}

impl From<&Request> for RequestDto {
    fn from(r: &Request) -> RequestDto {
        RequestDto {
            item: r.item.clone(),
            quantity: r.quantity,
            inventory: r.inventory.iter().filter(|(_, q)| **q > 0).map(|(k, q)| (k.clone(), *q)).collect(),
            recipe_choices: r.recipe_choices.clone(),
            ingredient_choices: r.ingredient_choices.clone(),
            supplied: r.supplied.iter().cloned().collect(),
        }
    }
}

impl From<RequestDto> for Request {
    fn from(d: RequestDto) -> Request {
        Request {
            item: d.item,
            quantity: d.quantity,
            inventory: d.inventory,
            recipe_choices: d.recipe_choices,
            ingredient_choices: d.ingredient_choices,
            supplied: d.supplied.into_iter().collect(),
        }
    }
}

impl From<&SavedPlan> for PlanDto {
    fn from(p: &SavedPlan) -> PlanDto {
        PlanDto {
            id: p.id,
            name: p.name.clone(),
            version: p.version.clone(),
            request: (&p.request).into(),
            done: p.done.iter().cloned().collect(),
            created_ms: p.created_ms,
            updated_ms: p.updated_ms,
        }
    }
}

impl From<PlanDto> for SavedPlan {
    fn from(d: PlanDto) -> SavedPlan {
        SavedPlan {
            id: d.id,
            name: d.name,
            version: d.version,
            request: d.request.into(),
            done: d.done.into_iter().collect(),
            created_ms: d.created_ms,
            updated_ms: d.updated_ms,
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// Loading and saving

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The folder the files live in.
pub fn craft_dir(dirs: &Dirs) -> PathBuf {
    dirs.configs.join("craft")
}

fn plans_path(dirs: &Dirs) -> PathBuf {
    craft_dir(dirs).join("plans.json")
}

fn stock_path(dirs: &Dirs) -> PathBuf {
    craft_dir(dirs).join("stock.json")
}

/// Read the saved plans and the stock. Never fails: what cannot be read is set aside (see the module docs).
pub fn load(dirs: &Dirs) -> Store {
    let mut store = Store::default();
    if let Some(file) = read_file::<PlansFile>(&plans_path(dirs)) {
        let mut dropped = false;
        for value in file.value.plans {
            let plan = serde_json::from_value::<PlanDto>(value)
                .map_err(|e| e.to_string())
                .and_then(|d| validate(SavedPlan::from(d)));
            match plan {
                Ok(mut p) if p.id != 0 && store.get(p.id).is_none() => {
                    // names come back as saved, only made unique
                    p.name = unique_name(&store, &p.name);
                    store.plans.push(p);
                }
                _ => dropped = true,
            }
        }
        if dropped {
            keep_copy(&file.path);
        }
    }
    if let Some(file) = read_file::<StockFile>(&stock_path(dirs)) {
        let mut dropped = false;
        for (version, items) in file.value.stock {
            for (item, qty) in items {
                if valid_version(&version) && valid_item_id(&item) && (1..=MAX_QUANTITY).contains(&qty) {
                    store.stock.entry(version.clone()).or_default().insert(item, qty);
                } else {
                    dropped = true;
                }
            }
        }
        if dropped {
            keep_copy(&file.path);
        }
    }
    store
}

struct Read<T> {
    value: T,
    path: PathBuf,
}

/// Read and parse one store file. A missing file is `None`; a broken or newer one is renamed aside and is `None`.
fn read_file<T: for<'a> Deserialize<'a> + Default + HasSchema>(path: &Path) -> Option<Read<T>> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    if meta.len() > MAX_STORE_FILE {
        set_aside(path);
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice::<T>(&bytes) {
        Ok(v) if v.schema() == SCHEMA => Some(Read { value: v, path: path.to_path_buf() }),
        _ => {
            set_aside(path);
            None
        }
    }
}

trait HasSchema {
    fn schema(&self) -> u32;
}
impl HasSchema for PlansFile {
    fn schema(&self) -> u32 {
        self.schema
    }
}
impl HasSchema for StockFile {
    fn schema(&self) -> u32 {
        self.schema
    }
}

fn bad_name(path: &Path) -> PathBuf {
    let ts = now_ms();
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut target = path.with_file_name(format!("{file}.bad-{ts}"));
    let mut n = 1;
    while target.exists() {
        target = path.with_file_name(format!("{file}.bad-{ts}-{n}"));
        n += 1;
    }
    target
}

/// Move a file that cannot be used out of the way, without changing it.
fn set_aside(path: &Path) {
    let _ = std::fs::rename(path, bad_name(path));
}

/// Keep a copy of a file that is about to lose entries.
fn keep_copy(path: &Path) {
    let _ = std::fs::copy(path, bad_name(path));
}

/// Write the whole store: plans and stock, each through a temp file and a rename.
pub fn save(dirs: &Dirs, store: &Store) -> Result<(), String> {
    let dir = craft_dir(dirs);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let plans = PlansFile {
        schema: SCHEMA,
        plans: store
            .plans
            .iter()
            .map(|p| serde_json::to_value(PlanDto::from(p)).map_err(|e| e.to_string()))
            .collect::<Result<_, _>>()?,
    };
    let stock = StockFile { schema: SCHEMA, stock: store.stock.clone() };
    write_atomic(&plans_path(dirs), &serde_json::to_vec_pretty(&plans).map_err(|e| e.to_string())?)?;
    write_atomic(&stock_path(dirs), &serde_json::to_vec_pretty(&stock).map_err(|e| e.to_string())?)
}

/// Write a file through a temp file next to it, so a crash never leaves half a file behind.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Could not write {}: {e}", path.display()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// Plan files

/// A plan file: versioned JSON of the request and the ticked-off items, to keep or share.
pub fn export_plan(plan: &SavedPlan) -> String {
    let file = PlanExport {
        blockforge_craft_plan: 1,
        name: plan.name.clone(),
        version: plan.version.clone(),
        request: (&plan.request).into(),
        done: plan.done.iter().cloned().collect(),
    };
    serde_json::to_string_pretty(&file).unwrap_or_default()
}

/// Read a plan file. The plan comes back with id 0 (so [`Store::add`] gives it a real one) and both timestamps set
/// to `now_ms`.
pub fn import_plan(text: &str, now_ms: i64) -> Result<SavedPlan, String> {
    if text.len() > MAX_PLAN_FILE {
        return Err(format!("The file is too big (the limit is {} MB)", MAX_PLAN_FILE / 1024 / 1024));
    }
    let value: Value =
        serde_json::from_str(text).map_err(|_| "This is not a plan file (the JSON is broken)".to_string())?;
    match value.get("blockforgeCraftPlan") {
        None => return Err("This is not a BlockForge plan file".into()),
        Some(v) if v.as_u64() == Some(1) => {}
        Some(_) => return Err("This plan file was made by a newer BlockForge".into()),
    }
    let file: PlanExport = serde_json::from_value(value).map_err(|e| format!("The plan file is not valid: {e}"))?;
    let plan = SavedPlan {
        id: 0,
        name: file.name,
        version: file.version,
        request: file.request.into(),
        done: file.done.into_iter().collect(),
        created_ms: now_ms,
        updated_ms: now_ms,
    };
    validate(plan)
}

// ---------------------------------------------------------------------------------------------------------
// Text

/// `2500` -> `2,500`.
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A count as full stacks and a remainder: `5`, `1 stack`, `2 stacks + 12`. Items that do not stack are a plain
/// count.
pub fn stacks_text(qty: u64, stack: u32) -> String {
    let stack = u64::from(stack);
    if stack <= 1 || qty < stack {
        return thousands(qty);
    }
    let (full, rest) = (qty / stack, qty % stack);
    let mut s = format!("{} stack{}", thousands(full), if full == 1 { "" } else { "s" });
    if rest > 0 {
        let _ = write!(s, " + {}", thousands(rest));
    }
    s
}

/// `2,500 (39 stacks + 4)`, or just the count when it fits one stack.
fn qty_with_stacks(qty: u64, stack: u32) -> String {
    if u64::from(stack) <= 1 || qty < u64::from(stack) {
        thousands(qty)
    } else {
        format!("{} ({})", thousands(qty), stacks_text(qty, stack))
    }
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", thousands(n), if n == 1 { one } else { many })
}

/// The shopping list as plain text, to paste into a chat or a note.
pub fn to_text(ds: &Dataset, plan: &Plan) -> String {
    text_with_storage(ds, plan, &super::stack::storage(ds, &plan.materials))
}

fn text_with_storage(ds: &Dataset, plan: &Plan, storage: &Storage) -> String {
    let mut o = String::new();
    let req = &plan.request;
    let _ = writeln!(o, "{} x {} (Minecraft {})", thousands(req.quantity), ds.name(&req.item), ds.version);

    o.push_str("\nGather\n");
    if plan.materials.is_empty() {
        o.push_str("- Nothing: you have it all\n");
    }
    for (id, qty) in &plan.materials {
        let _ = writeln!(o, "- {} x {}", ds.name(id), qty_with_stacks(*qty, ds.stack(id)));
    }

    if !plan.steps.is_empty() {
        o.push_str("\nCraft/Smelt in order\n");
        for (i, s) in plan.steps.iter().enumerate() {
            let inputs = s
                .inputs
                .iter()
                .map(|(id, q)| format!("{} {}", thousands(*q), ds.name(id)))
                .collect::<Vec<_>>()
                .join(" + ");
            let _ = writeln!(
                o,
                "{}. {} {} ({}): {} -> {} {}",
                i + 1,
                s.kind.verb(),
                plural(s.operations, "time", "times"),
                s.kind.station(),
                inputs,
                thousands(s.output),
                ds.name(&s.item),
            );
        }
    }

    let mut by: BTreeMap<&str, u64> = BTreeMap::new();
    for s in &plan.steps {
        for (id, q) in &s.byproducts {
            *by.entry(id).or_default() += q;
        }
    }
    if !by.is_empty() {
        o.push_str("\nBy-products\n");
        for (id, q) in by {
            let _ = writeln!(o, "- {} x {}", ds.name(id), thousands(q));
        }
    }

    if plan.cook_ticks > 0 && !plan.fuel.is_empty() {
        let fuels = plan
            .fuel
            .iter()
            .map(|f| format!("{} {}", thousands(f.count), ds.name(&f.item)))
            .collect::<Vec<_>>()
            .join(" or ");
        let _ = write!(o, "\nFuel: about {fuels} if smelted in one furnace\n");
    }

    let mut warnings = plan.warnings.clone();
    if !plan.complete {
        warnings.push("This list is not complete: some items have no recipe or need hands-on work.".into());
    }
    if !warnings.is_empty() {
        o.push_str("\nWarnings\n");
        for w in &warnings {
            let _ = writeln!(o, "- {w}");
        }
    }

    if storage.slots > 0 {
        let _ = write!(o, "\nStorage ({})\n", plural(storage.slots, "slot", "slots"));
        let _ = writeln!(o, "- Single chests: {}", thousands(storage.single_chests.count));
        let _ = writeln!(o, "- Double chests: {}", thousands(storage.double_chests.count));
        let _ = writeln!(o, "- Shulker boxes: {}", thousands(storage.shulker_boxes.count));
        let _ = writeln!(o, "- Player inventories: {}", thousands(storage.player_inventories.count));
    }

    o.push_str("\nMade with BlockForge\n");
    o
}

/// Just the gather list, one `Name x qty` per line, to paste.
pub fn clipboard_list(ds: &Dataset, plan: &Plan) -> String {
    let mut o = String::new();
    for (id, qty) in &plan.materials {
        let _ = writeln!(o, "{} x {qty}", ds.name(id));
    }
    o
}

// ---------------------------------------------------------------------------------------------------------
// JSON

/// The whole plan as JSON, to keep or share.
pub fn to_json(ds: &Dataset, plan: &Plan) -> String {
    json_with_storage(ds, plan, &super::stack::storage(ds, &plan.materials))
}

fn json_with_storage(ds: &Dataset, plan: &Plan, storage: &Storage) -> String {
    let qty = |rows: &[(ItemId, u64)]| -> Vec<Value> {
        rows.iter()
            .map(|(id, q)| json!({ "item": id, "name": ds.name(id), "quantity": q, "stack": ds.stack(id) }))
            .collect()
    };
    let req = &plan.request;
    let containers = |c: &Containers| json!({ "count": c.count, "capacity": c.capacity, "unused": c.unused });
    let value = json!({
        "blockforgeCraftExport": 1,
        "version": ds.version,
        "request": {
            "item": req.item,
            "name": ds.name(&req.item),
            "quantity": req.quantity,
            "inventory": req.inventory,
            "recipeChoices": req.recipe_choices,
            "ingredientChoices": req.ingredient_choices,
            "supplied": req.supplied,
        },
        "complete": plan.complete,
        "operations": plan.operations,
        "materials": qty(&plan.materials),
        "intermediates": qty(&plan.intermediates),
        "steps": plan.steps.iter().map(|s| json!({
            "recipe": s.recipe,
            "item": s.item,
            "name": ds.name(&s.item),
            "kind": s.kind.verb(),
            "station": s.kind.station(),
            "operations": s.operations,
            "output": s.output,
            "inputs": qty(&s.inputs),
            "byproducts": qty(&s.byproducts),
            "ticks": s.ticks,
        })).collect::<Vec<_>>(),
        "ledger": plan.ledger.iter().map(|r| json!({
            "item": r.item,
            "name": ds.name(&r.item),
            "required": r.required,
            "fromStock": r.from_stock,
            "fromSurplus": r.from_surplus,
            "produced": r.produced,
            "gather": r.gather,
            "surplus": r.surplus,
        })).collect::<Vec<_>>(),
        "cookTicks": plan.cook_ticks,
        "fuel": plan.fuel.iter().map(|f| json!({ "item": f.item, "name": ds.name(&f.item), "count": f.count }))
            .collect::<Vec<_>>(),
        "warnings": plan.warnings,
        "storage": {
            "slots": storage.slots,
            "rows": storage.rows.iter().map(|r| json!({
                "item": r.item,
                "name": ds.name(&r.item),
                "quantity": r.quantity,
                "stack": r.stack,
                "fullStacks": r.full_stacks,
                "partial": r.partial,
                "slots": r.slots,
            })).collect::<Vec<_>>(),
            "singleChests": containers(&storage.single_chests),
            "doubleChests": containers(&storage.double_chests),
            "shulkerBoxes": containers(&storage.shulker_boxes),
            "playerInventories": containers(&storage.player_inventories),
        },
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

#[cfg(test)]
#[path = "saved_tests.rs"]
mod saved_tests;
