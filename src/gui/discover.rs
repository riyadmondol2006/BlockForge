//! Discover: search Modrinth (or, with "All", Modrinth and CurseForge together, and Smithed for data packs) and
//! install into an instance.

use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime};

use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, CornerRadius, FontId, Layout, Response, RichText, ScrollArea, Sense, Stroke, Ui, vec2};

use super::theme::*;
use super::widgets::*;
use super::{App, Msg, Page};
use crate::modrinth::{self, Hit, Kind, Sort};
use crate::mods::metadata::ContentKind;

pub struct DiscoverState {
    pub kind: Kind,
    pub query: String,
    pub sort: Sort,
    /// The instance mods are installed into (folder name).
    pub target: Option<String>,
    pub hits: Vec<Hit>,
    pub total: u64,
    pub loading: bool,
    pub seq: u64,
    pub error: Option<String>,
    pub installing: HashSet<String>,
    offset_pending: u32,
    /// A search has been started since the page first opened.
    pub(super) searched: bool,
    /// When the search field was last typed in and the search it wants has not started yet (see [`live_search`]).
    edited: Option<Instant>,
    /// The Modrinth projects in the target instance, with the instance file's time they were read at.
    installed: Option<(String, Option<SystemTime>, HashSet<String>)>,
    /// Counts the result lists that replaced the one before, so a new search starts at the top of the list.
    pub list_gen: u64,
    /// "Both": CurseForge is searched too, and the two sites' results are shown as one list.
    pub both: bool,
    /// CurseForge's part of a search on every site.
    pub cf: super::discover_both::CfSide,
    /// Smithed's part of a search for data packs on every site.
    pub sm: super::discover_both::SmSide,
    /// Every site's results as one list (see [`super::discover_both::merge`]).
    pub merged: Vec<super::discover_both::Entry>,
}

impl Default for DiscoverState {
    fn default() -> Self {
        DiscoverState {
            kind: Kind::Mod,
            query: String::new(),
            sort: Sort::Downloads,
            target: None,
            hits: Vec::new(),
            total: 0,
            loading: false,
            seq: 0,
            error: None,
            installing: HashSet::new(),
            offset_pending: 0,
            searched: false,
            edited: None,
            installed: None,
            list_gen: 0,
            both: false,
            cf: Default::default(),
            sm: Default::default(),
            merged: Vec::new(),
        }
    }
}

/// The Modrinth kind as the mod manager files it.
fn content_kind(kind: Kind) -> Option<ContentKind> {
    match kind {
        Kind::Mod => Some(ContentKind::Mods),
        Kind::ResourcePack => Some(ContentKind::ResourcePacks),
        Kind::Shader => Some(ContentKind::Shaders),
        Kind::Modpack | Kind::DataPack => None,
    }
}

/// The worlds of an instance that already have a data pack, known by its Modrinth project or its file name.
pub(super) fn worlds_with(
    inst: &crate::instances::Instance,
    modrinth_id: Option<&str>,
    file: Option<&str>,
) -> HashSet<String> {
    inst.mod_records()
        .iter()
        .filter(|r| r.kind == "datapack")
        .filter(|r| {
            modrinth_id.is_some_and(|id| r.modrinth_project_id().is_some_and(|p| p.eq_ignore_ascii_case(id)))
                || file.is_some_and(|f| r.file == f)
        })
        .filter_map(|r| r.path.as_deref()?.strip_prefix("saves/")?.strip_suffix("/datapacks").map(str::to_string))
        .collect()
}

/// The worlds of an instance as menu entries, every world at once first. A world that has the pack already is
/// ticked. Gives the worlds chosen.
fn world_entries(ui: &mut Ui, inst: &crate::instances::Instance, have: &HashSet<String>) -> Option<Vec<String>> {
    let worlds = crate::content::service::worlds(&inst.root);
    if worlds.is_empty() {
        ui.label(RichText::new("No worlds yet. Play once to make one.").color(c_muted()));
        return None;
    }
    let mut chosen = None;
    if worlds.len() > 1 && ui.button(format!("All {} worlds", worlds.len())).clicked() {
        chosen = Some(worlds.clone());
        ui.close();
    }
    for w in &worlds {
        let r = if have.contains(w) {
            ui.button(format!("{w}  \u{2713}"))
                .on_hover_text("Has it already. Adding it again puts in the newest version.")
        } else {
            ui.button(w)
        };
        if r.clicked() {
            chosen = Some(vec![w.clone()]);
            ui.close();
        }
    }
    chosen
}

/// The Install button of a data pack, which goes into worlds: a menu of the chosen instance's worlds, or of the
/// instances and then their worlds. `have` says which worlds of an instance have it already. Gives the instance
/// folder and the worlds.
pub(super) fn data_pack_button(
    ui: &mut Ui,
    instances: &[crate::instances::Instance],
    target: Option<&str>,
    have: &dyn Fn(&crate::instances::Instance) -> HashSet<String>,
) -> Option<(String, Vec<String>)> {
    let r = secondary(ui, "Install");
    let mut chosen = None;
    egui::Popup::menu(&r).show(|ui| {
        ui.set_min_width(180.0);
        match target.and_then(|t| instances.iter().find(|i| i.folder == t)) {
            Some(inst) => {
                small_muted(ui, "Add to world");
                chosen = world_entries(ui, inst, &have(inst)).map(|w| (inst.folder.clone(), w));
            }
            None => {
                if instances.is_empty() {
                    ui.label("Create an instance first");
                }
                for inst in instances {
                    ui.menu_button(instance_choice(inst), |ui| {
                        if let Some(w) = world_entries(ui, inst, &have(inst)) {
                            chosen = Some((inst.folder.clone(), w));
                        }
                    });
                }
            }
        }
    });
    chosen
}

/// "New World", "New World and Survival", "3 worlds".
pub(super) fn worlds_text(worlds: &[String]) -> String {
    match worlds {
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        _ => format!("{} worlds", worlds.len()),
    }
}

impl DiscoverState {
    /// The Modrinth projects in the target instance, once read.
    pub(super) fn installed_ids(&self) -> Option<&HashSet<String>> {
        self.installed.as_ref().map(|(_, _, ids)| ids)
    }

    pub fn arrived(&mut self, seq: u64, result: Result<(Vec<Hit>, u64), String>) {
        if seq != self.seq {
            return;
        }
        self.loading = false;
        match result {
            Ok((hits, total)) => {
                if self.offset_pending == 0 {
                    self.hits = hits;
                    self.list_gen += 1;
                } else {
                    self.hits.extend(hits);
                }
                self.total = total;
                self.error = None;
            }
            Err(e) => {
                self.error = Some(e);
                if self.offset_pending == 0 {
                    // a new search failed: the old rows no longer match what was asked for
                    self.hits.clear();
                    self.total = 0;
                }
            }
        }
        self.remerge();
    }
}

/// An instance as the "For" list and the Install menu name it: its name, Minecraft version and mod loader, so the one
/// a mod fits can be told apart from the rest.
pub(super) fn instance_choice(inst: &crate::instances::Instance) -> String {
    let loader = inst.loader().map(|(kind, _)| kind.to_string()).unwrap_or_else(|| "no mod loader".into());
    format!("{} \u{00B7} {} \u{00B7} {loader}", inst.name(), inst.mc_version())
}

const PAGE: u32 = 20;

/// The width of the search field on these pages.
pub(super) const SEARCH_W: f32 = 280.0;

/// How long the search field waits after the last key before it searches.
const LIVE_DELAY: Duration = Duration::from_millis(300);

/// The width of the column at the right end of a result that holds its button.
const ACTION_W: f32 = 120.0;

/// The height a result takes (the picture and the room around it), for the grey rows shown while a list loads.
pub(super) const ROW_H: f32 = 73.0;

/// A number with thousands separators: 1,284.
pub(super) fn thousands(n: u64) -> String {
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

/// A list that couldn't be fetched, said in one calm line with a button to try again (what exactly went wrong is
/// in the tooltip). Returns whether "Try again" was pressed.
pub(super) fn fetch_error(ui: &mut Ui, text: &str, detail: &str) -> bool {
    let out = ui.scope(|ui| notice(ui, NoticeKind::Error, text, Some("Try again")));
    out.response.on_hover_text(detail);
    ui.add_space(8.0);
    out.inner.is_some_and(|r| r.clicked())
}

/// Link-looking text in ink (not green): underlined, with the hand cursor.
pub(super) fn text_link(ui: &mut Ui, text: &str, size: f32) -> Response {
    ui.add(egui::Label::new(RichText::new(text).size(size).color(c_text()).underline()).sense(Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The grey label in front of a control.
pub(super) fn filter_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(c_muted()));
}

/// A search field `width` wide with a magnifier in it.
pub(super) fn search_input(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    let r = ui.add_sized(
        [width, 32.0],
        egui::TextEdit::singleline(text)
            .hint_text(RichText::new(hint).color(c_faint()))
            .margin(egui::Margin { left: 34, right: 10, top: 7, bottom: 7 })
            .background_color(c_field()),
    );
    super::icons::paint(
        ui.painter(),
        super::icons::Icon::Search,
        egui::pos2(r.rect.left() + 18.0, r.rect.center().y),
        15.0,
        c_faint(),
    );
    r
}

/// What the search field wants on a frame.
#[derive(Debug, PartialEq)]
enum LiveStep {
    /// Nothing is waiting.
    Idle,
    /// Typing paused less than the delay ago: look again after this long.
    Wait(Duration),
    /// Search now.
    Go,
}

/// The timing of [`live_search`]: a change starts the wait, Enter ends it at once, and the wait ends by itself after
/// [`LIVE_DELAY`] without another change.
fn live_step(edited: &mut Option<Instant>, changed: bool, enter: bool, now: Instant) -> LiveStep {
    if changed {
        *edited = Some(now);
    }
    if enter {
        *edited = None;
        return LiveStep::Go;
    }
    let Some(at) = *edited else { return LiveStep::Idle };
    match LIVE_DELAY.checked_sub(now.saturating_duration_since(at)) {
        Some(left) if !left.is_zero() => LiveStep::Wait(left),
        _ => {
            *edited = None;
            LiveStep::Go
        }
    }
}

/// The search field of a page. There is no button: the search starts when typing has paused for a moment, or at once
/// on Enter. `edited` remembers when the text last changed. Returns true when a search should start now.
pub(super) fn live_search(ui: &mut Ui, text: &mut String, hint: &str, edited: &mut Option<Instant>) -> bool {
    let r = search_input(ui, text, hint, SEARCH_W);
    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    match live_step(edited, r.changed(), enter, Instant::now()) {
        LiveStep::Idle => false,
        LiveStep::Wait(left) => {
            ui.ctx().request_repaint_after(left);
            false
        }
        LiveStep::Go => true,
    }
}

/// The colour of a word on a result: loaders have theirs, everything else is quiet.
pub(super) fn tag_color(name: &str) -> Color32 {
    match name.to_ascii_lowercase().as_str() {
        "fabric" => loader_color("Fabric"),
        "quilt" => loader_color("Quilt"),
        "forge" => loader_color("Forge"),
        "neoforge" => loader_color("NeoForge"),
        _ => c_faint(),
    }
}

fn is_loader(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "fabric" | "quilt" | "forge" | "neoforge")
}

/// The words for the last line of a result: every loader in its own colour first, then at most `max` categories in
/// plain grey.
pub(super) fn tag_parts<'a>(names: impl Iterator<Item = &'a str>, max: usize) -> Vec<(String, Color32)> {
    let mut v: Vec<&str> = names.collect();
    v.sort_by_key(|n| !is_loader(n));
    let loaders = v.iter().filter(|n| is_loader(n)).count();
    v.truncate(loaders + max);
    v.into_iter().map(|n| (loader_display(n), tag_color(n))).collect()
}

/// The tags of a data pack: no loaders (a project that is a mod as well lists them, but the data pack needs none),
/// and not "datapack" itself.
pub(super) fn data_pack_tags<'a>(names: impl Iterator<Item = &'a str>, max: usize) -> Vec<(String, Color32)> {
    tag_parts(names.filter(|n| !is_loader(n) && !n.eq_ignore_ascii_case("datapack")), max)
}

/// A loader's name as it is written everywhere else ("neoforge" is "NeoForge"); other words stay as they are.
fn loader_display(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "fabric" => "Fabric".into(),
        "quilt" => "Quilt".into(),
        "forge" => "Forge".into(),
        "neoforge" => "NeoForge".into(),
        _ => name.to_string(),
    }
}

fn run(color: Color32, size: f32) -> TextFormat {
    TextFormat { font_id: FontId::proportional(size), color, ..Default::default() }
}

/// The first line of a result: the name, and who made it.
pub(super) fn name_line(ui: &mut Ui, name: &str, by: Option<&str>) {
    let note = by.filter(|b| !b.is_empty()).map(|b| (format!("by {b}"), c_muted()));
    name_line_with(ui, name, note);
}

/// The first line of a result with a short note after the name, in the colour given.
pub(super) fn name_line_with(ui: &mut Ui, name: &str, note: Option<(String, Color32)>) {
    let mut job = LayoutJob::default();
    job.append(
        name,
        0.0,
        TextFormat { font_id: super::fonts::semibold(FS_BODY), color: c_text(), ..Default::default() },
    );
    if let Some((text, color)) = note {
        job.append(&text, 8.0, run(color, FS_META));
    }
    ui.add(egui::Label::new(job).truncate());
}

/// The second line of a result: what it is, in one line.
pub(super) fn summary_line(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(FS_BODY).color(c_muted())).truncate());
}

/// The last line of a result, plain text divided by dots: figures, loader names in their own colours, categories.
pub(super) fn meta_line(ui: &mut Ui, parts: &[(String, Color32)]) {
    if parts.is_empty() {
        return;
    }
    let mut job = LayoutJob::default();
    for (i, (text, color)) in parts.iter().enumerate() {
        if i > 0 {
            job.append(" \u{00B7} ", 0.0, run(c_faint(), FS_META));
        }
        job.append(text, 0.0, run(*color, FS_META));
    }
    ui.add(egui::Label::new(job).truncate());
}

/// One result in a list: no box at rest, a tone step under it when pointed at, a hairline between rows.
pub(super) fn row_frame<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.spacing_mut().item_spacing.y = 0.0;
    let bg = ui.painter().add(egui::Shape::Noop);
    let out = egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    });
    let rect = out.response.rect;
    // the row's lower edge belongs to the next row, so two rows never light up at once
    let hot = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.max.y - 1.0));
    if ui.rect_contains_pointer(hot) {
        ui.painter().set(bg, egui::Shape::rect_filled(rect, CornerRadius::same(R_CARD), c_hover()));
    } else {
        ui.painter().hline(rect.x_range().shrink(12.0), rect.bottom(), Stroke::new(1.0, c_rule()));
    }
    out.inner
}

/// How wide the text of a row may be: what is left of the row after the icon and the actions (as wide as they were
/// on the last frame, found by [`row_actions`]).
pub(super) fn row_text_width(ui: &Ui, key: &str) -> f32 {
    let w = remembered_width(ui, egui::Id::unique(("row-actions", key)), 100.0);
    (ui.available_width() - w - 16.0).max(120.0)
}

/// The right-hand end of a row: buttons and figures, laid out from the right edge.
pub(super) fn row_actions(ui: &mut Ui, key: &str, add: impl FnOnce(&mut Ui)) {
    let id = egui::Id::unique(("row-actions", key));
    let used = ui
        .with_layout(Layout::right_to_left(Align::Center), |ui| {
            add(ui);
            ui.min_rect().width()
        })
        .inner;
    remember_width(ui, id, used);
}

/// The column of a row that holds its button, fixed in width with the content on the right edge, so the buttons of
/// a list line up whatever they say. `key` tells the rows apart.
pub(super) fn action_cell(ui: &mut Ui, key: &str, add: impl FnOnce(&mut Ui)) {
    let id = ui.scope_id().with(("action-cell", key));
    ui.allocate_ui_with_layout(vec2(ACTION_W, 32.0), Layout::left_to_right(Align::Center), |ui| {
        end_aligned(ui, id, add);
    });
}

/// A figure with its unit under it, right-aligned in a fixed column so a list of them lines up.
pub(super) fn stat_column(ui: &mut Ui, figure: &str, unit: &str) {
    ui.allocate_ui_with_layout(vec2(72.0, 34.0), Layout::top_down(Align::Max), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(RichText::new(figure).size(FS_BODY).semibold());
        ui.label(RichText::new(unit).size(FS_META).color(c_faint()));
    });
}

/// A check mark drawn with two lines, centred on `c`.
pub(super) fn paint_check(p: &egui::Painter, c: egui::Pos2, size: f32, color: Color32) {
    let s = size / 2.0;
    let at = |x: f32, y: f32| egui::pos2(c.x + x * s, c.y + y * s);
    p.add(egui::Shape::line(
        vec![at(-0.75, 0.05), at(-0.25, 0.6), at(0.8, -0.55)],
        Stroke::new((size / 9.0).max(1.4), color),
    ));
}

/// A short state in grey with a check in front ("Installed"): a state, not an action.
pub(super) fn check_label(ui: &mut Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(text.into(), FontId::proportional(FS_META), c_muted());
    let (rect, resp) = ui.allocate_exact_size(vec2(18.0 + galley.size().x, 20.0), Sense::hover());
    paint_check(ui.painter(), egui::pos2(rect.left() + 7.0, rect.center().y), 13.0, c_muted());
    ui.painter().galley(egui::pos2(rect.left() + 18.0, rect.center().y - galley.size().y / 2.0), galley, c_muted());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
}

/// "Installed" with a check in front.
pub(super) fn installed_mark(ui: &mut Ui) {
    check_label(ui, "Installed");
}

/// The small cross at the end of a dialog's title.
pub(super) fn close_button(ui: &mut Ui) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), c_hover());
    }
    let stroke = Stroke::new(1.5, if resp.hovered() { c_text() } else { c_muted() });
    let (c, d) = (rect.center(), 5.0);
    ui.painter().line_segment([c + vec2(-d, -d), c + vec2(d, d)], stroke);
    ui.painter().line_segment([c + vec2(-d, d), c + vec2(d, -d)], stroke);
    focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Close"));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The title row of a dialog: the title at the left (a section title), a cross at the right. Returns true when the
/// cross was pressed.
pub(super) fn dialog_title(ui: &mut Ui, title: &str) -> bool {
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(RichText::new(title).size(FS_SECTION).semibold()).truncate());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| close_button(ui).clicked()).inner
    })
    .inner
}

/// Lays out `add` from left to right but flush with the right edge of the row it is in, so Tab visits the widgets in
/// the order they are seen. It needs to know how wide `add` is, which it remembers from the last frame under `id`.
/// Returns how wide `add` turned out.
pub(super) fn end_aligned(ui: &mut Ui, id: egui::Id, add: impl FnOnce(&mut Ui)) -> f32 {
    let width = remembered_width(ui, id, 160.0);
    let room = ui.available_rect_before_wrap();
    let left = (room.right() - width).max(room.left());
    // the room runs past the edge on purpose: what is laid out never wraps, however wide it is now
    let rect = egui::Rect::from_min_size(egui::pos2(left, room.top()), vec2(room.width().max(width), room.height()));
    let builder = egui::UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center));
    let used = ui.scope_builder(builder, add).response.rect.width();
    remember_width(ui, id, used);
    used
}

/// The bottom row of a dialog: quiet things at the left, the buttons that finish it at the right with the main one
/// last. When both do not fit side by side (a narrow window) they wrap instead.
pub(super) fn dialog_footer(ui: &mut Ui, key: &str, left: impl FnOnce(&mut Ui), right: impl FnOnce(&mut Ui)) {
    let id = ui.scope_id().with(("dialog-footer", key));
    let need = remembered_width(ui, id.with("need"), 0.0);
    if need > ui.available_width() {
        ui.horizontal_wrapped(|ui| {
            left(ui);
            right(ui);
        });
        return;
    }
    ui.horizontal(|ui| {
        let gap = ui.spacing().item_spacing.x;
        let left_w = ui.scope(left).response.rect.width();
        let right_w = end_aligned(ui, id, right);
        remember_width(ui, id.with("need"), left_w + gap + right_w);
    });
}

/// A line of small faint text: counts and the like.
pub(super) fn small_faint(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).size(FS_META).color(c_faint()));
}

/// The line under the filters of a page: what the list is limited to at the left (drawn by `lead`), the number of
/// results at the right.
pub(super) fn context_line(ui: &mut Ui, count: Option<String>, buttons: bool, lead: impl FnOnce(&mut Ui)) {
    ui.scope(|ui| {
        // a row of text is as tall as its text, a row with buttons as tall as they are
        ui.spacing_mut().interact_size.y = if buttons { 32.0 } else { 20.0 };
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(count) = count {
                    ui.label(RichText::new(count).size(FS_META).color(c_faint()));
                }
                ui.with_layout(Layout::left_to_right(Align::Center), lead);
            });
        });
    });
}

impl App {
    /// Start a search. `reset` begins again from the first result; otherwise the next page is added.
    pub(super) fn discover_search(&mut self, reset: bool) {
        // what the target instance runs: its Minecraft version, and every loader whose mods it can use
        let kind = self.discover.kind;
        let (mc, loaders) = match (self.discover.target.as_ref().and_then(|f| self.instance(f)), kind) {
            (Some(inst), Kind::Mod | Kind::ResourcePack | Kind::Shader | Kind::DataPack) => {
                let loaders = if kind == Kind::Mod {
                    crate::mods::InstanceTarget::of(inst, &crate::mods::RemoteConfig::load(&self.dirs))
                        .accepted_loaders()
                } else {
                    Vec::new()
                };
                (Some(inst.mc_version().to_string()), loaders)
            }
            _ => (None, Vec::new()),
        };
        let d = &mut self.discover;
        d.searched = true;
        if d.both {
            // with every site, "more" asks each only for what it still has
            let mr_more = reset || (d.hits.len() as u64) < d.total;
            self.both_search_cf(reset);
            self.both_search_sm(reset);
            if !mr_more {
                return;
            }
        }
        let d = &mut self.discover;
        // a new search leaves the old rows in place until the new ones arrive; more results go after the ones shown
        d.offset_pending = if reset { 0 } else { d.hits.len() as u32 };
        d.seq += 1;
        d.loading = true;
        let (seq, query, sort, offset) = (d.seq, d.query.clone(), d.sort, d.offset_pending);
        let n = self.notifier();
        self.rt.spawn(async move {
            let r = modrinth::search_for_loaders(&query, kind, mc.as_deref(), &loaders, sort, offset, PAGE)
                .await
                .map_err(|e| format!("{e:#}"));
            n.send(Msg::Search { seq, result: r });
        });
    }

    pub(super) fn install_hit(&mut self, hit: &Hit, folder: Option<String>) {
        let id = hit.project_id.clone();
        let title = hit.title.clone();
        // the row's own kind: while another tab's search loads, the rows of the last one are still on screen
        let kind = if hit.project_type.is_empty() { self.discover.kind } else { Kind::from_api(&hit.project_type) };
        if kind == Kind::DataPack {
            // data packs go into worlds, which the Install menu asks for (see install_data_pack_hit)
            return;
        }
        let Some(content) = content_kind(kind) else {
            self.discover.installing.insert(id.clone());
            let dirs = self.dirs.clone();
            self.start_task(format!("Installing {title}"), move |p, _| async move {
                let inst = modrinth::install_modpack(&dirs, &id, &p).await?;
                Ok(format!("Created {}", inst.name()))
            });
            return;
        };
        let Some(inst) = folder.as_deref().and_then(|f| self.instance(f)) else {
            self.toast_err("That instance isn't there any more. Choose another one.");
            return;
        };
        let (root, inst_folder, inst_name) = (inst.root.clone(), inst.folder.clone(), inst.name().to_string());
        if content == ContentKind::Mods && inst.loader().is_none() {
            // a mod in an instance without a mod loader is never loaded: offer to add one instead
            self.toast_err(format!("{inst_name} has no mod loader, so {title} wouldn't load. Pick one first."));
            self.apply(super::instance_menu::Act::ChangeLoader(inst_folder));
            return;
        }
        self.discover.installing.insert(id.clone());
        // the mod manager's install: it records the file in the instance, adds what it needs, and says when what
        // it needs clashes with something already installed
        let (dirs, settings, lock) = (self.dirs.clone(), self.settings.clone(), super::mods::instance_lock(&root));
        self.start_task(format!("Installing {title}"), move |p, n| async move {
            let _turn = lock.lock().await;
            let mut inst = crate::instances::load_one(&root)
                .ok_or_else(|| anyhow::anyhow!("that instance can't be read any more"))?;
            let ctx = crate::mods::ModContext::build(&dirs, &inst, &settings).await;
            let r = crate::mods::install::install_project(&mut inst, &ctx, &id, content, &p).await?;
            if !r.warnings.is_empty() {
                n.send(Msg::Toast(format!("{title}: {}", r.warnings.join(" ")), true));
            }
            Ok(format!("Added {} to {}", r.installed.join(", "), inst.name()))
        });
        self.mods_cache = None;
    }

    /// Adds a Modrinth data pack to worlds of an instance.
    pub(super) fn install_data_pack_hit(&mut self, hit: &Hit, folder: &str, worlds: Vec<String>) {
        let Some(inst) = self.instance(folder) else {
            self.toast_err("That instance isn't there any more. Choose another one.");
            return;
        };
        let (id, title, root) = (hit.project_id.clone(), hit.title.clone(), inst.root.clone());
        self.discover.installing.insert(id.clone());
        let (dirs, settings, lock) = (self.dirs.clone(), self.settings.clone(), super::mods::instance_lock(&root));
        self.start_task(format!("Installing {title}"), move |p, _| async move {
            let _turn = lock.lock().await;
            let inst = crate::instances::load_one(&root)
                .ok_or_else(|| anyhow::anyhow!("that instance can't be read any more"))?;
            let options = crate::content::ServiceOptions::from_settings(&settings);
            let mut service = crate::content::ContentService::new(&dirs, inst, options);
            service.install_data_pack(&id, &worlds, &p).await?;
            Ok(format!("Added {title} to {}", worlds_text(&worlds)))
        });
        self.mods_cache = None;
    }

    /// Discover lists Modrinth's and CurseForge's modpacks; the other sources have pages of their own.
    pub(super) fn other_modpack_sources(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            small_muted(ui, "More modpacks:");
            if text_link(ui, "ATLauncher and Technic", FS_META).clicked() {
                self.page = Page::Packs;
            }
            small_muted(ui, "and");
            if text_link(ui, "Feed The Beast", FS_META).clicked() {
                self.page = Page::Ftb;
            }
        });
    }

    /// Keeps the list of Modrinth projects in the target instance current, reading it again only when the
    /// instance's file changes.
    pub(super) fn discover_refresh_installed(&mut self) {
        let Some(inst) = self.discover.target.as_ref().and_then(|f| self.instance(f)) else {
            self.discover.installed = None;
            return;
        };
        let stamp = std::fs::metadata(inst.instance_file()).and_then(|m| m.modified()).ok();
        let fresh = matches!(&self.discover.installed, Some((folder, at, _)) if *folder == inst.folder && *at == stamp);
        if !fresh {
            let ids = crate::mods::files::installed_project_ids(inst);
            self.discover.installed = Some((inst.folder.clone(), stamp, ids));
        }
    }

    pub(super) fn discover_ui(&mut self, ui: &mut Ui) {
        // an instance that was deleted or renamed can't be the target any more
        if self.discover.target.as_ref().is_some_and(|f| self.instance(f).is_none()) {
            self.discover.target = None;
            self.discover.searched = false;
        }
        // the CurseForge page, when that platform is chosen
        if self.cf_discover_ui(ui) {
            return;
        }
        // the first visit shows the most downloaded right away
        let mut search = !self.discover.searched;
        ui.horizontal(|ui| {
            heading(ui, "Discover");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.cf_platform_switch(ui);
            });
        });
        ui.add_space(8.0);
        tab_bar(ui, |ui| {
            for k in Kind::ALL {
                if tab(ui, k.label(), self.discover.kind == k).clicked() && self.discover.kind != k {
                    self.discover.kind = k;
                    search = true;
                }
            }
        });
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            let mut q = std::mem::take(&mut self.discover.query);
            let mut edited = self.discover.edited.take();
            if live_search(ui, &mut q, &format!("Search {}", self.discover.kind.label().to_lowercase()), &mut edited) {
                search = true;
            }
            self.discover.query = q;
            self.discover.edited = edited;
            if self.discover.kind != Kind::Modpack {
                let current = self
                    .discover
                    .target
                    .as_ref()
                    .and_then(|f| self.instance(f))
                    .map(|i| i.name().to_string())
                    .unwrap_or_else(|| "Any instance".into());
                let names: Vec<(String, String)> =
                    self.instances.iter().map(|i| (i.folder.clone(), instance_choice(i))).collect();
                egui::ComboBox::from_id_salt("target").selected_text(format!("For: {current}")).width(220.0).show_ui(
                    ui,
                    |ui| {
                        if ui.selectable_label(self.discover.target.is_none(), "Any instance").clicked() {
                            self.discover.target = None;
                            search = true;
                        }
                        for (folder, name) in &names {
                            if ui.selectable_label(self.discover.target.as_ref() == Some(folder), name).clicked() {
                                self.discover.target = Some(folder.clone());
                                search = true;
                            }
                        }
                    },
                );
            }
            egui::ComboBox::from_id_salt("sort")
                .selected_text(match self.discover.sort {
                    Sort::Relevance => "Sort: Most relevant",
                    Sort::Downloads => "Sort: Most downloaded",
                    Sort::Updated => "Sort: Recently updated",
                })
                .width(190.0)
                .show_ui(ui, |ui| {
                    for (s, l) in [
                        (Sort::Relevance, "Most relevant"),
                        (Sort::Downloads, "Most downloaded"),
                        (Sort::Updated, "Recently updated"),
                    ] {
                        if ui.selectable_value(&mut self.discover.sort, s, l).clicked() {
                            search = true;
                        }
                    }
                });
        });
        // under the filters: what the list is limited to, and how many results there are
        let kind = self.discover.kind;
        let limited_to = self.discover.target.as_ref().and_then(|f| self.instance(f)).map(|inst| {
            // a data pack runs without a mod loader
            let loader = inst.loader().filter(|_| kind != Kind::DataPack);
            format!(
                "Showing what works with Minecraft {}{}",
                inst.mc_version(),
                loader.map(|(k, _)| format!(" and {k}")).unwrap_or_default()
            )
        });
        let d = &self.discover;
        let total = d.total + if d.both { d.cf.total } else { 0 } + if d.with_smithed() { d.sm.total } else { 0 };
        let count = (total > 0).then(|| format!("{} results", thousands(total)));
        let mut show_everything = false;
        if kind == Kind::Modpack || limited_to.is_some() || count.is_some() {
            context_line(ui, count, limited_to.is_some() && kind != Kind::Modpack, |ui| {
                if kind == Kind::Modpack {
                    self.other_modpack_sources(ui);
                } else if let Some(text) = limited_to {
                    small_muted(ui, text);
                    if ghost(ui, "Show everything").clicked() {
                        show_everything = true;
                    }
                }
            });
        }
        if show_everything {
            self.discover.target = None;
            search = true;
        }
        if search {
            self.discover_search(true);
        }
        if self.discover.both {
            self.discover_both_list(ui);
            return;
        }
        // a new search over rows that are already shown: a thin bar says it is on its way
        if self.discover.loading && self.discover.offset_pending == 0 && !self.discover.hits.is_empty() {
            progress_bar(ui, None, 2.0);
        } else {
            ui.add_space(2.0);
        }

        if let Some(e) = self.discover.error.clone() {
            let text = "Couldn't reach Modrinth. Check your internet connection, or try again in a few minutes.";
            if fetch_error(ui, text, &e) && !self.discover.loading {
                // the first page again, or the page that failed to load
                self.discover_search(self.discover.hits.is_empty());
            }
        }
        if self.discover.hits.is_empty() && self.discover.loading {
            skeleton_rows(ui, 6, ROW_H);
            return;
        }
        if self.discover.hits.is_empty() && self.discover.error.is_none() {
            let query = self.discover.query.trim().to_string();
            let text = if query.is_empty() {
                "Nothing to show. Try another kind, or pick a different instance.".to_string()
            } else {
                format!("No results for \u{201C}{query}\u{201D} on Modrinth.")
            };
            if empty_row(ui, &text, (!query.is_empty()).then_some("Clear search")).is_some_and(|r| r.clicked()) {
                self.discover.query.clear();
                self.discover_search(true);
            }
            return;
        }

        self.discover_refresh_installed();
        let installed = self.discover.installed.take();
        let hits = std::mem::take(&mut self.discover.hits);
        let target = self.discover.target.clone();
        let mut install: Option<(Hit, Option<String>)> = None;
        let mut install_data: Option<(Hit, String, Vec<String>)> = None;
        let mut more = false;
        ScrollArea::vertical().id_salt(("discover-list", self.discover.list_gen)).auto_shrink(false).show(ui, |ui| {
            for hit in &hits {
                let key = format!("mr:{}", hit.project_id);
                let is_installed = kind != Kind::Modpack
                    && kind != Kind::DataPack
                    && installed.as_ref().is_some_and(|(_, _, ids)| ids.contains(&hit.project_id));
                let busy = self.discover.installing.contains(&hit.project_id)
                    && self.tasks.iter().any(|t| t.label == format!("Installing {}", hit.title));
                row_frame(ui, |ui| {
                    let wide = ui.available_width() >= 560.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        match &hit.icon_url {
                            Some(url) if !url.is_empty() => self.remote_image(ui, url, 48.0, &hit.title),
                            _ => {
                                avatar(ui, &hit.title, 48.0);
                            }
                        }
                        let text_w = row_text_width(ui, &key);
                        ui.vertical(|ui| {
                            ui.set_max_width(text_w);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            name_line(ui, &hit.title, Some(&hit.author));
                            summary_line(ui, &hit.description);
                            let mut parts = Vec::new();
                            if !wide {
                                parts.push((format!("{} downloads", human_count(hit.downloads)), c_faint()));
                            }
                            let tags = hit.categories.iter().map(String::as_str);
                            parts.extend(if kind == Kind::DataPack {
                                data_pack_tags(tags, 3)
                            } else {
                                tag_parts(tags, 3)
                            });
                            meta_line(ui, &parts);
                        });
                        row_actions(ui, &key, |ui| {
                            action_cell(ui, &key, |ui| {
                                if busy {
                                    spinner(ui);
                                    return;
                                }
                                if kind == Kind::DataPack {
                                    let have = |inst: &crate::instances::Instance| {
                                        worlds_with(inst, Some(&hit.project_id), None)
                                    };
                                    if let Some((folder, worlds)) =
                                        data_pack_button(ui, &self.instances, target.as_deref(), &have)
                                    {
                                        install_data = Some((hit.clone(), folder, worlds));
                                    }
                                } else if is_installed {
                                    installed_mark(ui);
                                } else if kind == Kind::Modpack {
                                    if secondary(ui, "Install").clicked() {
                                        install = Some((hit.clone(), None));
                                    }
                                } else if let Some(t) = &target {
                                    if secondary(ui, "Install").clicked() {
                                        install = Some((hit.clone(), Some(t.clone())));
                                    }
                                } else {
                                    let r = secondary(ui, "Install");
                                    egui::Popup::menu(&r).show(|ui| {
                                        ui.set_min_width(180.0);
                                        if self.instances.is_empty() {
                                            ui.label("Create an instance first");
                                        }
                                        for inst in &self.instances {
                                            if ui.button(instance_choice(inst)).clicked() {
                                                install = Some((hit.clone(), Some(inst.folder.clone())));
                                                ui.close();
                                            }
                                        }
                                    });
                                }
                                kebab_menu(ui, |ui| {
                                    if is_installed {
                                        if let Some(t) = &target {
                                            // installing again puts the newest fitting version in place of the old one
                                            if ui
                                                .button("Reinstall")
                                                .on_hover_text("Puts the newest version that fits in place of this one")
                                                .clicked()
                                            {
                                                install = Some((hit.clone(), Some(t.clone())));
                                                ui.close();
                                            }
                                        }
                                    }
                                    if ui.button("Open page").clicked() {
                                        let slug = if hit.slug.is_empty() { &hit.project_id } else { &hit.slug };
                                        let _ = open::that(format!("https://modrinth.com/{}/{slug}", hit.project_type));
                                        ui.close();
                                    }
                                });
                            });
                            if wide {
                                ui.add_space(8.0);
                                stat_column(ui, &human_count(hit.downloads), "downloads");
                            }
                        });
                    });
                });
            }
            ui.add_space(8.0);
            let shown = hits.len() as u64;
            if shown < self.discover.total {
                ui.vertical_centered(|ui| {
                    small_faint(ui, format!("Showing {} of {}", thousands(shown), thousands(self.discover.total)));
                    if self.discover.loading {
                        spinner(ui);
                    } else {
                        let next = (self.discover.total - shown).min(u64::from(PAGE));
                        if ghost(ui, &format!("Load {next} more")).clicked() {
                            more = true;
                        }
                    }
                });
            } else if self.discover.loading {
                ui.vertical_centered(|ui| {
                    spinner(ui);
                });
            }
            ui.add_space(8.0);
        });
        self.discover.hits = hits;
        if self.discover.installed.is_none() {
            self.discover.installed = installed;
        }
        if let Some((hit, folder)) = install {
            self.install_hit(&hit, folder);
        }
        if let Some((hit, folder, worlds)) = install_data {
            self.install_data_pack_hit(&hit, &folder, worlds);
        }
        if more {
            self.discover_search(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_searches_after_a_pause_and_enter_at_once() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut edited = None;
        // nothing typed: nothing to do
        assert_eq!(live_step(&mut edited, false, false, t0), LiveStep::Idle);
        // a key starts the wait
        assert_eq!(live_step(&mut edited, true, false, t0), LiveStep::Wait(ms(300)));
        assert_eq!(live_step(&mut edited, false, false, t0 + ms(100)), LiveStep::Wait(ms(200)));
        // another key starts it again
        assert_eq!(live_step(&mut edited, true, false, t0 + ms(250)), LiveStep::Wait(ms(300)));
        assert_eq!(live_step(&mut edited, false, false, t0 + ms(500)), LiveStep::Wait(ms(50)));
        // a pause long enough searches, once
        assert_eq!(live_step(&mut edited, false, false, t0 + ms(550)), LiveStep::Go);
        assert_eq!(live_step(&mut edited, false, false, t0 + ms(600)), LiveStep::Idle);
        // Enter does not wait, and the wait it cut short does not search a second time
        assert_eq!(live_step(&mut edited, true, true, t0 + ms(700)), LiveStep::Go);
        assert_eq!(live_step(&mut edited, false, false, t0 + ms(1200)), LiveStep::Idle);
    }

    #[test]
    fn counts_have_thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1284), "1,284");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn loaders_come_first_in_their_own_colours() {
        let names = ["optimization", "quilt", "utility", "fabric", "decoration", "library"];
        let parts = tag_parts(names.into_iter(), 2);
        let words: Vec<&str> = parts.iter().map(|(w, _)| w.as_str()).collect();
        // both loaders, then two categories in the order given
        assert_eq!(words, ["Quilt", "Fabric", "optimization", "utility"]);
        assert_eq!(parts[0].1, loader_color("Quilt"));
        assert_eq!(parts[1].1, loader_color("Fabric"));
        assert_eq!(parts[2].1, c_faint());
    }
}
