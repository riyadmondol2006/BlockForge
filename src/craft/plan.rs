//! The calculator: a request in, a plan out.
//!
//! The method is a fixed, deterministic strategy and not an optimizer: a depth-first walk that uses what the player
//! has first, then what earlier batches left over, and only expands what is still missing. A recipe's own surplus
//! becomes usable after its ingredients are settled. Everything is whole numbers, checked against
//! [`MAX_INTERNAL`] at every step, and the books must balance per item at the end.

use std::collections::{BTreeMap, HashMap};

use super::types::*;

/// The deepest recipe chain, and the most nodes the tree may hold.
const MAX_DEPTH: usize = 64;
const MAX_NODES: u32 = 12_000;

/// Ingredient alternatives the planner takes first when a recipe accepts them (a fixed preference, not a search).
const PREFERRED: [&str; 8] = [
    "minecraft:oak_planks",
    "minecraft:oak_log",
    "minecraft:cobblestone",
    "minecraft:coal",
    "minecraft:white_wool",
    "minecraft:sandstone",
    "minecraft:red_sandstone",
    "minecraft:quartz_block",
];

/// Furnace fuels and how many ticks one of them burns (vanilla values), best known first.
const FUELS: [(&str, u64); 9] = [
    ("minecraft:coal", 1600),
    ("minecraft:charcoal", 1600),
    ("minecraft:oak_planks", 300),
    ("minecraft:oak_log", 300),
    ("minecraft:stick", 100),
    ("minecraft:blaze_rod", 2400),
    ("minecraft:dried_kelp_block", 4000),
    ("minecraft:lava_bucket", 20000),
    ("minecraft:coal_block", 16000),
];

fn err(text: &str) -> PlanError {
    PlanError(text.to_string())
}
fn overflow() -> PlanError {
    err("Calculation exceeds the safe quantity limit (1 trillion items).")
}
fn safe(n: u64) -> Result<u64, PlanError> {
    if n > MAX_INTERNAL { Err(overflow()) } else { Ok(n) }
}
fn add(a: u64, b: u64) -> Result<u64, PlanError> {
    safe(a.checked_add(b).ok_or_else(overflow)?)
}
fn mul(a: u64, b: u64) -> Result<u64, PlanError> {
    safe(a.checked_mul(b).ok_or_else(overflow)?)
}

/// Work out what it takes to make `req.quantity` of `req.item`.
pub fn calculate(ds: &Dataset, req: &Request) -> Result<Plan, PlanError> {
    calculate_with(ds, req, MAX_NODES, MAX_DEPTH)
}

fn validate(ds: &Dataset, req: &Request) -> Result<(), PlanError> {
    if req.quantity == 0 || req.quantity > MAX_QUANTITY {
        return Err(err(
            "Enter a positive whole quantity, up to 1,000,000,000. Inventory must contain non-negative whole quantities.",
        ));
    }
    for id in std::iter::once(&req.item).chain(req.inventory.keys()).chain(req.supplied.iter()) {
        if !ds.items.contains_key(id) {
            return Err(PlanError(format!("Item unavailable in this game version: {id}")));
        }
    }
    for have in req.inventory.values() {
        safe(*have)?;
    }
    for (item, id) in &req.recipe_choices {
        if ds.recipe(id).is_none_or(|r| &r.output != item) {
            return Err(PlanError(format!("Invalid selected recipe for {item}.")));
        }
    }
    for (key, item) in &req.ingredient_choices {
        // `<recipe id>:<ingredient key>`; try every colon in case a key has one
        let valid = key.match_indices(':').any(|(at, _)| {
            ds.recipe(&key[..at])
                .is_some_and(|r| r.ingredients.iter().any(|i| i.key == key[at + 1..] && i.alternatives.contains(item)))
        });
        if !valid {
            return Err(PlanError(format!("Invalid ingredient alternative: {key}.")));
        }
    }
    Ok(())
}

/// The per-item books while the tree is walked.
struct Row<'a> {
    item: &'a str,
    required: u64,
    from_stock: u64,
    from_surplus: u64,
    produced: u64,
    gather: u64,
    /// What is still in the player's inventory, and what earlier batches left over.
    inventory: u64,
    surplus: u64,
}

/// One recipe's step while its nodes are added up.
struct Acc<'a> {
    recipe: &'a Recipe,
    item: &'a str,
    operations: u64,
    output: u64,
    inputs: BTreeMap<&'a str, u64>,
    choices: Vec<(String, ItemId)>,
    byproducts: BTreeMap<&'a str, u64>,
}

struct Planner<'a> {
    ds: &'a Dataset,
    req: &'a Request,
    rows: Vec<Row<'a>>,
    row_of: HashMap<&'a str, usize>,
    steps: Vec<Acc<'a>>,
    step_of: HashMap<&'a str, usize>,
    warnings: Vec<String>,
    nodes: u32,
    max_nodes: u32,
    max_depth: usize,
    complete: bool,
    /// The items being expanded above the current node.
    path: Vec<&'a str>,
}

/// The container an ingredient leaves behind when a recipe uses it up.
fn remainder(id: &str) -> Option<&'static str> {
    if id == "minecraft:honey_bottle" {
        Some("minecraft:glass_bottle")
    } else if id.starts_with("minecraft:") && id.ends_with("_bucket") && id != "minecraft:bucket" {
        Some("minecraft:bucket")
    } else {
        None
    }
}

impl<'a> Planner<'a> {
    fn warn(&mut self, text: String) {
        if !self.warnings.contains(&text) {
            self.warnings.push(text);
        }
    }

    /// The alternative of `ing` the plan uses: the player's pick, then the fixed preference, then the shortest id
    /// that is not the recipe's own output and not already being made above (that would be a cycle).
    fn choose(&self, recipe: &'a Recipe, ing: &'a Ingredient) -> &'a str {
        if !self.req.ingredient_choices.is_empty() {
            let key = format!("{}:{}", recipe.id, ing.key);
            if let Some(item) = self.req.ingredient_choices.get(&key) {
                return item;
            }
        }
        let alts = &ing.alternatives;
        if alts.len() == 1 {
            return &alts[0];
        }
        for pref in PREFERRED {
            if let Some(a) = alts.iter().find(|a| *a == pref) {
                return a;
            }
        }
        let free = |a: &&String| **a != recipe.output && !self.path.contains(&a.as_str());
        let best =
            |it: &mut dyn Iterator<Item = &'a String>| it.min_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
        best(&mut alts.iter().filter(free)).or_else(|| best(&mut alts.iter())).expect("an ingredient has alternatives")
    }

    fn row(&mut self, item: &'a str) -> usize {
        if let Some(&i) = self.row_of.get(item) {
            return i;
        }
        let inventory = self.req.inventory.get(item).copied().unwrap_or(0);
        self.rows.push(Row {
            item,
            required: 0,
            from_stock: 0,
            from_surplus: 0,
            produced: 0,
            gather: 0,
            inventory,
            surplus: 0,
        });
        self.row_of.insert(item, self.rows.len() - 1);
        self.rows.len() - 1
    }

    fn visit(&mut self, item: &'a str, required: u64, depth: usize) -> Result<Node, PlanError> {
        self.nodes += 1;
        if self.nodes > self.max_nodes {
            return Err(err(
                "Calculation complexity limit exceeded. Use externally supplied ingredients to shorten the plan.",
            ));
        }
        if depth > self.max_depth {
            return Err(err("Recipe depth limit exceeded. Supply an intermediate item to shorten the plan."));
        }
        safe(required)?;
        let ds = self.ds;
        let req = self.req;
        let ri = self.row(item);
        let (from_stock, from_surplus, needed) = {
            let row = &mut self.rows[ri];
            row.required = add(row.required, required)?;
            let from_stock = required.min(row.inventory);
            row.inventory -= from_stock;
            let left = required - from_stock;
            let from_surplus = left.min(row.surplus);
            row.surplus -= from_surplus;
            row.from_stock = add(row.from_stock, from_stock)?;
            row.from_surplus = add(row.from_surplus, from_surplus)?;
            (from_stock, from_surplus, left - from_surplus)
        };
        let explicit = req.recipe_choices.get(item);
        let mut node = Node {
            item: item.to_string(),
            required,
            from_stock,
            from_surplus,
            to_produce: needed,
            output: 0,
            surplus: 0,
            operations: 0,
            recipe: None,
            default_recipe: explicit.is_none(),
            stop: None,
            children: Vec::new(),
        };
        if needed == 0 {
            node.stop = Some(Stop::Inventory);
            return Ok(node);
        }
        let recipe = match explicit {
            Some(id) => ds.recipe(id),
            None => ds.recipes_for(item).next(),
        };
        let supplied = req.supplied.contains(item);
        let raw = explicit.is_none() && ds.raw_stops.contains(item);
        let on_path = self.path.contains(&item);
        let manual = recipe.is_some_and(|r| r.manual.is_some());
        let recipe = match recipe {
            Some(r) if !(supplied || raw || manual || on_path) => r,
            _ => {
                let stop = if supplied {
                    Stop::Supplied
                } else if raw {
                    Stop::Raw
                } else if on_path {
                    Stop::Cycle
                } else if manual {
                    Stop::Manual
                } else {
                    Stop::NoRecipe
                };
                node.stop = Some(stop);
                let row = &mut self.rows[ri];
                row.gather = add(row.gather, needed)?;
                match stop {
                    Stop::Cycle => {
                        self.complete = false;
                        self.warn(format!(
                            "Recipe cycle at {}. Supply this ingredient externally or choose another recipe.",
                            ds.name(item)
                        ));
                    }
                    Stop::Manual => {
                        self.complete = false;
                        let r = recipe.expect("a manual stop has a recipe");
                        node.recipe = Some(r.id.clone());
                        let why = r.manual.as_deref().filter(|w| !w.is_empty());
                        self.warn(format!(
                            "{}: {}",
                            ds.name(item),
                            why.unwrap_or("Special recipe requires manual handling.")
                        ));
                    }
                    Stop::NoRecipe if depth == 0 => {
                        self.complete = false;
                        self.warn(format!(
                            "{}: no normal survival recipe found. Supply or gather this item directly.",
                            ds.name(item)
                        ));
                    }
                    _ => {}
                }
                return Ok(node);
            }
        };
        node.recipe = Some(recipe.id.clone());
        let count = recipe.count as u64;
        let operations = needed.div_ceil(count);
        let output = mul(operations, count)?;
        let surplus = output - needed;
        node.operations = operations;
        node.output = output;
        node.surplus = surplus;

        self.path.push(item);
        let chosen: Vec<(&'a str, u64)> = recipe
            .ingredients
            .iter()
            .map(|ing| Ok((self.choose(recipe, ing), mul(ing.count as u64, operations)?)))
            .collect::<Result<_, PlanError>>()?;
        let mut wanted: BTreeMap<&'a str, u64> = BTreeMap::new();
        for &(id, quantity) in &chosen {
            let total = wanted.entry(id).or_insert(0);
            *total = add(*total, quantity)?;
        }
        for (&id, &quantity) in &wanted {
            let child = self.visit(id, quantity, depth + 1)?;
            node.children.push(child);
        }
        self.path.pop();

        // the surplus can be used only now, after the ingredients are settled
        let row = &mut self.rows[ri];
        row.produced = add(row.produced, output)?;
        row.surplus = add(row.surplus, surplus)?;

        let si = match self.step_of.get(recipe.id.as_str()) {
            Some(&i) => i,
            None => {
                let choices = recipe
                    .ingredients
                    .iter()
                    .zip(&chosen)
                    .map(|(ing, (id, _))| (ing.key.clone(), id.to_string()))
                    .collect();
                self.steps.push(Acc {
                    recipe,
                    item: &recipe.output,
                    operations: 0,
                    output: 0,
                    inputs: BTreeMap::new(),
                    choices,
                    byproducts: BTreeMap::new(),
                });
                self.step_of.insert(&recipe.id, self.steps.len() - 1);
                self.steps.len() - 1
            }
        };
        let step = &mut self.steps[si];
        step.operations = add(step.operations, operations)?;
        step.output = add(step.output, output)?;
        for (ing, (id, quantity)) in recipe.ingredients.iter().zip(chosen) {
            let total = step.inputs.entry(id).or_insert(0);
            *total = add(*total, quantity)?;
            // a bucket or bottle that held the ingredient comes back once per item used
            if let Some(back) = remainder(id).filter(|b| ds.items.contains_key(*b)) {
                let total = step.byproducts.entry(back).or_insert(0);
                *total = add(*total, mul(ing.count as u64, operations)?)?;
            }
        }
        Ok(node)
    }
}

/// Order `steps` so each comes after the steps that make its inputs. A cycle is cut where it closes.
fn order(steps: Vec<Step>) -> Vec<Step> {
    let producers: HashMap<&str, usize> = steps.iter().enumerate().map(|(i, s)| (s.item.as_str(), i)).collect();
    // 0 = not seen, 1 = on the way down, 2 = placed
    let mut state = vec![0u8; steps.len()];
    let mut sequence = Vec::with_capacity(steps.len());
    for start in 0..steps.len() {
        if state[start] != 0 {
            continue;
        }
        state[start] = 1;
        let mut stack = vec![(start, 0usize)];
        while let Some(top) = stack.last_mut() {
            let (i, next) = *top;
            if let Some((input, _)) = steps[i].inputs.get(next) {
                top.1 += 1;
                if let Some(&p) = producers.get(input.as_str()).filter(|&&p| state[p] == 0) {
                    state[p] = 1;
                    stack.push((p, 0));
                }
            } else {
                state[i] = 2;
                sequence.push(i);
                stack.pop();
            }
        }
    }
    let mut slots: Vec<Option<Step>> = steps.into_iter().map(Some).collect();
    sequence.into_iter().filter_map(|i| slots[i].take()).collect()
}

fn calculate_with(ds: &Dataset, req: &Request, max_nodes: u32, max_depth: usize) -> Result<Plan, PlanError> {
    validate(ds, req)?;
    let mut planner = Planner {
        ds,
        req,
        rows: Vec::new(),
        row_of: HashMap::new(),
        steps: Vec::new(),
        step_of: HashMap::new(),
        warnings: Vec::new(),
        nodes: 0,
        max_nodes,
        max_depth,
        complete: true,
        path: Vec::new(),
    };
    let tree = planner.visit(&req.item, req.quantity, 0)?;

    let mut ledger = Vec::with_capacity(planner.rows.len());
    for r in &planner.rows {
        // required = from stock + gathered + produced - what is left over (what came out of the surplus was
        // produced earlier, so it is inside `produced`)
        if r.required.checked_add(r.surplus) != Some(r.from_stock + r.gather + r.produced) {
            return Err(PlanError(format!("Material conservation failed for {}.", r.item)));
        }
        ledger.push(LedgerRow {
            item: r.item.to_string(),
            required: r.required,
            from_stock: r.from_stock,
            from_surplus: r.from_surplus,
            produced: r.produced,
            gather: r.gather,
            surplus: r.surplus,
        });
    }
    ledger.sort_by(|a, b| a.item.cmp(&b.item));
    let materials = ledger.iter().filter(|r| r.gather > 0).map(|r| (r.item.clone(), r.gather)).collect();
    let intermediates =
        ledger.iter().filter(|r| r.produced > 0 && r.item != req.item).map(|r| (r.item.clone(), r.produced)).collect();

    let mut cook_ticks = 0u64;
    let mut operations = 0u64;
    let mut steps = Vec::with_capacity(planner.steps.len());
    for a in &planner.steps {
        let ticks = match a.recipe.ticks {
            Some(t) if a.recipe.kind.burns_fuel() => {
                let total = a.operations.checked_mul(t as u64).ok_or_else(overflow)?;
                cook_ticks = cook_ticks.checked_add(total).ok_or_else(overflow)?;
                Some(total)
            }
            _ => None,
        };
        operations = add(operations, a.operations)?;
        steps.push(Step {
            recipe: a.recipe.id.clone(),
            item: a.item.to_string(),
            kind: a.recipe.kind,
            operations: a.operations,
            output: a.output,
            inputs: a.inputs.iter().map(|(id, n)| (id.to_string(), *n)).collect(),
            choices: a.choices.clone(),
            byproducts: a.byproducts.iter().map(|(id, n)| (id.to_string(), *n)).collect(),
            ticks,
        });
    }
    let fuel = if cook_ticks == 0 {
        Vec::new()
    } else {
        FUELS
            .iter()
            .filter(|(id, _)| ds.items.contains_key(*id))
            .map(|(id, burn)| FuelEstimate { item: id.to_string(), count: cook_ticks.div_ceil(*burn) })
            .collect()
    };
    Ok(Plan {
        request: req.clone(),
        tree,
        ledger,
        materials,
        intermediates,
        steps: order(steps),
        operations,
        cook_ticks,
        fuel,
        warnings: planner.warnings,
        complete: planner.complete,
    })
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod plan_tests;
