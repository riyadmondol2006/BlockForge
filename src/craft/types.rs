//! The data types the parts of the crafting calculator share: the recipe book of one game version, a request and the
//! plan that answers it.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// A namespaced item id such as `minecraft:oak_planks`.
pub type ItemId = String;

/// The most items one request may ask for, and the biggest number any step of the maths may reach.
pub const MAX_QUANTITY: u64 = 1_000_000_000;
pub const MAX_INTERNAL: u64 = 1_000_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Building,
    Redstone,
    Equipment,
    Food,
    Misc,
}

impl Category {
    pub const ALL: [Category; 5] =
        [Category::Building, Category::Redstone, Category::Equipment, Category::Food, Category::Misc];
    pub fn label(self) -> &'static str {
        match self {
            Category::Building => "Building",
            Category::Redstone => "Redstone",
            Category::Equipment => "Equipment",
            Category::Food => "Food",
            Category::Misc => "Misc",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: ItemId,
    /// The English name from the game's own language file, or the id made readable when there is none.
    pub name: String,
    /// How many fit in one inventory slot.
    pub stack: u32,
    pub category: Category,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Shaped,
    Shapeless,
    Smelting,
    Blasting,
    Smoking,
    Campfire,
    Stonecutting,
    /// Smithing table upgrades: kept as manual steps, they keep the item's enchantments and name.
    Smithing,
    /// A recipe that turns one item into another with a material (dyeing a bundle or a shulker box).
    Transmute,
}

impl Kind {
    pub fn station(self) -> &'static str {
        match self {
            Kind::Shaped | Kind::Shapeless | Kind::Transmute => "Crafting table",
            Kind::Smelting => "Furnace",
            Kind::Blasting => "Blast furnace",
            Kind::Smoking => "Smoker",
            Kind::Campfire => "Campfire",
            Kind::Stonecutting => "Stonecutter",
            Kind::Smithing => "Smithing table",
        }
    }
    /// What the step is called in a list ("Craft", "Smelt", ...).
    pub fn verb(self) -> &'static str {
        match self {
            Kind::Shaped | Kind::Shapeless | Kind::Transmute => "Craft",
            Kind::Smelting => "Smelt",
            Kind::Blasting => "Blast",
            Kind::Smoking => "Smoke",
            Kind::Campfire => "Cook",
            Kind::Stonecutting => "Cut",
            Kind::Smithing => "Upgrade",
        }
    }
    /// Whether the step cooks something (and so burns fuel).
    pub fn burns_fuel(self) -> bool {
        matches!(self, Kind::Smelting | Kind::Blasting | Kind::Smoking)
    }
}

/// One slot kind of a recipe: `count` items, each one of the `alternatives` (the planner picks one for the whole
/// ingredient, it never charges for every alternative).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ingredient {
    /// Stable inside its recipe: the key symbol of a shaped recipe, `#<index>` of a shapeless one, `input`, ...
    pub key: String,
    pub count: u32,
    /// Sorted, without duplicates, never empty.
    pub alternatives: Vec<ItemId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipe {
    /// The file name of the recipe without the extension: `iron_ingot_from_smelting_raw_iron`. Unique per version.
    pub id: String,
    pub kind: Kind,
    pub output: ItemId,
    pub count: u32,
    pub ingredients: Vec<Ingredient>,
    /// Why the planner leaves this recipe to the player (smithing keeps enchantments), when it does.
    pub manual: Option<String>,
    /// Ticks one operation of a smelting-type recipe takes.
    pub ticks: Option<u32>,
    /// A shaped recipe that fits the 2x2 grid of the inventory (or a shapeless one with up to four slots).
    pub small_grid: bool,
}

/// The recipe book of one game version.
#[derive(Debug, Default)]
pub struct Dataset {
    pub version: String,
    pub items: BTreeMap<ItemId, Item>,
    pub recipes: Vec<Recipe>,
    by_id: HashMap<String, usize>,
    by_output: HashMap<ItemId, Vec<usize>>,
    /// Items the planner stops at and tells the player to gather (ores, logs, wool, mob drops, ...).
    pub raw_stops: HashSet<ItemId>,
    /// Recipes left out and why (dynamic ones such as firework rockets, brewing, armor trims).
    pub skipped: Vec<(String, String)>,
}

impl Dataset {
    /// Check the references, index the recipes and order the recipes of each item by how sensible they are as the
    /// default (building beats unpacking a block, smelting beats blasting, ...).
    pub fn new(
        version: &str,
        items: BTreeMap<ItemId, Item>,
        recipes: Vec<Recipe>,
        raw_stops: HashSet<ItemId>,
        skipped: Vec<(String, String)>,
    ) -> Result<Dataset, String> {
        let mut by_id = HashMap::new();
        for (i, r) in recipes.iter().enumerate() {
            if by_id.insert(r.id.clone(), i).is_some() {
                return Err(format!("Two recipes are called {}", r.id));
            }
            if r.count == 0 {
                return Err(format!("Recipe {} makes nothing", r.id));
            }
            let mut ids = vec![&r.output];
            ids.extend(r.ingredients.iter().flat_map(|i| i.alternatives.iter()));
            for id in ids {
                if !items.contains_key(id) {
                    return Err(format!("Recipe {} uses {id}, which is not an item of {version}", r.id));
                }
            }
            if r.manual.is_none() && r.ingredients.is_empty() {
                return Err(format!("Recipe {} has no ingredients", r.id));
            }
            if r.ingredients.iter().any(|i| i.alternatives.is_empty() || i.count == 0) {
                return Err(format!("Recipe {} has an empty ingredient", r.id));
            }
        }
        let mut by_output: HashMap<ItemId, Vec<usize>> = HashMap::new();
        for (i, r) in recipes.iter().enumerate() {
            by_output.entry(r.output.clone()).or_default().push(i);
        }
        for list in by_output.values_mut() {
            list.sort_by(|&a, &b| {
                let (ra, rb) = (&recipes[a], &recipes[b]);
                score(ra).cmp(&score(rb)).then_with(|| ra.id.cmp(&rb.id))
            });
        }
        let raw_stops = raw_stops.into_iter().filter(|id| items.contains_key(id)).collect();
        Ok(Dataset { version: version.to_string(), items, recipes, by_id, by_output, raw_stops, skipped })
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.get(id)
    }
    /// The display name of an item (a readable form of the id for one that is not in the book).
    pub fn name(&self, id: &str) -> String {
        match self.items.get(id) {
            Some(i) => i.name.clone(),
            None => pretty_name(id),
        }
    }
    pub fn stack(&self, id: &str) -> u32 {
        self.items.get(id).map(|i| i.stack).unwrap_or(64)
    }
    pub fn recipe(&self, id: &str) -> Option<&Recipe> {
        self.by_id.get(id).map(|&i| &self.recipes[i])
    }
    /// The recipes that make `item`, the best default first.
    pub fn recipes_for(&self, item: &str) -> impl Iterator<Item = &Recipe> {
        self.by_output.get(item).into_iter().flatten().map(|&i| &self.recipes[i])
    }
    pub fn has_recipe(&self, item: &str) -> bool {
        self.by_output.contains_key(item)
    }
}

/// `oak_planks` -> `Oak Planks`.
pub fn pretty_name(id: &str) -> String {
    let short = id.rsplit(':').next().unwrap_or(id);
    short
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lower is better: the recipe that is the natural way to get the item comes first.
fn score(r: &Recipe) -> u32 {
    let single_in = |suffixes: &[&str]| {
        r.ingredients.len() == 1
            && suffixes.iter().any(|s| r.ingredients[0].alternatives.iter().all(|a| a.ends_with(s)))
    };
    // taking a block, ingot or nugget apart again is never how you get the item
    let unpacking = single_in(&["_block", "_ingot", "_nugget"]) && r.count >= 9;
    // an ingot from nuggets: the way back from the ingot
    let nuggets = r.output.ends_with("_ingot")
        && !r.ingredients.is_empty()
        && r.ingredients.iter().all(|i| i.alternatives.iter().all(|a| a.ends_with("_nugget")));
    // sugar from honey bottles is an alternative to sugar cane, not the main way
    let honey = r.id == "sugar_from_honey_bottle";
    // an ore block in a furnace is the older way: the raw metal is what you mine now
    let ore = matches!(r.kind, Kind::Smelting | Kind::Blasting | Kind::Smoking | Kind::Campfire)
        && r.ingredients.iter().all(|i| i.alternatives.iter().all(|a| a.ends_with("_ore")));
    // nuggets from smelting tools or armor are a way to recycle them, not to make nuggets
    let recycling = matches!(r.kind, Kind::Smelting | Kind::Blasting) && r.output.ends_with("_nugget");
    let kind = match r.kind {
        Kind::Shaped => 0,
        Kind::Shapeless => 1,
        Kind::Transmute => 2,
        Kind::Smelting => 3,
        Kind::Smoking => 4,
        Kind::Blasting => 5,
        Kind::Campfire => 6,
        Kind::Stonecutting => 7,
        Kind::Smithing => 8,
    };
    (if r.manual.is_some() { 100 } else { 0 })
        + (if unpacking || nuggets { 20 } else { 0 })
        + (if honey { 10 } else { 0 })
        + (if ore { 1 } else { 0 })
        + (if recycling { 30 } else { 0 })
        + kind
}

// ---------------------------------------------------------------------------------------------------------
// Requests and plans

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub item: ItemId,
    pub quantity: u64,
    /// What the player already has.
    pub inventory: BTreeMap<ItemId, u64>,
    /// Item -> id of the recipe to use for it instead of the default.
    pub recipe_choices: BTreeMap<ItemId, String>,
    /// `recipe id:ingredient key` -> the alternative to use for that ingredient.
    pub ingredient_choices: BTreeMap<String, ItemId>,
    /// Items the player will bring from elsewhere: the planner stops there and lists them to gather.
    pub supplied: BTreeSet<ItemId>,
}

/// Why a branch of the tree ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stop {
    /// Everything came from the inventory (or the surplus of an earlier step).
    Inventory,
    /// A natural resource: gather it.
    Raw,
    /// The player said they will bring it.
    Supplied,
    /// Nothing in the recipe book makes it: get it some other way.
    NoRecipe,
    /// Its recipe needs hands-on handling (smithing).
    Manual,
    /// It needs itself; pick another recipe or supply it.
    Cycle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub item: ItemId,
    pub required: u64,
    pub from_stock: u64,
    pub from_surplus: u64,
    pub to_produce: u64,
    /// What the operations make together (at least `to_produce`).
    pub output: u64,
    pub surplus: u64,
    pub operations: u64,
    pub recipe: Option<String>,
    /// Whether the recipe is the default and not one the player picked.
    pub default_recipe: bool,
    pub stop: Option<Stop>,
    pub children: Vec<Node>,
}

/// Per item: `required = from_stock + gather + produced - surplus`, where `surplus` is what is left at the end (what
/// came out of the surplus on the way was produced earlier, so it is inside `produced`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerRow {
    pub item: ItemId,
    pub required: u64,
    pub from_stock: u64,
    pub from_surplus: u64,
    pub produced: u64,
    pub gather: u64,
    pub surplus: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub recipe: String,
    pub item: ItemId,
    pub kind: Kind,
    pub operations: u64,
    pub output: u64,
    /// What this step uses up (ingredients as chosen), summed over its operations.
    pub inputs: Vec<(ItemId, u64)>,
    /// Which alternative of each ingredient the plan used: (ingredient key, item). One entry per ingredient of the
    /// recipe, so the page can offer the other alternatives.
    pub choices: Vec<(String, ItemId)>,
    /// Containers that come back (buckets, bottles).
    pub byproducts: Vec<(ItemId, u64)>,
    /// Ticks the cooking of all operations takes in one furnace, for smelting-type steps.
    pub ticks: Option<u64>,
}

/// How much of one fuel the smelting of the plan burns, when it runs in a single furnace the whole time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuelEstimate {
    pub item: ItemId,
    pub count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub request: Request,
    pub tree: Node,
    pub ledger: Vec<LedgerRow>,
    /// What to gather: the leaves of the tree, summed, sorted by id.
    pub materials: Vec<(ItemId, u64)>,
    /// What is made on the way (everything crafted except the asked item), sorted by id.
    pub intermediates: Vec<(ItemId, u64)>,
    /// Dependency-first order: every step comes after the steps that make its inputs.
    pub steps: Vec<Step>,
    pub operations: u64,
    /// Total cooking ticks of the smelting steps, and the fuel that would burn.
    pub cook_ticks: u64,
    pub fuel: Vec<FuelEstimate>,
    pub warnings: Vec<String>,
    /// False when a branch is a cycle or a manual recipe, so the lists are not the whole truth.
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanError(pub String);

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for PlanError {}

/// What storing a list of items takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageRow {
    pub item: ItemId,
    pub quantity: u64,
    pub stack: u32,
    pub full_stacks: u64,
    pub partial: u32,
    pub slots: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Containers {
    pub count: u64,
    pub capacity: u64,
    pub unused: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Storage {
    pub rows: Vec<StorageRow>,
    pub slots: u64,
    pub single_chests: Containers,
    pub double_chests: Containers,
    pub shulker_boxes: Containers,
    pub player_inventories: Containers,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str) -> (ItemId, Item) {
        (id.to_string(), Item { id: id.to_string(), name: pretty_name(id), stack: 64, category: Category::Misc })
    }
    fn recipe(id: &str, kind: Kind, out: &str, count: u32, input: &str) -> Recipe {
        Recipe {
            id: id.to_string(),
            kind,
            output: out.to_string(),
            count,
            ingredients: vec![Ingredient { key: "input".into(), count: 1, alternatives: vec![input.to_string()] }],
            manual: None,
            ticks: None,
            small_grid: false,
        }
    }

    #[test]
    fn a_nugget_comes_from_an_ingot_not_from_smelting_armor() {
        let items = ["minecraft:iron_nugget", "minecraft:iron_ingot", "minecraft:chainmail_boots"]
            .into_iter()
            .map(item)
            .collect();
        let recipes = vec![
            recipe(
                "iron_nugget_from_smelting",
                Kind::Smelting,
                "minecraft:iron_nugget",
                1,
                "minecraft:chainmail_boots",
            ),
            recipe("iron_nugget", Kind::Shapeless, "minecraft:iron_nugget", 9, "minecraft:iron_ingot"),
        ];
        let ds = Dataset::new("t", items, recipes, HashSet::new(), Vec::new()).unwrap();
        let first = ds.recipes_for("minecraft:iron_nugget").next().unwrap();
        assert_eq!(first.id, "iron_nugget");
    }

    #[test]
    fn pretty_names() {
        assert_eq!(pretty_name("minecraft:oak_planks"), "Oak Planks");
    }
}
