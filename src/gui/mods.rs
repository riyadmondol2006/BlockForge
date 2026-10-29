//! The Mods tab of an instance: a full manager for its mods, resource packs and shader packs. Each item shows its
//! picture, name, version and description, can be switched on and off, updated, rolled back, inspected and removed,
//! and the whole list can be searched, checked for updates and updated in one go. New files dropped on the window
//! are recorded, and "Add" opens Discover on the right kind.
//!
//! Everything slow (the network, reading jars, copying) runs in the background and reports back with a
//! [`ModsMsg`]; the screen only reads the cached lists.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use egui::{Align, CornerRadius, Layout, RichText, ScrollArea, Stroke, Ui};

use super::icons::{self, Icon};
use super::instance_menu::{secondary_menu, split_or_stack};
use super::motion;
use super::theme::*;
use super::widgets::*;
use super::{App, InstTab, Msg, Page};
use crate::instances::{self, Instance};
use crate::modrinth::Kind;
use crate::mods::api::MrVersion;
use crate::mods::files::{self, ModEntry, ModKey, ScanScope, SyncReport};
use crate::mods::metadata::{self, ContentKind, ModInfo};
use crate::mods::scan::ScanReport;
use crate::mods::updates::{self, UpdateOffer};
use crate::mods::{ModContext, ModSettings, install, performance, versions};
use crate::snapshots;

/// How tall a row of the list is, and how big the picture in it.
const ROW_H: f32 = 60.0;
const ICON: f32 = 38.0;

/// How wide the grey names before the values of a mod's details are.
const FACT_W: f32 = 84.0;

/// Between the parts of the grey line after a mod's name (its version, where it came from).
const META_SEP: &str = " \u{00B7} ";

/// Which items the list shows.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Show {
    #[default]
    All,
    On,
    Off,
    Updates,
}

/// How the list is ordered.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum Order {
    /// By name, as the list comes.
    #[default]
    Name,
    /// The newest files first.
    Added,
    /// The ones with an update first, by name within each part.
    Updates,
}

impl Order {
    const ALL: [Order; 3] = [Order::Name, Order::Added, Order::Updates];

    fn label(self) -> &'static str {
        match self {
            Order::Name => "Name",
            Order::Added => "Recently added",
            Order::Updates => "Updates first",
        }
    }
}

/// Where the items the list shows came from.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum Origin {
    #[default]
    Anywhere,
    Modrinth,
    CurseForge,
    /// Neither site: dropped onto the window, added from GitHub, or put in the folder by hand.
    ByHand,
}

impl Origin {
    const ALL: [Origin; 4] = [Origin::Anywhere, Origin::Modrinth, Origin::CurseForge, Origin::ByHand];

    fn label(self) -> &'static str {
        match self {
            Origin::Anywhere => "Anywhere",
            Origin::Modrinth => "Modrinth",
            Origin::CurseForge => "CurseForge",
            Origin::ByHand => "Added by hand",
        }
    }

    /// Whether an item came from here.
    fn has(self, rec: &crate::modrecord::ModRecord) -> bool {
        match self {
            Origin::Anywhere => true,
            Origin::Modrinth => files::is_from_modrinth(rec),
            Origin::CurseForge => files::is_from_curseforge(rec),
            Origin::ByHand => !files::is_from_modrinth(rec) && !files::is_from_curseforge(rec),
        }
    }
}

/// Puts the rows in `order`. They come by name; a stable sort keeps that order among equals, and files whose time
/// isn't known go last.
fn order_rows<T>(rows: &mut [T], order: Order, added: impl Fn(&T) -> Option<SystemTime>, update: impl Fn(&T) -> bool) {
    match order {
        Order::Name => {}
        Order::Added => rows.sort_by_key(|r| std::cmp::Reverse(added(r))),
        Order::Updates => rows.sort_by_key(|r| !update(r)),
    }
}

/// What was read from inside a jar, kept for the list and the details.
#[derive(Clone)]
pub struct Details {
    pub info: ModInfo,
    /// Why this might not work in the instance, if it looks that way.
    pub problem: Option<String>,
}

/// News from background work for the Mods tab.
pub enum ModsMsg {
    /// The folders were compared with the mod list.
    Synced {
        folder: String,
        result: Result<SyncReport, String>,
    },
    /// Pictures and metadata read from jars.
    Details {
        folder: String,
        batch: Vec<(ModKey, Details, Option<egui::ColorImage>)>,
    },
    Updates {
        folder: String,
        result: Result<Vec<UpdateOffer>, String>,
    },
    /// A quick change finished (switching on or off, removing, refreshing).
    Done {
        folder: String,
        result: Result<String, String>,
    },
    Versions {
        folder: String,
        key: ModKey,
        result: Result<Vec<MrVersion>, String>,
    },
    Scan {
        folder: String,
        report: ScanReport,
    },
}

struct VersionsDialog {
    key: ModKey,
    name: String,
    /// `None` while loading.
    list: Option<Result<Vec<MrVersion>, String>>,
    installed_id: Option<String>,
}

#[derive(Default)]
pub struct ModsState {
    folder: String,
    signature: Option<SystemTime>,
    kind: Option<ContentKind>,
    entries: HashMap<ContentKind, Vec<ModEntry>>,
    textures: HashMap<ModKey, egui::TextureHandle>,
    details: HashMap<ModKey, Details>,
    asked_details: HashSet<ModKey>,
    updates: HashMap<ModKey, UpdateOffer>,
    checking: bool,
    checked: bool,
    recheck: bool,
    /// The last update check was asked for with the button (its failure is said out loud) rather than run on its own.
    manual_check: bool,
    /// Why the last update check failed, shown next to its button until one works.
    update_error: Option<String>,
    synced: bool,
    filter: String,
    show: Show,
    order: Order,
    origin: Origin,
    /// When each file was last written (put in place or updated), for "Recently added"; read when the list changes.
    added: HashMap<ModKey, SystemTime>,
    select_mode: bool,
    selected: HashSet<ModKey>,
    expanded: Option<ModKey>,
    busy: Option<String>,
    /// Quick changes still running for this instance; `busy` clears when the last one ends.
    jobs: usize,
    /// What the instance is (loader, Minecraft version) and what shaders need, worked out when the instance changes
    /// rather than on every frame (both read files).
    target: Option<crate::mods::InstanceTarget>,
    shader_setup: Option<versions::ShaderSetup>,
    confirm_remove: Vec<ModKey>,
    confirm_restore: Option<PathBuf>,
    confirm_pack: bool,
    versions: Option<VersionsDialog>,
    scan: Option<ScanReport>,
    scanning: bool,
    last_report: Option<SyncReport>,
    /// Why looking through the folders failed, shown in place of the list until it works.
    sync_error: Option<String>,
}

impl ModsState {
    fn kind(&self) -> ContentKind {
        self.kind.unwrap_or(ContentKind::Mods)
    }

    fn reset_for(&mut self, folder: &str) {
        let kind = self.kind;
        *self = ModsState::default();
        self.kind = kind;
        self.folder = folder.to_string();
    }
}

/// What the player asked for, collected while drawing and carried out afterwards.
enum Act {
    Kind(ContentKind),
    Toggle(ModKey, bool),
    Remove(Vec<ModKey>),
    Update(ModKey),
    UpdateAll,
    CheckUpdates,
    CheckOne(ModKey),
    Versions(ModKey),
    Reinstall(ModKey),
    Refresh(Vec<ModKey>),
    Rescan,
    Details(ModKey),
    ShowInFolder(ModKey),
    Url(String),
    Add,
    OpenFolder,
    Scan,
    Snapshot,
    Restore(PathBuf),
    Pack,
    ShaderLoader,
    EnableAll(Vec<ModKey>, bool),
    Select(ModKey, bool),
    ClearFilter,
    /// Open the mod loader dialog of the instance.
    Loader,
    DismissScan,
}

/// The platforms besides Modrinth that can offer updates. The CurseForge module adds its source here.
fn external_sources() -> Vec<Box<dyn updates::ExternalSource>> {
    Vec::new()
}

fn discover_kind(kind: ContentKind) -> Kind {
    match kind {
        ContentKind::Mods => Kind::Mod,
        ContentKind::ResourcePacks => Kind::ResourcePack,
        ContentKind::Shaders => Kind::Shader,
    }
}

/// Reads the pictures and details of some jars. This opens each file, so it is meant for a background thread.
pub fn read_details(
    items: Vec<(ModKey, PathBuf)>,
    loader_key: Option<&str>,
    mc: &str,
) -> Vec<(ModKey, Details, Option<egui::ColorImage>)> {
    items
        .into_iter()
        .map(|(key, path)| {
            let d = metadata::detect(&path);
            let problem = if key.kind == "mods" { d.problem_for(loader_key, mc) } else { None };
            let image = metadata::read_icon(&path, &d.info).and_then(|bytes| thumbnail(&bytes));
            (key, Details { info: d.info, problem }, image)
        })
        .collect()
}

/// A picture shrunk to what a list row shows, so hundreds of mods don't hold hundreds of big textures.
fn thumbnail(bytes: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(bytes).ok()?.thumbnail(72, 72).to_rgba8();
    // a wide or tall logo is centred on a square, so the row shows it undistorted
    let side = img.width().max(img.height());
    let mut square = image::RgbaImage::new(side, side);
    image::imageops::overlay(
        &mut square,
        &img,
        i64::from((side - img.width()) / 2),
        i64::from((side - img.height()) / 2),
    );
    Some(egui::ColorImage::from_rgba_unmultiplied([side as usize, side as usize], square.as_raw()))
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// A long list in a scroll area where only the rows in view (and a little around them) are laid out; the others
/// take the height they had when last drawn, or `guess`. Hundreds of rows then cost what a screenful costs.
pub(super) fn lazy_list<T>(ui: &mut Ui, id: egui::Id, items: &[T], guess: f32, row: impl FnMut(&mut Ui, &T)) {
    lazy_list_in(ui, id, items, guess, false, row);
}

/// [`lazy_list`], and with `shrink` the list is only as tall as its rows (up to the room there is), so the card
/// around a short list ends under its last row.
fn lazy_list_in<T>(ui: &mut Ui, id: egui::Id, items: &[T], guess: f32, shrink: bool, mut row: impl FnMut(&mut Ui, &T)) {
    let mut heights: Vec<f32> = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    heights.resize(items.len(), guess);
    ScrollArea::vertical().id_salt(id).auto_shrink([false, shrink]).show_viewport(ui, |ui, viewport| {
        let origin = ui.max_rect().top();
        let margin = 120.0;
        for (i, item) in items.iter().enumerate() {
            let top = ui.cursor().min.y - origin;
            if top + heights[i] < viewport.min.y - margin || top > viewport.max.y + margin {
                ui.add_space(heights[i]);
                continue;
            }
            row(ui, item);
            // what the row took, gaps included, so skipping it later moves the same distance
            heights[i] = ui.cursor().min.y - origin - top;
        }
    });
    ui.data_mut(|d| d.insert_temp(id, heights));
}

/// One lock per instance. Changes read `instance.json`, change the mod list and save it; two at once (switching
/// two mods off quickly, or an update while one is removed) would each save their own copy and lose the other's.
pub(super) fn instance_lock(root: &Path) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    locks.entry(root.to_path_buf()).or_default().clone()
}

fn short(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}

/// The kinds of content as one control: each a tab with its count in a small badge, the open one on a raised pill
/// that slides to the one opened next. Returns the kind that was opened.
fn kind_tabs(ui: &mut Ui, kinds: &[(ContentKind, usize)], open: ContentKind) -> Option<ContentKind> {
    const PAD: f32 = 14.0;
    let font = super::fonts::medium(13.0);
    let badge_font = super::fonts::medium(FS_META - 1.0);
    let badge_w = |n: usize, ui: &Ui| {
        ui.painter().layout_no_wrap(n.to_string(), badge_font.clone(), c_text()).size().x.max(8.0) + 12.0
    };
    let widths: Vec<f32> = kinds
        .iter()
        .map(|(k, n)| {
            let label = ui.painter().layout_no_wrap(k.label().to_string(), font.clone(), c_text()).size().x;
            label + 2.0 * PAD + if *n > 0 { badge_w(*n, ui) + 7.0 } else { 0.0 }
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 6.0;
    let (outer, outer_resp) = ui.allocate_exact_size(egui::vec2(total, 36.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect(outer, CornerRadius::same(R_CONTROL + 1), c_field(), Stroke::new(1.0, c_border()), egui::StrokeKind::Inside);
    let chosen = kinds.iter().position(|(k, _)| *k == open).unwrap_or(0);
    let sel_x = outer.left() + 3.0 + widths.iter().take(chosen).sum::<f32>();
    if let Some(w) = widths.get(chosen) {
        let id = outer_resp.id.with("pill");
        let x = motion::glide(ui.ctx(), id.with("x"), sel_x, motion::MEDIUM);
        let pw = motion::glide(ui.ctx(), id.with("w"), *w, motion::MEDIUM);
        let pill = egui::Rect::from_min_size(egui::pos2(x, outer.top() + 3.0), egui::vec2(pw, 30.0));
        let radius = CornerRadius::same(R_CONTROL - 2);
        let shade = egui::Color32::from_black_alpha(if is_dark() { 90 } else { 26 });
        p.add(egui::Shadow { offset: [0, 2], blur: 8, spread: 0, color: shade }.as_shape(pill, radius));
        p.rect_filled(pill, radius, if is_dark() { c_active() } else { c_card() });
        glass_edge(p, pill, R_CONTROL - 2);
    }
    let mut pressed = None;
    let mut x = outer.left() + 3.0;
    for (i, ((k, n), w)) in kinds.iter().zip(widths).enumerate() {
        let rect = egui::Rect::from_min_size(egui::pos2(x, outer.top() + 3.0), egui::vec2(w, 30.0));
        x += w;
        let resp = ui.interact(rect, ui.scope_id().with(("kind-tab", i)), egui::Sense::click());
        let on = *k == open;
        let t = motion::hover(ui, resp.id, on || resp.hovered());
        let s = motion::toward(ui.ctx(), resp.id.with("on"), on, motion::MEDIUM);
        let fg = motion::mix(c_muted(), c_text(), t);
        let galley = ui.painter().layout_no_wrap(k.label().to_string(), font.clone(), fg);
        let label_y = rect.center().y - galley.size().y / 2.0;
        let label_w = galley.size().x;
        ui.painter().galley(egui::pos2(rect.left() + PAD, label_y), galley, fg);
        if *n > 0 {
            // the count: a quiet badge, in the brand's soft tint on the open kind
            let bw = badge_w(*n, ui);
            let badge = egui::Rect::from_min_size(
                egui::pos2(rect.left() + PAD + label_w + 7.0, rect.center().y - 9.0),
                egui::vec2(bw, 18.0),
            );
            let ink = if is_dark() { c_accent() } else { c_accent_hover() };
            let bg = motion::mix(c_hover(), c_accent_soft(), s);
            ui.painter().rect_filled(badge, CornerRadius::same(9), bg);
            let bfg = motion::mix(c_muted(), ink, s);
            let g = ui.painter().layout_no_wrap(n.to_string(), badge_font.clone(), bfg);
            ui.painter().galley(badge.center() - g.size() / 2.0, g, bfg);
        }
        focus_ring(ui, &resp, rect, R_CONTROL - 2);
        let label = if *n > 0 { format!("{} {n}", k.label()) } else { k.label().to_string() };
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, on, &label));
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
            pressed = Some(*k);
        }
    }
    pressed
}

/// The toolbar over the list: the search and the On / Off choice (`find`), the origin and the order (`view`), and
/// the tools at the right end. All on one line when they fit; otherwise the search on a line of its own with the
/// rest sharing the next; and in a very narrow window each part wraps on lines of its own, the tools at the right
/// while they fit on one line. What the closures share (`state`) is handed to each.
fn tool_rows<S>(
    ui: &mut Ui,
    id: egui::Id,
    state: &mut S,
    find: impl FnOnce(&mut Ui, &mut S),
    view: impl FnOnce(&mut Ui, &mut S),
    tools: impl FnOnce(&mut Ui, &mut S),
) {
    let (find_id, view_id, tools_id) = (id.with("find"), id.with("view"), id.with("tools"));
    let find_w = remembered_width(ui, find_id, 0.0);
    let view_w = remembered_width(ui, view_id, 0.0);
    let tools_w = remembered_width(ui, tools_id, 240.0);
    let avail = ui.available_width();
    let gap = ui.spacing().item_spacing.x;
    /// A part laid out on one line, keeping what it took.
    fn natural<S>(ui: &mut Ui, state: &mut S, id: egui::Id, add: impl FnOnce(&mut Ui, &mut S)) {
        let used = ui.horizontal(|ui| add(ui, state)).response.rect.width();
        remember_width(ui, id, used);
    }
    /// A part that wraps, keeping what it took: the whole line once it wraps, so the parts stay apart until there
    /// is room for them side by side.
    fn wrapped<S>(ui: &mut Ui, state: &mut S, id: egui::Id, add: impl FnOnce(&mut Ui, &mut S)) {
        let used = ui
            .horizontal_wrapped(|ui| {
                add(ui, state);
                ui.min_rect().width()
            })
            .inner;
        remember_width(ui, id, used);
    }
    /// The tools at the right end of the line they are on.
    fn at_end<S>(ui: &mut Ui, state: &mut S, id: egui::Id, add: impl FnOnce(&mut Ui, &mut S)) {
        let used = super::discover::end_aligned(ui, id.with("end"), |ui| add(ui, state));
        remember_width(ui, id, used);
    }
    if avail >= find_w + view_w + tools_w + 2.0 * gap + 16.0 {
        ui.horizontal(|ui| {
            natural(ui, state, find_id, find);
            natural(ui, state, view_id, view);
            at_end(ui, state, tools_id, tools);
        });
    } else if avail >= find_w && avail >= view_w + tools_w + gap + 8.0 {
        natural(ui, state, find_id, find);
        ui.horizontal(|ui| {
            natural(ui, state, view_id, view);
            at_end(ui, state, tools_id, tools);
        });
    } else {
        wrapped(ui, state, find_id, find);
        wrapped(ui, state, view_id, view);
        if tools_w <= avail {
            ui.horizontal(|ui| at_end(ui, state, tools_id, tools));
        } else {
            wrapped(ui, state, tools_id, tools);
        }
    }
}

/// The search box of the list, with a magnifier at its start.
fn filter_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    let r = ui.add_sized(
        [width, 34.0],
        egui::TextEdit::singleline(text)
            .hint_text(RichText::new(hint).color(c_faint()))
            .margin(egui::Margin { left: 32, right: 10, top: 7, bottom: 7 })
            .background_color(c_field()),
    );
    let color = if r.has_focus() { c_text() } else { c_faint() };
    icons::paint(ui.painter(), Icon::Search, egui::pos2(r.rect.left() + 17.0, r.rect.center().y), 13.0, color);
    r
}

/// Something running for the list: a small spinner and what it is.
fn working(ui: &mut Ui, text: &str) {
    spinner_sized(ui, 14.0);
    ui.label(RichText::new(text).size(FS_META).color(c_muted()));
}

/// The picture of an empty list of a kind.
fn kind_icon(kind: ContentKind) -> Icon {
    match kind {
        ContentKind::Mods => Icon::Package,
        ContentKind::ResourcePacks => Icon::Layers,
        ContentKind::Shaders => Icon::Sliders,
    }
}

/// An empty list: a soft round picture, one sentence and, if there is one, the action. Returns the action's response.
fn empty_state(ui: &mut Ui, icon: Icon, text: &str, action: Option<&str>) -> Option<egui::Response> {
    ui.add_space(18.0);
    let out = ui
        .vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 24.0, c_hover());
            icons::paint(ui.painter(), icon, r.center(), 20.0, c_muted());
            ui.add_space(6.0);
            ui.label(RichText::new(text).color(c_muted()));
            action.map(|a| {
                ui.add_space(6.0);
                secondary(ui, a)
            })
        })
        .inner;
    ui.add_space(18.0);
    out
}

/// A quiet action of a list row: text that fades in with the row's fill (`shown`, 0 to 1) while the row is pointed
/// at, or at once when the keyboard reaches it, with a fill of its own under the pointer. `hot` is its colour then.
fn row_action(ui: &mut Ui, text: &str, hot: Option<egui::Color32>, shown: f32) -> egui::Response {
    let font = super::fonts::medium(FS_BODY);
    let w = ui.painter().layout_no_wrap(text.to_string(), font.clone(), c_muted()).size().x;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w + 20.0, 28.0), egui::Sense::click());
    let pointed = resp.hovered() || resp.is_pointer_button_down_on();
    let p = motion::hover(ui, resp.id, pointed);
    let alpha = if resp.has_focus() { 1.0 } else { shown.max(p) };
    if ui.is_rect_visible(rect) && alpha > 0.0 {
        if p > 0.0 {
            ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL - 2), c_active().gamma_multiply(p));
        }
        let fg = motion::mix(c_muted(), hot.unwrap_or_else(c_text), p).gamma_multiply(alpha);
        let galley = ui.painter().layout_no_wrap(text.to_string(), font, fg);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    }
    focus_ring(ui, &resp, rect, R_CONTROL - 2);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The tick box of a row while choosing several: an outlined rounded square, filled with the brand gradient and a
/// tick when it is ticked.
fn check(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = motion::toward(ui.ctx(), resp.id.with("on"), *on, motion::FAST);
    let h = motion::hover(ui, resp.id, resp.hovered());
    let r = rect.shrink(1.0);
    let line = motion::mix(c_control(), c_text(), h * 0.5).gamma_multiply(1.0 - t);
    ui.painter().rect(r, CornerRadius::same(R_TAG), c_field(), Stroke::new(1.5, line), egui::StrokeKind::Inside);
    if t > 0.0 {
        motion::gradient_rect(ui.painter(), r, R_TAG, c_accent().gamma_multiply(t), c_accent2().gamma_multiply(t));
        let c = r.center();
        let tick = vec![c + egui::vec2(-4.5, 0.2), c + egui::vec2(-1.3, 3.4), c + egui::vec2(4.6, -3.2)];
        ui.painter().add(egui::Shape::line(tick, Stroke::new(2.0, c_on_accent().gamma_multiply(t))));
    }
    focus_ring(ui, &resp, r, R_TAG);
    let state = *on;
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, state, ""));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// What an update pill adds to the width of its text: the arrow and the padding.
const UPDATE_PAD: f32 = 36.0;

/// A row's update: a small pill in the brand's soft tint with an arrow, always shown so what can be updated is seen at
/// a glance, that turns into the brand gradient under the pointer. Greyed while the list is busy.
fn update_pill(ui: &mut Ui, text: &str) -> egui::Response {
    let enabled = ui.is_enabled();
    let font = super::fonts::medium(FS_META);
    let w = ui.painter().layout_no_wrap(text.to_string(), font.clone(), c_text()).size().x;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w + UPDATE_PAD, 26.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let t = motion::hover(ui, resp.id, enabled && resp.hovered());
        let ink = if is_dark() { c_accent() } else { c_accent_hover() };
        let fg = if enabled { motion::mix(ink, c_on_accent(), t) } else { c_faint() };
        let line = if enabled { c_accent().gamma_multiply(0.35 * (1.0 - t)) } else { c_border() };
        ui.painter().rect(
            rect,
            CornerRadius::same(13),
            if enabled { c_accent_soft() } else { c_field() },
            Stroke::new(1.0, line),
            egui::StrokeKind::Inside,
        );
        if t > 0.0 {
            motion::gradient_rect(ui.painter(), rect, 13, c_accent().gamma_multiply(t), c_accent2().gamma_multiply(t));
        }
        // an arrow pointing up: a newer version
        let a = egui::pos2(rect.left() + 15.0, rect.center().y);
        let s = Stroke::new(1.6, fg);
        ui.painter().line_segment([a + egui::vec2(0.0, 4.5), a + egui::vec2(0.0, -4.5)], s);
        ui.painter().line_segment([a + egui::vec2(-3.5, -1.0), a + egui::vec2(0.0, -4.5)], s);
        ui.painter().line_segment([a + egui::vec2(3.5, -1.0), a + egui::vec2(0.0, -4.5)], s);
        let galley = ui.painter().layout_no_wrap(text.to_string(), font, fg);
        ui.painter().galley(egui::pos2(rect.left() + 25.0, rect.center().y - galley.size().y / 2.0), galley, fg);
    }
    focus_ring(ui, &resp, rect, 13);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, text));
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

impl App {
    // -----------------------------------------------------------------------------------------------------
    // Background work

    /// Runs a change to one instance in the background with a fresh copy of it, then reloads the screen.
    pub(super) fn mods_job<F, Fut>(&mut self, folder: &str, label: &str, work: F)
    where
        F: FnOnce(Instance, ModContext) -> Fut + Send + 'static,
        Fut: Future<Output = anyhow::Result<String>> + Send + 'static,
    {
        let Some(root) = self.instance(folder).map(|i| i.root.clone()) else { return };
        // a change made from elsewhere (a CurseForge remove for another instance) isn't this tab's to wait for
        if folder == self.mods.folder {
            self.mods.busy = Some(label.to_string());
            self.mods.jobs += 1;
        }
        let (dirs, settings, n, folder) =
            (self.dirs.clone(), self.settings.clone(), self.notifier(), folder.to_string());
        let lock = instance_lock(&root);
        self.rt.spawn(async move {
            let result = async {
                let _turn = lock.lock().await;
                let inst = instances::load_one(&root)
                    .ok_or_else(|| anyhow::anyhow!("that instance can't be read any more"))?;
                let ctx = ModContext::build(&dirs, &inst, &settings).await;
                work(inst, ctx).await
            }
            .await
            .map_err(|e| format!("{e:#}"));
            n.send(Msg::Mods(ModsMsg::Done { folder, result }));
        });
    }

    /// Like [`Self::mods_job`] but shown in the sidebar with a progress bar, for downloads.
    fn mods_task<F, Fut>(&mut self, folder: &str, label: String, work: F)
    where
        F: FnOnce(Instance, ModContext, std::sync::Arc<crate::install::Progress>) -> Fut + Send + 'static,
        Fut: Future<Output = anyhow::Result<String>> + Send + 'static,
    {
        let Some(root) = self.instance(folder).map(|i| i.root.clone()) else { return };
        let (dirs, settings, lock) = (self.dirs.clone(), self.settings.clone(), instance_lock(&root));
        self.start_task(label, move |progress, _| async move {
            let _turn = lock.lock().await;
            let inst =
                instances::load_one(&root).ok_or_else(|| anyhow::anyhow!("that instance can't be read any more"))?;
            let ctx = ModContext::build(&dirs, &inst, &settings).await;
            work(inst, ctx, progress).await
        });
        self.mods.recheck = true;
    }

    /// Compares the folders with the mod list, then reads what is in the jars and looks for updates.
    fn mods_sync(&mut self, inst: &Instance) {
        let (root, settings, n, folder) =
            (inst.root.clone(), ModSettings::from_settings(&self.settings), self.notifier(), inst.folder.clone());
        self.mods.synced = true;
        let lock = instance_lock(&root);
        self.rt.spawn(async move {
            let result = async {
                let _turn = lock.lock().await;
                let mut inst = instances::load_one(&root)
                    .ok_or_else(|| anyhow::anyhow!("that instance can't be read any more"))?;
                files::sync(&mut inst, &settings, ScanScope::Full).await
            }
            .await
            .map_err(|e| format!("{e:#}"));
            n.send(Msg::Mods(ModsMsg::Synced { folder, result }));
        });
    }

    fn mods_load_details(&mut self, inst: &Instance) {
        let loader = inst.loader().and_then(|(k, _)| versions::loader_key(k));
        let mc = inst.mc_version().to_string();
        let mut wanted: Vec<(ModKey, PathBuf)> = Vec::new();
        for list in self.mods.entries.values() {
            for e in list {
                if let Some(p) = e.path.clone().filter(|_| e.exists) {
                    if self.mods.asked_details.insert(e.key()) {
                        wanted.push((e.key(), p));
                    }
                }
            }
        }
        if wanted.is_empty() {
            return;
        }
        let (n, folder) = (self.notifier(), inst.folder.clone());
        self.rt.spawn_blocking(move || {
            for chunk in wanted.chunks(8) {
                let batch = read_details(chunk.to_vec(), loader, &mc);
                n.send(Msg::Mods(ModsMsg::Details { folder: folder.clone(), batch }));
            }
        });
    }

    fn mods_check_updates(&mut self, inst: &Instance) {
        if self.mods.checking {
            return;
        }
        self.mods.checking = true;
        let (root, dirs, settings, n, folder) =
            (inst.root.clone(), self.dirs.clone(), self.settings.clone(), self.notifier(), inst.folder.clone());
        self.rt.spawn(async move {
            let result = async {
                let inst = instances::load_one(&root)
                    .ok_or_else(|| anyhow::anyhow!("that instance can't be read any more"))?;
                let ctx = ModContext::build(&dirs, &inst, &settings).await;
                let sources = external_sources();
                let refs: Vec<&dyn updates::ExternalSource> = sources.iter().map(|b| b.as_ref()).collect();
                let mut all = Vec::new();
                for kind in ContentKind::ALL {
                    all.extend(updates::find_updates_with(&inst, &ctx, kind, &refs).await?);
                }
                Ok(all)
            }
            .await
            .map_err(|e: anyhow::Error| format!("{e:#}"));
            n.send(Msg::Mods(ModsMsg::Updates { folder, result }));
        });
    }

    /// Brings the cached lists up to date when the instance, or its `instance.json`, changed.
    fn mods_prepare(&mut self, inst: &Instance) {
        let first_time = self.mods.folder != inst.folder;
        if first_time {
            self.mods.reset_for(&inst.folder);
        }
        let stamp = modified(&inst.instance_file());
        if first_time || stamp != self.mods.signature {
            self.mods.signature = stamp;
            let target = crate::mods::InstanceTarget::of(inst, &crate::mods::RemoteConfig::load(&self.dirs));
            self.mods.shader_setup = Some(install::shader_setup(inst, &target));
            self.mods.target = Some(target);
            self.mods.entries = ContentKind::ALL.iter().map(|k| (*k, files::entries(inst, *k))).collect();
            self.mods.added = self
                .mods
                .entries
                .values()
                .flatten()
                .filter_map(|e| Some((e.key(), modified(e.path.as_deref()?)?)))
                .collect();
            let keys: HashSet<ModKey> = self.mods.entries.values().flatten().map(|e| e.key()).collect();
            self.mods.selected.retain(|k| keys.contains(k));
            self.mods.updates.retain(|k, _| keys.contains(k));
            // a file replaced by an update has a new name, so its picture is read again
            self.mods.textures.retain(|k, _| keys.contains(k));
            self.mods.details.retain(|k, _| keys.contains(k));
            self.mods.asked_details.retain(|k| keys.contains(k));
            self.mods_load_details(inst);
            if self.mods.checked && self.mods.recheck {
                self.mods.recheck = false;
                self.mods_check_updates(inst);
            }
        }
        if !self.mods.synced {
            self.mods_sync(inst);
        }
    }

    pub(super) fn mods_handle(&mut self, msg: ModsMsg) {
        match msg {
            ModsMsg::Synced { folder, result } => {
                if folder != self.mods.folder {
                    return;
                }
                match result {
                    Ok(report) => {
                        if report.changed() {
                            if !report.added.is_empty() {
                                self.toast(format!("Found {} new file(s) in the folders", report.added.len()));
                            }
                            self.mods.signature = None;
                            self.reload_instances();
                        }
                        self.mods.last_report = Some(report);
                        self.mods.sync_error = None;
                    }
                    Err(e) => {
                        self.toast_err(format!("Couldn't look through the mod folders: {e}"));
                        self.mods.sync_error = Some(e);
                    }
                }
                if !self.mods.checked {
                    self.mods.checked = true;
                    if let Some(inst) = self.instance(&folder).cloned() {
                        self.mods_check_updates(&inst);
                    }
                }
            }
            ModsMsg::Details { folder, batch } => {
                if folder != self.mods.folder {
                    return;
                }
                for (key, details, image) in batch {
                    if let Some(img) = image {
                        let name = format!("modicon/{folder}/{}/{}", key.kind, key.file);
                        self.mods
                            .textures
                            .insert(key.clone(), self.ctx.load_texture(name, img, egui::TextureOptions::LINEAR));
                    }
                    self.mods.details.insert(key, details);
                }
            }
            ModsMsg::Updates { folder, result } => {
                if folder != self.mods.folder {
                    return;
                }
                self.mods.checking = false;
                let manual = std::mem::take(&mut self.mods.manual_check);
                match result {
                    Ok(list) => {
                        self.mods.updates = list.into_iter().map(|o| (o.key.clone(), o)).collect();
                        self.mods.update_error = None;
                    }
                    Err(e) => {
                        // a check that ran on its own (after an install, say) fails quietly next to its button;
                        // one asked for with the button says so
                        if manual {
                            self.toast_err(format!("Couldn't check for updates: {e}"));
                        }
                        self.mods.update_error = Some(e);
                    }
                }
            }
            ModsMsg::Done { folder, result } => {
                if folder == self.mods.folder {
                    self.mods.jobs = self.mods.jobs.saturating_sub(1);
                    if self.mods.jobs == 0 {
                        self.mods.busy = None;
                    }
                }
                match result {
                    Ok(m) if !m.is_empty() => self.toast(m),
                    Ok(_) => {}
                    Err(e) => self.toast_err(e),
                }
                self.mods.signature = None;
                self.reload_instances();
            }
            ModsMsg::Versions { folder, key, result } => {
                if folder != self.mods.folder {
                    return;
                }
                if let Some(d) = self.mods.versions.as_mut().filter(|d| d.key == key) {
                    d.list = Some(result);
                }
            }
            ModsMsg::Scan { folder, report } => {
                if folder != self.mods.folder {
                    return;
                }
                self.mods.scanning = false;
                self.mods.scan = Some(report);
            }
        }
    }

    /// A file dropped on the window while the Mods tab is open is added to the instance. Returns whether it was
    /// taken, so the caller knows not to complain about it.
    pub(super) fn mods_accept_drop(&mut self, path: &Path) -> bool {
        if self.page != Page::Instance || self.tab != InstTab::Mods {
            return false;
        }
        let Some(folder) = self.selected.clone() else { return false };
        let detected = metadata::detect(path);
        // a modpack becomes an instance of its own: the window's usual drop handling imports it
        if detected.kind.is_modpack() {
            return false;
        }
        let record_type = match detected.kind {
            metadata::FileKind::DataPack => "datapack",
            k => match k.content_kind() {
                Some(c) => c.record_type(),
                None => {
                    self.toast_err(format!("{} is not a mod, resource pack or shader pack", detected.file_name));
                    return true;
                }
            },
        };
        let source = path.to_path_buf();
        let settings = ModSettings::from_settings(&self.settings);
        let name = detected.display_name().to_string();
        self.mods_job(&folder, "Adding", move |mut inst, ctx| async move {
            // still added (the player decides), but said: a mod for another loader or version won't load
            let problem = detected.problem_for(ctx.target.loader_key, &ctx.target.minecraft_version);
            match files::add_file_identified(&mut inst, &source, record_type, &settings).await? {
                files::AddOutcome::Added(_) => match problem {
                    Some(p) => Err(anyhow::anyhow!("Added {name}, but it may not load: {p}")),
                    None => Ok(format!("Added {name}")),
                },
                files::AddOutcome::AlreadyThere => Ok(format!("{name} is already in this instance")),
            }
        });
        // a new file may have an update of its own
        self.mods.recheck = self.mods.checked;
        true
    }

    // -----------------------------------------------------------------------------------------------------
    // Carrying out what was asked

    fn mods_apply(&mut self, inst: &Instance, act: Act) {
        let folder = inst.folder.clone();
        let settings = ModSettings::from_settings(&self.settings);
        // the game holds its files open (on Windows they can't be renamed or deleted at all), and a file switched or
        // removed under it confuses the list once it closes
        let changes_files = matches!(
            act,
            Act::Toggle(..)
                | Act::EnableAll(..)
                | Act::Remove(_)
                | Act::Update(_)
                | Act::UpdateAll
                | Act::Restore(_)
                | Act::Reinstall(_)
        );
        if changes_files && self.is_playing(&folder) {
            self.toast_err("Close Minecraft first. Its files can't be changed while it runs.");
            return;
        }
        match act {
            Act::Kind(k) => {
                self.mods.kind = Some(k);
                self.mods.expanded = None;
                self.mods.selected.clear();
            }
            Act::Toggle(key, on) => {
                self.mods_job(&folder, "Switching", move |mut inst, _| async move {
                    tokio::task::spawn_blocking(move || files::set_enabled(&mut inst, &key, on).map(|_| String::new()))
                        .await?
                });
            }
            Act::EnableAll(keys, on) => {
                self.mods_job(&folder, "Switching", move |mut inst, _| async move {
                    tokio::task::spawn_blocking(move || {
                        let mut failed = Vec::new();
                        for k in &keys {
                            if let Err(e) = files::set_enabled(&mut inst, k, on) {
                                failed.push(format!("{}: {e:#}", k.file));
                            }
                        }
                        if failed.is_empty() { Ok(String::new()) } else { Err(anyhow::anyhow!(failed.join("; "))) }
                    })
                    .await?
                });
            }
            Act::ClearFilter => {
                self.mods.filter.clear();
                self.mods.show = Show::All;
                self.mods.origin = Origin::Anywhere;
            }
            Act::Loader => self.apply(super::instance_menu::Act::ChangeLoader(folder)),
            Act::DismissScan => self.mods.scan = None,
            Act::Select(key, on) => {
                if on {
                    self.mods.selected.insert(key);
                } else {
                    self.mods.selected.remove(&key);
                }
            }
            Act::Remove(keys) => self.mods.confirm_remove = keys,
            Act::Update(key) => {
                if let Some(offer) = self.mods.updates.get(&key).cloned() {
                    let label = format!("Updating {}", offer.name);
                    self.mods_task(&folder, label, move |mut inst, ctx, progress| async move {
                        let sources = external_sources();
                        let refs: Vec<&dyn updates::ExternalSource> = sources.iter().map(|b| b.as_ref()).collect();
                        updates::apply_update_with(&mut inst, &ctx, &offer, &progress, &refs).await?;
                        Ok(format!("Updated {} to {}", offer.name, offer.newer))
                    });
                }
            }
            Act::UpdateAll => {
                let kind = self.mods.kind();
                let offers: Vec<UpdateOffer> = self.mods.updates.values().filter(|o| o.kind == kind).cloned().collect();
                if !offers.is_empty() {
                    let what = if offers.len() == 1 { "item".to_string() } else { kind.label().to_lowercase() };
                    self.mods_task(
                        &folder,
                        format!("Updating {} {what}", offers.len()),
                        move |mut inst, ctx, progress| async move {
                            let sources = external_sources();
                            let refs: Vec<&dyn updates::ExternalSource> = sources.iter().map(|b| b.as_ref()).collect();
                            let report = updates::update_all(&mut inst, &ctx, &offers, &progress, &refs).await;
                            if report.failed.is_empty() {
                                Ok(format!("Updated {} item(s)", report.updated.len()))
                            } else {
                                let names: Vec<String> =
                                    report.failed.iter().map(|(n, e)| format!("{n} ({e})")).collect();
                                Err(anyhow::anyhow!(
                                    "Updated {}, but couldn't update: {}",
                                    report.updated.len(),
                                    names.join(", ")
                                ))
                            }
                        },
                    );
                }
            }
            Act::CheckUpdates => {
                self.mods.manual_check = true;
                self.mods_check_updates(inst);
                self.mods.checked = true;
            }
            Act::CheckOne(key) => {
                let name =
                    self.mods.entries.values().flatten().find(|e| e.key() == key).map(|e| e.name()).unwrap_or_default();
                self.mods_job(&folder, "Checking", move |inst, ctx| async move {
                    Ok(match updates::check_single(&inst, &ctx, &key).await? {
                        Some(v) => {
                            format!("{name}: a newer version is available, {}. Open Versions to install it.", v.label())
                        }
                        None => format!("{name} is up to date"),
                    })
                });
            }
            Act::Versions(key) => {
                let name =
                    self.mods.entries.values().flatten().find(|e| e.key() == key).map(|e| e.name()).unwrap_or_default();
                let installed_id = inst
                    .mod_records()
                    .iter()
                    .find(|m| ModKey::of(m) == key)
                    .and_then(|m| m.modrinth_version_id().map(str::to_string));
                self.mods.versions = Some(VersionsDialog { key: key.clone(), name, list: None, installed_id });
                let (root, dirs, s, n) = (inst.root.clone(), self.dirs.clone(), self.settings.clone(), self.notifier());
                self.rt.spawn(async move {
                    let result = async {
                        let inst = instances::load_one(&root)
                            .ok_or_else(|| anyhow::anyhow!("that instance can't be read any more"))?;
                        let ctx = ModContext::build(&dirs, &inst, &s).await;
                        updates::available_versions(&inst, &ctx, &key).await
                    }
                    .await
                    .map_err(|e| format!("{e:#}"));
                    n.send(Msg::Mods(ModsMsg::Versions { folder, key, result }));
                });
            }
            Act::Reinstall(key) => {
                self.mods_task(
                    &folder,
                    format!("Reinstalling {}", key.file),
                    move |mut inst, ctx, progress| async move {
                        updates::reinstall(&mut inst, &ctx, &key, &progress).await?;
                        Ok(format!("Reinstalled {}", key.file))
                    },
                );
            }
            Act::Refresh(keys) => {
                self.mods_job(&folder, "Refreshing", move |mut inst, ctx| async move {
                    let n = crate::mods::identify::refresh_metadata(&mut inst, &keys, &ctx.settings).await?;
                    Ok(format!("{n} of {} recognised on Modrinth", keys.len()))
                });
            }
            Act::Rescan => {
                self.mods.synced = false;
                self.mods.sync_error = None;
            }
            Act::Details(key) => {
                self.mods.expanded = if self.mods.expanded.as_ref() == Some(&key) { None } else { Some(key) };
            }
            Act::ShowInFolder(key) => {
                if let Some(p) =
                    self.mods.entries.values().flatten().find(|e| e.key() == key).and_then(|e| e.path.clone())
                {
                    let _ = open::that(p.parent().unwrap_or(&inst.root));
                }
            }
            Act::Url(u) => {
                let _ = open::that(u);
            }
            Act::Add => {
                self.discover.kind = discover_kind(self.mods.kind());
                self.discover.target = Some(folder);
                self.page = Page::Discover;
                self.discover_search(true);
                // when CurseForge is the side shown, it opens on the same kind with a search for this instance
                self.cf_show_for(self.mods.kind());
            }
            Act::OpenFolder => {
                let dir = inst.root.join(self.mods.kind().folder());
                let _ = std::fs::create_dir_all(&dir);
                let _ = open::that(dir);
            }
            Act::Scan => {
                self.mods.scanning = true;
                let (dirs, inst2, n) = (self.dirs.clone(), inst.clone(), self.notifier());
                // a manual scan always runs, whatever scanModsOnLaunch says
                let mut s = settings;
                s.scan_mods_on_launch = true;
                self.rt.spawn_blocking(move || {
                    let report = crate::mods::scan::scan_instance(&dirs, &inst2, &s);
                    n.send(Msg::Mods(ModsMsg::Scan { folder, report }));
                });
            }
            Act::Snapshot => {
                let dirs = self.dirs.clone();
                self.mods_job(&folder, "Saving a snapshot", move |inst, _| async move {
                    tokio::task::spawn_blocking(move || {
                        snapshots::save_before(&dirs, &inst, "manual").map(|_| "Snapshot saved".to_string())
                    })
                    .await?
                });
            }
            Act::Restore(file) => self.mods.confirm_restore = Some(file),
            Act::Pack => self.mods.confirm_pack = true,
            Act::ShaderLoader => {
                self.mods_task(&folder, "Adding a shader loader".into(), move |mut inst, ctx, progress| async move {
                    let r = install::install_shader_loader(&mut inst, &ctx, &progress).await?;
                    Ok(format!("Added {}", r.installed.join(", ")))
                });
            }
        }
    }

    // -----------------------------------------------------------------------------------------------------
    // Drawing

    /// The Mods tab.
    pub(super) fn mods_tab(&mut self, ui: &mut Ui, inst: &Instance) {
        self.mods_prepare(inst);
        let mut acts: Vec<Act> = Vec::new();
        let kind = self.mods.kind();
        let busy = self.mods.busy.is_some()
            || self.tasks.iter().any(|t| {
                t.label.starts_with("Updating") || t.label.starts_with("Reinstalling") || t.label.starts_with("Adding")
            });
        let target = self
            .mods
            .target
            .clone()
            .unwrap_or_else(|| crate::mods::InstanceTarget::of(inst, &crate::mods::RemoteConfig::load(&self.dirs)));

        // kinds on the left, the folder and the main action on the right; then the search and the filters on the left
        // and the tools on the right. Each pair shares a line, or the right part goes on the next line in a narrow window.
        let total = self.mods.entries.get(&kind).map_or(0, Vec::len);
        let offers_n = self.mods.updates.values().filter(|o| o.kind == kind).count();
        let kinds_id = ui.scope_id().with("mods-kinds");
        let tools_id = ui.scope_id().with("mods-tools");
        let mut st = (&mut *self, &mut acts);
        split_or_stack(
            ui,
            kinds_id,
            34.0,
            &mut st,
            |ui, (this, acts)| {
                let kinds: Vec<(ContentKind, usize)> =
                    ContentKind::ALL.iter().map(|k| (*k, this.mods.entries.get(k).map_or(0, Vec::len))).collect();
                if let Some(k) = kind_tabs(ui, &kinds, kind) {
                    acts.push(Act::Kind(k));
                }
            },
            |ui, (_, acts)| {
                if secondary_icon(ui, Icon::Folder, "Open folder").clicked() {
                    acts.push(Act::OpenFolder);
                }
                if primary_icon(ui, Icon::Plus, "Add").clicked() {
                    acts.push(Act::Add);
                }
            },
        );
        ui.add_space(10.0);
        tool_rows(
            ui,
            tools_id,
            &mut st,
            |ui, (this, _)| {
                let mut f = std::mem::take(&mut this.mods.filter);
                let width = 220.0f32.min(ui.available_width());
                filter_field(ui, &mut f, &format!("Filter {}", kind.label().to_lowercase()), width);
                this.mods.filter = f;
                ui.add_space(4.0);
                let updates = if offers_n > 0 { format!("Updates {offers_n}") } else { "Updates".to_string() };
                let choices =
                    [(Show::All, "All"), (Show::On, "On"), (Show::Off, "Off"), (Show::Updates, updates.as_str())];
                ui.spacing_mut().item_spacing.x = 6.0;
                for (show, label) in choices {
                    if chip(ui, label, this.mods.show == show).clicked() {
                        this.mods.show = show;
                    }
                }
            },
            |ui, (this, _)| {
                this.mods_origin_dropdown(ui, kind);
                let order = &mut this.mods.order;
                super::discover::dropdown(ui, "mods-order", "Sort", order.label(), false, false, |ui| {
                    for o in Order::ALL {
                        ui.selectable_value(order, o, o.label());
                    }
                });
            },
            |ui, (this, acts)| {
                if let Some(b) = &this.mods.busy {
                    working(ui, &format!("{b}\u{2026}"));
                }
                if this.mods.checking {
                    working(ui, "Checking for updates");
                } else {
                    if let Some(e) = &this.mods.update_error {
                        ui.label(RichText::new("Couldn't check").size(FS_META).color(c_warn()))
                            .on_hover_text(format!("The last update check failed: {e}"));
                    }
                    if secondary(ui, "Check for updates").clicked() {
                        acts.push(Act::CheckUpdates);
                    }
                }
                if offers_n > 0 {
                    let all = format!("Update all ({offers_n})");
                    if ui
                        .add_enabled_ui(!busy, |ui| secondary(ui, &all))
                        .inner
                        .on_disabled_hover_text("Wait for the change that is running to finish")
                        .clicked()
                    {
                        acts.push(Act::UpdateAll);
                    }
                }
                let select = if this.mods.select_mode { "Done selecting" } else { "Select" };
                if ghost(ui, select).clicked() {
                    this.mods.select_mode = !this.mods.select_mode;
                    this.mods.selected.clear();
                }
                secondary_menu(ui, "More", |ui| {
                    if ui.button("Look for new and missing files").clicked() {
                        acts.push(Act::Rescan);
                        ui.close();
                    }
                    if ui.add_enabled(!this.mods.scanning, egui::Button::new("Scan for malware")).clicked() {
                        acts.push(Act::Scan);
                        ui.close();
                    }
                    if kind == ContentKind::Mods
                        && performance::offered_for(target.loader_key)
                        && ui.add_enabled(!busy, egui::Button::new("Make Minecraft run faster")).clicked()
                    {
                        acts.push(Act::Pack);
                        ui.close();
                    }
                    if ui.button("Refresh names from Modrinth").clicked() {
                        let keys: Vec<ModKey> =
                            this.mods.entries.get(&kind).into_iter().flatten().map(|e| e.key()).collect();
                        acts.push(Act::Refresh(keys));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Save a snapshot").clicked() {
                        acts.push(Act::Snapshot);
                        ui.close();
                    }
                    let saved = snapshots::latest(&snapshots::folder_among(&this.dirs, inst, &this.instances), 8);
                    if saved.is_empty() {
                        ui.add_enabled(false, egui::Button::new("No snapshots yet"));
                    }
                    for s in saved {
                        let when = time_ago(s.created_at);
                        if ui.button(format!("Restore: {} ({when})", s.label())).clicked() {
                            acts.push(Act::Restore(s.file.clone()));
                            ui.close();
                        }
                    }
                });
            },
        );

        if self.mods.select_mode {
            let chosen: Vec<ModKey> = self.mods.selected.iter().cloned().collect();
            let shown: Vec<ModKey> = self.mods_rows(kind).iter().map(|e| e.key()).collect();
            let all_shown = !shown.is_empty() && shown.iter().all(|k| self.mods.selected.contains(k));
            let (mut select_all, mut select_none, mut done) = (false, false, false);
            ui.add_space(10.0);
            // the selection and what can be done with it, on a strip in the brand's soft tint: how many and the quick
            // ways to choose at the left, what to do with them at the right
            egui::Frame::new()
                .fill(c_accent_soft())
                .stroke(Stroke::new(1.0, c_accent().gamma_multiply(0.35)))
                .corner_radius(CornerRadius::same(R_CONTROL))
                .inner_margin(egui::Margin::symmetric(14, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let any = !chosen.is_empty();
                    super::discover::dialog_footer(
                        ui,
                        "mods-selection",
                        |ui| {
                            let ink = if is_dark() { c_accent() } else { c_accent_hover() };
                            let n =
                                RichText::new(format!("{} selected", chosen.len())).font(super::fonts::medium(FS_BODY));
                            ui.label(n.color(ink));
                            ui.add_space(4.0);
                            if !all_shown && ghost(ui, &format!("Select all {}", shown.len())).clicked() {
                                select_all = true;
                            }
                            if any && ghost(ui, "Select none").clicked() {
                                select_none = true;
                            }
                        },
                        |ui| {
                            // greyed in place (not in a scope of their own), so they still wrap in a narrow window
                            if ui.add_enabled(any, |ui: &mut Ui| secondary(ui, "Switch on")).clicked() {
                                acts.push(Act::EnableAll(chosen.clone(), true));
                            }
                            if ui.add_enabled(any, |ui: &mut Ui| secondary(ui, "Switch off")).clicked() {
                                acts.push(Act::EnableAll(chosen.clone(), false));
                            }
                            if ui.add_enabled(any, |ui: &mut Ui| secondary(ui, "Refresh names")).clicked() {
                                acts.push(Act::Refresh(chosen.clone()));
                            }
                            if ui.add_enabled(any, |ui: &mut Ui| danger(ui, "Remove")).clicked() {
                                acts.push(Act::Remove(chosen.clone()));
                            }
                            if ghost(ui, "Done").clicked() {
                                done = true;
                            }
                        },
                    );
                });
            if select_all {
                self.mods.selected.extend(shown);
            }
            if select_none {
                self.mods.selected.clear();
            }
            if done {
                self.mods.select_mode = false;
                self.mods.selected.clear();
            }
        }
        ui.add_space(12.0);

        // things worth knowing before the list
        self.mods_banners(ui, inst, &target, kind, &mut acts);

        // the list
        let rows = self.mods_rows(kind);
        let loading = total == 0 && self.mods.last_report.is_none() && self.mods.sync_error.is_none();
        if total == 0 && !loading {
            // nothing to list: why, and what to do, without a card around nothing
            if let Some(e) = &self.mods.sync_error {
                let text = format!("Couldn't look through the folders: {}", short(e, 200));
                if notice(ui, NoticeKind::Error, &text, Some("Try again")).is_some_and(|r| r.clicked()) {
                    acts.push(Act::Rescan);
                }
            } else if kind == ContentKind::Mods && target.loader_key.is_none() {
                let text =
                    "No mod loader yet. This instance runs plain Minecraft, and mods need a mod loader such as Fabric.";
                if notice(ui, NoticeKind::Info, text, Some("Add a mod loader")).is_some_and(|r| r.clicked()) {
                    acts.push(Act::Loader);
                }
            } else {
                let text = match kind {
                    ContentKind::Mods => "No mods yet. Add some from Modrinth, or drop .jar files onto this window.",
                    ContentKind::ResourcePacks => {
                        "No resource packs yet. Browse Modrinth, or drop .zip files onto this window."
                    }
                    ContentKind::Shaders => "No shaders yet. Browse Modrinth, or drop .zip files onto this window.",
                };
                card(ui, |ui| empty_state(ui, kind_icon(kind), text, None));
            }
        } else {
            // one glass card holds the list; it ends under the last row, or at the bottom of the window
            let last = rows.last().map(|e| e.key());
            let out = card_frame().inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if loading {
                    skeleton_rows(ui, 6, ROW_H);
                } else if rows.is_empty() {
                    let what = kind.label().to_lowercase();
                    let text = format!("No {what} match your filter.");
                    if empty_state(ui, Icon::Search, &text, Some("Clear filter")).is_some_and(|r| r.clicked()) {
                        acts.push(Act::ClearFilter);
                    }
                } else {
                    lazy_list_in(ui, egui::Id::unique(("mod-list", kind)), &rows, ROW_H, true, |ui, e| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let is_last = last.as_ref() == Some(&e.key());
                        self.mod_row(ui, e, busy, is_last, &mut acts);
                    });
                }
            });
            glass_edge(ui.painter(), out.response.rect, R_CARD);
        }

        let ctx = ui.ctx().clone();
        self.mods_dialogs(&ctx, inst);
        for a in acts {
            self.mods_apply(inst, a);
        }
    }

    /// The items of a kind the list shows: those the filter, the On / Off / Updates choice and the origin let through,
    /// in the order chosen.
    fn mods_rows(&self, kind: ContentKind) -> Vec<&ModEntry> {
        let m = &self.mods;
        let filter = m.filter.trim().to_lowercase();
        let mut rows: Vec<&ModEntry> = m
            .entries
            .get(&kind)
            .into_iter()
            .flatten()
            .filter(|e| {
                let matches = filter.is_empty()
                    || e.name().to_lowercase().contains(&filter)
                    || e.record.file.to_lowercase().contains(&filter)
                    || e.record.description.to_lowercase().contains(&filter);
                let shown = match m.show {
                    Show::All => true,
                    Show::On => !e.record.disabled,
                    Show::Off => e.record.disabled,
                    Show::Updates => m.updates.contains_key(&e.key()),
                };
                matches && shown && m.origin.has(&e.record)
            })
            .collect();
        order_rows(&mut rows, m.order, |e| m.added.get(&e.key()).copied(), |e| m.updates.contains_key(&e.key()));
        rows
    }

    /// The control that narrows the list to where its items came from, each choice with how many it has.
    fn mods_origin_dropdown(&mut self, ui: &mut Ui, kind: ContentKind) {
        let entries = self.mods.entries.get(&kind);
        let origin = &mut self.mods.origin;
        let active = *origin != Origin::Anywhere;
        super::discover::dropdown(ui, "mods-origin", "From", origin.label(), active, false, |ui| {
            ui.set_min_width(200.0);
            for o in Origin::ALL {
                let n = entries.into_iter().flatten().filter(|e| o.has(&e.record)).count();
                let r = ui.selectable_value(origin, o, format!("{}  ({n})", o.label()));
                if o == Origin::ByHand {
                    r.on_hover_text("Files that are on neither site: dropped onto the window, added from GitHub, or put in the folder");
                }
            }
        });
    }

    /// Notes above the list: the malware scan result, shader setup, a missing loader.
    fn mods_banners(
        &mut self,
        ui: &mut Ui,
        inst: &Instance,
        target: &crate::mods::InstanceTarget,
        kind: ContentKind,
        acts: &mut Vec<Act>,
    ) {
        if let Some(report) = &self.mods.scan {
            let (level, mut text) = match report.warning() {
                Some(w) => (NoticeKind::Error, w.to_string()),
                None => (
                    NoticeKind::Info,
                    format!(
                        "No malware found ({} files read, {} already known to be fine)",
                        report.scanned, report.known_clean
                    ),
                ),
            };
            if !report.unreadable.is_empty() {
                text.push_str(&format!(". {} file(s) couldn't be opened as a zip", report.unreadable.len()));
            }
            if notice(ui, level, &text, Some("Dismiss")).is_some_and(|r| r.clicked()) {
                acts.push(Act::DismissScan);
            }
            ui.add_space(6.0);
        } else if self.mods.scanning {
            ui.horizontal(|ui| {
                spinner_sized(ui, 14.0);
                muted(ui, "Scanning for malware");
            });
        }
        if kind == ContentKind::Shaders {
            let setup = self.mods.shader_setup.unwrap_or_else(|| install::shader_setup(inst, target));
            let (text, action) = match setup {
                versions::ShaderSetup::AddIris => (
                    "Shaders need Iris to work. Iris and Sodium will be added from Modrinth; they make shaders run fast."
                        .to_string(),
                    Some("Add Iris"),
                ),
                versions::ShaderSetup::AddOculus => (
                    format!(
                        "Shaders on Forge need Oculus or OptiFine. Oculus can be added from Modrinth if it's made for Minecraft {}.",
                        inst.mc_version()
                    ),
                    Some("Add Oculus"),
                ),
                versions::ShaderSetup::NeedsModLoader => (
                    "Shaders need a mod loader. Plain Minecraft can't show shaders; they work once the instance has Fabric and Iris."
                        .to_string(),
                    None,
                ),
                versions::ShaderSetup::Ready => return,
            };
            if notice(ui, NoticeKind::Info, &text, action).is_some_and(|r| r.clicked()) {
                acts.push(Act::ShaderLoader);
            }
            ui.add_space(6.0);
        }
    }

    /// One mod as a row: its picture, its name with the version and where it came from, a line about it, and on the
    /// right its update, its actions (shown while the row is pointed at) and its switch.
    fn mod_row(&self, ui: &mut Ui, e: &ModEntry, busy: bool, last: bool, acts: &mut Vec<Act>) {
        let key = e.key();
        ui.push_id(&key.file, |ui| self.mod_row_inner(ui, e, &key, busy, last, acts));
    }

    fn mod_row_inner(&self, ui: &mut Ui, e: &ModEntry, key: &ModKey, busy: bool, last: bool, acts: &mut Vec<Act>) {
        let rec = &e.record;
        let offer = self.mods.updates.get(key);
        let details = self.mods.details.get(key);
        let expanded = self.mods.expanded.as_ref() == Some(key);
        let name = e.name();
        let off = rec.disabled;

        // the fill under the row is laid down first and sized at the end, when it is known how far the details reach
        let fill = ui.painter().add(egui::Shape::Noop);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), egui::Sense::hover());
        let hovered = ui.rect_contains_pointer(rect);
        // the row's fill and its quiet actions fade in together under the pointer
        let hot = motion::hover(ui, ui.scope_id().with("row"), hovered || expanded);
        let inner = rect.shrink2(egui::vec2(10.0, 0.0));
        let mut left = inner.left();
        if self.mods.select_mode {
            let mut on = self.mods.selected.contains(key);
            let r = egui::Rect::from_center_size(egui::pos2(left + 10.0, rect.center().y), egui::vec2(20.0, 20.0));
            if check(&mut ui.new_child(egui::UiBuilder::new().max_rect(r)), &mut on).changed() {
                acts.push(Act::Select(key.clone(), on));
            }
            left += 32.0;
        }
        // the picture, dimmed while the mod is off
        let icon = egui::Rect::from_min_size(egui::pos2(left, rect.center().y - ICON / 2.0), egui::vec2(ICON, ICON));
        let mut picture = ui.new_child(egui::UiBuilder::new().max_rect(icon));
        if off {
            picture.set_opacity(0.4);
        }
        match self.mods.textures.get(key) {
            Some(tex) => {
                picture.add(
                    egui::Image::from_texture(tex)
                        .fit_to_exact_size(egui::vec2(ICON, ICON))
                        .corner_radius(CornerRadius::same(R_CONTROL)),
                );
            }
            None => paint_avatar(&picture, icon, &name),
        }
        left += ICON + 14.0;
        // the switch, at the right edge
        let mut on = !off;
        let switch =
            egui::Rect::from_center_size(egui::pos2(inner.right() - 18.0, rect.center().y), egui::vec2(36.0, 20.0));
        let switched = toggle(&mut ui.new_child(egui::UiBuilder::new().max_rect(switch)), &mut on);
        if switched.on_hover_text(if on { "Switched on" } else { "Switched off" }).changed() {
            acts.push(Act::Toggle(key.clone(), on));
        }
        // what can be done with it, left of the switch
        let update_label = offer.map(|o| format!("Update: {}", short(&o.newer, 18)));
        let text_w = |ui: &Ui, t: &str, size: f32| {
            ui.painter().layout_no_wrap(t.to_string(), super::fonts::medium(size), c_text()).size().x
        };
        let actions_w = 2.0 * 8.0
            + text_w(ui, "Remove", FS_BODY)
            + 20.0
            + text_w(ui, "Details", FS_BODY)
            + 20.0
            + update_label.as_deref().map_or(0.0, |t| text_w(ui, t, FS_META) + UPDATE_PAD + 8.0);
        let actions = egui::Rect::from_min_max(
            egui::pos2((switch.left() - 14.0 - actions_w).max(left + 120.0), rect.top()),
            egui::pos2(switch.left() - 14.0, rect.bottom()),
        );
        let mut ac =
            ui.new_child(egui::UiBuilder::new().max_rect(actions).layout(Layout::right_to_left(Align::Center)));
        ac.spacing_mut().item_spacing.x = 0.0;
        if row_action(&mut ac, "Remove", Some(c_danger()), hot).clicked() {
            acts.push(Act::Remove(vec![key.clone()]));
        }
        if row_action(&mut ac, if expanded { "Hide" } else { "Details" }, None, hot).clicked() {
            acts.push(Act::Details(key.clone()));
        }
        if let (Some(o), Some(label)) = (offer, update_label) {
            ac.add_space(8.0);
            let r = ac.add_enabled_ui(!busy, |ui| update_pill(ui, &label)).inner.on_hover_text(format!(
                "{} \u{2192} {} on {}",
                o.current,
                o.newer,
                o.platform.label()
            ));
            if r.clicked() {
                acts.push(Act::Update(key.clone()));
            }
        }
        // the name and what is known about it
        let text = egui::Rect::from_min_max(
            egui::pos2(left, rect.top() + 9.0),
            egui::pos2(actions.left() - 8.0, rect.bottom() - 9.0),
        );
        let mut tc = ui.new_child(egui::UiBuilder::new().max_rect(text).layout(Layout::top_down(Align::Min)));
        tc.set_clip_rect(tc.clip_rect().intersect(text));
        tc.spacing_mut().item_spacing = egui::vec2(8.0, 3.0);
        tc.spacing_mut().interact_size.y = 20.0;
        let version =
            if rec.version.is_empty() || rec.version == "Unknown" { String::new() } else { short(&rec.version, 40) };
        // a tag is for what needs a look; where the mod came from is plain grey text at the end of the version
        let mut tags: Vec<(&str, egui::Color32)> = Vec::new();
        if !e.exists {
            tags.push(("File missing", c_danger()));
        }
        let tags_w: f32 = tags.iter().map(|(t, _)| text_w(&tc, t, FS_META) + 12.0 + 8.0).sum();
        let name_text = RichText::new(&name).semibold().color(if off { c_muted() } else { c_text() });
        let name_nat = tc.painter().layout_no_wrap(name.clone(), super::fonts::semibold(FS_BODY), c_text()).size().x;
        let name_w = name_nat.min((text.width() - tags_w - 8.0).max(text.width() * 0.4));
        // the version and the rest take what is left on the line: a part that does not fit is left out whole, and
        // all of it is left to the details when there is no room
        let left_over = text.width() - name_w - tags_w - 16.0;
        let mut meta: Vec<&str> = Vec::new();
        if !version.is_empty() {
            meta.push(&version);
        }
        if files::is_from_modrinth(rec) {
            meta.push("Modrinth");
        }
        if files::is_from_curseforge(rec) {
            meta.push("CurseForge");
        }
        if !rec.user_added {
            meta.push("Part of the modpack");
        }
        let meta_w = |parts: &[&str]| {
            let font = egui::FontId::proportional(FS_META);
            tc.painter().layout_no_wrap(parts.join(META_SEP), font, c_muted()).size().x
        };
        while meta.len() > 1 && meta_w(&meta) > left_over {
            meta.pop();
        }
        let show_meta = !meta.is_empty() && meta_w(&meta) <= left_over;
        tc.horizontal(|ui| {
            ui.scope(|ui| {
                ui.set_max_width(name_w);
                ui.add(egui::Label::new(name_text).truncate());
            });
            if show_meta {
                ui.scope(|ui| {
                    ui.set_max_width(left_over);
                    ui.add(
                        egui::Label::new(RichText::new(meta.join(META_SEP)).color(c_faint()).size(FS_META)).truncate(),
                    );
                });
            }
            for (t, c) in &tags {
                tag(ui, t, *c);
            }
        });
        if let Some(p) = details.and_then(|d| d.problem.as_ref()) {
            tc.add(
                egui::Label::new(RichText::new(format!("May not work here: {p}")).color(c_warn()).size(FS_META))
                    .truncate(),
            );
        } else if !rec.description.is_empty() {
            let color = if off { c_faint() } else { c_muted() };
            tc.add(egui::Label::new(RichText::new(short(&rec.description, 160)).color(color).size(FS_META)).truncate());
        }
        if expanded {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_space(left - rect.left());
                ui.vertical(|ui| {
                    ui.set_max_width(inner.right() - left);
                    ui.spacing_mut().interact_size.y = 20.0;
                    ui.spacing_mut().item_spacing.y = 4.0;
                    self.mod_details(ui, e, details, offer, busy, acts)
                });
            });
            ui.add_space(14.0);
        }
        let bottom = if expanded { ui.cursor().top() } else { rect.bottom() };
        let block = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), bottom));
        if hot > 0.0 {
            let shape = egui::epaint::RectShape::new(
                block,
                CornerRadius::same(R_CONTROL),
                c_hover().gamma_multiply(hot),
                Stroke::new(1.0, c_border().gamma_multiply(if expanded { hot } else { 0.0 })),
                egui::StrokeKind::Inside,
            );
            ui.painter().set(fill, shape);
        }
        // a hairline between rows, from where the text starts; it gives way to the fill of a row that is pointed at
        if !last && hot < 1.0 {
            let y = bottom - 0.5;
            ui.painter().hline(left..=inner.right(), y, Stroke::new(1.0, c_rule().gamma_multiply(1.0 - hot)));
        }
    }

    /// The details of one mod: what the file says about itself, links, and the actions that need a closer look.
    fn mod_details(
        &self,
        ui: &mut Ui,
        e: &ModEntry,
        details: Option<&Details>,
        offer: Option<&UpdateOffer>,
        busy: bool,
        acts: &mut Vec<Act>,
    ) {
        let rec = &e.record;
        let key = e.key();
        let info = details.map(|d| &d.info);
        // what is known, a grey name before each value; a long value wraps under itself, never past the card
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let row = |ui: &mut Ui, k: &str, v: String| {
                if !v.is_empty() {
                    ui.horizontal_top(|ui| {
                        let name = RichText::new(k).size(FS_META).color(c_faint());
                        ui.allocate_ui_with_layout(
                            egui::vec2(FACT_W, 18.0),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.set_min_width(FACT_W);
                                ui.add(egui::Label::new(name).truncate());
                            },
                        );
                        ui.add(egui::Label::new(v).wrap());
                    });
                }
            };
            row(ui, "File", format!("{}  ({})", rec.file, human_size(e.size)));
            if let Some(i) = info {
                row(ui, "Version", i.version.clone().unwrap_or_default());
                row(ui, "Authors", i.authors.join(", "));
                row(ui, "License", i.license.clone().unwrap_or_default());
                row(
                    ui,
                    "Loader",
                    i.loaders.iter().map(|l| versions::display_name(Some(l))).collect::<Vec<_>>().join(", "),
                );
                row(ui, "Minecraft", i.minecraft_versions.join(" or "));
                row(
                    ui,
                    "Runs on",
                    i.environment
                        .clone()
                        .map(|e| match e.as_str() {
                            "*" => "Client and server".to_string(),
                            "client" => "Client only".to_string(),
                            "server" => "Server only".to_string(),
                            other => other.to_string(),
                        })
                        .unwrap_or_default(),
                );
                row(ui, "Needs", i.depends.join(", "));
            }
            if let Some(o) = offer {
                row(ui, "Update", format!("{} \u{2192} {} ({})", o.current, o.newer, o.platform.label()));
            }
        });
        let text = info
            .and_then(|i| i.description.clone())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| rec.description.clone());
        if !text.is_empty() {
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(text).color(c_muted())).wrap());
        }
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if let Some(p) = &rec.modrinth_project {
                let s = |k: &str| p.get(k).and_then(|v| v.as_str()).filter(|v| !v.is_empty()).map(str::to_string);
                if let Some(slug) = s("slug") {
                    let t = s("project_type").unwrap_or_else(|| "mod".into());
                    if secondary(ui, "Open on Modrinth").clicked() {
                        acts.push(Act::Url(format!("https://modrinth.com/{t}/{slug}")));
                    }
                }
                for (k, label) in [
                    ("source_url", "Source"),
                    ("issues_url", "Issues"),
                    ("wiki_url", "Wiki"),
                    ("discord_url", "Discord"),
                ] {
                    if let Some(u) = s(k) {
                        if secondary(ui, label).clicked() {
                            acts.push(Act::Url(u));
                        }
                    }
                }
                for d in p.get("donation_urls").and_then(|d| d.as_array()).into_iter().flatten() {
                    let (platform, url) =
                        (d.get("platform").and_then(|v| v.as_str()), d.get("url").and_then(|v| v.as_str()));
                    if let (Some(platform), Some(url)) = (platform, url) {
                        if secondary(ui, &format!("Donate ({platform})")).clicked() {
                            acts.push(Act::Url(url.to_string()));
                        }
                    }
                }
            } else if let Some(h) = info.and_then(|i| i.homepage.clone()) {
                if secondary(ui, "Website").clicked() {
                    acts.push(Act::Url(h));
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            if secondary(ui, "Show in folder").clicked() {
                acts.push(Act::ShowInFolder(key.clone()));
            }
            if files::is_from_modrinth(rec) {
                if ui.add_enabled(!busy, |ui: &mut Ui| secondary(ui, "Check this one")).clicked() {
                    acts.push(Act::CheckOne(key.clone()));
                }
                if ui
                    .add_enabled(!busy, |ui: &mut Ui| secondary(ui, "Versions\u{2026}"))
                    .on_hover_text("Pick another version, or go back to an older one")
                    .clicked()
                {
                    acts.push(Act::Versions(key.clone()));
                }
                if ui.add_enabled(!busy, |ui: &mut Ui| secondary(ui, "Reinstall")).clicked() {
                    acts.push(Act::Reinstall(key.clone()));
                }
            }
            if ui.add_enabled(!busy, |ui: &mut Ui| secondary(ui, "Refresh from Modrinth")).clicked() {
                acts.push(Act::Refresh(vec![key.clone()]));
            }
        });
    }

    /// The confirmations and the version list.
    fn mods_dialogs(&mut self, ctx: &egui::Context, inst: &Instance) {
        // remove
        if !self.mods.confirm_remove.is_empty() {
            let keys = self.mods.confirm_remove.clone();
            let label = if keys.len() == 1 { format!("\"{}\"", keys[0].file) } else { format!("{} items", keys.len()) };
            let names =
                remove_list(&keys, |k| self.mods.entries.values().flatten().find(|e| e.key() == *k).map(|e| e.name()));
            let where_to = if ModSettings::from_settings(&self.settings).use_recycle_bin {
                "They go to the Recycle Bin (Trash). You can add them again at any time."
            } else {
                "They are deleted from this instance. You can add them again at any time."
            };
            let (mut yes, mut no) = (false, false);
            let r = egui::Modal::new(egui::Id::unique("mods-confirm-remove")).frame(super::modal_frame(ctx)).show(
                ctx,
                |ui| {
                    ui.set_width(380.0);
                    ui.label(RichText::new(format!("Remove {label}?")).size(FS_SECTION).semibold());
                    ui.add_space(4.0);
                    if keys.len() > 1 {
                        small_muted(ui, &names);
                        ui.add_space(4.0);
                    }
                    muted(ui, where_to);
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        yes = danger(ui, "Remove").clicked();
                        no = secondary(ui, "Cancel").clicked();
                    });
                },
            );
            no |= r.should_close() && !yes;
            if yes || no {
                self.mods.confirm_remove.clear();
            }
            if yes {
                let s = ModSettings::from_settings(&self.settings);
                self.mods_job(&inst.folder, "Removing", move |mut inst, _| async move {
                    tokio::task::spawn_blocking(move || {
                        let mut failed = Vec::new();
                        for k in &keys {
                            if let Err(e) = files::remove(&mut inst, k, &s) {
                                failed.push(format!("{}: {e:#}", k.file));
                            }
                        }
                        if failed.is_empty() {
                            Ok(format!("Removed {label}"))
                        } else {
                            Err(anyhow::anyhow!(failed.join("; ")))
                        }
                    })
                    .await?
                });
                self.mods.selected.clear();
            }
        }
        // restore a snapshot
        if let Some(file) = self.mods.confirm_restore.clone() {
            let (mut yes, mut no) = (false, false);
            let r = egui::Modal::new(egui::Id::unique("mods-confirm-restore")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                ui.set_width(420.0);
                ui.label(RichText::new("Restore this snapshot?").size(FS_SECTION).semibold());
                ui.add_space(4.0);
                muted(ui, format!("Puts the mods and configs of \"{}\" back to how they were. Your worlds are not changed, and the current state is saved first so you can undo this.", inst.name()));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    yes = primary(ui, "Restore").clicked();
                    no = secondary(ui, "Cancel").clicked();
                });
            });
            no |= r.should_close() && !yes;
            if yes || no {
                self.mods.confirm_restore = None;
            }
            if yes {
                let dirs = self.dirs.clone();
                self.mods_job(&inst.folder, "Restoring", move |inst, _| async move {
                    tokio::task::spawn_blocking(move || {
                        let dir = snapshots::folder_for(&dirs, &inst);
                        let snapshot = snapshots::list(&dir)
                            .into_iter()
                            .find(|s| s.file == file)
                            .ok_or_else(|| anyhow::anyhow!("that snapshot is gone"))?;
                        snapshots::restore(&snapshot, &inst.root, &dir).map(|_| "Snapshot restored".to_string())
                    })
                    .await?
                });
            }
        }
        // performance pack
        if self.mods.confirm_pack {
            let target =
                self.mods.target.clone().unwrap_or_else(|| {
                    crate::mods::InstanceTarget::of(inst, &crate::mods::RemoteConfig::load(&self.dirs))
                });
            let (mut yes, mut no) = (false, false);
            let r = egui::Modal::new(egui::Id::unique("mods-confirm-pack")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                ui.set_width(420.0);
                ui.label(RichText::new("Make Minecraft run faster").size(FS_SECTION).semibold());
                ui.add_space(4.0);
                muted(
                    ui,
                    format!(
                        "Adds {} from Modrinth. They make the game smoother and load faster, especially on older computers, and don't change how it plays. Anything not made for Minecraft {} yet is skipped.",
                        performance::describe(target.loader_key),
                        inst.mc_version()
                    ),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    yes = primary(ui, "Add them").clicked();
                    no = secondary(ui, "Cancel").clicked();
                });
            });
            no |= r.should_close() && !yes;
            if yes || no {
                self.mods.confirm_pack = false;
            }
            if yes {
                self.mods_task(
                    &inst.folder,
                    "Adding performance mods".into(),
                    move |mut inst, ctx, progress| async move {
                        let r = install::install_performance_pack(&mut inst, &ctx, &progress).await;
                        Ok(if r.installed.is_empty() {
                            if r.skipped.is_empty() {
                                "Performance mods are already installed".to_string()
                            } else {
                                format!(
                                    "No performance mods are available for Minecraft {} yet",
                                    ctx.target.minecraft_version
                                )
                            }
                        } else {
                            format!("Added {}", r.installed.join(", "))
                        })
                    },
                );
            }
        }
        // versions of one mod
        let mut install_version: Option<(ModKey, String)> = None;
        let mut close = false;
        if let Some(d) = &self.mods.versions {
            let r = egui::Modal::new(egui::Id::unique("mods-versions")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                ui.set_width(560.0);
                ui.label(RichText::new(format!("Versions of {}", d.name)).size(FS_SECTION).semibold());
                small_muted(ui, "Pick one to install it in place of the current one. Older versions are fine if a newer one gives you trouble.");
                ui.add_space(8.0);
                match &d.list {
                    None => {
                        ui.horizontal(|ui| {
                            spinner_sized(ui, 14.0);
                            muted(ui, "Asking Modrinth");
                        });
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).color(c_danger()));
                    }
                    Some(Ok(list)) if list.is_empty() => {
                        muted(ui, "No versions are made for this instance.");
                    }
                    Some(Ok(list)) => {
                        ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            for v in list {
                                let current = d.installed_id.as_deref() == Some(v.id.as_str());
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.set_max_width(380.0);
                                        ui.add(egui::Label::new(RichText::new(v.label()).semibold()).truncate());
                                        small_muted(ui, format!("{}  ·  {}", v.date_published.get(..10).unwrap_or(&v.date_published), v.game_versions.iter().take(4).cloned().collect::<Vec<_>>().join(", ")));
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if current {
                                            small_muted(ui, "\u{2713} Installed");
                                        } else if secondary(ui, "Install").clicked() {
                                            install_version = Some((d.key.clone(), v.id.clone()));
                                        }
                                    });
                                });
                                ui.separator();
                            }
                        });
                    }
                }
                ui.add_space(8.0);
                if secondary(ui, "Close").clicked() {
                    close = true;
                }
            });
            close |= r.should_close() && install_version.is_none();
        }
        if close || install_version.is_some() {
            self.mods.versions = None;
        }
        if let Some((key, version_id)) = install_version {
            self.mods_task(
                &inst.folder,
                format!("Installing {}", key.file),
                move |mut inst, ctx, progress| async move {
                    updates::install_specific(&mut inst, &ctx, &key, &version_id, &progress).await?;
                    Ok(format!("Installed {version_id} over {}", key.file))
                },
            );
        }
    }
}

/// The names a "Remove N items?" window lists: the first eight, then how many more, so nothing goes unseen (a
/// selection outlives a filter that hides some of it).
fn remove_list(keys: &[ModKey], name_of: impl Fn(&ModKey) -> Option<String>) -> String {
    const SHOWN: usize = 8;
    let mut names: Vec<String> =
        keys.iter().take(SHOWN).map(|k| name_of(k).unwrap_or_else(|| k.file.clone())).collect();
    if keys.len() > SHOWN {
        names.push(format!("and {} more", keys.len() - SHOWN));
    }
    names.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn the_remove_window_names_what_goes() {
        let keys: Vec<ModKey> = (1..=10).map(|i| ModKey { file: format!("mod{i}.jar"), kind: "mods".into() }).collect();
        let named = |k: &ModKey| (k.file == "mod1.jar").then(|| "Sodium".to_string());
        assert_eq!(
            remove_list(&keys, named),
            "Sodium, mod2.jar, mod3.jar, mod4.jar, mod5.jar, mod6.jar, mod7.jar, mod8.jar, and 2 more"
        );
        assert_eq!(remove_list(&keys[..2], |_| None), "mod1.jar, mod2.jar");
    }

    #[test]
    fn details_and_a_small_picture_are_read_from_a_jar() {
        let dir = std::env::temp_dir().join(format!("blockforge-gui-mods-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = {
            let mut out = Vec::new();
            let img = image::RgbaImage::from_pixel(200, 200, image::Rgba([10, 200, 90, 255]));
            image::DynamicImage::ImageRgba8(img)
                .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .unwrap();
            out
        };
        let jar = dir.join("sodium.jar");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&jar).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("fabric.mod.json", o).unwrap();
        z.write_all(br#"{"name":"Sodium","version":"1","icon":"icon.png","depends":{"minecraft":"1.20.1"}}"#).unwrap();
        z.start_file("icon.png", o).unwrap();
        z.write_all(&png).unwrap();
        z.finish().unwrap();

        let key = ModKey { file: "sodium.jar".into(), kind: "mods".into() };
        let out = read_details(vec![(key.clone(), jar.clone())], Some("fabric"), "1.21");
        let (k, details, image) = &out[0];
        assert_eq!(k, &key);
        assert_eq!(details.info.name.as_deref(), Some("Sodium"));
        assert!(details.problem.as_ref().unwrap().contains("Minecraft"));
        let img = image.as_ref().unwrap();
        assert!(img.size[0] <= 72 && img.size[1] <= 72);
        // fine for the version it asks for
        assert!(read_details(vec![(key, jar)], Some("fabric"), "1.20.1")[0].1.problem.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_list_only_lays_out_what_is_in_view() {
        let ctx = egui::Context::default();
        let items: Vec<u32> = (0..300).collect();
        let mut drawn: Vec<u32> = Vec::new();
        for _ in 0..3 {
            drawn.clear();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                lazy_list(ui, egui::Id::unique("test-list"), &items, 40.0, |ui, i| {
                    drawn.push(*i);
                    ui.allocate_space(egui::vec2(100.0, 40.0));
                });
            });
            // nothing paints here, so the font pictures are thrown away
            out.textures_delta.clear();
        }
        // a screenful and a margin around it, not all 300
        assert_eq!(drawn.first(), Some(&0));
        assert!(drawn.len() > 10 && drawn.len() < 30, "{} rows laid out", drawn.len());
    }

    #[test]
    fn changes_to_one_instance_take_turns() {
        let a = instance_lock(Path::new("/instances/a"));
        assert!(Arc::ptr_eq(&a, &instance_lock(Path::new("/instances/a"))));
        assert!(!Arc::ptr_eq(&a, &instance_lock(Path::new("/instances/b"))));
        let held = a.try_lock().unwrap();
        assert!(instance_lock(Path::new("/instances/a")).try_lock().is_err());
        assert!(instance_lock(Path::new("/instances/b")).try_lock().is_ok());
        drop(held);
        assert!(a.try_lock().is_ok());
    }

    #[test]
    fn long_text_is_cut() {
        assert_eq!(short("abcdef", 4), "abcd…");
        assert_eq!(short("abc", 4), "abc");
    }

    #[test]
    fn the_list_is_ordered_as_asked() {
        let at = |s: u64| Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(s));
        // (name, when the file was written, has an update), in the name order the list comes in
        let rows = [("a", at(10), false), ("b", None, true), ("c", at(30), false), ("d", at(20), true)];
        let names = |order: Order| {
            let mut v = rows.to_vec();
            order_rows(&mut v, order, |r| r.1, |r| r.2);
            v.iter().map(|r| r.0).collect::<Vec<_>>()
        };
        assert_eq!(names(Order::Name), ["a", "b", "c", "d"]);
        // the newest first, a file whose time isn't known last
        assert_eq!(names(Order::Added), ["c", "d", "a", "b"]);
        // updates first, each part still by name
        assert_eq!(names(Order::Updates), ["b", "d", "a", "c"]);
    }

    #[test]
    fn items_are_told_apart_by_where_they_came_from() {
        let by_hand = crate::modrecord::ModRecord::new("Hand", "hand.jar", "mods");
        let mut mr = by_hand.clone();
        mr.modrinth_project = Some(serde_json::json!({"id": "AANobbMI"}));
        mr.modrinth_version = Some(serde_json::json!({"id": "v1"}));
        let mut cf = by_hand.clone();
        cf.curse_forge_project_id = Some(394468);
        cf.curse_forge_file_id = Some(1);
        // a record with only half of what a site keeps (no version) counts as added by hand
        let mut half = by_hand.clone();
        half.modrinth_project = Some(serde_json::json!({"id": "x"}));
        for (rec, from) in
            [(&by_hand, Origin::ByHand), (&mr, Origin::Modrinth), (&cf, Origin::CurseForge), (&half, Origin::ByHand)]
        {
            for o in Origin::ALL {
                assert_eq!(o.has(rec), o == from || o == Origin::Anywhere, "{} in {o:?}", rec.name);
            }
        }
    }
}
