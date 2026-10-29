//! The Crafting page: pick an item and an amount, see what to gather, craft and smelt. The recipes are read from
//! the player's own game jar on a background thread; the page itself only holds the state and draws it. Nothing
//! here runs while another page is open (the state is made when the page is first shown, and gives its memory back
//! after a while of not being looked at).
//!
//! This file has the state, the glue to the rest of the window and the header; the left column is in
//! `craft_pick.rs`, the right column (the plan) in `craft_view.rs`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

use egui::{Align, CornerRadius, Layout, RichText, Sense, TextureHandle, Ui, vec2};

use super::craft_fmt::*;
use super::theme::*;
use super::widgets::*;
use super::{App, Msg, Page};
use crate::craft::icons::{IconImage, Icons};
use crate::craft::load::{self, InstalledVersion};
use crate::craft::saved::{self, SavedPlan, Store};
use crate::craft::{Category, Dataset, ItemId, Node, Plan, Request, Storage, plan, stack};
use crate::install::{Manifest, ManifestVersion};

/// The width of the left column when the page has room for two columns.
pub(super) const LEFT_W: f32 = 352.0;
/// The window content width from which the page uses two columns.
pub(super) const TWO_COLUMNS: f32 = 900.0;
/// How many new pictures one frame may ask for.
const ICONS_PER_FRAME: u32 = 12;
/// How long the page's memory is kept after it was last shown.
const KEEP_AFTER_LEAVING: Duration = Duration::from_secs(600);

/// What the background work tells the page.
pub enum CraftMsg {
    /// The games on this computer and the saved plans, the first time the page is opened.
    Listed {
        versions: Vec<InstalledVersion>,
        store: Store,
    },
    /// The games on this computer again (after a download).
    Versions(Vec<InstalledVersion>),
    Loaded {
        rev: u64,
        result: Result<Dataset, String>,
    },
    Icon {
        rev: u64,
        id: ItemId,
        image: Option<IconImage>,
    },
    /// A download of a game version ended (well or not).
    Downloaded(String),
    /// The saved plans and stock were written.
    Saved(Result<(), String>),
    /// A plan file the player chose to import.
    PlanFile(Result<SavedPlan, String>),
    /// What happened to a file the player asked for.
    Notice(Result<String, String>),
}

/// What the page asks the window to do (it cannot, being taken out of the window while it is drawn).
pub(super) enum Act {
    Load {
        rev: u64,
        version: String,
        jar: PathBuf,
    },
    Relist,
    Download(String),
    RefreshManifest,
    SaveStore(Store),
    Copy(String),
    /// Ask where to put a file and write `content` there.
    Export {
        name: String,
        content: String,
        ext: &'static str,
    },
    ImportPlan,
    Toast(String, bool),
}

/// What the window lends the page for one frame.
pub(super) struct Env<'a> {
    pub manifest: Option<&'a Manifest>,
    pub manifest_error: Option<&'a str>,
}

/// One row of the item list, made once per loaded version.
pub(super) struct Entry {
    pub id: ItemId,
    pub name: String,
    pub lname: String,
    pub lid: String,
    pub stack: u32,
    pub category: Category,
}

/// The recipe book of the chosen version and the list of its items.
pub(super) struct Book {
    pub ds: Dataset,
    pub entries: Vec<Entry>,
}

impl Book {
    fn new(ds: Dataset) -> Book {
        let mut entries: Vec<Entry> = ds
            .items
            .values()
            .map(|i| Entry {
                id: i.id.clone(),
                name: i.name.clone(),
                lname: i.name.to_lowercase(),
                lid: i.id.rsplit(':').next().unwrap_or(&i.id).to_lowercase(),
                stack: i.stack,
                category: i.category,
            })
            .collect();
        entries.sort_by(|a, b| a.lname.cmp(&b.lname).then_with(|| a.id.cmp(&b.id)));
        Book { ds, entries }
    }
}

#[derive(PartialEq, Eq)]
pub(super) enum Load {
    Idle,
    Loading,
    Failed(String),
    Ready,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tab {
    Materials,
    Steps,
    Tree,
    Storage,
}

pub(super) enum Dialog {
    Saved,
    Save(String),
    Rename(u64, String),
    Delete(u64),
    ClearStock,
    GetVersion,
}

/// A plan with what the page derives from it once, when it is made.
pub(super) struct PlanView {
    pub plan: Plan,
    /// Per step, the steps that make something it uses; and the other way round.
    pub deps: Vec<Vec<usize>>,
    pub dependants: Vec<Vec<usize>>,
    /// Storage of what to gather, of the finished product and of what is made on the way.
    pub storage: [Storage; 3],
    pub nodes: usize,
}

/// What identifies a request apart from the stock: when it changes, the ticks are cleared.
pub(super) type Sig = (ItemId, u64, BTreeMap<ItemId, String>, BTreeMap<String, ItemId>, BTreeSet<ItemId>);

/// What a screenshot wants shown.
pub(super) struct ShotSpec {
    pub target: Option<(&'static str, u64)>,
    pub tab: Tab,
    pub stock: Vec<(&'static str, u64)>,
    pub dialog: Option<&'static str>,
}

pub(super) enum Slot {
    Pending,
    Ready(TextureHandle),
    Missing,
}

#[derive(Default)]
pub(super) struct IconCache {
    pub map: HashMap<ItemId, Slot>,
    pub tx: Option<Sender<ItemId>>,
    pub budget: u32,
}

pub struct CraftState {
    pub(super) rev: u64,
    pub(super) versions: Option<Vec<InstalledVersion>>,
    pub(super) preferred: Option<String>,
    pub(super) want: Option<String>,
    pub(super) store: Store,
    pub(super) store_due: Option<Instant>,
    pub(super) saving: bool,
    pub(super) save_again: bool,
    pub(super) version: Option<String>,
    pub(super) load: Load,
    pub(super) book: Option<Arc<Book>>,
    pub(super) icons: IconCache,
    pub(super) last_seen: Instant,
    pub(super) downloading: HashSet<String>,
    pub(super) dialog: Option<Dialog>,
    pub(super) shot: Option<ShotSpec>,
    // the item list
    pub(super) search: String,
    pub(super) category: Option<Category>,
    pub(super) filtered: Vec<u32>,
    pub(super) filtered_for: Option<(String, Option<Category>, u64)>,
    // what to make
    pub(super) target: Option<ItemId>,
    pub(super) qty_text: String,
    pub(super) use_stock: bool,
    pub(super) recipe_choices: BTreeMap<ItemId, String>,
    pub(super) ingredient_choices: BTreeMap<String, ItemId>,
    pub(super) supplied: BTreeSet<ItemId>,
    // the stock being edited
    pub(super) stock_search: String,
    pub(super) stock_hits: Vec<u32>,
    pub(super) stock_hits_for: String,
    pub(super) stock_pick: Option<ItemId>,
    pub(super) stock_qty: String,
    pub(super) stock_bufs: HashMap<ItemId, String>,
    // the plan
    pub(super) dirty: bool,
    pub(super) plan: Option<Result<Arc<PlanView>, String>>,
    pub(super) done: BTreeSet<String>,
    pub(super) done_sig: Option<Sig>,
    pub(super) active: Option<u64>,
    pub(super) active_sig: Option<Sig>,
    pub(super) unsaved: bool,
    pub(super) pending_open: Option<SavedPlan>,
    // the view
    pub(super) tab: Tab,
    /// On a narrow page the item list is folded away once something is chosen; this opens it again.
    pub(super) picking: bool,
    pub(super) tree_epoch: u32,
    pub(super) tree_all: bool,
}

impl CraftState {
    pub(super) fn new(preferred: Option<String>) -> CraftState {
        CraftState {
            rev: 0,
            versions: None,
            preferred,
            want: None,
            store: Store::default(),
            store_due: None,
            saving: false,
            save_again: false,
            version: None,
            load: Load::Idle,
            book: None,
            icons: IconCache::default(),
            last_seen: Instant::now(),
            downloading: HashSet::new(),
            dialog: None,
            shot: None,
            search: String::new(),
            category: None,
            filtered: Vec::new(),
            filtered_for: None,
            target: None,
            qty_text: "1".into(),
            use_stock: true,
            recipe_choices: BTreeMap::new(),
            ingredient_choices: BTreeMap::new(),
            supplied: BTreeSet::new(),
            stock_search: String::new(),
            stock_hits: Vec::new(),
            stock_hits_for: String::new(),
            stock_pick: None,
            stock_qty: "1".into(),
            stock_bufs: HashMap::new(),
            dirty: false,
            plan: None,
            done: BTreeSet::new(),
            done_sig: None,
            active: None,
            active_sig: None,
            unsaved: false,
            pending_open: None,
            tab: Tab::Materials,
            picking: false,
            tree_epoch: 0,
            tree_all: false,
        }
    }

    // ----------------------------------------------------------------------------------------------------
    // loading

    /// Start reading the recipes of `version`, forgetting everything that belonged to the one before.
    pub(super) fn begin_load(&mut self, version: &str, acts: &mut Vec<Act>) {
        let Some(jar) =
            self.versions.as_ref().and_then(|v| v.iter().find(|i| i.version == version)).map(|i| i.jar.clone())
        else {
            return;
        };
        self.rev += 1;
        // the same version read again (the page gave back its memory while it wasn't looked at, or Try again):
        // the plan, its ticks and the recipe choices stay; another version starts afresh
        let same = self.version.as_deref() == Some(version);
        self.version = Some(version.to_string());
        self.load = Load::Loading;
        // a saved plan waiting for its version only opens with that version
        if self.pending_open.as_ref().is_some_and(|p| p.version != version) {
            self.pending_open = None;
        }
        self.book = None;
        self.icons = IconCache::default();
        self.plan = None;
        self.filtered.clear();
        self.filtered_for = None;
        self.stock_hits.clear();
        self.stock_hits_for.clear();
        self.stock_pick = None;
        self.stock_bufs.clear();
        if !same {
            self.active = None;
            self.active_sig = None;
            self.unsaved = false;
            self.done.clear();
            self.done_sig = None;
            self.recipe_choices.clear();
            self.ingredient_choices.clear();
        }
        acts.push(Act::Load { rev: self.rev, version: version.to_string(), jar });
    }

    /// Give back what a page that is not looked at holds: the recipe book, the pictures.
    pub(super) fn release(&mut self) {
        self.book = None;
        self.icons = IconCache::default();
        self.plan = None;
        self.filtered.clear();
        self.filtered_for = None;
        self.stock_hits.clear();
        if self.load == Load::Ready {
            self.load = Load::Idle;
        }
        self.rev += 1;
    }

    /// The version the page should show first: the selected instance's, else the newest.
    fn default_version(&self) -> Option<String> {
        let list = self.versions.as_ref()?;
        self.preferred
            .as_ref()
            .filter(|p| list.iter().any(|v| &v.version == *p))
            .cloned()
            .or_else(|| list.first().map(|v| v.version.clone()))
    }

    pub(super) fn on_msg(&mut self, ctx: &egui::Context, msg: CraftMsg) -> Vec<Act> {
        let mut acts = Vec::new();
        match msg {
            CraftMsg::Listed { versions, store } => {
                self.versions = Some(versions);
                self.store = store;
                if let Some(v) = self.default_version() {
                    self.begin_load(&v, &mut acts);
                }
            }
            CraftMsg::Versions(list) => {
                self.versions = Some(list);
                let gone = self.version.as_ref().is_some_and(|v| !self.has_version(v));
                if let Some(w) = self.want.take().filter(|w| self.has_version(w)) {
                    self.begin_load(&w, &mut acts);
                } else if self.version.is_none() || gone {
                    if let Some(v) = self.default_version() {
                        self.begin_load(&v, &mut acts);
                    }
                }
            }
            CraftMsg::Loaded { rev, result } => {
                if rev != self.rev {
                    return acts;
                }
                match result {
                    Ok(ds) => self.loaded(ds),
                    Err(e) => self.load = Load::Failed(e),
                }
            }
            CraftMsg::Icon { rev, id, image } => {
                if rev != self.rev {
                    return acts;
                }
                let slot = match image {
                    Some(img) if img.width > 0 && img.height > 0 && img.rgba.len() == img.width * img.height * 4 => {
                        let color = egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.rgba);
                        Slot::Ready(ctx.load_texture(format!("craft:{id}"), color, egui::TextureOptions::NEAREST))
                    }
                    _ => Slot::Missing,
                };
                self.icons.map.insert(id, slot);
            }
            CraftMsg::Downloaded(v) => {
                self.downloading.remove(&v);
                if self.has_version(&v) {
                    return acts;
                }
                // the page switches to it when nothing is being planned, or when it was just asked for; a plan that
                // is open stays, and the new version waits in the list
                let asked = matches!(self.dialog, Some(Dialog::GetVersion));
                if self.book.is_none() || self.target.is_none() || asked {
                    self.want = Some(v);
                } else {
                    acts.push(Act::Toast(format!("Minecraft {v} is ready. Pick it in the Version list."), false));
                }
                acts.push(Act::Relist);
            }
            CraftMsg::Saved(r) => {
                self.saving = false;
                if let Err(e) = r {
                    acts.push(Act::Toast(format!("Couldn't save your crafting plans: {e}"), true));
                }
                if self.save_again {
                    self.save_again = false;
                    self.store_due = Some(Instant::now());
                }
            }
            CraftMsg::PlanFile(r) => match r {
                Ok(p) => {
                    let name = p.name.clone();
                    self.add_plan(p);
                    acts.push(Act::Toast(format!("Imported the plan \"{name}\""), false));
                }
                Err(e) => acts.push(Act::Toast(e, true)),
            },
            CraftMsg::Notice(r) => match r {
                Ok(t) => acts.push(Act::Toast(t, false)),
                Err(e) => acts.push(Act::Toast(e, true)),
            },
        }
        acts
    }

    fn has_version(&self, v: &str) -> bool {
        self.versions.as_ref().is_some_and(|l| l.iter().any(|i| i.version == v))
    }

    /// The recipes arrived.
    fn loaded(&mut self, ds: Dataset) {
        let book = Arc::new(Book::new(ds));
        if self.target.as_ref().is_some_and(|t| book.ds.item(t).is_none()) {
            self.target = None;
        }
        self.supplied.retain(|s| book.ds.item(s).is_some());
        self.book = Some(book.clone());
        self.load = Load::Ready;
        self.dirty = true;
        if let Some(p) = self.pending_open.take() {
            self.open_plan_now(p);
        }
        if let Some(spec) = self.shot.take() {
            self.apply_shot(spec);
        }
    }

    /// Put the page in the state a screenshot asks for.
    pub(super) fn apply_shot(&mut self, spec: ShotSpec) {
        self.target = spec.target.map(|(id, _)| id.to_string());
        if let Some((_, qty)) = spec.target {
            self.qty_text = thousands(qty);
        }
        self.recipe_choices.clear();
        self.ingredient_choices.clear();
        self.supplied.clear();
        self.active = None;
        self.done.clear();
        if let Some(v) = self.version.clone() {
            self.store.stock.remove(&v);
            for (id, n) in spec.stock {
                self.store.set_stock(&v, id, n);
            }
        }
        self.stock_bufs.clear();
        self.tab = spec.tab;
        self.dialog = match spec.dialog {
            Some("get") => Some(Dialog::GetVersion),
            Some("saved") => Some(Dialog::Saved),
            Some("save") => Some(Dialog::Save("My plan".into())),
            _ => None,
        };
        self.dirty = true;
    }

    // ----------------------------------------------------------------------------------------------------
    // the plan

    pub(super) fn touch_plan(&mut self) {
        self.dirty = true;
    }

    /// The player's stock for the version on show.
    pub(super) fn stock(&self) -> Option<&BTreeMap<ItemId, u64>> {
        Some(self.store.stock_for(self.version.as_deref()?))
    }

    /// Say how many of an item the player has (none takes it off the list).
    pub(super) fn set_stock(&mut self, id: &str, qty: u64) {
        if let Some(v) = self.version.clone() {
            self.store.set_stock(&v, id, qty);
        }
    }

    pub(super) fn clear_stock(&mut self) {
        if let Some(v) = self.version.clone() {
            self.store.stock.remove(&v);
        }
    }

    /// Something in the store changed: write it a moment from now.
    pub(super) fn touch_store(&mut self) {
        self.store_due = Some(Instant::now() + Duration::from_millis(900));
    }

    fn sig_of(req: &Request) -> Sig {
        (
            req.item.clone(),
            req.quantity,
            req.recipe_choices.clone(),
            req.ingredient_choices.clone(),
            req.supplied.clone(),
        )
    }

    /// Work the plan out again. Called only when something the plan depends on changed.
    pub(super) fn recalc(&mut self) {
        self.dirty = false;
        let Some(book) = self.book.clone() else { return };
        let Some(target) = self.target.clone() else {
            self.plan = None;
            return;
        };
        let Ok(quantity) = parse_qty(&self.qty_text) else {
            self.plan = None;
            return;
        };
        let inventory = if self.use_stock { self.stock().cloned().unwrap_or_default() } else { BTreeMap::new() };
        let req = Request {
            item: target,
            quantity,
            inventory,
            recipe_choices: self.recipe_choices.clone(),
            ingredient_choices: self.ingredient_choices.clone(),
            supplied: self.supplied.clone(),
        };
        let sig = Self::sig_of(&req);
        match plan::calculate(&book.ds, &req) {
            Ok(p) => {
                // ticks belong to one request: a different one starts clean, the same one (with other stock) keeps
                // those that still mean something
                if self.done_sig.as_ref() != Some(&sig) {
                    self.done.clear();
                    self.done_sig = Some(sig.clone());
                }
                let view = Self::view_of(&book.ds, p, quantity);
                self.done.retain(|k| {
                    if let Some(item) = k.strip_prefix("gather:") {
                        view.plan.materials.iter().any(|(i, _)| i == item)
                    } else if let Some(r) = k.strip_prefix("step:") {
                        view.plan.steps.iter().any(|s| s.recipe == r)
                    } else {
                        false
                    }
                });
                self.unsaved = self.active.is_some() && self.active_sig.as_ref() != Some(&sig);
                self.plan = Some(Ok(Arc::new(view)));
            }
            Err(e) => self.plan = Some(Err(e.to_string())),
        }
    }

    fn view_of(ds: &Dataset, plan: Plan, quantity: u64) -> PlanView {
        let producer: HashMap<&str, usize> = plan.steps.iter().enumerate().map(|(i, s)| (s.item.as_str(), i)).collect();
        let mut deps: Vec<Vec<usize>> = Vec::with_capacity(plan.steps.len());
        for (i, s) in plan.steps.iter().enumerate() {
            let mut d: Vec<usize> =
                s.inputs.iter().filter_map(|(it, _)| producer.get(it.as_str()).copied()).filter(|&j| j != i).collect();
            d.sort_unstable();
            d.dedup();
            deps.push(d);
        }
        let mut dependants = vec![Vec::new(); deps.len()];
        for (i, d) in deps.iter().enumerate() {
            for &j in d {
                dependants[j].push(i);
            }
        }
        fn count(n: &Node) -> usize {
            1 + n.children.iter().map(count).sum::<usize>()
        }
        let storage = [
            stack::storage(ds, &plan.materials),
            stack::storage(ds, &[(plan.request.item.clone(), quantity)]),
            stack::storage(ds, &plan.intermediates),
        ];
        let nodes = count(&plan.tree);
        PlanView { plan, deps, dependants, storage, nodes }
    }

    /// The ticks changed: a saved plan that is open keeps them.
    pub(super) fn done_changed(&mut self) {
        if let Some(id) = self.active {
            let done = self.done.clone();
            if let Some(p) = self.store.get_mut(id) {
                p.done = done;
                p.updated_ms = saved::now_ms();
            }
            self.touch_store();
        }
    }

    pub(super) fn set_target(&mut self, id: &str) {
        // picking the same item again folds the list (in a narrow window) and keeps the plan as it is
        self.picking = false;
        if self.target.as_deref() == Some(id) {
            return;
        }
        self.target = Some(id.to_string());
        self.recipe_choices.clear();
        self.ingredient_choices.clear();
        self.supplied.clear();
        self.active = None;
        self.active_sig = None;
        self.unsaved = false;
        self.touch_plan();
    }

    pub(super) fn choose_recipe(&mut self, item: &str, recipe: &str) {
        let default =
            self.book.as_ref().and_then(|b| b.ds.recipes_for(item).next().map(|r| r.id.clone())).unwrap_or_default();
        if recipe == default {
            self.recipe_choices.remove(item);
        } else {
            self.recipe_choices.insert(item.to_string(), recipe.to_string());
        }
        self.touch_plan();
    }

    pub(super) fn choose_ingredient(&mut self, recipe: &str, key: &str, item: &str) {
        self.ingredient_choices.insert(format!("{recipe}:{key}"), item.to_string());
        self.touch_plan();
    }

    // ----------------------------------------------------------------------------------------------------
    // saved plans (the list lives in the store)

    pub(super) fn add_plan(&mut self, p: SavedPlan) -> u64 {
        let id = self.store.add(p);
        self.touch_store();
        id
    }

    /// Open a saved plan: its version, item, amount and choices, and what was ticked.
    pub(super) fn open_plan(&mut self, p: SavedPlan, acts: &mut Vec<Act>) {
        if self.version.as_deref() == Some(p.version.as_str()) && self.book.is_some() {
            self.open_plan_now(p);
        } else if self.has_version(&p.version) {
            let v = p.version.clone();
            self.pending_open = Some(p);
            self.begin_load(&v, acts);
        } else {
            acts.push(Act::Toast(
                format!("Minecraft {} isn't on this computer. Get it first to open this plan.", p.version),
                true,
            ));
        }
    }

    fn open_plan_now(&mut self, p: SavedPlan) {
        let r = &p.request;
        self.target = Some(r.item.clone());
        self.qty_text = thousands(r.quantity);
        self.recipe_choices = r.recipe_choices.clone();
        self.ingredient_choices = r.ingredient_choices.clone();
        self.supplied = r.supplied.clone();
        self.use_stock = !r.inventory.is_empty();
        self.done = p.done.clone();
        self.done_sig = Some(Self::sig_of(r));
        self.active = Some(p.id);
        self.active_sig = Some(Self::sig_of(r));
        self.unsaved = false;
        self.dirty = true;
    }

    /// Keep the plan on show as a saved plan: a new one, or the open one brought up to date.
    pub(super) fn save_plan(&mut self, name: &str, as_new: bool) {
        let Some(Ok(view)) = &self.plan else { return };
        let Some(version) = self.version.clone() else { return };
        let req = view.plan.request.clone();
        let now = saved::now_ms();
        let name = name.trim().to_string();
        if let (false, Some(id)) = (as_new, self.active) {
            self.store.rename(id, &name, now);
            if let Some(p) = self.store.get_mut(id) {
                p.request = req.clone();
                p.done = self.done.clone();
                p.updated_ms = now;
                self.active_sig = Some(Self::sig_of(&req));
                self.unsaved = false;
                self.touch_store();
                return;
            }
        }
        let sig = Self::sig_of(&req);
        let p =
            SavedPlan { id: 0, name, version, request: req, done: self.done.clone(), created_ms: now, updated_ms: now };
        self.active = Some(self.add_plan(p));
        self.active_sig = Some(sig);
        self.unsaved = false;
    }

    // ----------------------------------------------------------------------------------------------------
    // pictures

    /// A picture of `id` in a square of `size`, a plain tile with a letter while it isn't there.
    pub(super) fn icon(&mut self, ui: &mut Ui, id: &str, name: &str, size: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
        self.paint_icon(ui, rect, id, name);
    }

    pub(super) fn paint_icon(&mut self, ui: &Ui, rect: egui::Rect, id: &str, name: &str) {
        if !ui.is_rect_visible(rect) {
            return;
        }
        match self.icons.map.get(id) {
            Some(Slot::Ready(tex)) => {
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(tex.id(), rect, uv, egui::Color32::WHITE);
                return;
            }
            Some(_) => {}
            None => {
                if self.icons.budget > 0 && self.icons.tx.as_ref().is_some_and(|tx| tx.send(id.to_string()).is_ok()) {
                    self.icons.budget -= 1;
                    self.icons.map.insert(id.to_string(), Slot::Pending);
                } else if self.icons.tx.is_some() {
                    ui.ctx().request_repaint();
                }
            }
        }
        ui.painter().rect_filled(rect, CornerRadius::same(R_TAG), c_field());
        let letter: String =
            name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default();
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            letter,
            egui::FontId::proportional(rect.width() * 0.46),
            c_muted(),
        );
    }

    // ----------------------------------------------------------------------------------------------------
    // the page

    /// Draw the page; what it wants done is returned.
    pub(super) fn show(&mut self, ui: &mut Ui, env: &Env) -> Vec<Act> {
        let mut acts = Vec::new();
        self.last_seen = Instant::now();
        self.icons.budget = ICONS_PER_FRAME;
        if self.load == Load::Idle {
            if let Some(v) = self.version.clone() {
                self.begin_load(&v, &mut acts);
            }
        }
        self.hotkeys(ui.ctx());
        if self.dirty {
            self.recalc();
        }
        self.header(ui, env, &mut acts);
        ui.add_space(8.0);
        match (&self.versions, &self.load) {
            (None, _) => {
                ui.horizontal(|ui| {
                    spinner(ui);
                    muted(ui, "Looking for the games on this computer");
                });
            }
            (Some(v), _) if v.is_empty() => self.no_game(ui, env, &mut acts),
            (_, Load::Failed(e)) => {
                let e = e.clone();
                self.failed(ui, &e, &mut acts);
            }
            (_, Load::Ready) => self.body(ui, &mut acts),
            _ => {
                ui.horizontal(|ui| {
                    spinner(ui);
                    muted(ui, "Reading the recipes of this version");
                });
            }
        }
        self.dialogs(ui.ctx(), env, &mut acts);
        acts
    }

    fn hotkeys(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() {
            return;
        }
        let search_id = egui::Id::unique("craft-search");
        if !ctx.egui_wants_keyboard_input() {
            let slash = ctx.input_mut(|i| {
                let at = i.events.iter().position(|e| matches!(e, egui::Event::Text(t) if t == "/"));
                at.map(|p| i.events.remove(p)).is_some()
            });
            if slash && self.load == Load::Ready {
                // in a narrow window the search is folded away once an item is picked: "/" opens it again
                self.picking = true;
                ctx.memory_mut(|m| m.request_focus(search_id));
            }
        }
        let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if esc
            && (ctx.memory(|m| m.has_focus(search_id)) || !ctx.egui_wants_keyboard_input())
            && !self.search.is_empty()
        {
            self.search.clear();
        }
    }

    fn header(&mut self, ui: &mut Ui, env: &Env, acts: &mut Vec<Act>) {
        let narrow = ui.available_width() < 760.0;
        let status = self.book.as_ref().map(|b| {
            format!(
                "{} items \u{00B7} {} recipes",
                thousands(b.ds.items.len() as u64),
                thousands(b.ds.recipes.len() as u64)
            )
        });
        ui.horizontal(|ui| {
            heading(ui, "Crafting calculator");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.version_combo(ui, acts);
                // the plans saved earlier, also when no plan is open (after a restart, or a version that failed)
                let n = self.store.plans.len();
                if n > 0 && self.target.is_none() && ghost(ui, &format!("Saved plans ({n})")).clicked() {
                    self.dialog = Some(Dialog::Saved);
                }
                if let (false, Some(s)) = (narrow, &status) {
                    small_muted(ui, s.clone());
                }
            });
        });
        muted(ui, "Pick what you want to build. BlockForge works out what to gather, craft and smelt.");
        if let (true, Some(s)) = (narrow, status) {
            small_muted(ui, s);
        }
        let _ = env;
    }

    fn version_combo(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let Some(list) = self.versions.clone().filter(|l| !l.is_empty()) else { return };
        let shown = self.version.clone().unwrap_or_else(|| "Choose".into());
        let mut pick: Option<String> = None;
        egui::ComboBox::from_id_salt("craft-version").selected_text(shown).width(110.0).show_ui(ui, |ui| {
            ui.set_min_width(190.0);
            for v in &list {
                let on = self.version.as_deref() == Some(v.version.as_str());
                if ui.selectable_label(on, &v.version).clicked() && !on {
                    pick = Some(v.version.clone());
                }
            }
            ui.separator();
            if ui.button("Get another version\u{2026}").clicked() {
                self.dialog = Some(Dialog::GetVersion);
                ui.close();
            }
        });
        muted(ui, "Version");
        if let Some(v) = pick {
            self.begin_load(&v, acts);
        }
    }

    /// No game is on this computer: explain, and offer to fetch one.
    fn no_game(&mut self, ui: &mut Ui, env: &Env, acts: &mut Vec<Act>) {
        card(ui, |ui| {
            ui.label(RichText::new("The recipes come from your game files").size(FS_SECTION).semibold());
            ui.add_space(2.0);
            muted(
                ui,
                "BlockForge reads crafting, smelting and the other recipes straight from a Minecraft version, so the \
                 plan always matches the game you play and nothing of Mojang's is bundled. No Minecraft version is \
                 on this computer yet. Pick one to download; only the game file is fetched (a few tens of MB), not \
                 the sounds or the libraries.",
            );
            ui.add_space(8.0);
            // as tall as the window allows, not a fixed height that leaves a gap on a tall window and is cut short on
            // a small one
            let height = (ui.available_height() - 28.0).max(160.0);
            self.get_versions(ui, env, acts, height);
        });
    }

    fn failed(&mut self, ui: &mut Ui, error: &str, acts: &mut Vec<Act>) {
        let version = self.version.clone().unwrap_or_default();
        card(ui, |ui| {
            ui.label(
                RichText::new(format!("Couldn't read the recipes of Minecraft {version}")).size(FS_SECTION).semibold(),
            );
            ui.add_space(2.0);
            ui.label(RichText::new(error).color(c_danger()));
            ui.add_space(6.0);
            muted(ui, "Pick another version in the list at the top right, or get a newer one.");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if secondary(ui, "Try again").clicked() {
                    self.begin_load(&version, acts);
                }
                if secondary(ui, "Get another version").clicked() {
                    self.dialog = Some(Dialog::GetVersion);
                }
            });
        });
    }

    /// The list of versions that can be downloaded, with their buttons.
    pub(super) fn get_versions(&mut self, ui: &mut Ui, env: &Env, acts: &mut Vec<Act>, height: f32) {
        let Some(m) = env.manifest else {
            match env.manifest_error {
                Some(e) => {
                    ui.label(RichText::new(format!("Couldn't load the version list: {e}")).color(c_danger()));
                    if secondary(ui, "Try again").clicked() {
                        acts.push(Act::RefreshManifest);
                    }
                }
                None => {
                    ui.horizontal(|ui| {
                        spinner(ui);
                        muted(ui, "Loading the version list");
                    });
                }
            }
            return;
        };
        let latest = m.latest.get("release").cloned().unwrap_or_default();
        let list: Vec<&ManifestVersion> =
            m.versions.iter().filter(|v| v.kind == "release" && supported(&v.id)).collect();
        small_muted(ui, "Releases from 1.13 on. Older versions keep their recipes in a form this page can't read.");
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("craft-get-versions")
            .max_height(height)
            .auto_shrink([false, true])
            .show_rows(ui, 46.0, list.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for v in &list[range] {
                    let have = self.has_version(&v.id);
                    let busy = self.downloading.contains(&v.id);
                    ui.horizontal(|ui| {
                        ui.set_height(45.0);
                        ui.label(RichText::new(&v.id).semibold());
                        if v.id == latest {
                            small_faint(ui, "Latest");
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if have {
                                small_muted(ui, "On this computer");
                            } else if busy {
                                small_muted(ui, "Downloading\u{2026}");
                                spinner(ui);
                            } else if secondary(ui, "Download").clicked() {
                                self.downloading.insert(v.id.clone());
                                acts.push(Act::Download(v.id.clone()));
                            }
                        });
                    });
                    rule(ui);
                }
            });
    }
}

/// Whether a release's recipes are in the form the reader understands: 1.13 and newer.
pub(super) fn supported(id: &str) -> bool {
    let mut parts = id.split('.');
    let (Some(a), b) = (parts.next().and_then(|p| p.parse::<u32>().ok()), parts.next()) else { return false };
    if a > 1 {
        return true;
    }
    a == 1 && b.and_then(|p| p.parse::<u32>().ok()).is_some_and(|m| m >= 13)
}

// ---------------------------------------------------------------------------------------------------------
// The window's side

impl App {
    /// The page was opened: make its state the first time and look for the games and the saved plans.
    fn craft_open(&mut self) {
        if self.craft.is_some() {
            return;
        }
        let preferred = self
            .selected
            .as_ref()
            .and_then(|f| self.instance(f))
            .map(|i| i.mc_version().to_string())
            .filter(|v| !v.is_empty());
        self.craft = Some(Box::new(CraftState::new(preferred)));
        let (dirs, n) = (self.dirs.clone(), self.notifier());
        self.rt.spawn_blocking(move || {
            let versions = load::installed_versions(&dirs);
            let store = saved::load(&dirs);
            n.send(Msg::Craft(CraftMsg::Listed { versions, store }));
        });
    }

    pub(super) fn craft_ui(&mut self, ui: &mut Ui) {
        self.craft_open();
        let Some(mut st) = self.craft.take() else { return };
        let acts = {
            let env = Env { manifest: self.manifest.as_deref(), manifest_error: self.manifest_error.as_deref() };
            st.show(ui, &env)
        };
        self.craft = Some(st);
        for a in acts {
            self.craft_act(a);
        }
    }

    pub(super) fn craft_msg(&mut self, m: CraftMsg) {
        let ctx = self.ctx.clone();
        let Some(st) = self.craft.as_mut() else { return };
        // the pictures of a version that has just been read come from a thread that owns the jar
        let started = if let CraftMsg::Loaded { rev, result: Ok(ds) } = &m {
            (*rev == st.rev).then(|| {
                (
                    *rev,
                    st.versions
                        .as_ref()
                        .and_then(|l| l.iter().find(|v| v.version == ds.version))
                        .map(|v| v.jar.clone()),
                )
            })
        } else {
            None
        };
        let acts = st.on_msg(&ctx, m);
        if let Some((rev, jar)) = started {
            if st.book.is_some() {
                let (tx, rx) = channel::<ItemId>();
                st.icons.tx = Some(tx);
                let n = self.notifier();
                std::thread::spawn(move || {
                    let mut icons = jar.as_deref().and_then(|j| Icons::open(j).ok());
                    for id in rx {
                        let image = icons.as_mut().and_then(|i| i.image(&id));
                        n.send(Msg::Craft(CraftMsg::Icon { rev, id, image }));
                    }
                });
            }
        }
        for a in acts {
            self.craft_act(a);
        }
    }

    fn craft_act(&mut self, a: Act) {
        match a {
            Act::Load { rev, version, jar } => {
                let n = self.notifier();
                self.rt.spawn_blocking(move || {
                    let result = std::panic::catch_unwind(|| load::load_jar(&version, &jar))
                        .unwrap_or_else(|_| Err("The recipe reader stopped unexpectedly on this file.".into()));
                    n.send(Msg::Craft(CraftMsg::Loaded { rev, result }));
                });
            }
            Act::Relist => {
                let (dirs, n) = (self.dirs.clone(), self.notifier());
                self.rt.spawn_blocking(move || n.send(Msg::Craft(CraftMsg::Versions(load::installed_versions(&dirs)))));
            }
            Act::Download(version) => self.craft_download(version),
            Act::RefreshManifest => {
                self.manifest_error = None;
                self.refresh_manifest();
            }
            Act::SaveStore(store) => {
                let (dirs, n) = (self.dirs.clone(), self.notifier());
                self.rt.spawn_blocking(move || n.send(Msg::Craft(CraftMsg::Saved(saved::save(&dirs, &store)))));
            }
            Act::Copy(text) => {
                self.ctx.copy_text(text);
                self.toast("Shopping list copied");
            }
            Act::Export { name, content, ext } => {
                let n = self.notifier();
                self.rt.spawn_blocking(move || {
                    let Some(path) = pick_save_path(&name, ext) else { return };
                    let r = std::fs::write(&path, content)
                        .map(|_| format!("Saved {}", path.display()))
                        .map_err(|e| format!("Couldn't write {}: {e}", path.display()));
                    n.send(Msg::Craft(CraftMsg::Notice(r)));
                });
            }
            Act::ImportPlan => {
                let n = self.notifier();
                self.rt.spawn_blocking(move || {
                    let Some(path) = pick_open_path() else { return };
                    let r = std::fs::read_to_string(&path)
                        .map_err(|e| format!("Couldn't read {}: {e}", path.display()))
                        .and_then(|t| saved::import_plan(&t, saved::now_ms()));
                    n.send(Msg::Craft(CraftMsg::PlanFile(r)));
                });
            }
            Act::Toast(t, err) => {
                if err {
                    self.toast_err(t)
                } else {
                    self.toast(t)
                }
            }
        }
    }

    /// Fetch just the game file of one version into the shared libraries folder.
    fn craft_download(&mut self, version: String) {
        let dirs = self.dirs.clone();
        let label = format!("Minecraft {version} for the crafting calculator");
        self.start_task(label, move |progress, n| async move {
            let done = n.clone();
            let v = version.clone();
            let result: anyhow::Result<String> = async {
                progress.stage("Looking up the game file");
                let json = crate::install::vanilla_json(&dirs, &version).await?;
                let client = json
                    .downloads
                    .as_ref()
                    .and_then(|d| d.client.as_ref())
                    .filter(|c| !c.url.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("Minecraft {version} has no game file to download"))?;
                progress.stage("Downloading the game file");
                let job = crate::install::Job::new(&client.url, crate::install::client_jar(&dirs, &version))
                    .checked(client.sha1.as_deref(), client.size);
                crate::install::download_all(vec![job], &progress, true).await?;
                Ok(format!("Minecraft {version} is ready for the crafting calculator"))
            }
            .await;
            done.send(Msg::Craft(CraftMsg::Downloaded(v)));
            result
        });
    }

    /// Called every frame: the delayed write of plans and stock, and giving memory back when the page is long left.
    pub(super) fn craft_tick(&mut self, ctx: &egui::Context) {
        let open = self.page == Page::Craft;
        let Some(st) = self.craft.as_mut() else { return };
        if let Some(due) = st.store_due {
            match due.checked_duration_since(Instant::now()) {
                Some(left) if !left.is_zero() => ctx.request_repaint_after(left),
                _ => {
                    if st.saving {
                        st.save_again = true;
                        st.store_due = None;
                    } else {
                        st.store_due = None;
                        st.saving = true;
                        let store = st.store.clone();
                        self.craft_act(Act::SaveStore(store));
                        return;
                    }
                }
            }
        }
        if !open && st.book.is_some() && st.last_seen.elapsed() > KEEP_AFTER_LEAVING {
            st.release();
        }
    }

    /// The window is closing: write what is still waiting.
    pub(super) fn craft_flush_now(&mut self) {
        if let Some(st) = self.craft.as_ref() {
            if st.store_due.is_some() || st.save_again {
                let _ = saved::save(&self.dirs, &st.store);
            }
        }
    }

    /// For the screenshot tour: open the page in a given state.
    pub(super) fn shot_craft(&mut self, variant: &str) {
        self.page = Page::Craft;
        self.craft_open();
        let Some(st) = self.craft.as_mut() else { return };
        let spec =
            |target: Option<(&'static str, u64)>, tab: Tab| ShotSpec { target, tab, stock: Vec::new(), dialog: None };
        st.shot = match variant {
            "craft" => Some(spec(Some(("minecraft:diamond_pickaxe", 64)), Tab::Materials)),
            "craft-steps" => Some(spec(Some(("minecraft:diamond_pickaxe", 64)), Tab::Steps)),
            "craft-tree" => Some(spec(Some(("minecraft:diamond_pickaxe", 64)), Tab::Tree)),
            "craft-storage" => Some(spec(Some(("minecraft:cookie", 10_000)), Tab::Storage)),
            "craft-cookies" => Some(spec(Some(("minecraft:cookie", 10_000)), Tab::Materials)),
            "craft-furnace" => Some(spec(Some(("minecraft:iron_ingot", 2_000)), Tab::Materials)),
            "craft-stock" => Some(ShotSpec {
                stock: vec![
                    ("minecraft:diamond", 12),
                    ("minecraft:stick", 40),
                    ("minecraft:iron_ingot", 200),
                    ("minecraft:oak_planks", 640),
                ],
                ..spec(Some(("minecraft:diamond_pickaxe", 64)), Tab::Materials)
            }),
            "craft-get" => Some(ShotSpec { dialog: Some("get"), ..spec(None, Tab::Materials) }),
            "craft-saved" => Some(ShotSpec {
                dialog: Some("saved"),
                ..spec(Some(("minecraft:diamond_pickaxe", 64)), Tab::Materials)
            }),
            _ => Some(spec(None, Tab::Materials)),
        };
        if st.book.is_some() {
            if let Some(spec) = st.shot.take() {
                st.apply_shot(spec);
            }
        }
    }
}

/// Ask where to save a file. Blocks until the chooser closes, so call it from a background thread.
fn pick_save_path(name: &str, ext: &'static str) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let mut zenity = crate::platform::command("zenity");
        zenity.args(["--file-selection", "--save", "--confirm-overwrite"]).arg(format!("--filename={name}"));
        // zenity answers with a non-zero code when the player cancels; only a missing program moves on
        if let Ok(out) = zenity.output() {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            return if out.status.success() && !text.is_empty() { Some(PathBuf::from(text)) } else { None };
        }
    }
    rfd::FileDialog::new().set_file_name(name).add_filter(ext, &[ext]).save_file()
}

/// Ask for a plan file to read. Blocks, like [`pick_save_path`].
fn pick_open_path() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let mut zenity = crate::platform::command("zenity");
        zenity.args(["--file-selection", "--file-filter=Crafting plans | *.json", "--file-filter=All files | *"]);
        if let Ok(out) = zenity.output() {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            return if out.status.success() && !text.is_empty() { Some(PathBuf::from(text)) } else { None };
        }
    }
    rfd::FileDialog::new().add_filter("Crafting plans", &["json"]).pick_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_from_1_13_are_offered() {
        assert!(supported("1.21.1"));
        assert!(supported("1.13"));
        assert!(supported("1.13.2"));
        assert!(supported("26.1"));
        assert!(!supported("1.12.2"));
        assert!(!supported("1.7.10"));
        assert!(!supported("b1.7.3"));
        assert!(!supported("1"));
    }
}
