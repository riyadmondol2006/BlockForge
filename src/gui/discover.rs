//! Discover: search Modrinth (or, with "All", Modrinth and CurseForge together, and Smithed for data packs) and
//! install into an instance.

use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime};

use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, CornerRadius, FontId, Layout, Response, RichText, ScrollArea, Sense, Stroke, Ui, vec2};

use super::icons::{self, Icon};
use super::motion;
use super::theme::*;
use super::widgets::*;
use super::{App, Msg, Page};
use crate::modrinth::{self, Hit, Kind, Narrow, Side, Sort};
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
    /// When the search field was last typed in and the search it wants has not started yet (see [`live_search_w`]).
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
    /// Modrinth's categories the results must all be in (see [`categories_of`]).
    pub categories: Vec<String>,
    /// The Minecraft version asked for while no instance is chosen.
    pub mc_filter: Option<String>,
    /// The mod loader asked for while no instance is chosen, as Modrinth names it (`fabric`).
    pub loader_filter: Option<String>,
    /// Where the mods have to run.
    pub side: Side,
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
            categories: Vec::new(),
            mc_filter: None,
            loader_filter: None,
            side: Side::Any,
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

/// How long the search field waits after the last key before it searches.
const LIVE_DELAY: Duration = Duration::from_millis(300);

/// The width of the column at the right end of a result that holds its button.
const ACTION_W: f32 = 120.0;

/// The height a result takes (the picture and the room around it), for the grey rows shown while a list loads.
pub(super) const ROW_H: f32 = 80.0;

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
        [width, 34.0],
        egui::TextEdit::singleline(text)
            .hint_text(RichText::new(hint).color(c_faint()))
            .margin(egui::Margin { left: 36, right: 12, top: 8, bottom: 8 })
            .background_color(c_field()),
    );
    // the magnifier brightens while the field is being typed in
    let t = motion::toward(ui.ctx(), r.id.with("search-focus"), r.has_focus(), motion::FAST);
    icons::paint(
        ui.painter(),
        Icon::Search,
        egui::pos2(r.rect.left() + 19.0, r.rect.center().y),
        15.0,
        motion::mix(c_faint(), c_accent(), t),
    );
    r
}

/// The ground of a well: a panel set one step into what it sits on (a dialog, a card), for a list inside it.
pub(super) fn c_well() -> Color32 {
    if is_dark() { motion::mix(c_raised(), c_bg(), 0.45) } else { motion::mix(c_raised(), c_bg(), 0.6) }
}

/// A list or a group of fields set into a dialog: the well's ground, a hairline and rounded corners.
pub(super) fn well<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(c_well())
        .stroke(Stroke::new(1.0, c_rule()))
        .corner_radius(CornerRadius::same(R_CONTROL + 2))
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A short word on a small tinted pill: where a result comes from, or something to know about it.
pub(super) fn pill(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(FS_META), color);
    let size = vec2(galley.size().x + 14.0, 18.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let tint = if is_dark() { 0.16 } else { 0.11 };
        ui.painter().rect(
            rect,
            CornerRadius::same(9),
            color.gamma_multiply(tint),
            Stroke::new(1.0, color.gamma_multiply(tint * 1.6)),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
    resp
}

/// The last line of a result with pills in front: the pills (where it comes from, a warning), then the plain parts.
pub(super) fn pill_meta_line(ui: &mut Ui, pills: &[(String, Color32)], parts: &[(String, Color32)]) {
    if pills.is_empty() {
        meta_line(ui, parts);
        return;
    }
    ui.scope(|ui| {
        // a line of text with pills is as tall as a pill, not as a button
        ui.spacing_mut().interact_size.y = 18.0;
        ui.spacing_mut().item_spacing.x = 5.0;
        ui.horizontal(|ui| {
            for (text, color) in pills {
                pill(ui, text, *color);
            }
            if !parts.is_empty() {
                ui.add_space(3.0);
                meta_line(ui, parts);
            }
        });
    });
}

/// The side of a result's picture.
pub(super) const ICON: f32 = 48.0;

impl App {
    /// A result's picture (the project's icon, or its first letter while there is none) set on a small tile with a
    /// soft shadow and a fine edge, so icons with see-through corners and white ones sit alike on the page.
    pub(super) fn result_icon(&mut self, ui: &mut Ui, url: Option<&str>, title: &str, size: f32) {
        let under = ui.painter().add(egui::Shape::Noop);
        let rect = ui
            .scope(|ui| match url.filter(|u| !u.is_empty()) {
                Some(url) => self.remote_image(ui, url, size, title),
                None => {
                    avatar(ui, title, size);
                }
            })
            .response
            .rect;
        let radius = CornerRadius::same((size * 0.22) as u8);
        let shadow = egui::Shadow {
            offset: [0, 3],
            blur: 10,
            spread: 0,
            color: Color32::from_black_alpha(if is_dark() { 90 } else { 26 }),
        };
        ui.painter().set(
            under,
            egui::Shape::Vec(vec![
                shadow.as_shape(rect, radius).into(),
                egui::Shape::rect_filled(rect, radius, c_field()),
            ]),
        );
        let edge = if is_dark() { Color32::from_white_alpha(16) } else { Color32::from_black_alpha(18) };
        ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
    }
}

/// What a list says when it has nothing to show: a quiet round mark, one sentence and, if there is one, the action.
pub(super) fn empty_state(ui: &mut Ui, text: &str, action: Option<&str>) -> Option<Response> {
    ui.add_space(36.0);
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
        let p = ui.painter();
        motion::glow(p, rect.center(), 44.0, c_accent().gamma_multiply(if is_dark() { 0.10 } else { 0.14 }));
        p.circle(rect.center(), 26.0, c_card(), Stroke::new(1.0, c_border()));
        icons::paint(p, Icon::Search, rect.center(), 20.0, c_faint());
        ui.add_space(12.0);
        ui.add(egui::Label::new(RichText::new(text).size(FS_BODY).color(c_muted())).wrap());
        action.map(|a| {
            ui.add_space(10.0);
            secondary(ui, a)
        })
    })
    .inner
}

/// Rows in the shape of results (picture, name, summary, the button at the right) while the first ones are on their
/// way; the rows further down are fainter, and they all breathe gently.
pub(super) fn result_skeleton(ui: &mut Ui, rows: usize) {
    let reduced = motion::reduced(ui.ctx());
    let phase = if reduced { 0.5 } else { (ui.input(|i| i.time) * 2.4).sin() as f32 * 0.5 + 0.5 };
    if !reduced {
        ui.ctx().request_repaint_after(ANIMATION_FRAME);
    }
    let shade = motion::mix(c_field(), c_active(), phase * 0.7);
    for n in 0..rows {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let fade = 1.0 - n as f32 / (rows as f32 + 1.0);
        let c = shade.gamma_multiply(fade);
        let p = ui.painter();
        let icon = egui::Rect::from_min_size(rect.min + vec2(14.0, (ROW_H - ICON) / 2.0), vec2(ICON, ICON));
        p.rect_filled(icon, CornerRadius::same((ICON * 0.22) as u8), c);
        let x = icon.right() + 14.0;
        let bar = |y: f32, w: f32, h: f32| {
            p.rect_filled(
                egui::Rect::from_min_size(egui::pos2(x, rect.top() + y), vec2(w, h)),
                CornerRadius::same(R_TAG),
                c,
            )
        };
        bar(16.0, (rect.width() * 0.22).min(220.0), 11.0);
        bar(35.0, (rect.width() * 0.46).min(460.0), 9.0);
        bar(52.0, (rect.width() * 0.16).min(150.0), 8.0);
        if rect.width() > 420.0 {
            let button = egui::Rect::from_min_size(
                egui::pos2(rect.right() - 14.0 - 86.0, rect.center().y - 16.0),
                vec2(86.0, 32.0),
            );
            p.rect_filled(button, CornerRadius::same(R_CONTROL), c);
        }
        if n + 1 < rows {
            p.hline(rect.x_range().shrink(14.0), rect.bottom() - 0.5, Stroke::new(1.0, c_rule()));
        }
    }
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

/// The timing of [`live_search_w`]: a change starts the wait, Enter ends it at once, and the wait ends by itself after
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
/// on Enter. `edited` remembers when the text last changed. The field is `width` wide. Returns true when a search
/// should start now.
pub(super) fn live_search_w(
    ui: &mut Ui,
    text: &mut String,
    hint: &str,
    edited: &mut Option<Instant>,
    width: f32,
) -> bool {
    let r = search_input(ui, text, hint, width);
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

/// One result in a list: no box at rest and a hairline between rows; under the pointer the row rises into a pane of
/// glass (its fill, line and soft shadow fade in) and the hairline fades out.
pub(super) fn row_frame<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.spacing_mut().item_spacing.y = 0.0;
    let id = ui.next_auto_id().with("result-row");
    let bg = ui.painter().add(egui::Shape::Noop);
    let out = egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    });
    let rect = out.response.rect;
    // the row's lower edge belongs to the next row, so two rows never light up at once
    let hot = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.max.y - 1.0));
    let t = motion::hover(ui, id, ui.rect_contains_pointer(hot));
    if t > 0.0 {
        let radius = CornerRadius::same(R_CARD);
        let s = card_shadow();
        let shadow = egui::Shadow { color: s.color.gamma_multiply(t), ..s };
        let pane = egui::Shape::rect_filled(rect, radius, c_card().gamma_multiply(t));
        let line = egui::Shape::rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, c_border().gamma_multiply(t)),
            egui::StrokeKind::Inside,
        );
        let edge = egui::Shape::hline(
            rect.left() + R_CARD as f32..=rect.right() - R_CARD as f32,
            rect.top() + 0.5,
            Stroke::new(1.0, c_glass_edge().gamma_multiply(t)),
        );
        ui.painter().set(bg, egui::Shape::Vec(vec![shadow.as_shape(rect, radius).into(), pane, line, edge]));
    }
    if t < 1.0 {
        ui.painter().hline(
            rect.x_range().shrink(14.0),
            rect.bottom(),
            Stroke::new(1.0, c_rule().gamma_multiply(1.0 - t)),
        );
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
/// The figure has a small arrow in front of it (down, into the computer) in the faint ink of the unit.
pub(super) fn stat_column(ui: &mut Ui, figure: &str, unit: &str) {
    let (rect, resp) = ui.allocate_exact_size(vec2(78.0, 36.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let fig = p.layout_no_wrap(figure.to_string(), super::fonts::semibold(FS_BODY), c_text());
        let sub = p.layout_no_wrap(unit.to_string(), FontId::proportional(FS_META), c_faint());
        let fig_at = egui::pos2(rect.right() - fig.size().x, rect.top() + 1.0);
        // the arrow: a stem, a head and the tray it falls into
        let (cx, cy) = (fig_at.x - 9.0, fig_at.y + fig.size().y / 2.0);
        let stroke = Stroke::new(1.4, c_faint());
        p.line_segment([egui::pos2(cx, cy - 5.0), egui::pos2(cx, cy + 2.0)], stroke);
        p.line_segment([egui::pos2(cx - 3.0, cy - 1.0), egui::pos2(cx, cy + 2.0)], stroke);
        p.line_segment([egui::pos2(cx + 3.0, cy - 1.0), egui::pos2(cx, cy + 2.0)], stroke);
        p.line_segment([egui::pos2(cx - 4.0, cy + 5.0), egui::pos2(cx + 4.0, cy + 5.0)], stroke);
        p.galley(fig_at, fig.clone(), c_text());
        p.galley(egui::pos2(rect.right() - sub.size().x, fig_at.y + fig.size().y + 1.0), sub, c_faint());
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, format!("{figure} {unit}")));
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

/// "Installed" with a check in front, on a pill in the brand's soft tint: a state, not an action.
pub(super) fn installed_mark(ui: &mut Ui) {
    let text = "Installed";
    let fg = c_accent();
    let galley = ui.painter().layout_no_wrap(text.into(), super::fonts::medium(FS_META), fg);
    let (rect, resp) = ui.allocate_exact_size(vec2(32.0 + galley.size().x, 26.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            CornerRadius::same(13),
            c_accent_soft(),
            Stroke::new(1.0, fg.gamma_multiply(0.25)),
            egui::StrokeKind::Inside,
        );
        paint_check(ui.painter(), egui::pos2(rect.left() + 13.0, rect.center().y), 11.0, fg);
        ui.painter().galley(egui::pos2(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0), galley, fg);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
}

/// The small cross at the end of a dialog's title: a round tone fades in under the pointer.
pub(super) fn close_button(ui: &mut Ui) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    let t = motion::hover(ui, resp.id, resp.hovered());
    if t > 0.0 {
        ui.painter().circle_filled(rect.center(), 15.0, c_hover().gamma_multiply(t));
    }
    let stroke = Stroke::new(1.5, motion::mix(c_muted(), c_text(), t));
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
/// results at the right, and after it `end` (the switch between rows and cards).
pub(super) fn context_line_with(
    ui: &mut Ui,
    count: Option<String>,
    buttons: bool,
    lead: impl FnOnce(&mut Ui),
    end: impl FnOnce(&mut Ui),
) {
    ui.add_space(4.0);
    ui.scope(|ui| {
        // a row of text is as tall as its text, a row with buttons as tall as they are
        ui.spacing_mut().interact_size.y = if buttons { 32.0 } else { 20.0 };
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                end(ui);
                if let Some(count) = count {
                    ui.label(RichText::new(count).size(FS_META).color(c_faint()));
                }
                ui.with_layout(Layout::left_to_right(Align::Center), lead);
            });
        });
    });
}

/// A group of Modrinth's categories with its heading, as the Category menu lists them.
pub(super) type CategoryGroup = (&'static str, &'static [(&'static str, &'static str)]);

const MOD_CATEGORIES: &[CategoryGroup] = &[(
    "Categories",
    &[
        ("adventure", "Adventure"),
        ("cursed", "Cursed"),
        ("decoration", "Decoration"),
        ("economy", "Economy"),
        ("equipment", "Equipment"),
        ("food", "Food"),
        ("game-mechanics", "Game mechanics"),
        ("library", "Library"),
        ("magic", "Magic"),
        ("management", "Management"),
        ("minigame", "Minigame"),
        ("mobs", "Mobs"),
        ("optimization", "Optimization"),
        ("social", "Social"),
        ("storage", "Storage"),
        ("technology", "Technology"),
        ("transportation", "Transportation"),
        ("utility", "Utility"),
        ("worldgen", "World generation"),
    ],
)];

const MODPACK_CATEGORIES: &[CategoryGroup] = &[(
    "Categories",
    &[
        ("adventure", "Adventure"),
        ("challenging", "Challenging"),
        ("combat", "Combat"),
        ("kitchen-sink", "Kitchen sink"),
        ("lightweight", "Lightweight"),
        ("magic", "Magic"),
        ("multiplayer", "Multiplayer"),
        ("optimization", "Optimization"),
        ("quests", "Quests"),
        ("technology", "Technology"),
    ],
)];

const RESOURCE_PACK_CATEGORIES: &[CategoryGroup] = &[
    (
        "Categories",
        &[
            ("combat", "Combat"),
            ("cursed", "Cursed"),
            ("decoration", "Decoration"),
            ("modded", "Modded"),
            ("realistic", "Realistic"),
            ("simplistic", "Simplistic"),
            ("themed", "Themed"),
            ("tweaks", "Tweaks"),
            ("utility", "Utility"),
            ("vanilla-like", "Vanilla-like"),
        ],
    ),
    (
        "Features",
        &[
            ("audio", "Audio"),
            ("blocks", "Blocks"),
            ("core-shaders", "Core shaders"),
            ("entities", "Entities"),
            ("environment", "Environment"),
            ("equipment", "Equipment"),
            ("fonts", "Fonts"),
            ("gui", "GUI"),
            ("items", "Items"),
            ("locale", "Locale"),
            ("models", "Models"),
        ],
    ),
    (
        "Resolution",
        &[
            ("8x-", "8x or lower"),
            ("16x", "16x"),
            ("32x", "32x"),
            ("48x", "48x"),
            ("64x", "64x"),
            ("128x", "128x"),
            ("256x", "256x"),
            ("512x+", "512x or higher"),
        ],
    ),
];

const SHADER_CATEGORIES: &[CategoryGroup] = &[
    (
        "Style",
        &[
            ("cartoon", "Cartoon"),
            ("cursed", "Cursed"),
            ("fantasy", "Fantasy"),
            ("realistic", "Realistic"),
            ("semi-realistic", "Semi-realistic"),
            ("vanilla-like", "Vanilla-like"),
        ],
    ),
    (
        "Features",
        &[
            ("atmosphere", "Atmosphere"),
            ("bloom", "Bloom"),
            ("colored-lighting", "Coloured lighting"),
            ("foliage", "Foliage"),
            ("path-tracing", "Path tracing"),
            ("pbr", "PBR"),
            ("reflections", "Reflections"),
            ("shadows", "Shadows"),
        ],
    ),
    (
        "Performance",
        &[("potato", "Potato"), ("low", "Low"), ("medium", "Medium"), ("high", "High"), ("screenshot", "Screenshot")],
    ),
];

/// Modrinth's categories for a kind, in groups. Data packs share the mods' categories. Kept here rather than asked
/// for: they change rarely, and the menu then opens at once.
pub(super) fn categories_of(kind: Kind) -> &'static [CategoryGroup] {
    match kind {
        Kind::Mod | Kind::DataPack => MOD_CATEGORIES,
        Kind::Modpack => MODPACK_CATEGORIES,
        Kind::ResourcePack => RESOURCE_PACK_CATEGORIES,
        Kind::Shader => SHADER_CATEGORIES,
    }
}

/// How a category of a kind is written in the menu and on its chip, when the kind has it.
pub(super) fn category_label(kind: Kind, name: &str) -> Option<&'static str> {
    categories_of(kind).iter().flat_map(|(_, list)| list.iter()).find(|(n, _)| *n == name).map(|(_, label)| *label)
}

/// The mod loaders the loader filter offers, as Modrinth names them and as they are written.
pub(super) const LOADERS: [(&str, &str); 4] =
    [("fabric", "Fabric"), ("quilt", "Quilt"), ("forge", "Forge"), ("neoforge", "NeoForge")];

/// The loaders a search for mods of `loader` asks for: Quilt also runs Fabric's mods, as an instance with it does.
pub(super) fn loaders_for(loader: &str) -> Vec<String> {
    if loader == "quilt" { vec!["quilt".into(), "fabric".into()] } else { vec![loader.to_string()] }
}

/// CurseForge's numbers for the loaders a search for mods of `loader` asks for.
pub(super) fn cf_loaders_for(loader: &str) -> Vec<i64> {
    use crate::curseforge::{MODLOADER_FABRIC, MODLOADER_FORGE, MODLOADER_NEOFORGE, MODLOADER_QUILT};
    match loader {
        "fabric" => vec![MODLOADER_FABRIC],
        "quilt" => vec![MODLOADER_FABRIC, MODLOADER_QUILT],
        "forge" => vec![MODLOADER_FORGE],
        "neoforge" => vec![MODLOADER_NEOFORGE],
        _ => Vec::new(),
    }
}

/// How the environment filter names where a mod runs.
fn side_label(side: Side) -> &'static str {
    match side {
        Side::Any => "Any",
        Side::Client => "Client side",
        Side::Server => "Server side",
    }
}

/// Text cut to `max` characters with an ellipsis, for a value on a small control.
pub(super) fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}\u{2026}", text.chars().take(max).collect::<String>())
    }
}

/// A filter or an order as a compact control: its name in grey, what is chosen in ink and a small chevron. One that
/// narrows the list (`active`) is tinted with the brand colour, like a chip that is on. Its menu (`add`) opens under
/// it; with `keep_open` it stays open while choices are ticked in it. `salt` keeps the menu open across frames when
/// the controls before it come and go. Returns what `add` returned while the menu is open.
pub(super) fn dropdown<R>(
    ui: &mut Ui,
    salt: &str,
    label: &str,
    value: &str,
    active: bool,
    keep_open: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let name = ui.painter().layout_no_wrap(format!("{label}: "), FontId::proportional(13.0), c_muted());
    let chosen = ui.painter().layout_no_wrap(value.to_string(), super::fonts::medium(13.0), Color32::PLACEHOLDER);
    let width = 12.0 + name.size().x + chosen.size().x + 10.0 + 9.0 + 12.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
    let resp = ui.interact(rect, ui.make_persistent_id(("dropdown", salt)), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let t = motion::hover(ui, resp.id, resp.hovered() || open);
    let s = motion::toward(ui.ctx(), resp.id.with("on"), active, motion::FAST);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let bg = motion::mix(motion::mix(c_field(), c_field_hover(), t), c_accent_soft(), s);
        let line = motion::mix(motion::mix(c_border(), c_control(), t * 0.6), c_accent().gamma_multiply(0.6), s);
        p.rect(rect, CornerRadius::same(R_CONTROL), bg, Stroke::new(1.0, line), egui::StrokeKind::Inside);
        let fg = motion::mix(c_text(), if is_dark() { c_accent() } else { c_accent_hover() }, s);
        let y = rect.center().y;
        let name_w = name.size().x;
        p.galley(egui::pos2(rect.left() + 12.0, y - name.size().y / 2.0), name, c_muted());
        p.galley(egui::pos2(rect.left() + 12.0 + name_w, y - chosen.size().y / 2.0), chosen, fg);
        // the chevron points down, and up while the menu is open
        let (cx, d) = (rect.right() - 16.5, if open { -1.0 } else { 1.0 });
        let chevron =
            vec![egui::pos2(cx - 4.0, y - 2.0 * d), egui::pos2(cx, y + 2.0 * d), egui::pos2(cx + 4.0, y - 2.0 * d)];
        p.add(egui::Shape::line(chevron, Stroke::new(1.5, motion::mix(c_muted(), fg, t.max(s)))));
    }
    focus_ring(ui, &resp, rect, R_CONTROL);
    let text = format!("{label}: {value}");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &text));
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let mut popup = egui::Popup::menu(&resp).width(width.max(200.0));
    if keep_open {
        popup = popup.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    }
    popup.show(add).map(|r| r.inner)
}

/// A filter that is on, as a pill in the brand's soft tint with a cross: pressing it takes the filter off. Returns
/// true when it was pressed.
pub(super) fn filter_chip(ui: &mut Ui, text: &str) -> bool {
    let ink = if is_dark() { c_accent() } else { c_accent_hover() };
    let galley = ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(FS_META), ink);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 36.0, 26.0), Sense::click());
    let t = motion::hover(ui, resp.id, resp.hovered());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let line = ink.gamma_multiply(0.3 + 0.3 * t);
        p.rect(rect, CornerRadius::same(13), c_accent_soft(), Stroke::new(1.0, line), egui::StrokeKind::Inside);
        p.galley(egui::pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0), galley, ink);
        // the cross, on a round tone of its own under the pointer
        let c = egui::pos2(rect.right() - 14.0, rect.center().y);
        if t > 0.0 {
            p.circle_filled(c, 8.0, ink.gamma_multiply(0.18 * t));
        }
        let stroke = Stroke::new(1.4, ink);
        p.line_segment([c + vec2(-3.0, -3.0), c + vec2(3.0, 3.0)], stroke);
        p.line_segment([c + vec2(-3.0, 3.0), c + vec2(3.0, -3.0)], stroke);
    }
    focus_ring(ui, &resp, rect, 13);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Remove filter: {text}")));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Remove this filter").clicked()
}

/// What a row of filter chips asked for.
#[derive(Debug, PartialEq)]
pub(super) enum ChipAct {
    /// Take off the filter of this chip.
    Remove(usize),
    /// Take off every filter.
    Clear,
}

/// The filters that are on, under a page's search bar: a chip for each, "Clear filters" after them, and a note on a
/// line of its own under them (a filter only some sites have, say). Draws nothing when no filter is on.
pub(super) fn chip_row(ui: &mut Ui, chips: &[String], note: Option<&str>) -> Option<ChipAct> {
    if chips.is_empty() {
        return None;
    }
    let mut act = None;
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        ui.spacing_mut().interact_size.y = 26.0;
        for (i, text) in chips.iter().enumerate() {
            if filter_chip(ui, text) {
                act = Some(ChipAct::Remove(i));
            }
        }
        ui.scope(|ui| {
            // a quiet word, as tall as the chips
            ui.spacing_mut().interact_size.y = 26.0;
            if text_link(ui, "Clear filters", FS_META).clicked() {
                act = Some(ChipAct::Clear);
            }
        });
    });
    if let Some(note) = note {
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(note).size(FS_META).color(c_faint())).wrap());
    }
    act
}

/// The smallest a search field gets beside the controls of its bar before they move to a line of their own.
const SEARCH_MIN: f32 = 220.0;

/// The widest a search field gets with every control on its line; wider only spreads a few words over an empty field.
const SEARCH_MAX: f32 = 640.0;

/// The bar above a list. The search field (`search` is given its width) comes first with `lead` (the instance and
/// the order) beside it, then `rest` (the filters). On a wide window all of it shares one line; when the rest
/// doesn't fit, it goes on the lines under the field; and when not even `lead` fits beside a field of a useful
/// width, the field goes across the bar with everything else wrapping under it. What the closures share (`state`)
/// is handed to each. `key` tells pages apart.
pub(super) fn search_bar<S>(
    ui: &mut Ui,
    key: &str,
    state: &mut S,
    search: impl FnOnce(&mut Ui, &mut S, f32),
    lead: impl FnOnce(&mut Ui, &mut S),
    rest: impl FnOnce(&mut Ui, &mut S),
) {
    let id = egui::Id::unique(("search-bar", key));
    let (lead_id, rest_id) = (id.with("lead"), id.with("rest"));
    let out = egui::Frame::new()
        .fill(c_card())
        .stroke(Stroke::new(1.0, c_border()))
        .corner_radius(CornerRadius::same(R_CARD))
        .inner_margin(egui::Margin::same(8))
        .shadow(card_shadow())
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let avail = ui.available_width();
            let gap = ui.spacing().item_spacing.x;
            let lead_w = remembered_width(ui, lead_id, 0.0);
            let rest_w = remembered_width(ui, rest_id, 0.0);
            let one_line = avail - lead_w - rest_w - 2.0 * gap;
            let beside = avail - lead_w - gap;
            if one_line >= SEARCH_MIN {
                ui.horizontal(|ui| {
                    search(ui, state, one_line.min(SEARCH_MAX));
                    let w = ui.horizontal(|ui| lead(ui, state)).response.rect.width();
                    remember_width(ui, lead_id, w);
                    let w = ui.horizontal(|ui| rest(ui, state)).response.rect.width();
                    remember_width(ui, rest_id, w);
                });
            } else if beside >= SEARCH_MIN {
                ui.horizontal(|ui| {
                    search(ui, state, beside);
                    let w = ui.horizontal(|ui| lead(ui, state)).response.rect.width();
                    remember_width(ui, lead_id, w);
                });
                ui.add_space(2.0);
                // what the rest takes on its own line, or all of it once it wraps, so it stays there until the
                // window is wide enough for it beside the field
                let w = ui
                    .horizontal_wrapped(|ui| {
                        rest(ui, state);
                        ui.min_rect().width()
                    })
                    .inner;
                remember_width(ui, rest_id, w);
            } else {
                search(ui, state, avail);
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    lead(ui, state);
                    rest(ui, state);
                });
            }
        });
    glass_edge(ui.painter(), out.response.rect, R_CARD);
}

/// The two small pictures at the end of a list's count line that switch between rows and cards, the one in use
/// raised. Returns true for cards, false for rows, when the other was pressed.
pub(super) fn view_toggle(ui: &mut Ui, cards: bool) -> Option<bool> {
    let side = 26.0;
    let (outer, outer_resp) = ui.allocate_exact_size(vec2(2.0 * side + 6.0, side + 4.0), Sense::hover());
    let p = ui.painter().clone();
    p.rect(outer, CornerRadius::same(R_CONTROL - 2), c_field(), Stroke::new(1.0, c_border()), egui::StrokeKind::Inside);
    let x = motion::glide(ui.ctx(), outer_resp.id.with("x"), if cards { side } else { 0.0 }, motion::MEDIUM);
    let pill = egui::Rect::from_min_size(egui::pos2(outer.left() + 3.0 + x, outer.top() + 2.0), vec2(side, side));
    p.rect_filled(pill, CornerRadius::same(R_TAG), if is_dark() { c_active() } else { c_card() });
    let mut picked = None;
    for (i, (as_cards, text)) in [(false, "Show as rows"), (true, "Show as cards")].into_iter().enumerate() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(outer.left() + 3.0 + i as f32 * side, outer.top() + 2.0),
            vec2(side, side),
        );
        let resp = ui.interact(rect, outer_resp.id.with(i), Sense::click());
        let on = as_cards == cards;
        let t = motion::hover(ui, resp.id, on || resp.hovered());
        let color = motion::mix(c_faint(), c_text(), t);
        let c = rect.center();
        if as_cards {
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let at = c + vec2(dx * 3.25, dy * 3.25);
                p.rect_filled(egui::Rect::from_center_size(at, vec2(5.0, 5.0)), CornerRadius::same(1), color);
            }
        } else {
            for dy in [-4.0, 0.0, 4.0] {
                p.line_segment([c + vec2(-5.5, dy), c + vec2(5.5, dy)], Stroke::new(1.6, color));
            }
        }
        focus_ring(ui, &resp, rect, R_TAG);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, text));
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(text).clicked() && !on {
            picked = Some(as_cards);
        }
    }
    picked
}

/// The switch between rows and cards ([`view_toggle`]), shown only when a list `width` wide has room for two cards.
pub(super) fn view_switch(ui: &mut Ui, cards: bool, width: f32) -> Option<bool> {
    if cards_fit(width) { view_toggle(ui, cards) } else { None }
}

/// The narrowest a card of results gets; a list with no room for two of them side by side stays in rows.
pub(super) const CARD_MIN_W: f32 = 300.0;

/// Between the cards of a list.
pub(super) const CARD_GAP: f32 = 12.0;

/// How tall a card of results is: all the cards of a line are the same, so the line reads as one.
pub(super) const CARD_H: f32 = 178.0;

/// Whether a list `width` wide has room for cards side by side.
pub(super) fn cards_fit(width: f32) -> bool {
    grid_columns(width, CARD_MIN_W, CARD_GAP).0 > 1
}

/// What a result shows, laid out as a row or as a card.
pub(super) struct ResultView<'a> {
    /// Tells the results of a list apart.
    pub key: String,
    pub title: &'a str,
    pub by: Option<&'a str>,
    pub summary: &'a str,
    /// Where it comes from or something to know about it, on pills in front of the last line.
    pub pills: Vec<(String, Color32)>,
    /// The rest of the last line: loaders, categories, figures.
    pub parts: Vec<(String, Color32)>,
    /// The figure that tells results apart (its number and its unit): at the right end of a wide row, at the foot of
    /// a card, on the last line of a narrow row.
    pub figure: Option<(String, &'static str)>,
}

/// The parts of a result its page draws itself.
pub(super) enum Slot {
    /// The picture, this big.
    Icon(f32),
    /// The button and the three dots.
    Actions,
}

/// One result: a row when `width` is none, else a card that wide. `fill` draws the picture and the actions.
pub(super) fn result_item(ui: &mut Ui, width: Option<f32>, view: &ResultView, mut fill: impl FnMut(&mut Ui, Slot)) {
    match width {
        None => result_row(ui, view, &mut fill),
        Some(w) => result_card(ui, w, view, &mut fill),
    }
}

fn result_row(ui: &mut Ui, view: &ResultView, fill: &mut dyn FnMut(&mut Ui, Slot)) {
    let key = view.key.as_str();
    row_frame(ui, |ui| {
        let wide = ui.available_width() >= 560.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            fill(ui, Slot::Icon(ICON));
            let text_w = row_text_width(ui, key);
            ui.vertical(|ui| {
                ui.set_max_width(text_w);
                ui.spacing_mut().item_spacing.y = 2.0;
                name_line(ui, view.title, view.by);
                if !view.summary.is_empty() {
                    summary_line(ui, view.summary);
                }
                let mut parts = Vec::new();
                if let Some((figure, unit)) = view.figure.as_ref().filter(|_| !wide) {
                    parts.push((format!("{figure} {unit}"), c_faint()));
                }
                parts.extend(view.parts.iter().cloned());
                pill_meta_line(ui, &view.pills, &parts);
            });
            row_actions(ui, key, |ui| {
                action_cell(ui, key, |ui| fill(ui, Slot::Actions));
                if let Some((figure, unit)) = view.figure.as_ref().filter(|_| wide) {
                    ui.add_space(8.0);
                    stat_column(ui, figure, unit);
                }
            });
        });
    });
}

/// A result as a card: the picture and the name with who made it, two lines about it, the tags, and at the foot the
/// figure with the actions at the right. It rises a little under the pointer.
fn result_card(ui: &mut Ui, width: f32, view: &ResultView, fill: &mut dyn FnMut(&mut Ui, Slot)) {
    let id = egui::Id::unique(("result-card", view.key.as_str()));
    ui.allocate_ui_with_layout(vec2(width, CARD_H), Layout::top_down(Align::Min), |ui| {
        ui.set_width(width);
        hover_card(ui, id, |ui| {
            // the room inside the card's margins and line, the same for every card
            ui.set_width(width - 34.0);
            ui.set_height(CARD_H - 34.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                fill(ui, Slot::Icon(40.0));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let name = RichText::new(view.title).font(super::fonts::semibold(FS_BODY)).color(c_text());
                    ui.add(egui::Label::new(name).truncate());
                    if let Some(by) = view.by.filter(|b| !b.is_empty()) {
                        let by = RichText::new(format!("by {by}")).size(FS_META).color(c_muted());
                        ui.add(egui::Label::new(by).truncate());
                    }
                });
            });
            ui.add_space(6.0);
            if !view.summary.is_empty() {
                // two lines at most, ended with an ellipsis when there is more
                let mut job = LayoutJob::single_section(view.summary.to_string(), run(c_muted(), FS_BODY - 1.0));
                job.wrap = egui::text::TextWrapping {
                    max_width: ui.available_width(),
                    max_rows: 2,
                    break_anywhere: false,
                    overflow_character: Some('\u{2026}'),
                };
                let galley = ui.painter().layout_job(job);
                ui.add(egui::Label::new(galley));
            }
            // the foot first, from the bottom up: the figure and the actions, then the tags over them
            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                ui.horizontal(|ui| {
                    if let Some((figure, unit)) = &view.figure {
                        let mut job = LayoutJob::default();
                        job.append(
                            figure,
                            0.0,
                            TextFormat {
                                font_id: super::fonts::semibold(FS_BODY),
                                color: c_text(),
                                ..Default::default()
                            },
                        );
                        job.append(unit, 5.0, run(c_faint(), FS_META));
                        ui.add(egui::Label::new(job));
                    }
                    end_aligned(ui, id.with("actions"), |ui| fill(ui, Slot::Actions));
                });
                ui.add_space(6.0);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    pill_meta_line(ui, &view.pills, &view.parts);
                });
            });
        });
    });
}

/// A page's results, each drawn by `each` given the width of its card, or none for a row: cards side by side when
/// `cards` is asked for and two fit, otherwise rows.
pub(super) fn result_list<T>(ui: &mut Ui, cards: bool, items: &[T], mut each: impl FnMut(&mut Ui, &T, Option<f32>)) {
    let (cols, width) = grid_columns(ui.available_width(), CARD_MIN_W, CARD_GAP);
    if !cards || cols < 2 {
        for item in items {
            each(ui, item, None);
        }
        return;
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.add_space(6.0);
    for line in items.chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = CARD_GAP;
            for item in line {
                each(ui, item, Some(width));
            }
        });
        ui.add_space(CARD_GAP);
    }
}

/// A long list as cards side by side in a scroll area of its own, where only the lines in view (and a little around
/// them) are laid out: every card is as tall as the next, so the others take a line's height without being drawn.
pub(super) fn lazy_cards<T>(ui: &mut Ui, id: egui::Id, items: &[T], mut each: impl FnMut(&mut Ui, &T, f32)) {
    ScrollArea::vertical().id_salt(id).auto_shrink(false).show_viewport(ui, |ui, viewport| {
        let (cols, width) = grid_columns(ui.available_width(), CARD_MIN_W, CARD_GAP);
        let line = CARD_H + CARD_GAP;
        let margin = 120.0;
        ui.spacing_mut().item_spacing.y = 0.0;
        for (i, cards) in items.chunks(cols).enumerate() {
            let top = i as f32 * line;
            if top + line < viewport.min.y - margin || top > viewport.max.y + margin {
                ui.add_space(line);
                continue;
            }
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = CARD_GAP;
                for item in cards {
                    each(ui, item, width);
                }
            });
            ui.add_space(CARD_GAP);
        }
    });
}

/// Minecraft versions once each, in the order of Mojang's list (`known`, newest first); a version the list doesn't
/// have goes last.
fn newest_first(mut versions: Vec<String>, known: Vec<&str>) -> Vec<String> {
    versions.sort();
    versions.dedup();
    versions.sort_by_key(|v| known.iter().position(|k| k == v).unwrap_or(usize::MAX));
    versions
}

/// "1 result", "1,284 results".
pub(super) fn results_text(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", thousands(n), if n == 1 { one } else { many })
}

impl App {
    /// Whether lists of results are shown as cards rather than rows (the switch at the end of their count line).
    pub(super) fn results_as_cards(&self) -> bool {
        self.settings.extras.get("resultsView").and_then(|v| v.as_str()) == Some("cards")
    }

    /// Keeps the choice of [`view_switch`] with the settings, when one was made.
    pub(super) fn set_results_view(&mut self, cards: Option<bool>) {
        if let Some(cards) = cards {
            self.settings.extras.insert("resultsView".into(), serde_json::json!(if cards { "cards" } else { "rows" }));
            self.save_settings();
        }
    }

    /// The instance Discover's list is matched to: none while none is chosen, and none for modpacks, which make an
    /// instance of their own.
    fn discover_instance(&self) -> Option<&crate::instances::Instance> {
        if self.discover.kind == Kind::Modpack {
            return None;
        }
        self.discover.target.as_ref().and_then(|f| self.instance(f))
    }

    /// The "For" control: the instance the list is matched to and installed into.
    pub(super) fn target_dropdown(&mut self, ui: &mut Ui, search: &mut bool) {
        let current = self
            .discover
            .target
            .as_ref()
            .and_then(|f| self.instance(f))
            .map(|i| clip(i.name(), 24))
            .unwrap_or_else(|| "Any instance".into());
        let instances = &self.instances;
        let target = &mut self.discover.target;
        dropdown(ui, "target", "For", &current, target.is_some(), false, |ui| {
            if ui.selectable_label(target.is_none(), "Any instance").clicked() {
                *target = None;
                *search = true;
            }
            ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for inst in instances {
                    let on = target.as_ref() == Some(&inst.folder);
                    if ui.selectable_label(on, instance_choice(inst)).clicked() {
                        *target = Some(inst.folder.clone());
                        *search = true;
                    }
                }
            });
        });
    }

    /// The Minecraft version filter, for while no instance is chosen: the versions the instances run first, then
    /// every release.
    pub(super) fn version_dropdown(&mut self, ui: &mut Ui, search: &mut bool) {
        let manifest = self.manifest.clone();
        let instances = &self.instances;
        let filter = &mut self.discover.mc_filter;
        let value = filter.clone().unwrap_or_else(|| "Any".into());
        dropdown(ui, "mc-version", "Minecraft", &value, filter.is_some(), false, |ui| {
            if ui.selectable_label(filter.is_none(), "Any version").clicked() {
                *filter = None;
                *search = true;
            }
            ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                let mut pick = |ui: &mut Ui, v: &str| {
                    if ui.selectable_label(filter.as_deref() == Some(v), v).clicked() {
                        *filter = Some(v.to_string());
                        *search = true;
                    }
                };
                // worked out only while the menu is open
                let mine = newest_first(
                    instances.iter().map(|i| i.mc_version().to_string()).collect(),
                    manifest.as_ref().map(|m| m.versions.iter().map(|v| v.id.as_str()).collect()).unwrap_or_default(),
                );
                if !mine.is_empty() {
                    small_faint(ui, "In your instances");
                    for v in &mine {
                        pick(ui, v);
                    }
                    ui.separator();
                }
                match &manifest {
                    Some(m) => {
                        small_faint(ui, "Every release");
                        for v in m.versions.iter().filter(|v| v.kind == "release") {
                            pick(ui, &v.id);
                        }
                    }
                    None => small_faint(ui, "Mojang's list of versions is on its way\u{2026}"),
                }
            });
        });
    }

    /// The mod loader filter, for mods while no instance is chosen.
    pub(super) fn loader_dropdown(&mut self, ui: &mut Ui, search: &mut bool) {
        let filter = &mut self.discover.loader_filter;
        let value =
            filter.as_deref().and_then(|f| LOADERS.iter().find(|(n, _)| *n == f)).map_or("Any", |(_, label)| *label);
        dropdown(ui, "loader", "Loader", value, filter.is_some(), false, |ui| {
            if ui.selectable_label(filter.is_none(), "Any loader").clicked() {
                *filter = None;
                *search = true;
            }
            for (name, label) in LOADERS {
                if ui.selectable_label(filter.as_deref() == Some(name), label).clicked() {
                    *filter = Some(name.to_string());
                    *search = true;
                }
            }
        });
    }

    /// Modrinth's categories for the kind shown, ticked in a menu that stays open; a result has to be in every one.
    fn category_dropdown(&mut self, ui: &mut Ui, search: &mut bool) {
        let kind = self.discover.kind;
        let both = self.discover.both;
        let chosen = &mut self.discover.categories;
        let value = match chosen.as_slice() {
            [] => "Any".to_string(),
            [one] => category_label(kind, one).unwrap_or(one).to_string(),
            many => format!("{} chosen", many.len()),
        };
        dropdown(ui, "category", "Category", &value, !chosen.is_empty(), true, |ui| {
            ui.set_min_width(220.0);
            if both {
                ui.add(
                    egui::Label::new(
                        RichText::new("Only Modrinth has these: while one is ticked, the other sites aren't searched.")
                            .size(FS_META)
                            .color(c_faint()),
                    )
                    .wrap(),
                );
                ui.add_space(4.0);
            }
            ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                for (i, (group, list)) in categories_of(kind).iter().enumerate() {
                    if i > 0 {
                        ui.add_space(4.0);
                    }
                    small_faint(ui, *group);
                    for (name, label) in list.iter() {
                        let mut on = chosen.iter().any(|c| c == name);
                        if ui.checkbox(&mut on, *label).changed() {
                            if on {
                                chosen.push(name.to_string());
                            } else {
                                chosen.retain(|c| c != name);
                            }
                            *search = true;
                        }
                    }
                }
            });
        });
    }

    /// Where the mods have to run: anywhere, in the player's game alone or on a server alone.
    fn side_dropdown(&mut self, ui: &mut Ui, search: &mut bool) {
        let side = &mut self.discover.side;
        let both = self.discover.both;
        dropdown(ui, "side", "Side", side_label(*side), *side != Side::Any, false, |ui| {
            ui.set_min_width(220.0);
            for (s, hint) in [
                (Side::Any, "Every mod, wherever it runs"),
                (Side::Client, "Mods for the player's game that a server can do without"),
                (Side::Server, "Mods for a server that the players' games can do without"),
            ] {
                if ui.selectable_label(*side == s, side_label(s)).on_hover_text(hint).clicked() {
                    *side = s;
                    *search = true;
                }
            }
            if both {
                ui.add_space(4.0);
                small_faint(ui, "Only Modrinth knows this; the other sites aren't searched while it is set.");
            }
        });
    }

    /// The filters of Discover's search bar, after the field, the instance and the order.
    fn discover_filter_controls(&mut self, ui: &mut Ui, search: &mut bool) {
        let kind = self.discover.kind;
        // an instance chosen says which version and loader; the filters for them are for when none is
        if self.discover_instance().is_none() {
            self.version_dropdown(ui, search);
            if kind == Kind::Mod {
                self.loader_dropdown(ui, search);
            }
        }
        self.category_dropdown(ui, search);
        if kind == Kind::Mod {
            self.side_dropdown(ui, search);
        }
    }

    /// The order of Discover's list.
    fn discover_sort_dropdown(&mut self, ui: &mut Ui, search: &mut bool) {
        let sort = &mut self.discover.sort;
        let label = |s: Sort| match s {
            Sort::Relevance => "Most relevant",
            Sort::Downloads => "Most downloaded",
            Sort::Updated => "Recently updated",
        };
        dropdown(ui, "sort", "Sort", label(*sort), false, false, |ui| {
            for s in [Sort::Relevance, Sort::Downloads, Sort::Updated] {
                if ui.selectable_value(sort, s, label(s)).clicked() {
                    *search = true;
                }
            }
        });
    }

    /// The chips of Discover's filters that are on, each with what taking it off does.
    fn discover_chips(&self) -> Vec<(String, ChipOf)> {
        let d = &self.discover;
        let mut chips = Vec::new();
        if self.discover_instance().is_none() {
            if let Some(v) = &d.mc_filter {
                chips.push((format!("Minecraft {v}"), ChipOf::Version));
            }
            if let Some(l) = d.loader_filter.as_deref().filter(|_| d.kind == Kind::Mod) {
                let label = LOADERS.iter().find(|(n, _)| *n == l).map_or(l, |(_, label)| *label);
                chips.push((label.to_string(), ChipOf::Loader));
            }
        }
        for c in &d.categories {
            chips.push((category_label(d.kind, c).unwrap_or(c).to_string(), ChipOf::Category(c.clone())));
        }
        if d.kind == Kind::Mod && d.side != Side::Any {
            chips.push((side_label(d.side).to_string(), ChipOf::Side));
        }
        chips
    }

    /// Whether a filter of Discover's narrows the list (the instance chosen is not counted).
    pub(super) fn discover_filtered(&self) -> bool {
        !self.discover_chips().is_empty()
    }

    /// Takes every filter of Discover's off: the categories, the version, the loader and where mods run.
    pub(super) fn discover_clear_filters(&mut self) {
        let d = &mut self.discover;
        d.categories.clear();
        d.mc_filter = None;
        d.loader_filter = None;
        d.side = Side::Any;
    }

    /// The chip row under Discover's search bar. Returns true when a filter was taken off.
    fn discover_chip_row(&mut self, ui: &mut Ui) -> bool {
        let chips = self.discover_chips();
        let labels: Vec<String> = chips.iter().map(|(l, _)| l.clone()).collect();
        let note = (self.discover.both && self.discover.modrinth_only())
            .then_some("Only Modrinth can filter by these, so the other sites are left out.");
        match chip_row(ui, &labels, note) {
            Some(ChipAct::Remove(i)) => {
                let d = &mut self.discover;
                match &chips[i].1 {
                    ChipOf::Version => d.mc_filter = None,
                    ChipOf::Loader => d.loader_filter = None,
                    ChipOf::Category(c) => d.categories.retain(|x| x != c),
                    ChipOf::Side => d.side = Side::Any,
                }
                true
            }
            Some(ChipAct::Clear) => {
                self.discover_clear_filters();
                true
            }
            None => false,
        }
    }
}

/// Which of Discover's filters a chip shows.
enum ChipOf {
    Version,
    Loader,
    Category(String),
    Side,
}

impl DiscoverState {
    /// Whether a filter only Modrinth has is on (a category, where mods run), so the other sites are left out of a
    /// search on every site: their results would not match it.
    pub fn modrinth_only(&self) -> bool {
        !self.categories.is_empty() || (self.kind == Kind::Mod && self.side != Side::Any)
    }

    /// What is narrowed to besides the version and the loader, for a Modrinth search of the kind shown.
    fn narrow(&self) -> Narrow {
        Narrow { categories: self.categories.clone(), side: if self.kind == Kind::Mod { self.side } else { Side::Any } }
    }

    /// A new kind keeps the filters that still mean something for it.
    pub(super) fn kind_changed(&mut self) {
        let kind = self.kind;
        self.categories.retain(|c| category_label(kind, c).is_some());
        if kind != Kind::Mod {
            self.side = Side::Any;
            self.loader_filter = None;
        }
    }
}

impl App {
    /// Start a search. `reset` begins again from the first result; otherwise the next page is added.
    pub(super) fn discover_search(&mut self, reset: bool) {
        // what the target instance runs: its Minecraft version, and every loader whose mods it can use
        // without one, the version and loader asked for in the filters
        let kind = self.discover.kind;
        let (mc, loaders) = match self.discover_instance() {
            Some(inst) => {
                let loaders = if kind == Kind::Mod {
                    crate::mods::InstanceTarget::of(inst, &crate::mods::RemoteConfig::load(&self.dirs))
                        .accepted_loaders()
                } else {
                    Vec::new()
                };
                (Some(inst.mc_version().to_string()), loaders)
            }
            None => {
                let loader = self.discover.loader_filter.as_deref().filter(|_| kind == Kind::Mod);
                (self.discover.mc_filter.clone(), loader.map(loaders_for).unwrap_or_default())
            }
        };
        let narrow = self.discover.narrow();
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
            let r = modrinth::search_narrowed(&query, kind, mc.as_deref(), &loaders, &narrow, sort, offset, PAGE)
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
                    self.discover.kind_changed();
                    search = true;
                }
            }
        });
        ui.add_space(14.0);
        let page_w = ui.available_width();
        let hint = format!("Search {}", self.discover.kind.label().to_lowercase());
        let mut st = (&mut *self, &mut search);
        search_bar(
            ui,
            "discover",
            &mut st,
            |ui, (this, search), width| {
                let mut q = std::mem::take(&mut this.discover.query);
                let mut edited = this.discover.edited.take();
                if live_search_w(ui, &mut q, &hint, &mut edited, width) {
                    **search = true;
                }
                this.discover.query = q;
                this.discover.edited = edited;
            },
            |ui, (this, search)| {
                if this.discover.kind != Kind::Modpack {
                    this.target_dropdown(ui, search);
                }
                this.discover_sort_dropdown(ui, search);
            },
            |ui, (this, search)| this.discover_filter_controls(ui, search),
        );
        if self.discover_chip_row(ui) {
            search = true;
        }
        // under the filters: what the list is limited to, and how many results there are
        let kind = self.discover.kind;
        let limited_to = self.discover_instance().map(|inst| {
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
        let count = (total > 0).then(|| results_text(total, "result", "results"));
        let mut show_everything = false;
        let (cards, mut flip) = (self.results_as_cards(), None);
        if kind == Kind::Modpack || limited_to.is_some() || count.is_some() {
            context_line_with(
                ui,
                count,
                limited_to.is_some(),
                |ui| {
                    if kind == Kind::Modpack {
                        self.other_modpack_sources(ui);
                    } else if let Some(text) = limited_to {
                        small_muted(ui, text);
                        if ghost(ui, "Show everything").clicked() {
                            show_everything = true;
                        }
                    }
                },
                |ui| flip = view_switch(ui, cards, page_w),
            );
        }
        self.set_results_view(flip);
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
            result_skeleton(ui, 6);
            return;
        }
        if self.discover.hits.is_empty() && self.discover.error.is_none() {
            self.discover_nothing(ui, "Modrinth");
            return;
        }

        self.discover_refresh_installed();
        let installed = self.discover.installed.take();
        let hits = std::mem::take(&mut self.discover.hits);
        let target = self.discover.target.clone();
        let cards = self.results_as_cards();
        let mut install: Option<(Hit, Option<String>)> = None;
        let mut install_data: Option<(Hit, String, Vec<String>)> = None;
        let mut more = false;
        ScrollArea::vertical().id_salt(("discover-list", self.discover.list_gen)).auto_shrink(false).show(ui, |ui| {
            result_list(ui, cards, &hits, |ui, hit, width| {
                let is_installed = kind != Kind::Modpack
                    && kind != Kind::DataPack
                    && installed.as_ref().is_some_and(|(_, _, ids)| ids.contains(&hit.project_id));
                let busy = self.discover.installing.contains(&hit.project_id)
                    && self.tasks.iter().any(|t| t.label == format!("Installing {}", hit.title));
                let tags = hit.categories.iter().map(String::as_str);
                let view = ResultView {
                    key: format!("mr:{}", hit.project_id),
                    title: &hit.title,
                    by: Some(&hit.author),
                    summary: &hit.description,
                    pills: Vec::new(),
                    parts: if kind == Kind::DataPack { data_pack_tags(tags, 3) } else { tag_parts(tags, 3) },
                    figure: Some((human_count(hit.downloads), "downloads")),
                };
                result_item(ui, width, &view, |ui, slot| {
                    if let Slot::Icon(size) = slot {
                        self.result_icon(ui, hit.icon_url.as_deref(), &hit.title, size);
                        return;
                    }
                    if busy {
                        spinner(ui);
                        return;
                    }
                    if kind == Kind::DataPack {
                        let have = |inst: &crate::instances::Instance| worlds_with(inst, Some(&hit.project_id), None);
                        if let Some((folder, worlds)) = data_pack_button(ui, &self.instances, target.as_deref(), &have)
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
            });
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

    /// What Discover says when a search found nothing on `sites`: with filters on, the way to take them off; with
    /// words, the way to clear them.
    pub(super) fn discover_nothing(&mut self, ui: &mut Ui, sites: &str) {
        let query = self.discover.query.trim().to_string();
        let filtered = self.discover_filtered();
        let text = match (query.is_empty(), filtered) {
            (true, false) => "Nothing to show. Try another kind, or pick a different instance.".to_string(),
            (true, true) => format!("Nothing on {sites} matches these filters."),
            (false, false) => format!("No results for \u{201C}{query}\u{201D} on {sites}."),
            (false, true) => format!("No results for \u{201C}{query}\u{201D} on {sites} with these filters."),
        };
        let action = if filtered { Some("Clear filters") } else { (!query.is_empty()).then_some("Clear search") };
        if empty_state(ui, &text, action).is_some_and(|r| r.clicked()) {
            if filtered {
                self.discover_clear_filters();
            } else {
                self.discover.query.clear();
            }
            self.discover_search(true);
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

    #[test]
    fn categories_belong_to_their_kind() {
        assert_eq!(category_label(Kind::Mod, "worldgen"), Some("World generation"));
        assert_eq!(category_label(Kind::DataPack, "worldgen"), Some("World generation"));
        assert_eq!(category_label(Kind::Shader, "worldgen"), None);
        assert_eq!(category_label(Kind::ResourcePack, "16x"), Some("16x"));
        // every name is listed once for its kind
        for kind in Kind::ALL {
            let mut names: Vec<&str> =
                categories_of(kind).iter().flat_map(|(_, l)| l.iter().map(|(n, _)| *n)).collect();
            let n = names.len();
            names.sort();
            names.dedup();
            assert_eq!(names.len(), n, "{kind:?}");
        }
    }

    #[test]
    fn another_kind_keeps_the_filters_that_still_apply() {
        let mut d = DiscoverState::default();
        d.categories = vec!["optimization".into(), "magic".into(), "storage".into()];
        d.side = Side::Client;
        d.loader_filter = Some("fabric".into());
        d.mc_filter = Some("1.20.1".into());
        assert!(d.modrinth_only());
        d.kind = Kind::Modpack;
        d.kind_changed();
        // modpacks have optimization and magic but no storage; where mods run and the loader are for mods
        assert_eq!(d.categories, ["optimization", "magic"]);
        assert_eq!(d.side, Side::Any);
        assert_eq!(d.loader_filter, None);
        assert_eq!(d.mc_filter.as_deref(), Some("1.20.1"));
        d.kind = Kind::Shader;
        d.kind_changed();
        assert!(d.categories.is_empty());
        // with nothing only Modrinth has, every site is searched
        assert!(!d.modrinth_only());
        assert_eq!(d.narrow(), Narrow::default());
    }

    #[test]
    fn a_quilt_filter_finds_fabric_mods_too() {
        assert_eq!(loaders_for("quilt"), ["quilt", "fabric"]);
        assert_eq!(loaders_for("neoforge"), ["neoforge"]);
        assert_eq!(cf_loaders_for("quilt"), [4, 5]);
        assert_eq!(cf_loaders_for("forge"), [1]);
        assert!(cf_loaders_for("nothing").is_empty());
    }

    #[test]
    fn long_values_are_clipped() {
        assert_eq!(clip("Fabric Survival", 6), "Fabric\u{2026}");
        assert_eq!(clip("Short", 6), "Short");
    }

    #[test]
    fn the_instances_versions_come_newest_first() {
        let mine = vec!["1.9".to_string(), "1.21.1".into(), "1.20.1".into(), "1.21.1".into(), "rd-132211".into()];
        let known = vec!["1.21.4", "1.21.1", "1.20.1", "1.9"];
        assert_eq!(newest_first(mine.clone(), known), ["1.21.1", "1.20.1", "1.9", "rd-132211"]);
        // before Mojang's list has arrived: once each
        assert_eq!(newest_first(mine, Vec::new()).len(), 4);
    }

    #[test]
    fn one_result_is_not_results() {
        assert_eq!(results_text(1, "result", "results"), "1 result");
        assert_eq!(results_text(1284, "pack", "packs"), "1,284 packs");
        assert_eq!(results_text(0, "pack", "packs"), "0 packs");
    }
}
