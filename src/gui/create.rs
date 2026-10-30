//! Create: pick a Minecraft version and a loader, then download it all. Or pick Horizon, the Rust rewrite, which
//! plays the version it is made for and needs no loader.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Color32, CornerRadius, RichText, ScrollArea, Stroke, Ui, vec2};

use super::covers::{CoverSpec, CoverTheme};
use super::edition::Edition;
use super::icons::Icon;
use super::motion;
use super::theme::*;
use super::widgets::*;
use super::{App, Msg, Page};
use crate::horizon;
use crate::install::ManifestVersion;
use crate::instances::{self, LauncherBlock};
use crate::loaders::{self, Loader};
use crate::mods::performance;

pub struct CreateState {
    /// The step of the form that is open: the version, the loader, or the rest.
    pub step: Step,
    /// The game the instance plays: Minecraft: Java Edition, or Horizon.
    pub edition: Edition,
    pub name: String,
    pub mc: String,
    /// Which kinds of Minecraft version the grid shows.
    pub kinds: KindFilter,
    pub filter: String,
    pub loader: Loader,
    pub loader_versions: Vec<String>,
    /// Empty means "the recommended one" (see [`recommended_loader`]).
    pub loader_version: String,
    pub loading: bool,
    requested: Option<(Loader, String)>,
    pub error: Option<String>,
    /// Add Sodium and the other performance mods once the instance is made.
    pub performance: bool,
    /// The sidebar label of the instance being made, so Create waits for that job (and not for any other whose
    /// label happens to start the same way, such as "Creating a support bundle").
    task_label: Option<String>,
    /// What "More options" holds: put into the instance right after it is made.
    extras: Extras,
    /// A preview of the picture in `extras`, read when it was chosen.
    picture_preview: Option<egui::TextureHandle>,
    /// The file chooser for the picture is open.
    picking: bool,
    /// Why the chosen picture can't be used, until another is chosen.
    picture_error: Option<String>,
    /// Answers from the file chooser, which runs on a thread of its own: the file, and the picture read from it.
    picked_tx: Sender<Option<(PathBuf, Option<egui::ColorImage>)>>,
    picked_rx: Receiver<Option<(PathBuf, Option<egui::ColorImage>)>>,
}

/// The panel beside the steps on a wide page: the instance as it will be, and the button that goes on.
const SIDE_W: f32 = 296.0;
/// The room between the steps and that panel.
const SIDE_GAP: f32 = 22.0;
/// The narrowest the steps get with the panel beside them; on a narrower page the panel becomes a bar under them.
const STEPS_MIN_BESIDE: f32 = 440.0;
/// The smallest a version's tile gets before the grid has one column fewer.
const TILE_MIN_W: f32 = 104.0;
/// How tall a version's tile is.
const TILE_H: f32 = 58.0;
/// About how tall the summary panel is without its cover: the cover is as tall as what is left of the page.
const PANEL_BESIDES_COVER: f32 = 330.0;

/// The three steps of making an instance. They can be gone back to at any time, and forward to once a version is
/// picked. Horizon plays the version it is made for and has no loader: its steps are the first and the last.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Version,
    Loader,
    Configure,
}

impl Step {
    pub const ALL: [Step; 3] = [Step::Version, Step::Loader, Step::Configure];

    fn label(self) -> &'static str {
        match self {
            Step::Version => "Choose version",
            Step::Loader => "Choose loader",
            Step::Configure => "Configure",
        }
    }

    fn index(self) -> usize {
        match self {
            Step::Version => 0,
            Step::Loader => 1,
            Step::Configure => 2,
        }
    }

    /// Whether the step is left out for `edition`: Horizon has no loader to choose.
    fn skipped(self, edition: Edition) -> bool {
        edition == Edition::Horizon && self == Step::Loader
    }

    /// The step after this one for `edition`, if there is one.
    fn next(self, edition: Edition) -> Option<Step> {
        let next = Step::ALL.get(self.index() + 1).copied()?;
        if next.skipped(edition) { next.next(edition) } else { Some(next) }
    }

    /// The step before this one for `edition`, if there is one.
    fn back(self, edition: Edition) -> Option<Step> {
        let back = self.index().checked_sub(1).map(|i| Step::ALL[i])?;
        if back.skipped(edition) { back.back(edition) } else { Some(back) }
    }

    /// Whether the step can be opened: the first always, the others once a Minecraft version is picked (the loaders
    /// and their versions depend on it). Horizon's version is its own, so its last step can be opened at once.
    fn reachable(self, mc: &str, edition: Edition) -> bool {
        match edition {
            Edition::Java => self == Step::Version || !mc.is_empty(),
            Edition::Horizon => !self.skipped(edition),
        }
    }
}

/// The folder a new instance called `name` gets: its name as a folder name, with a number after it when an instance
/// already has that folder (as the instance is made, see `paths::claim_unique_folder`).
fn preview_folder(name: &str, taken: &[&str]) -> String {
    let base = crate::paths::folder_name(name);
    (1u32..)
        .map(|n| if n == 1 { base.clone() } else { format!("{base}{n}") })
        .find(|f| !taken.iter().any(|t| t == f))
        .unwrap_or(base)
}

/// The kinds of Minecraft version the manifest lists, as the version picker sorts them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VersionKind {
    Release,
    Snapshot,
    /// The old betas and alphas from before 1.0.
    Old,
}

impl VersionKind {
    pub const ALL: [VersionKind; 3] = [VersionKind::Release, VersionKind::Snapshot, VersionKind::Old];

    /// The kind of a manifest entry's `type`. Anything new Mojang adds is counted with the snapshots: it is not a
    /// release.
    pub fn of(kind: &str) -> VersionKind {
        match kind {
            "release" => VersionKind::Release,
            "old_beta" | "old_alpha" => VersionKind::Old,
            _ => VersionKind::Snapshot,
        }
    }

    fn label(self) -> &'static str {
        match self {
            VersionKind::Release => "Releases",
            VersionKind::Snapshot => "Snapshots",
            VersionKind::Old => "Old versions",
        }
    }

    fn index(self) -> usize {
        match self {
            VersionKind::Release => 0,
            VersionKind::Snapshot => 1,
            VersionKind::Old => 2,
        }
    }
}

/// Which kinds of version the picker shows: releases only until more are switched on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KindFilter {
    pub release: bool,
    pub snapshot: bool,
    pub old: bool,
}

impl Default for KindFilter {
    fn default() -> Self {
        KindFilter { release: true, snapshot: false, old: false }
    }
}

impl KindFilter {
    pub fn shows(self, kind: VersionKind) -> bool {
        match kind {
            VersionKind::Release => self.release,
            VersionKind::Snapshot => self.snapshot,
            VersionKind::Old => self.old,
        }
    }

    pub fn toggle(&mut self, kind: VersionKind) {
        match kind {
            VersionKind::Release => self.release = !self.release,
            VersionKind::Snapshot => self.snapshot = !self.snapshot,
            VersionKind::Old => self.old = !self.old,
        }
    }

    fn none(self) -> bool {
        !self.release && !self.snapshot && !self.old
    }
}

/// Whether a version's id has the search text in it, in any case. Empty text matches everything.
fn version_matches(id: &str, query: &str) -> bool {
    let q = query.trim();
    q.is_empty() || id.to_lowercase().contains(&q.to_lowercase())
}

/// How many versions of each kind match the search text, in the order of [`VersionKind::ALL`]: the numbers on the
/// filter chips, so it is seen what switching one on would add.
pub fn kind_counts(versions: &[ManifestVersion], query: &str) -> [usize; 3] {
    let mut counts = [0; 3];
    for v in versions.iter().filter(|v| version_matches(&v.id, query)) {
        counts[VersionKind::of(&v.kind).index()] += 1;
    }
    counts
}

/// The versions the grid shows: of a kind that is switched on and matching the search text, in the manifest's order
/// (newest first).
pub fn shown_versions<'a>(versions: &'a [ManifestVersion], kinds: KindFilter, query: &str) -> Vec<&'a ManifestVersion> {
    versions.iter().filter(|v| kinds.shows(VersionKind::of(&v.kind)) && version_matches(&v.id, query)).collect()
}

/// Whether a loader version is a test build (a beta, an alpha, a pre-release or a release candidate) by its name.
fn is_test_build(version: &str) -> bool {
    version
        .to_lowercase()
        .split(['-', '+', '.', '_'])
        .any(|part| ["beta", "alpha", "pre", "rc", "snapshot"].iter().any(|t| part.starts_with(t)))
}

/// The loader version used unless another is chosen: the newest that isn't a test build, or the newest when all of
/// them are. `list` is newest first.
pub fn recommended_loader(list: &[String]) -> Option<&str> {
    list.iter().find(|v| !is_test_build(v)).or(list.first()).map(String::as_str)
}

/// How much memory a new instance may use.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Memory {
    /// Whatever the launcher's Settings say, now and after a change there.
    #[default]
    Follow,
    /// Worked out from the mods and the computer each time the game starts.
    Automatic,
    /// A fixed amount, `Extras::memory_mb`.
    Custom,
}

/// The options under "More options": put into the new instance right after it is made, the way its settings
/// would put them there afterwards.
#[derive(Clone, Debug, Default, PartialEq)]
struct Extras {
    /// A few words about the instance, shown on its page.
    description: String,
    memory: Memory,
    memory_mb: u32,
    /// A picture file, made into the instance's `instance.png`.
    picture: Option<PathBuf>,
    /// The kind of landscape on the instance's cover.
    cover: CoverTheme,
}

impl Extras {
    /// Whether anything has to be written into the instance's file.
    fn changes_file(&self) -> bool {
        !self.description.trim().is_empty() || self.memory != Memory::Follow || self.cover != CoverTheme::Auto
    }

    /// Write the description and the memory into an instance's launcher block, as its settings page would.
    fn apply(&self, l: &mut LauncherBlock) {
        let description = self.description.trim();
        if !description.is_empty() {
            l.description = description.to_string();
        }
        match self.memory {
            Memory::Follow => {}
            Memory::Automatic => {
                l.automatic_memory = Some(true);
                l.maximum_memory = None;
            }
            Memory::Custom => {
                l.automatic_memory = None;
                l.maximum_memory = Some(self.memory_mb);
            }
        }
    }
}

/// "2024-08-08T08:24:24+00:00" as "8 August 2024".
fn release_date(time: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(time).ok().map(|t| t.format("%-d %B %Y").to_string())
}

/// What a version is, for the tooltip of its cell: "Release, 8 August 2024".
fn version_tooltip(v: &ManifestVersion, latest_release: bool, latest_snapshot: bool) -> String {
    let kind = match v.kind.as_str() {
        "release" => "Release",
        "old_beta" => "Old beta",
        "old_alpha" => "Old alpha",
        _ => "Snapshot",
    };
    let mut text = match release_date(&v.release_time) {
        Some(d) => format!("{kind}, {d}"),
        None => kind.to_string(),
    };
    if latest_release {
        text.push_str("\nThe latest release");
    } else if latest_snapshot {
        text.push_str("\nThe latest snapshot");
    }
    text
}

/// One version in the grid: a tile with the version in the monospace face, plain until pointed at (the fill fades
/// in); the chosen one in the brand tint with a brand outline and a check in its corner. The latest release (or
/// snapshot) says so under the version.
fn version_tile(ui: &mut Ui, text: &str, selected: bool, note: Option<&str>, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, TILE_H), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let t = motion::hover(ui, resp.id, resp.hovered());
        let s = motion::toward(ui.ctx(), resp.id.with("on"), selected, motion::FAST);
        let p = ui.painter();
        let bg = motion::mix(motion::mix(c_field(), c_field_hover(), t), c_accent_soft(), s);
        let line = motion::mix(motion::mix(c_border(), c_control().gamma_multiply(0.7), t), c_accent(), s);
        p.rect(rect, CornerRadius::same(R_CONTROL + 2), bg, Stroke::new(1.0 + 0.5 * s, line), egui::StrokeKind::Inside);
        let fg = motion::mix(c_text(), accent_ink(), s);
        let galley = p.layout_no_wrap(text.to_string(), super::fonts::mono(15.0), fg);
        let y = if note.is_some() { rect.center().y - 8.0 } else { rect.center().y };
        p.galley(egui::pos2(rect.center().x - galley.size().x / 2.0, y - galley.size().y / 2.0), galley, fg);
        if let Some(note) = note {
            let g = p.layout_no_wrap(note.to_string(), egui::FontId::proportional(FS_META - 1.0), accent_ink());
            p.galley(egui::pos2(rect.center().x - g.size().x / 2.0, rect.center().y + 6.0), g, accent_ink());
        }
        if s > 0.0 {
            let dot = egui::Rect::from_center_size(rect.left_top() + vec2(13.0, 13.0), vec2(14.0, 14.0));
            motion::gradient_rect(p, dot, 7, c_accent().gamma_multiply(s), c_accent2().gamma_multiply(s));
            super::discover::paint_check(p, dot.center(), 8.0, c_on_accent().gamma_multiply(s));
        }
    }
    focus_ring(ui, &resp, rect, R_CONTROL + 2);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, text));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The brand colour as text: the bright one on the dark look, the deep one on the light look (4.5:1 on white).
fn accent_ink() -> Color32 {
    if is_dark() { c_accent() } else { c_accent_hover() }
}

/// A small label in the brand's soft tint: "Latest release", "Recommended".
fn soft_tag(ui: &mut Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(FS_META), accent_ink());
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 14.0, 22.0), egui::Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(11),
        c_accent_soft(),
        Stroke::new(1.0, c_accent().gamma_multiply(0.35)),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, accent_ink());
}

/// The steps at the top of the page: a numbered circle and the name of each, joined by lines. The open step is in
/// the brand gradient, the ones before it have a check; a step that can be opened is pressed to go to it. Returns the
/// step pressed.
fn stepper(ui: &mut Ui, open: Step, mc: &str, edition: Edition) -> Option<Step> {
    let mut picked = None;
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(vec2(width, 32.0), egui::Sense::hover());
    // a step left out says so in its place, so the numbers stay those of the steps
    let label = |s: Step| if s.skipped(edition) { "Loader not needed" } else { s.label() };
    let labels: Vec<std::sync::Arc<egui::Galley>> = Step::ALL
        .iter()
        .map(|s| ui.painter().layout_no_wrap(label(*s).to_string(), super::fonts::medium(FS_BODY), c_text()))
        .collect();
    // the names are left out when they don't fit, the numbers stay
    let named = labels.iter().map(|g| g.size().x + 40.0).sum::<f32>() + 2.0 * 40.0 <= width;
    let items_w: f32 = labels.iter().map(|g| 28.0 + if named { g.size().x + 10.0 } else { 0.0 }).sum();
    let line_w = ((width - items_w) / 2.0 - 24.0).max(12.0);
    let mut x = row.left();
    for (i, step) in Step::ALL.into_iter().enumerate() {
        let item_w = 28.0 + if named { labels[i].size().x + 10.0 } else { 0.0 };
        let rect = egui::Rect::from_min_size(egui::pos2(x, row.top()), vec2(item_w, 32.0));
        let reachable = step.reachable(mc, edition);
        let resp = ui.interact(rect, ui.scope_id().with(("create-step", i)), egui::Sense::click());
        let skipped = step.skipped(edition);
        let passed = step.index() < open.index();
        let done = passed && !skipped;
        let on = step == open;
        let t = motion::hover(ui, resp.id, reachable && !on && resp.hovered());
        let p = ui.painter();
        let circle = egui::Rect::from_center_size(egui::pos2(x + 14.0, row.center().y), vec2(28.0, 28.0));
        if on {
            motion::brand_rect(p, circle, 14);
            p.text(
                circle.center(),
                egui::Align2::CENTER_CENTER,
                (i + 1).to_string(),
                super::fonts::semibold(FS_BODY),
                c_on_accent(),
            );
        } else if done {
            p.rect(
                circle,
                CornerRadius::same(14),
                c_accent_soft(),
                Stroke::new(1.0, accent_ink().gamma_multiply(0.5 + 0.5 * t)),
                egui::StrokeKind::Inside,
            );
            super::discover::paint_check(p, circle.center(), 12.0, accent_ink());
        } else if skipped {
            // a dash in a quiet ring: nothing to do here
            p.rect(circle, CornerRadius::same(14), c_field(), Stroke::new(1.0, c_border()), egui::StrokeKind::Inside);
            p.hline(circle.center().x - 5.0..=circle.center().x + 5.0, circle.center().y, Stroke::new(1.5, c_faint()));
        } else {
            p.rect(
                circle,
                CornerRadius::same(14),
                c_field(),
                Stroke::new(1.0, motion::mix(c_border(), c_control(), t)),
                egui::StrokeKind::Inside,
            );
            let ink = if reachable { c_muted() } else { c_faint() };
            p.text(
                circle.center(),
                egui::Align2::CENTER_CENTER,
                (i + 1).to_string(),
                super::fonts::medium(FS_BODY),
                ink,
            );
        }
        if named {
            let ink = if on {
                c_text()
            } else if reachable {
                motion::mix(c_muted(), c_text(), t)
            } else {
                c_faint()
            };
            let g = ui.painter().layout_no_wrap(label(step).to_string(), super::fonts::medium(FS_BODY), ink);
            ui.painter().galley(egui::pos2(circle.right() + 10.0, row.center().y - g.size().y / 2.0), g, ink);
        }
        x += item_w;
        if i + 1 < Step::ALL.len() {
            // the line to the next step is in the brand colour once this step is behind
            let color = if passed { accent_ink().gamma_multiply(0.6) } else { c_border() };
            ui.painter().hline(x + 12.0..=x + 12.0 + line_w, row.center().y, Stroke::new(1.5, color));
            x += line_w + 24.0;
        }
        focus_ring(ui, &resp, rect.expand(2.0), 16);
        let name = format!("Step {}: {}", i + 1, label(step));
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, reachable, on, &name));
        if reachable && !on {
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                picked = Some(step);
            }
        }
    }
    picked
}

/// The loaders in the order the list shows them: none first, then the ones most mods are made for.
const LOADER_ORDER: [Loader; 5] = [Loader::Vanilla, Loader::Fabric, Loader::Forge, Loader::NeoForge, Loader::Quilt];

/// A line under a loader's name in the list.
fn loader_blurb(l: Loader) -> &'static str {
    match l {
        Loader::Vanilla => "No mods: the game as it comes",
        Loader::Fabric => "Lightweight and fast",
        Loader::Quilt => "Alternative mod loader, runs most Fabric mods",
        Loader::Forge => "Wide mod support",
        Loader::NeoForge => "Modern fork of Forge",
    }
}

/// One mod loader in the list, chosen the way a radio button is: a ring that fills with a check in the brand
/// gradient, the loader's letter on a tile of its colour, its name and a line about it. The chosen one sits on the
/// brand's soft tint, a little taller with the line under the name, and holds `body` under its name (the loader's
/// version); the others are one compact line each, so the version stays in view in a small window.
fn loader_row(ui: &mut Ui, l: Loader, selected: bool, body: Option<&mut dyn FnMut(&mut Ui)>) -> egui::Response {
    let head_h = if selected { LOADER_ROW_OPEN } else { LOADER_ROW };
    let bg = ui.painter().add(egui::Shape::Noop);
    let (head, resp) = ui.allocate_exact_size(vec2(ui.available_width(), head_h), egui::Sense::click());
    let mut rect = head;
    if let Some(body) = body {
        // the body lines up with the name, inside the row's tint
        let inner = ui.horizontal(|ui| {
            ui.add_space(LOADER_TEXT_X);
            ui.vertical(|ui| {
                ui.set_max_width((head.width() - LOADER_TEXT_X - 14.0).max(120.0));
                ui.spacing_mut().item_spacing.y = 6.0;
                body(ui);
                ui.add_space(6.0);
            });
        });
        rect = rect.union(inner.response.rect);
    }
    if ui.is_rect_visible(rect) {
        let t = motion::hover(ui, resp.id, resp.hovered());
        let s = motion::toward(ui.ctx(), resp.id.with("on"), selected, motion::MEDIUM);
        let p = ui.painter();
        let fill = motion::mix(motion::mix(c_card(), c_hover(), t), c_accent_soft(), s);
        let line =
            motion::mix(motion::mix(c_border(), c_control().gamma_multiply(0.7), t), c_accent().gamma_multiply(0.8), s);
        let pane = egui::epaint::RectShape::new(
            rect,
            CornerRadius::same(R_CARD),
            fill,
            Stroke::new(1.0, line),
            egui::StrokeKind::Inside,
        );
        p.set(bg, pane);
        // the ring at the left, a check in the brand gradient when chosen
        let ring = egui::Rect::from_center_size(egui::pos2(head.left() + 26.0, head.center().y), vec2(20.0, 20.0));
        p.rect_stroke(
            ring,
            CornerRadius::same(10),
            Stroke::new(1.5, c_control().gamma_multiply(1.0 - s)),
            egui::StrokeKind::Inside,
        );
        if s > 0.0 {
            motion::gradient_rect(p, ring, 10, c_accent().gamma_multiply(s), c_accent2().gamma_multiply(s));
            super::discover::paint_check(p, ring.center(), 10.0, c_on_accent().gamma_multiply(s));
        }
        let hue = loader_color(l.label());
        let tile = egui::Rect::from_center_size(egui::pos2(ring.right() + 28.0, head.center().y), vec2(28.0, 28.0));
        p.rect_filled(tile, CornerRadius::same(R_CONTROL), hue.gamma_multiply(0.18));
        let letter = l.label().chars().next().unwrap_or('?').to_string();
        p.text(tile.center(), egui::Align2::CENTER_CENTER, letter, super::fonts::semibold(FS_BODY + 1.0), hue);
        let left = head.left() + LOADER_TEXT_X;
        let name = p.layout_no_wrap(l.label().to_string(), super::fonts::semibold(FS_BODY), c_text());
        let name_w = name.size().x;
        // the line about it: under the name on the chosen row, after it on the others
        let (name_y, blurb_at) = if selected {
            (head.center().y - 17.0, egui::pos2(left, head.center().y + 2.0))
        } else {
            let y = head.center().y - name.size().y / 2.0;
            (y, egui::pos2(left + name_w + 10.0, y + 1.0))
        };
        p.galley(egui::pos2(left, name_y), name, c_text());
        let room = (head.right() - blurb_at.x - 12.0).max(40.0);
        let mut job = egui::text::LayoutJob::single_section(
            loader_blurb(l).to_string(),
            egui::TextFormat::simple(egui::FontId::proportional(FS_META), c_muted()),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        p.galley(blurb_at, p.layout_job(job), c_muted());
    }
    focus_ring(ui, &resp, head, R_CARD);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, l.label()));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The heights of a loader's row (the chosen one's head is taller: its line goes under the name), and where its
/// name starts.
const LOADER_ROW: f32 = 44.0;
const LOADER_ROW_OPEN: f32 = 56.0;
const LOADER_TEXT_X: f32 = 91.0;

/// The width of the label in front of the name field.
const NAME_LABEL_W: f32 = 64.0;

/// The size of the cover's picture on the Configure step, and the size of the gallery of themes that opens from it.
const COVER_PREVIEW: egui::Vec2 = egui::Vec2::new(96.0, 54.0);
const COVER_GALLERY_W: f32 = 408.0;
const COVER_GALLERY_H: f32 = 260.0;

/// A cover theme to pick: a small picture of the landscape it draws, its name under it, the chosen one outlined in
/// the brand colour.
fn theme_tile(ui: &mut Ui, tex: &egui::TextureHandle, label: &str, selected: bool, width: f32) -> egui::Response {
    let pic_h = (width * 0.5).round();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, pic_h + 22.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let t = motion::hover(ui, resp.id, resp.hovered());
        let s = motion::toward(ui.ctx(), resp.id.with("on"), selected, motion::FAST);
        let pic = egui::Rect::from_min_size(rect.min, vec2(width, pic_h));
        let size = tex.size();
        egui::Image::from_texture((tex.id(), vec2(size[0] as f32, size[1] as f32)))
            .uv(super::workbench::cover_uv((size[0], size[1]), pic.size()))
            .corner_radius(R_CONTROL)
            .paint_at(ui, pic);
        let line = motion::mix(motion::mix(c_border(), c_control(), t), c_accent(), s);
        ui.painter().rect_stroke(
            pic,
            CornerRadius::same(R_CONTROL),
            Stroke::new(1.0 + s, line),
            egui::StrokeKind::Inside,
        );
        let ink = motion::mix(motion::mix(c_muted(), c_text(), t), accent_ink(), s);
        let mut job = egui::text::LayoutJob::single_section(
            label.to_string(),
            egui::TextFormat::simple(super::fonts::medium(FS_META), ink),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(width);
        let g = ui.painter().layout_job(job);
        ui.painter().galley(egui::pos2(rect.center().x - g.size().x / 2.0, pic.bottom() + 5.0), g, ink);
    }
    focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// What the finished task says when performance mods were asked for.
fn performance_note(ready: String, installed: &[String], skipped: &[String]) -> String {
    let mut text = ready;
    if !installed.is_empty() {
        text.push_str(&format!(", with {}", installed.join(", ")));
    }
    if !skipped.is_empty() {
        text.push_str(&format!(". Not made for this version yet: {}", skipped.join(", ")));
    }
    text
}

/// The name an instance gets when the name field is left empty.
fn default_name(mc: &str, loader: Loader) -> String {
    match loader {
        Loader::Vanilla => format!("Minecraft {mc}"),
        l => format!("Minecraft {mc} with {}", l.label()),
    }
}

/// The loader version that will be installed: the one chosen, or the recommended one. `None` for plain Minecraft
/// and while the list isn't there.
fn chosen_loader_version(c: &CreateState) -> Option<&str> {
    if c.loader == Loader::Vanilla {
        return None;
    }
    if c.loader_version.is_empty() { recommended_loader(&c.loader_versions) } else { Some(c.loader_version.as_str()) }
}

/// What is about to be made, in a line: "Minecraft 1.21.1 with Fabric 0.16.5".
fn summary(c: &CreateState) -> String {
    if c.mc.is_empty() {
        return String::new();
    }
    if c.loader == Loader::Vanilla {
        return format!("Minecraft {}", c.mc);
    }
    match chosen_loader_version(c) {
        Some(v) => format!("Minecraft {} with {} {v}", c.mc, c.loader.label()),
        None => format!("Minecraft {} with {}", c.mc, c.loader.label()),
    }
}

/// The memory a new instance gets, in a word or two: "Launcher setting", "Automatic", "6 GB".
fn memory_summary(e: &Extras) -> String {
    match e.memory {
        Memory::Follow => "Launcher setting".into(),
        Memory::Automatic => "Automatic".into(),
        Memory::Custom => super::settings::memory_text(e.memory_mb),
    }
}

impl Default for CreateState {
    fn default() -> Self {
        let (picked_tx, picked_rx) = channel();
        CreateState {
            step: Step::Version,
            edition: Edition::Java,
            name: String::new(),
            mc: String::new(),
            kinds: KindFilter::default(),
            filter: String::new(),
            loader: Loader::Vanilla,
            loader_versions: Vec::new(),
            loader_version: String::new(),
            loading: false,
            requested: None,
            error: None,
            performance: false,
            task_label: None,
            extras: Extras::default(),
            picture_preview: None,
            picking: false,
            picture_error: None,
            picked_tx,
            picked_rx,
        }
    }
}

impl CreateState {
    pub fn loader_versions_arrived(&mut self, loader: Loader, mc: &str, result: Result<Vec<String>, String>) {
        if self.requested.as_ref() != Some(&(loader, mc.to_string())) {
            return;
        }
        self.loading = false;
        match result {
            Ok(list) => {
                self.loader_versions = list;
                self.error = None;
            }
            Err(e) => {
                self.loader_versions.clear();
                self.error = Some(e);
            }
        }
    }
}

impl App {
    pub(super) fn request_loader_versions(&mut self) {
        let c = &mut self.create;
        c.loader_version.clear();
        if c.loader == Loader::Vanilla || c.mc.is_empty() {
            c.loader_versions.clear();
            c.requested = None;
            c.loading = false;
            return;
        }
        let key = (c.loader, c.mc.clone());
        if c.requested.as_ref() == Some(&key) {
            return;
        }
        c.requested = Some(key.clone());
        c.loading = true;
        c.loader_versions.clear();
        let (dirs, n) = (self.dirs.clone(), self.notifier());
        self.rt.spawn(async move {
            let r = loaders::versions(&dirs, key.0, &key.1).await.map_err(|e| format!("{e:#}"));
            n.send(Msg::LoaderVersions { loader: key.0, mc: key.1, result: r });
        });
    }

    /// Ask the system's file chooser for the new instance's picture. It runs on a thread of its own (it blocks until
    /// it closes); the file and the picture read from it come back through `picked_rx`.
    fn pick_create_picture(&mut self) {
        if self.create.picking {
            return;
        }
        self.create.picking = true;
        let (tx, ctx) = (self.create.picked_tx.clone(), self.ctx.clone());
        self.rt.spawn_blocking(move || {
            let answer = super::import::pick_file_blocking("Images", &["png", "jpg", "jpeg", "webp"]).map(|path| {
                let image = super::instance_icon::read_square(&path);
                (path, image)
            });
            let _ = tx.send(answer);
            ctx.request_repaint();
        });
    }

    /// Take what the file chooser answered, if it did.
    fn poll_create_picture(&mut self, ctx: &egui::Context) {
        while let Ok(answer) = self.create.picked_rx.try_recv() {
            let c = &mut self.create;
            c.picking = false;
            // nothing chosen: the picture there was (if any) stays
            let Some((path, image)) = answer else { continue };
            match image {
                Some(image) => {
                    c.picture_preview = Some(ctx.load_texture("create-picture", image, egui::TextureOptions::LINEAR));
                    c.extras.picture = Some(path);
                    c.picture_error = None;
                }
                None => {
                    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    c.picture_error =
                        Some(format!("{name} isn't a picture BlockForge can read. Choose a PNG, JPEG or WebP file."));
                }
            }
        }
    }

    pub(super) fn create_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.poll_create_picture(&ctx);
        let mut import = false;
        let subtitle = match self.create.edition {
            Edition::Java => "Pick a Minecraft version and a mod loader, then BlockForge downloads it all.",
            Edition::Horizon => "Horizon plays the version it is made for. BlockForge gets it ready.",
        };
        PageHeader::new("New instance").subtitle(subtitle).show(ui, |ui| {
            // a modpack file or another launcher's instance is made into an instance from the import window
            import = super::discover::ghost_icon(ui, Icon::Download, "Import a modpack or instance")
                .on_hover_text("A modpack file (.mrpack, .zip) or an instance from another launcher")
                .clicked();
        });
        if import {
            self.open_import(None);
        }
        if let Some(step) = stepper(ui, self.create.step, &self.create.mc, self.create.edition) {
            self.create.step = step;
        }
        ui.add_space(18.0);
        let mut changed_version = false;
        let mut act = None;
        let mut enter = false;
        let area = ui.available_rect_before_wrap();
        // a wide page has the summary and the button in a panel beside the steps; a narrower one has them in a bar
        // under them. Either way the button stays in view: only the step scrolls
        let beside = area.width() >= STEPS_MIN_BESIDE + SIDE_GAP + SIDE_W;
        if beside {
            let steps_w = area.width() - SIDE_W - SIDE_GAP;
            let steps_rect = egui::Rect::from_min_size(area.min, vec2(steps_w, area.height()));
            let side_rect =
                egui::Rect::from_min_size(egui::pos2(area.right() - SIDE_W, area.top()), vec2(SIDE_W, area.height()));
            ui.scope_builder(egui::UiBuilder::new().max_rect(steps_rect), |ui| {
                // nothing of the step may reach under the panel
                ui.shrink_clip_rect(steps_rect);
                self.create_step(ui, steps_rect.width(), &mut changed_version, &mut enter);
            });
            ui.scope_builder(egui::UiBuilder::new().max_rect(side_rect), |ui| {
                act = self.create_side_panel(ui, side_rect.height());
            });
            ui.advance_cursor_after_rect(area);
        } else {
            let steps_h = (area.height() - 92.0).max(160.0);
            let steps_rect = egui::Rect::from_min_size(area.min, vec2(area.width(), steps_h));
            ui.scope_builder(egui::UiBuilder::new().max_rect(steps_rect), |ui| {
                self.create_step(ui, steps_rect.width(), &mut changed_version, &mut enter);
            });
            ui.advance_cursor_after_rect(steps_rect);
            ui.add_space(10.0);
            act = self.create_bottom_bar(ui);
        }
        if changed_version {
            self.request_loader_versions();
        }
        if enter && self.create_blocker().is_none() {
            act = Some(Go::Create);
        }
        match act {
            Some(Go::Next) => {
                if let Some(next) = self.create.step.next(self.create.edition) {
                    self.create.step = next;
                }
            }
            Some(Go::Back) => {
                if let Some(back) = self.create.step.back(self.create.edition) {
                    self.create.step = back;
                }
            }
            Some(Go::Create) => self.start_create(),
            None => {}
        }
    }

    /// Whether the instance being made is still on its way.
    fn create_running(&self) -> bool {
        self.create.task_label.as_ref().is_some_and(|l| self.tasks.iter().any(|t| &t.label == l))
    }

    /// What Create waits for, said when the greyed-out button is pointed at; `None` when it can be pressed.
    fn create_blocker(&self) -> Option<String> {
        let c = &self.create;
        if c.edition == Edition::Horizon {
            // its version is its own, and it has no loader to wait for
            return if !horizon::supported() {
                Some(NO_HORIZON_BUILD.to_string())
            } else if self.create_running() {
                Some("The instance is being made. Its progress is in the sidebar".to_string())
            } else {
                None
            };
        }
        if c.mc.is_empty() {
            Some("Pick a Minecraft version first".to_string())
        } else if self.create_running() {
            Some("The instance is being made. Its progress is in the sidebar".to_string())
        } else if c.loader == Loader::Vanilla || !c.loader_versions.is_empty() {
            None
        } else if c.loading {
            Some(format!("Looking up the {} versions", c.loader.label()))
        } else {
            Some(format!("{} isn't available for this Minecraft version", c.loader.label()))
        }
    }

    /// What keeps the open step's Continue greyed out, said beside it; `None` when it can be pressed. The last step's
    /// button makes the instance, so it waits for everything ([`App::create_blocker`]).
    fn step_blocker(&self) -> Option<String> {
        let c = &self.create;
        match c.step {
            Step::Version if c.edition == Edition::Horizon => self.create_blocker().filter(|_| !self.create_running()),
            Step::Version if c.mc.is_empty() => Some("Pick a Minecraft version first".to_string()),
            Step::Version => None,
            Step::Loader => self.create_blocker().filter(|_| !self.create_running()),
            Step::Configure => self.create_blocker(),
        }
    }

    /// The open step, in the room given (`width` points wide).
    fn create_step(&mut self, ui: &mut Ui, width: f32, changed_version: &mut bool, enter: &mut bool) {
        match self.create.step {
            // the grid of versions scrolls by itself, under its title and filters
            Step::Version => self.version_step(ui, width, changed_version),
            Step::Loader => {
                ScrollArea::vertical().id_salt("create-loader").auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_max_width(ui.available_width() - 14.0);
                    self.loader_step(ui, changed_version);
                    ui.add_space(8.0);
                });
            }
            Step::Configure => {
                ScrollArea::vertical().id_salt("create-configure").auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_max_width(ui.available_width() - 14.0);
                    self.configure_step(ui, enter);
                    ui.add_space(8.0);
                });
            }
        }
    }

    /// Step 1: the game first (Minecraft: Java Edition or Horizon), then for Java Edition the Minecraft version, from
    /// a grid of tiles that can be searched and narrowed to the kinds wanted. Horizon has a version of its own: in
    /// place of the grid it says what it is.
    fn version_step(&mut self, ui: &mut Ui, width: f32, changed_version: &mut bool) {
        self.edition_choice(ui, width);
        ui.add_space(14.0);
        if self.create.edition == Edition::Horizon {
            ScrollArea::vertical().id_salt("create-horizon").auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(ui.available_width() - 14.0);
                self.horizon_step(ui);
                ui.add_space(8.0);
            });
            return;
        }
        let manifest = self.manifest.clone();
        let latest_release = manifest.as_ref().and_then(|m| m.latest.get("release").cloned()).unwrap_or_default();
        let latest_snapshot = manifest.as_ref().and_then(|m| m.latest.get("snapshot").cloned()).unwrap_or_default();
        step_title(ui, "Choose a Minecraft version", "Pick the version you want to play.");
        match (&manifest, &self.manifest_error) {
            (Some(m), _) => {
                let counts = kind_counts(&m.versions, &self.create.filter);
                // the kinds to show, each with how many versions it has (that match the search), and the search at
                // the right end of their row, or on a row of its own when they don't fit beside each other
                let search_w = 200.0;
                let texts: Vec<String> =
                    VersionKind::ALL.iter().map(|k| format!("{}  {}", k.label(), counts[k.index()])).collect();
                let chips_w: f32 = texts
                    .iter()
                    .map(|t| {
                        ui.painter().layout_no_wrap(t.clone(), super::fonts::medium(13.0), c_text()).size().x + 26.0
                    })
                    .sum::<f32>()
                    + 12.0;
                let beside = chips_w + 16.0 + search_w <= width;
                let mut f = std::mem::take(&mut self.create.filter);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for (kind, text) in VersionKind::ALL.into_iter().zip(&texts) {
                        let on = self.create.kinds.shows(kind);
                        let resp = chip(ui, text, on);
                        let resp = if kind == VersionKind::Old {
                            resp.on_hover_text("The betas and alphas from before Minecraft 1.0")
                        } else {
                            resp
                        };
                        if resp.clicked() {
                            self.create.kinds.toggle(kind);
                        }
                    }
                    if beside {
                        super::discover::end_aligned(ui, ui.scope_id().with("version-search"), |ui| {
                            super::discover::search_input(ui, &mut f, "Search versions", search_w);
                        });
                    }
                });
                if !beside {
                    ui.add_space(2.0);
                    super::discover::search_input(ui, &mut f, "Search versions", width.min(320.0));
                }
                self.create.filter = f;
                // the newest release is one click away when another version is picked
                if !latest_release.is_empty() && self.create.mc != latest_release {
                    ui.add_space(2.0);
                    if super::discover::text_link(ui, &format!("Use the latest release, {latest_release}"), FS_META)
                        .clicked()
                    {
                        self.create.mc = latest_release.clone();
                        self.create.kinds.release = true;
                        *changed_version = true;
                    }
                }
                ui.add_space(10.0);
                let versions = shown_versions(&m.versions, self.create.kinds, &self.create.filter);
                if versions.is_empty() {
                    let hidden: usize = VersionKind::ALL
                        .iter()
                        .filter(|k| !self.create.kinds.shows(**k))
                        .map(|k| counts[k.index()])
                        .sum();
                    let text = if self.create.kinds.none() {
                        "Switch on releases, snapshots or old versions above to see them.".to_string()
                    } else if hidden > 0 {
                        format!(
                            "Nothing shown matches \u{201C}{}\u{201D}; {hidden} versions of the other kinds do. Switch them on above.",
                            self.create.filter.trim()
                        )
                    } else {
                        format!("No version matches \u{201C}{}\u{201D}.", self.create.filter.trim())
                    };
                    notice(ui, NoticeKind::Info, &text, None);
                    return;
                }
                // the tiles fill the width, and the rest of the height scrolls
                let gap = 8.0;
                let (cols, tile_w) = grid_columns(width - 14.0, TILE_MIN_W, gap);
                ScrollArea::vertical().id_salt("versions").auto_shrink([false, false]).show_rows(
                    ui,
                    TILE_H + gap,
                    versions.len().div_ceil(cols),
                    |ui, range| {
                        for r in range {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = gap;
                                for v in versions.iter().skip(r * cols).take(cols) {
                                    let selected = self.create.mc == v.id;
                                    let is_release = v.id == latest_release;
                                    let is_snapshot = v.id == latest_snapshot && !is_release;
                                    let note = if is_release {
                                        Some("Latest release")
                                    } else if is_snapshot {
                                        Some("Latest snapshot")
                                    } else {
                                        None
                                    };
                                    let resp = version_tile(ui, &v.id, selected, note, tile_w)
                                        .on_hover_text(version_tooltip(v, is_release, is_snapshot));
                                    if resp.clicked() && !selected {
                                        self.create.mc = v.id.clone();
                                        *changed_version = true;
                                    }
                                    // a double click picks it and goes on
                                    if resp.double_clicked() {
                                        self.create.step = Step::Loader;
                                    }
                                }
                            });
                            ui.add_space(gap);
                        }
                    },
                );
            }
            (None, Some(e)) => {
                let text = format!("Couldn't load the version list: {e}");
                if notice(ui, NoticeKind::Error, &text, Some("Try again")).is_some_and(|r| r.clicked()) {
                    self.manifest_error = None;
                    self.refresh_manifest();
                }
            }
            (None, None) => {
                skeleton_rows(ui, 4, 36.0);
            }
        }
    }

    /// The game a new instance plays, as two cards side by side: Mojang's, and Horizon (greyed out where it has no
    /// build for this computer).
    fn edition_choice(&mut self, ui: &mut Ui, width: f32) {
        const GAP: f32 = 10.0;
        let w = ((width - 14.0 - GAP) / 2.0).floor().max(120.0);
        let supported = horizon::supported();
        let mc = self.horizon_minecraft();
        let on = self.create.edition;
        let mut picked = None;
        // shorter words on narrow cards, so nothing is cut
        let narrow = w < EDITION_ROOMY;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            let (title, line) = if narrow {
                ("Java Edition", "Every version, mods")
            } else {
                ("Minecraft: Java Edition", "Every version, with mod loaders")
            };
            let java = edition_card(ui, title, line, false, on == Edition::Java, true, w)
                .on_hover_text("Minecraft: Java Edition, Mojang's game: every version, with mod loaders");
            if java.clicked() {
                picked = Some(Edition::Java);
            }
            // what it is in full when pointed at; on the card, the version it plays
            let (line, hint) = if supported {
                let line = if narrow {
                    format!("Plays Minecraft {mc}")
                } else {
                    format!("Experimental \u{00B7} plays Minecraft {mc}")
                };
                (line, format!("{}, plays Minecraft {mc}", horizon::TAGLINE))
            } else {
                (NO_HORIZON_BUILD.to_string(), NO_HORIZON_BUILD.to_string())
            };
            let hz = edition_card(ui, horizon::NAME, &line, true, on == Edition::Horizon, supported, w);
            let hz = hz.on_hover_text(hint);
            if hz.clicked() && supported {
                picked = Some(Edition::Horizon);
            }
        });
        if let Some(e) = picked.filter(|e| *e != on) {
            self.create.edition = e;
            if self.create.step.skipped(e) {
                self.create.step = Step::Version;
            }
        }
    }

    /// Horizon chosen on the first step: what it is, what plays and what doesn't, what it needs and what BlockForge
    /// does to get it, and who makes it.
    fn horizon_step(&mut self, ui: &mut Ui) {
        let mc = self.horizon_minecraft();
        let build = self.horizon_tag().map(str::to_string);
        let mut link = None;
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new(horizon::NAME).size(FS_FIGURE).semibold());
                super::edition::horizon_tags(ui);
            });
            let about = format!(
                "{}, by the Horizon project. It plays Minecraft {mc}, the version its builds are made for.",
                horizon::TAGLINE
            );
            ui.add(egui::Label::new(RichText::new(about).color(c_muted())).wrap());
            ui.add_space(4.0);
            facts(
                ui,
                &[
                    ("Minecraft", mc.clone()),
                    ("Build", build.unwrap_or_else(|| "Newest release".into())),
                    ("First download", "1.5 GB".into()),
                ],
            );
            ui.add_space(4.0);
            rule(ui);
            ui.add_space(6.0);
            let works = |ui: &mut Ui| {
                ui.label(RichText::new("What plays").semibold());
                for text in HORIZON_PLAYS {
                    point(ui, true, text);
                }
            };
            let doesnt = |ui: &mut Ui| {
                ui.label(RichText::new("What doesn't").semibold());
                for text in HORIZON_DOESNT {
                    point(ui, false, text);
                }
            };
            if ui.available_width() >= 520.0 {
                ui.columns(2, |cols| {
                    cols[0].spacing_mut().item_spacing.y = 6.0;
                    cols[1].spacing_mut().item_spacing.y = 6.0;
                    works(&mut cols[0]);
                    doesnt(&mut cols[1]);
                });
            } else {
                works(ui);
                ui.add_space(6.0);
                doesnt(ui);
            }
            ui.add_space(6.0);
            rule(ui);
            ui.add_space(6.0);
            fact_line(
                ui,
                Icon::User,
                "It needs a Microsoft account that owns Minecraft: Java Edition, as Horizon's own launcher does.",
            );
            let download = format!(
                "BlockForge downloads Horizon's signed release, checks it, and prepares it from BlockForge's own copy \
                 of Minecraft {mc}: about 1.5 GB the first time. Your normal Minecraft and its instances are left as \
                 they are."
            );
            fact_line(ui, Icon::Download, &download);
            ui.add_space(6.0);
            rule(ui);
            ui.add_space(6.0);
            link = super::edition::credit(ui);
        });
        if let Some(url) = link {
            self.open_link(&url);
        }
    }

    /// Step 2: the mod loader as a list to pick one from, and the version of it.
    fn loader_step(&mut self, ui: &mut Ui, changed_version: &mut bool) {
        let mc = self.create.mc.clone();
        step_title(ui, "Choose a mod loader", &format!("Mods need one. Vanilla plays Minecraft {mc} without mods."));
        ui.spacing_mut().item_spacing.y = 8.0;
        for l in LOADER_ORDER {
            let on = l == self.create.loader;
            // the chosen loader's version goes right under its name, so it is in view wherever the loader is
            let mut version = |ui: &mut Ui| self.loader_version(ui, changed_version);
            let body: Option<&mut dyn FnMut(&mut Ui)> = (on && l != Loader::Vanilla).then_some(&mut version);
            if loader_row(ui, l, on, body).clicked() && !on {
                self.create.loader = l;
                *changed_version = true;
            }
        }
    }

    /// The chosen loader's version: a drop-down of them with the recommended one marked (and picked unless another
    /// is), or what keeps the list from coming.
    fn loader_version(&mut self, ui: &mut Ui, changed_version: &mut bool) {
        let mc = self.create.mc.clone();
        let loader = self.create.loader.label();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if self.create.loading {
                spinner_sized(ui, 16.0);
                small_muted(ui, format!("Looking up the {loader} versions"));
                return;
            }
            let Some(recommended) = recommended_loader(&self.create.loader_versions).map(str::to_string) else {
                return;
            };
            super::discover::filter_label(ui, "Version");
            let newest = self.create.loader_versions[0].clone();
            let chosen = chosen_loader_version(&self.create).unwrap_or(&recommended).to_string();
            // what each version is in the list: the one used unless another is picked, the newest, a test build. The
            // chosen one is shown plain: the tag after it says when it is the recommended one
            let describe = |v: &str| {
                if v == recommended {
                    format!("{v}  (recommended)")
                } else if v == newest {
                    format!("{v}  (newest)")
                } else if is_test_build(v) {
                    format!("{v}  (test build)")
                } else {
                    v.to_string()
                }
            };
            // narrower in a small window, so the tag after it stays on its line
            let combo_w = (ui.available_width() - 190.0).clamp(130.0, 200.0);
            egui::ComboBox::from_id_salt("loader-version")
                .selected_text(RichText::new(&chosen).font(super::fonts::mono(12.5)))
                .width(combo_w)
                .show_ui(ui, |ui| {
                    for v in &self.create.loader_versions {
                        let text = RichText::new(describe(v)).font(super::fonts::mono(12.5));
                        if ui.selectable_label(chosen == *v, text).clicked() {
                            // the recommended one is kept as "no choice", so it follows the list
                            self.create.loader_version = if *v == recommended { String::new() } else { v.clone() };
                        }
                    }
                });
            if chosen == recommended {
                soft_tag(ui, "Recommended");
            } else if ghost(ui, "Use the recommended one").clicked() {
                self.create.loader_version.clear();
            }
            let n = self.create.loader_versions.len();
            small_muted(ui, if n == 1 { "1 version".to_string() } else { format!("{n} versions") });
        });
        if !self.create.loading {
            if let Some(e) = self.create.error.clone() {
                let text = format!("Couldn't get the versions: {e}");
                if notice(ui, NoticeKind::Error, &text, Some("Try again")).is_some_and(|r| r.clicked()) {
                    // forget the failed request so it is made again
                    self.create.requested = None;
                    self.create.error = None;
                    *changed_version = true;
                }
            } else if self.create.loader_versions.is_empty() && !mc.is_empty() {
                let text = format!("{loader} isn't available for Minecraft {mc}.");
                notice(ui, NoticeKind::Warn, &text, None);
            }
        }
        if matches!(self.create.loader, Loader::Forge | Loader::NeoForge) {
            small_muted(
                ui,
                "Forge and NeoForge run their own installer, so this takes a little longer the first time.",
            );
        }
    }

    /// Step 3: what matters to how the game runs first (the name, the memory, the performance mods, in one card),
    /// then the look (the cover, with its gallery a click away, and the picture), and a description.
    fn configure_step(&mut self, ui: &mut Ui, enter: &mut bool) {
        step_title(ui, "Configure", "Everything here can be changed later in the instance's settings.");
        ui.spacing_mut().item_spacing.y = 8.0;
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                ui.allocate_ui_with_layout(
                    vec2(NAME_LABEL_W, 34.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_width(NAME_LABEL_W);
                        ui.label(RichText::new("Name").semibold());
                    },
                );
                // the hint is the name the instance gets when the field is left empty
                let hint = self.create_default_name();
                let c = &mut self.create;
                let name_field = ui.add_sized(
                    [ui.available_width().min(420.0), 34.0],
                    egui::TextEdit::singleline(&mut c.name)
                        .hint_text(RichText::new(hint).color(c_faint()))
                        .margin(egui::Margin::symmetric(10, 8))
                        .background_color(c_field()),
                );
                // Enter in the name makes the instance, like the Create button
                *enter = name_field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            });
            rule(ui);
            if self.create.edition == Edition::Horizon {
                // Horizon isn't Java: no performance mods, and its memory budget is set in its settings later
                small_muted(
                    ui,
                    "Horizon runs without Java and takes no mods from here. Its memory and graphics card are in the \
                     instance's settings once it is made.",
                );
                return;
            }
            self.memory_row(ui);
            let key = self.create.loader.modrinth_name();
            if performance::offered_for(key) {
                rule(ui);
                let hint = format!(
                    "{} from Modrinth, for a smoother game. Those not out for this version yet are left out.",
                    performance::describe(key)
                );
                let title = "Add performance mods";
                setting_row(ui, title, &hint, |ui| {
                    toggle_named(ui, &mut self.create.performance, title);
                });
            }
        });
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            self.cover_row(ui);
            rule(ui);
            self.picture_row(ui);
        });
        card(ui, |ui| {
            ui.label(RichText::new("Description").semibold());
            small_muted(ui, "A few words about this instance, shown on its page.");
            ui.add_space(2.0);
            ui.add(
                egui::TextEdit::multiline(&mut self.create.extras.description)
                    .hint_text(RichText::new("What is this instance for?").color(c_faint()))
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .margin(egui::Margin::symmetric(10, 7))
                    .background_color(c_field()),
            );
        });
    }

    /// The cover's landscape, in one row: a small picture of the cover as it will be, its theme, and "Change cover",
    /// which opens the gallery of themes (each drawn for the instance as it will be) beside it.
    fn cover_row(&mut self, ui: &mut Ui) {
        let (name, mc, loader) = self.create_identity();
        let key = self.create_cover_key(&name);
        let chosen = self.create.extras.cover;
        let mut picked = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            let (rect, _) = ui.allocate_exact_size(COVER_PREVIEW, egui::Sense::hover());
            let spec = CoverSpec::new(&key, &mc, loader, chosen);
            let (tw, th) = super::workbench::cover_size(rect.size());
            let tex = self.covers.texture(&self.ctx, &spec, tw, th);
            egui::Image::from_texture((tex.id(), vec2(tw as f32, th as f32)))
                .uv(super::workbench::cover_uv((tw, th), rect.size()))
                .corner_radius(R_CONTROL)
                .paint_at(ui, rect);
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(R_CONTROL),
                Stroke::new(1.0, c_border()),
                egui::StrokeKind::Inside,
            );
            ui.vertical(|ui| {
                ui.set_max_width((ui.available_width() - 150.0).max(120.0));
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new("Cover").semibold());
                small_muted(ui, format!("{} \u{00B7} on Home and in the Library", chosen.label()));
            });
            let id = ui.scope_id().with("cover-button");
            super::discover::end_aligned(ui, id, |ui| {
                let r = secondary(ui, "Change cover");
                // it opens over the form, never over the summary, which shows the cover picked
                egui::Popup::from_toggle_button_response(&r)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .align(egui::RectAlign::TOP_END)
                    .align_alternatives(&[egui::RectAlign::BOTTOM_END, egui::RectAlign::LEFT_END])
                    .gap(6.0)
                    .show(|ui| {
                        ui.set_width(COVER_GALLERY_W);
                        small_muted(ui, "Automatic picks one for the instance. Pick another to see it in the summary.");
                        ui.add_space(6.0);
                        egui::ScrollArea::vertical().max_height(COVER_GALLERY_H).show(ui, |ui| {
                            let gap = 8.0;
                            let (cols, w) = grid_columns(ui.available_width(), 92.0, gap);
                            for row in CoverTheme::ALL.chunks(cols) {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = gap;
                                    for theme in row {
                                        let spec = CoverSpec::new(&key, &mc, loader, *theme);
                                        let tex = self.covers.texture(&self.ctx, &spec, 160, 90);
                                        if theme_tile(ui, &tex, theme.label(), chosen == *theme, w).clicked() {
                                            picked = Some(*theme);
                                        }
                                    }
                                });
                                ui.add_space(4.0);
                            }
                        });
                    });
            });
        });
        if let Some(t) = picked {
            self.create.extras.cover = t;
        }
    }

    /// What the new instance's cover is drawn from: its name (the one it gets when the field is left empty), its
    /// Minecraft version and its loader.
    fn create_identity(&self) -> (String, String, Option<&'static str>) {
        let c = &self.create;
        if c.edition == Edition::Horizon {
            return (self.create_name(), self.horizon_minecraft(), Some(horizon::NAME));
        }
        let loader = (c.loader != Loader::Vanilla).then(|| c.loader.label());
        (self.create_name(), c.mc.clone(), loader)
    }

    /// What the new instance's cover is drawn from: the folder it will get, as `workbench::cover_spec` does for an
    /// instance that exists, so the preview is the cover it ends up with.
    fn create_cover_key(&self, name: &str) -> String {
        let taken: Vec<&str> = self.instances.iter().map(|i| i.folder.as_str()).collect();
        preview_folder(name, &taken)
    }

    /// The name the instance will get.
    fn create_name(&self) -> String {
        let c = &self.create;
        if !c.name.trim().is_empty() { c.name.trim().to_string() } else { self.create_default_name() }
    }

    /// The name the instance gets when the name field is left empty.
    fn create_default_name(&self) -> String {
        let c = &self.create;
        if c.edition == Edition::Horizon {
            horizon::NAME.to_string()
        } else if c.mc.is_empty() {
            "My instance".to_string()
        } else {
            default_name(&c.mc, c.loader)
        }
    }

    /// The picture of the new instance: the one chosen, or the cube it gets otherwise, with the buttons to change it.
    fn picture_row(&mut self, ui: &mut Ui) {
        let mut choose = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            let (rect, _) = ui.allocate_exact_size(vec2(48.0, 48.0), egui::Sense::hover());
            self.paint_create_picture(ui, rect);
            ui.vertical(|ui| {
                ui.set_max_width((ui.available_width() - 250.0).max(160.0));
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new("Picture").semibold());
                let hint = if self.create.picking {
                    "Waiting for the file chooser".to_string()
                } else {
                    match &self.create.extras.picture {
                        Some(p) => p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                        None => "Shown on its tile and page. Without one, it gets a cube in its loader's colour."
                            .to_string(),
                    }
                };
                ui.add(egui::Label::new(RichText::new(hint).size(FS_META).color(c_muted())).wrap());
            });
            let id = ui.scope_id().with("picture-buttons");
            super::discover::end_aligned(ui, id, |ui| {
                let text = if self.create.extras.picture.is_some() { "Choose another" } else { "Choose a picture" };
                if secondary_enabled(ui, text, !self.create.picking).clicked() {
                    choose = true;
                }
                if self.create.extras.picture.is_some() && ghost(ui, "Remove").clicked() {
                    self.create.extras.picture = None;
                    self.create.picture_preview = None;
                }
            });
        });
        if let Some(e) = self.create.picture_error.clone() {
            ui.add_space(6.0);
            notice(ui, NoticeKind::Error, &e, None);
        }
        if choose {
            self.pick_create_picture();
        }
    }

    /// The picture the new instance will have, painted into `rect`.
    fn paint_create_picture(&self, ui: &Ui, rect: egui::Rect) {
        match &self.create.picture_preview {
            Some(tex) => {
                egui::Image::from_texture(egui::load::SizedTexture::from_handle(tex))
                    .corner_radius(CornerRadius::same(R_CONTROL))
                    .paint_at(ui, rect);
            }
            None => {
                let (_, _, loader) = self.create_identity();
                super::instance_icon::paint_default_icon(ui, rect, loader);
            }
        }
    }

    /// How much memory the new instance may use: what the launcher's settings say (the default), automatic, or a
    /// fixed amount on a slider.
    fn memory_row(&mut self, ui: &mut Ui) {
        let launcher_auto = self.settings.automatic_memory != Some(false);
        let launcher_mb = self.settings.maximum_memory;
        let (total_mb, _) = super::settings::memory_now();
        let e = &mut self.create.extras;
        // with the launcher set to automatic, "Automatic" would be the launcher setting again
        let choices: Vec<(Memory, &str)> = if launcher_auto {
            vec![(Memory::Follow, "Launcher setting"), (Memory::Custom, "Custom")]
        } else {
            vec![(Memory::Follow, "Launcher setting"), (Memory::Automatic, "Automatic"), (Memory::Custom, "Custom")]
        };
        if !choices.iter().any(|c| c.0 == e.memory) {
            e.memory = Memory::Follow;
        }
        let hint = match e.memory {
            Memory::Follow if launcher_auto => {
                "As in Settings: picked when the game starts. A change there applies here too.".to_string()
            }
            Memory::Follow => format!(
                "As in Settings: {}. A change there applies here too.",
                super::settings::memory_text(launcher_mb)
            ),
            Memory::Automatic => {
                "Picked each time the game starts, from its mods and this computer's free memory.".into()
            }
            Memory::Custom => "Always this much, whatever Settings say.".into(),
        };
        // the choice on the title's line and what it means under both, so the row stays short in a small window
        let mut pick = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Memory").semibold());
            let id = ui.scope_id().with("memory-choice");
            super::discover::end_aligned(ui, id, |ui| {
                let labels: Vec<&str> = choices.iter().map(|c| c.1).collect();
                let current = choices.iter().position(|c| c.0 == e.memory).unwrap_or(0);
                pick = segmented(ui, &labels, current);
            });
        });
        small_muted(ui, hint);
        if let Some(i) = pick {
            e.memory = choices[i].0;
            if e.memory == Memory::Custom && e.memory_mb == 0 {
                e.memory_mb = launcher_mb.max(1024);
            }
        }
        if e.memory == Memory::Custom {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 110.0).clamp(160.0, 420.0);
                // the part of the rail up to the handle in the brand colour: how much is given
                ui.visuals_mut().selection.bg_fill = c_accent();
                let max = if total_mb == 0 { 16384 } else { total_mb as u32 }.max(1024);
                ui.add(
                    egui::Slider::new(&mut e.memory_mb, 512..=max)
                        .step_by(256.0)
                        .show_value(false)
                        .trailing_fill(true)
                        .clamping(egui::SliderClamping::Always),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new(super::settings::memory_text(e.memory_mb)).font(super::fonts::medium(FS_FIGURE)),
                );
            });
            // the same line as in an instance's settings: more than three quarters of the computer is too much
            if total_mb > 0 && u64::from(e.memory_mb) * 4 > total_mb * 3 {
                ui.add_space(4.0);
                let text = format!(
                    "That leaves too little for the rest of this computer ({:.1} GB in all). The game may fail to start, or freeze the computer.",
                    total_mb as f64 / 1024.0
                );
                notice(ui, NoticeKind::Warn, &text, None);
            }
        }
    }

    /// The panel beside the steps on a wide page: "Selected", the instance as it will be (its cover, name, version,
    /// loader and folder), and the button that goes on to the next step or makes it.
    fn create_side_panel(&mut self, ui: &mut Ui, room: f32) -> Option<Go> {
        let mut act = None;
        let blocker = self.step_blocker();
        let creating = self.create_running();
        let (name, mc, loader) = self.create_identity();
        let theme = self.create.extras.cover;
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(RichText::new("Selected").size(FS_META).color(c_muted()).semibold());
            ui.add_space(2.0);
            // the cover it will have, with its picture in the corner
            // the cover gives way on a short page, so the button stays in the panel
            let w = ui.available_width();
            let cover_h = (room - PANEL_BESIDES_COVER).clamp(64.0, (w * 0.42).round());
            let (rect, _) = ui.allocate_exact_size(vec2(w, cover_h), egui::Sense::hover());
            let spec = CoverSpec::new(&self.create_cover_key(&name), &mc, loader, theme).with_fade_left(true);
            let (tw, th) = super::workbench::cover_size(rect.size());
            let tex = self.covers.texture(&self.ctx, &spec, tw, th);
            egui::Image::from_texture((tex.id(), vec2(tw as f32, th as f32)))
                .uv(super::workbench::cover_uv((tw, th), rect.size()))
                .corner_radius(R_CONTROL)
                .paint_at(ui, rect);
            let pic = egui::Rect::from_min_size(rect.left_bottom() + vec2(10.0, -46.0), vec2(36.0, 36.0));
            ui.painter().rect_filled(
                pic.expand(2.0),
                CornerRadius::same(R_CONTROL),
                egui::Color32::from_black_alpha(90),
            );
            self.paint_create_picture(ui, pic);
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(&name).font(super::fonts::semibold(FS_BODY + 1.0))).wrap());
            let c = &self.create;
            let horizon = c.edition == Edition::Horizon;
            let latest = self.manifest.as_ref().and_then(|m| m.latest.get("release")).is_some_and(|l| *l == c.mc);
            let line = match (c.mc.is_empty(), latest) {
                _ if horizon => format!("{} \u{00B7} Minecraft {mc}", horizon::NAME),
                (true, _) => "No version picked yet".to_string(),
                (false, true) => format!("Minecraft {} \u{00B7} Latest release", c.mc),
                (false, false) => format!("Minecraft {}", c.mc),
            };
            small_muted(ui, line);
            ui.add_space(4.0);
            rule(ui);
            ui.add_space(2.0);
            let loader_text = match (c.loader, chosen_loader_version(c)) {
                (Loader::Vanilla, _) => "None".to_string(),
                (l, Some(v)) => format!("{} {v}", l.label()),
                (l, None) => l.label().to_string(),
            };
            let taken: Vec<&str> = self.instances.iter().map(|i| i.folder.as_str()).collect();
            let folder = preview_folder(&name, &taken);
            let mut rows: Vec<(&str, String, bool)> = if horizon {
                let build = self.horizon_tag().map(str::to_string).unwrap_or_else(|| "Newest".into());
                vec![
                    ("Edition", horizon::NAME.to_string(), false),
                    ("Minecraft", mc.clone(), true),
                    ("Build", build, true),
                    ("Folder", folder, true),
                ]
            } else {
                vec![
                    ("Minecraft", if c.mc.is_empty() { "\u{2014}".to_string() } else { c.mc.clone() }, true),
                    ("Loader", loader_text, true),
                    ("Folder", folder, true),
                    ("Memory", memory_summary(&c.extras), false),
                ]
            };
            if !horizon && performance::offered_for(c.loader.modrinth_name()) {
                rows.push(("Performance mods", if c.performance { "Added" } else { "None" }.to_string(), false));
            }
            for (key, value, mono) in rows {
                summary_row(ui, key, &value, mono);
            }
            ui.add_space(8.0);
            act = self.create_buttons(ui, blocker.as_deref(), creating, true);
        });
        act
    }

    /// The button that goes on (Continue, or Create instance on the last step) and Back, with what keeps it greyed
    /// out. `stacked` puts them one under the other, the main one across the panel.
    fn create_buttons(&self, ui: &mut Ui, blocker: Option<&str>, creating: bool, stacked: bool) -> Option<Go> {
        let mut act = None;
        let last = self.create.step == Step::Configure;
        let text = if last { "Create instance" } else { "Continue \u{2192}" };
        if stacked {
            if creating {
                ui.horizontal(|ui| {
                    spinner_sized(ui, 18.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new("Working on it, you can follow along in the sidebar")
                                .size(FS_META)
                                .color(c_muted()),
                        )
                        .wrap(),
                    );
                });
            }
            ui.horizontal(|ui| {
                if self.create.step != Step::Version && ghost(ui, "\u{2190} Back").clicked() {
                    act = Some(Go::Back);
                }
                let w = ui.available_width();
                if primary_sized(ui, text, vec2(w, 40.0), blocker.is_none()).clicked() {
                    act = Some(if last { Go::Create } else { Go::Next });
                }
            });
            if let Some(why) = blocker.filter(|_| !creating) {
                ui.add(egui::Label::new(RichText::new(why).size(FS_META).color(c_muted())).wrap());
            }
        } else {
            if self.create.step != Step::Version && ghost(ui, "\u{2190} Back").clicked() {
                act = Some(Go::Back);
            }
            let resp = primary_enabled(ui, text, blocker.is_none());
            let resp = match blocker {
                Some(why) => resp.on_disabled_hover_text(why),
                None => resp,
            };
            if resp.clicked() {
                act = Some(if last { Go::Create } else { Go::Next });
            }
        }
        act
    }

    /// The bar under the steps on a narrower page: what is about to be made, and the buttons that go on and back.
    fn create_bottom_bar(&mut self, ui: &mut Ui) -> Option<Go> {
        let mut act = None;
        let blocker = self.step_blocker();
        let creating = self.create_running();
        let (name, mc, loader) = self.create_identity();
        let summary = match self.create.edition {
            Edition::Horizon => format!("{} \u{00B7} Minecraft {mc}", horizon::NAME),
            Edition::Java => summary(&self.create),
        };
        let spec = CoverSpec::new(&self.create_cover_key(&name), &mc, loader, self.create.extras.cover);
        let tex = self.covers.texture(&self.ctx, &spec, 128, 128);
        card_frame().inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            super::discover::dialog_footer(
                ui,
                "create",
                |ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), egui::Sense::hover());
                    egui::Image::from_texture((tex.id(), vec2(128.0, 128.0)))
                        .corner_radius(R_CONTROL)
                        .paint_at(ui, rect);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new(&name).semibold());
                        if creating {
                            small_muted(ui, "Working on it, you can follow along in the sidebar");
                        } else {
                            ui.label(
                                RichText::new(if summary.is_empty() {
                                    "No version picked yet".into()
                                } else {
                                    summary.clone()
                                })
                                .font(super::fonts::mono(11.5))
                                .color(c_muted()),
                            );
                        }
                    });
                },
                |ui| {
                    act = self.create_buttons(ui, blocker.as_deref(), creating, false);
                },
            );
        });
        act
    }

    /// The screenshot tour's New instance on `step`: Fabric picked for the steps after the first, and a name and a
    /// cover on the last.
    pub(super) fn shot_create(&mut self, step: Step) {
        self.page = Page::Create;
        let c = &mut self.create;
        c.edition = Edition::Java;
        c.step = step;
        c.loader = if step == Step::Version { Loader::Vanilla } else { Loader::Fabric };
        if step == Step::Configure {
            c.name = "My Fabric world".into();
            c.extras.cover = CoverTheme::Cherry;
        }
        self.request_loader_versions();
    }

    /// The screenshot tour's New instance with Horizon chosen, on `step`; a name and a cover on the last.
    pub(super) fn shot_create_horizon(&mut self, step: Step) {
        self.page = Page::Create;
        let c = &mut self.create;
        c.edition = Edition::Horizon;
        c.step = step;
        if step == Step::Configure {
            c.name = "Horizon survival".into();
            c.extras.cover = CoverTheme::Auto;
        }
    }

    /// Make a Horizon instance as the form says: Horizon installs it (its build, the Minecraft it prepares from, the
    /// instance), then the form's description, cover and picture go into it.
    fn start_create_horizon(&mut self) {
        let c = &self.create;
        let wanted = if c.name.trim().is_empty() { horizon::NAME.to_string() } else { c.name.trim().to_string() };
        let name = unique_instance_name(&self.instances, &wanted);
        // the memory is Java's, and the form hides it for Horizon: a choice made before switching is left out
        let extras = Extras { memory: Memory::Follow, ..c.extras.clone() };
        let dirs = self.dirs.clone();
        let label = format!("Creating {name}");
        self.create.task_label = Some(label.clone());
        self.start_task(label, move |p, _| async move {
            let mut inst = horizon::create_instance(&dirs, &name, &p).await?;
            Ok(finish_extras(&mut inst, &extras).await)
        });
        let c = &mut self.create;
        c.name.clear();
        c.step = Step::Version;
        c.extras = Extras::default();
        c.picture_preview = None;
        c.picture_error = None;
        self.page = Page::Library;
    }

    /// Make the instance as the form says, in a sidebar task, and go to the library where it shows up.
    fn start_create(&mut self) {
        if self.create.edition == Edition::Horizon {
            self.start_create_horizon();
            return;
        }
        let c = &self.create;
        let loader = c.loader;
        let mc = c.mc.clone();
        let wanted = if c.name.trim().is_empty() { default_name(&mc, loader) } else { c.name.trim().to_string() };
        // two instances of one name would share their backups (they are found by name): the second gets a number
        let name = unique_instance_name(&self.instances, &wanted);
        let version = chosen_loader_version(c).map(str::to_string);
        let performance = c.performance && performance::offered_for(loader.modrinth_name());
        let extras = c.extras.clone();
        let (dirs, settings) = (self.dirs.clone(), self.settings.clone());
        let label = format!("Creating {name}");
        self.create.task_label = Some(label.clone());
        self.start_task(label, move |p, _| async move {
            let mut inst = instances::create(&dirs, &name, &mc, loader, version.as_deref(), &p).await?;
            let ready = finish_extras(&mut inst, &extras).await;
            if !performance {
                return Ok(ready);
            }
            let ctx = crate::mods::ModContext::build(&dirs, &inst, &settings).await;
            let r = crate::mods::install::install_performance_pack(&mut inst, &ctx, &p).await;
            Ok(performance_note(ready, &r.installed, &r.skipped))
        });
        // the next instance starts from a clean form; the version, loader and kinds shown stay as they were
        let c = &mut self.create;
        c.name.clear();
        c.step = Step::Version;
        c.extras = Extras::default();
        c.picture_preview = None;
        c.picture_error = None;
        self.page = Page::Library;
    }
}

/// Put the options of the form into the instance just made, and say it is ready. The instance is there already, so
/// an option that can't be applied is said in the message rather than undoing it all.
async fn finish_extras(inst: &mut instances::Instance, extras: &Extras) -> String {
    let mut ready = format!("\"{}\" is ready", inst.name());
    if extras.changes_file() {
        extras.apply(&mut inst.file.launcher);
        super::workbench::set_cover_theme(inst, extras.cover);
        if let Err(e) = inst.save() {
            ready.push_str(&format!(". Its memory, cover and description weren't saved: {e:#}"));
        }
    }
    if let Some(path) = extras.picture.clone() {
        let target = inst.root.join("instance.png");
        // the same as choosing a picture for an instance afterwards: shrunk to 256 points, saved as a PNG
        let saved = tokio::task::spawn_blocking(move || {
            image::open(&path)
                .and_then(|img| {
                    img.resize(256, 256, image::imageops::FilterType::Lanczos3)
                        .save_with_format(&target, image::ImageFormat::Png)
                })
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        if let Err(e) = saved {
            ready.push_str(&format!(". The picture couldn't be used: {e}"));
        }
    }
    ready
}

/// What the Horizon card is greyed out with where Horizon publishes no build.
const NO_HORIZON_BUILD: &str = "Horizon has no build for this computer yet";

/// What plays in Horizon, and what doesn't, as its card on the first step lists them.
const HORIZON_PLAYS: [&str; 4] =
    ["Singleplayer and servers", "Worlds in Minecraft's own format", "Resource packs", "Horizon's own mods"];
const HORIZON_DOESNT: [&str; 2] = [
    "Java mods and mod loaders (Fabric, Forge, NeoForge, Quilt)",
    "Shader packs, and what is Java's: the Java used, its arguments, fast start",
];

/// How tall a card of the choice of game is, and from what width it has room for the long words.
const EDITION_H: f32 = 64.0;
const EDITION_ROOMY: f32 = 270.0;

/// A game to make an instance of, chosen the way a radio button is (like a loader's row): a ring that fills in the
/// brand gradient, the name with Horizon's label after it (`horizon`), and a line under it. Greyed out and not
/// pressable when not `enabled`.
fn edition_card(
    ui: &mut Ui,
    title: &str,
    line: &str,
    horizon: bool,
    selected: bool,
    enabled: bool,
    width: f32,
) -> egui::Response {
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, EDITION_H), sense);
    if ui.is_rect_visible(rect) {
        let t = motion::hover(ui, resp.id, enabled && resp.hovered());
        let s = motion::toward(ui.ctx(), resp.id.with("on"), selected, motion::MEDIUM);
        let p = ui.painter();
        let fill = motion::mix(motion::mix(c_card(), c_hover(), t), c_accent_soft(), s);
        let edge =
            motion::mix(motion::mix(c_border(), c_control().gamma_multiply(0.7), t), c_accent().gamma_multiply(0.8), s);
        p.rect(rect, CornerRadius::same(R_CARD), fill, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
        let ring = egui::Rect::from_center_size(egui::pos2(rect.left() + 26.0, rect.center().y), vec2(20.0, 20.0));
        let ring_ink = if enabled { c_control() } else { c_border() };
        p.rect_stroke(
            ring,
            CornerRadius::same(10),
            Stroke::new(1.5, ring_ink.gamma_multiply(1.0 - s)),
            egui::StrokeKind::Inside,
        );
        if s > 0.0 {
            motion::gradient_rect(p, ring, 10, c_accent().gamma_multiply(s), c_accent2().gamma_multiply(s));
            super::discover::paint_check(p, ring.center(), 10.0, c_on_accent().gamma_multiply(s));
        }
        let left = rect.left() + 50.0;
        let room = rect.right() - left - 12.0;
        let (ink, sub) = if enabled { (c_text(), c_muted()) } else { (c_faint(), c_faint()) };
        let mut job = egui::text::LayoutJob::single_section(
            title.to_string(),
            egui::TextFormat::simple(super::fonts::semibold(FS_BODY + 1.0), ink),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let name = p.layout_job(job);
        let name_y = rect.center().y - 19.0;
        let name_w = name.size().x;
        p.galley(egui::pos2(left, name_y), name, ink);
        if horizon {
            // what it is, on a label after the name when there is room for it
            let color = if enabled { loader_color(horizon::NAME) } else { c_faint() };
            let g = p.layout_no_wrap("Rust rewrite".to_string(), super::fonts::medium(11.5), color);
            let tag =
                egui::Rect::from_min_size(egui::pos2(left + name_w + 8.0, name_y + 1.0), vec2(g.size().x + 12.0, 19.0));
            if tag.right() <= rect.right() - 12.0 {
                p.rect_filled(tag, CornerRadius::same(R_TAG), color.gamma_multiply(0.14));
                p.galley(tag.center() - g.size() / 2.0, g, color);
            }
        }
        let mut job = egui::text::LayoutJob::single_section(
            line.to_string(),
            egui::TextFormat::simple(egui::FontId::proportional(FS_META), sub),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        p.galley(egui::pos2(left, rect.center().y + 4.0), p.layout_job(job), sub);
    }
    focus_ring(ui, &resp, rect, R_CARD);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, enabled, selected, title));
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// A point of Horizon's card: a check in the brand colour for what plays, a cross for what doesn't, then the words.
fn point(ui: &mut Ui, plays: bool, text: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (rect, _) = ui.allocate_exact_size(vec2(14.0, 18.0), egui::Sense::hover());
        let c = rect.center();
        if plays {
            super::discover::paint_check(ui.painter(), c, 12.0, accent_ink());
        } else {
            let stroke = Stroke::new(1.5, c_muted());
            ui.painter().line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], stroke);
            ui.painter().line_segment([c + vec2(-4.0, 4.0), c + vec2(4.0, -4.0)], stroke);
        }
        let ink = if plays { c_text() } else { c_muted() };
        ui.add(egui::Label::new(RichText::new(text).color(ink)).wrap());
    });
}

/// A line of Horizon's card about what it needs or what is done for it, with an icon in front.
fn fact_line(ui: &mut Ui, icon: Icon, text: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let (rect, _) = ui.allocate_exact_size(vec2(16.0, 18.0), egui::Sense::hover());
        super::icons::paint(ui.painter(), icon, rect.center(), 14.0, c_muted());
        ui.add(egui::Label::new(RichText::new(text).size(FS_META + 1.0).color(c_muted())).wrap());
    });
}

/// One line of the summary panel: what, in grey, and its value at the right (in the monospace face with `mono`, for
/// versions and the folder).
fn summary_row(ui: &mut Ui, key: &str, value: &str, mono: bool) {
    ui.scope(|ui| {
        // rows of text, not of buttons
        ui.spacing_mut().interact_size.y = 24.0;
        ui.horizontal(|ui| summary_cells(ui, key, value, mono));
    });
}

fn summary_cells(ui: &mut Ui, key: &str, value: &str, mono: bool) {
    ui.label(RichText::new(key).color(c_muted()));
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let font = if mono { super::fonts::mono(12.5) } else { super::fonts::medium(FS_BODY) };
        ui.add(egui::Label::new(RichText::new(value).font(font)).truncate()).on_hover_text(value);
    });
}

/// What a button of the panel asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Go {
    Next,
    Back,
    Create,
}

/// The title of a step and a line under it.
fn step_title(ui: &mut Ui, title: &str, line: &str) {
    ui.label(RichText::new(title).size(FS_FIGURE).semibold());
    ui.add_space(2.0);
    muted(ui, line);
    ui.add_space(12.0);
}

/// `wanted`, or with " (2)", " (3)" ... after it when an instance of that name (in any case) is there already.
pub(super) fn unique_instance_name(instances: &[crate::instances::Instance], wanted: &str) -> String {
    let taken = |n: &str| instances.iter().any(|i| i.name().trim().eq_ignore_ascii_case(n));
    if !taken(wanted) {
        return wanted.to_string();
    }
    (2..).map(|n| format!("{wanted} ({n})")).find(|n| !taken(n)).unwrap_or_else(|| wanted.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_that_is_taken_gets_a_number() {
        let inst = |name: &str| {
            let mut file = crate::instances::InstanceFile::default();
            file.launcher.name = name.into();
            crate::instances::Instance { folder: name.into(), mod_count: 0, root: std::env::temp_dir(), file }
        };
        let all = [inst("Survival"), inst("Survival (2)"), inst("Creative")];
        assert_eq!(unique_instance_name(&all, "Survival"), "Survival (3)");
        // case doesn't make a name new: "survival (2)" is taken by "Survival (2)" too
        assert_eq!(unique_instance_name(&all, "survival"), "survival (3)");
        assert_eq!(unique_instance_name(&all, "CREATIVE"), "CREATIVE (2)");
        assert_eq!(unique_instance_name(&all, "Skyblock"), "Skyblock");
    }

    #[test]
    fn the_empty_name_and_the_finished_message_read_well() {
        assert_eq!(default_name("1.21.1", Loader::Vanilla), "Minecraft 1.21.1");
        assert_eq!(default_name("1.21.1", Loader::Fabric), "Minecraft 1.21.1 with Fabric");
        let ready = "\"A\" is ready".to_string();
        assert_eq!(performance_note(ready.clone(), &[], &[]), ready);
        assert_eq!(
            performance_note(ready, &["Sodium".into(), "Lithium".into()], &["ModernFix".into()]),
            "\"A\" is ready, with Sodium, Lithium. Not made for this version yet: ModernFix"
        );
        // the line above the button says what is about to be made
        let mut c = CreateState::default();
        assert_eq!(summary(&c), "");
        c.mc = "1.21.1".into();
        assert_eq!(summary(&c), "Minecraft 1.21.1");
        c.loader = Loader::Fabric;
        assert_eq!(summary(&c), "Minecraft 1.21.1 with Fabric");
        c.loader_versions = vec!["0.16.5".into(), "0.16.4".into()];
        assert_eq!(summary(&c), "Minecraft 1.21.1 with Fabric 0.16.5");
        c.loader_version = "0.16.4".into();
        assert_eq!(summary(&c), "Minecraft 1.21.1 with Fabric 0.16.4");
        // a test build at the top of the list isn't the one recommended
        c.loader_version.clear();
        c.loader_versions = vec!["0.17.0-beta.2".into(), "0.16.5".into()];
        assert_eq!(summary(&c), "Minecraft 1.21.1 with Fabric 0.16.5");
        // performance mods are offered where they exist, not for plain Minecraft
        assert!(performance::offered_for(Loader::Fabric.modrinth_name()));
        assert!(!performance::offered_for(Loader::Vanilla.modrinth_name()));
    }

    fn version(id: &str, kind: &str) -> ManifestVersion {
        ManifestVersion {
            id: id.into(),
            kind: kind.into(),
            url: String::new(),
            sha1: String::new(),
            release_time: "2024-08-08T08:24:24+00:00".into(),
        }
    }

    #[test]
    fn versions_are_sorted_into_kinds_counted_and_filtered() {
        let all = [
            version("1.21.1", "release"),
            version("24w33a", "snapshot"),
            version("1.21", "release"),
            version("1.21-pre1", "snapshot"),
            version("b1.7.3", "old_beta"),
            version("a1.0.4", "old_alpha"),
            version("1.20.1", "release"),
            version("something-new", "pending"),
        ];
        assert_eq!(VersionKind::of("release"), VersionKind::Release);
        assert_eq!(VersionKind::of("old_alpha"), VersionKind::Old);
        // a kind Mojang adds later is not a release
        assert_eq!(VersionKind::of("pending"), VersionKind::Snapshot);
        assert_eq!(kind_counts(&all, ""), [3, 3, 2]);
        // the numbers follow the search, in any case
        assert_eq!(kind_counts(&all, "1.21"), [2, 1, 0]);
        assert_eq!(kind_counts(&all, " B1. "), [0, 0, 1]);
        let ids = |kinds: KindFilter, q: &str| -> Vec<String> {
            shown_versions(&all, kinds, q).iter().map(|v| v.id.clone()).collect()
        };
        // releases only at first, newest first as the manifest has them
        assert_eq!(ids(KindFilter::default(), ""), ["1.21.1", "1.21", "1.20.1"]);
        assert_eq!(ids(KindFilter::default(), "1.21"), ["1.21.1", "1.21"]);
        let mut kinds = KindFilter::default();
        kinds.toggle(VersionKind::Snapshot);
        assert_eq!(ids(kinds, "1.21"), ["1.21.1", "1.21", "1.21-pre1"]);
        kinds.toggle(VersionKind::Release);
        kinds.toggle(VersionKind::Old);
        assert_eq!(ids(kinds, ""), ["24w33a", "1.21-pre1", "b1.7.3", "a1.0.4", "something-new"]);
        let none = KindFilter { release: false, snapshot: false, old: false };
        assert!(none.none() && ids(none, "").is_empty());
        assert!(ids(KindFilter::default(), "nothing like this").is_empty());
    }

    #[test]
    fn the_recommended_loader_is_the_newest_that_is_not_a_test_build() {
        let list = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(recommended_loader(&list(&["0.16.5", "0.16.4"])), Some("0.16.5"));
        assert_eq!(recommended_loader(&list(&["0.27.0-beta.1", "0.26.4"])), Some("0.26.4"));
        assert_eq!(recommended_loader(&list(&["21.2.0-beta", "21.1.262"])), Some("21.1.262"));
        assert_eq!(recommended_loader(&list(&["1.0.0-rc2", "1.0.0-pre.1", "0.9.9"])), Some("0.9.9"));
        // Forge's names carry the Minecraft version, which is not a test build
        assert_eq!(recommended_loader(&list(&["1.20.1-47.4.28", "1.20.1-47.4.27"])), Some("1.20.1-47.4.28"));
        // nothing but test builds: the newest of them
        assert_eq!(recommended_loader(&list(&["0.2.0-beta", "0.1.0-alpha"])), Some("0.2.0-beta"));
        assert_eq!(recommended_loader(&[]), None);
    }

    #[test]
    fn more_options_go_into_the_instance_file() {
        let mut e = Extras::default();
        assert!(!e.changes_file());
        let mut l = LauncherBlock { maximum_memory: Some(4096), ..Default::default() };
        e.apply(&mut l);
        // left as they are, nothing is touched
        assert_eq!((l.maximum_memory, l.automatic_memory, l.description.as_str()), (Some(4096), None, ""));
        e.description = "  With friends  ".into();
        e.memory = Memory::Custom;
        e.memory_mb = 6144;
        e.picture = Some(PathBuf::from("/pictures/castle.png"));
        assert!(e.changes_file());
        e.apply(&mut l);
        assert_eq!((l.maximum_memory, l.automatic_memory, l.description.as_str()), (Some(6144), None, "With friends"));
        e.memory = Memory::Automatic;
        e.apply(&mut l);
        assert_eq!((l.maximum_memory, l.automatic_memory), (None, Some(true)));
        assert_eq!(memory_summary(&e), "Automatic");
        // a picture alone needs no change to the file: it becomes instance.png
        let only_picture = Extras { picture: Some(PathBuf::from("a.png")), ..Default::default() };
        assert!(!only_picture.changes_file());
        // a cover theme is kept in the instance's file
        let cover = Extras { cover: CoverTheme::Snow, ..Default::default() };
        assert!(cover.changes_file());
    }

    #[test]
    fn the_steps_go_on_and_back_and_wait_for_a_version() {
        let java = Edition::Java;
        assert_eq!(Step::Version.next(java), Some(Step::Loader));
        assert_eq!(Step::Configure.next(java), None);
        assert_eq!(Step::Loader.back(java), Some(Step::Version));
        assert_eq!(Step::Version.back(java), None);
        // without a version only the first step can be opened; with one, any step can, in any order
        assert!(Step::Version.reachable("", java));
        assert!(!Step::Loader.reachable("", java) && !Step::Configure.reachable("", java));
        assert!(Step::ALL.iter().all(|s| s.reachable("1.21.1", java)));
    }

    #[test]
    fn horizon_has_no_loader_step() {
        let hz = Edition::Horizon;
        // Continue goes from the version straight to Configure, and Back the other way
        assert_eq!(Step::Version.next(hz), Some(Step::Configure));
        assert_eq!(Step::Configure.back(hz), Some(Step::Version));
        assert_eq!(Step::Configure.next(hz), None);
        assert_eq!(Step::Version.back(hz), None);
        assert!(Step::Loader.skipped(hz) && !Step::Loader.skipped(Edition::Java));
        assert!(!Step::Version.skipped(hz) && !Step::Configure.skipped(hz));
        // its version is its own: Configure can be opened without one picked, the loader never
        assert!(Step::Version.reachable("", hz) && Step::Configure.reachable("", hz));
        assert!(!Step::Loader.reachable("", hz) && !Step::Loader.reachable("1.21.1", hz));
        assert_eq!(CreateState::default().edition, Edition::Java);
    }

    #[test]
    fn the_folder_shown_is_the_one_the_instance_gets() {
        assert_eq!(preview_folder("Minecraft 1.21.1 with Fabric", &[]), "Minecraft1211withFabric");
        // taken folders get the next number, as when the folder is claimed
        let taken = ["Minecraft1211withFabric", "Minecraft1211withFabric2"];
        assert_eq!(preview_folder("Minecraft 1.21.1 with Fabric", &taken), "Minecraft1211withFabric3");
        assert_eq!(preview_folder("!!!", &[]), "Instance");
    }

    #[test]
    fn a_version_tooltip_says_what_and_when() {
        let v = version("1.21.1", "release");
        assert_eq!(version_tooltip(&v, true, false), "Release, 8 August 2024\nThe latest release");
        let b = ManifestVersion { release_time: "not a date".into(), ..version("b1.7.3", "old_beta") };
        assert_eq!(version_tooltip(&b, false, false), "Old beta");
    }
}
