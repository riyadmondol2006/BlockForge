//! Create: pick a Minecraft version and a loader, then download it all.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Color32, CornerRadius, RichText, ScrollArea, Stroke, Ui, vec2};

use super::discover::filter_label;
use super::motion;
use super::theme::*;
use super::widgets::*;
use super::{App, Msg, Page};
use crate::install::ManifestVersion;
use crate::instances::{self, LauncherBlock};
use crate::loaders::{self, Loader};
use crate::mods::performance;

pub struct CreateState {
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

/// The widest the form gets.
const FORM_W: f32 = 720.0;
/// The panel beside the form on a wide window: what is about to be made, and the button that makes it.
const SIDE_W: f32 = 300.0;
/// The room between the form and that panel.
const SIDE_GAP: f32 = 20.0;
/// The narrowest the form gets with the panel beside it; on a narrower page the panel becomes a bar under the form.
const FORM_MIN_BESIDE: f32 = 640.0;

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
}

impl Extras {
    /// Whether anything has to be written into the instance's file.
    fn changes_file(&self) -> bool {
        !self.description.trim().is_empty() || self.memory != Memory::Follow
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

    /// The options that were changed, in a few words for the head of "More options" ("6 GB memory, picture").
    fn brief(&self) -> String {
        let mut parts = Vec::new();
        match self.memory {
            Memory::Follow => {}
            Memory::Automatic => parts.push("automatic memory".to_string()),
            Memory::Custom => parts.push(format!("{} memory", super::settings::memory_text(self.memory_mb))),
        }
        if self.picture.is_some() {
            parts.push("picture".into());
        }
        if !self.description.trim().is_empty() {
            parts.push("description".into());
        }
        parts.join(", ")
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

/// One version in the grid of versions: plain until pointed at (the fill fades in), the chosen one in the brand tint
/// with a brand outline. The latest release (and snapshot) has a small dot of the brand colour in its corner.
fn version_cell(ui: &mut Ui, text: &str, selected: bool, latest: bool, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 28.0), egui::Sense::click());
    let t = motion::hover(ui, resp.id, resp.hovered());
    let s = motion::toward(ui.ctx(), resp.id.with("on"), selected, motion::FAST);
    let bg = motion::mix(c_hover().gamma_multiply(t), c_accent_soft(), s);
    let line = c_accent().gamma_multiply(0.7 * s);
    ui.painter().rect(rect, CornerRadius::same(R_TAG + 2), bg, Stroke::new(1.0, line), egui::StrokeKind::Inside);
    let font = if selected { super::fonts::semibold(FS_BODY) } else { egui::FontId::proportional(FS_BODY) };
    let fg = motion::mix(c_text(), accent_ink(), s);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, font, fg);
    if latest {
        ui.painter().circle_filled(rect.right_top() + vec2(-7.0, 7.0), 3.0, accent_ink());
    }
    focus_ring(ui, &resp, rect, R_TAG + 2);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, text));
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

/// The head of a step of the form: its number in a small badge of the brand gradient, the title, and what was picked
/// so far after it.
fn step_head(ui: &mut Ui, n: usize, title: &str, picked: Option<&str>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        step_title(ui, n, title);
        if let Some(p) = picked.filter(|p| !p.is_empty()) {
            ui.label(RichText::new(p).color(c_muted()));
        }
    });
    ui.add_space(4.0);
}

/// The badge and the title of a step, in a row that is already there.
fn step_title(ui: &mut Ui, n: usize, title: &str) {
    let (badge, _) = ui.allocate_exact_size(vec2(24.0, 24.0), egui::Sense::hover());
    motion::brand_rect(ui.painter(), badge, 12);
    ui.painter().text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        n.to_string(),
        super::fonts::semibold(FS_META),
        c_on_accent(),
    );
    ui.label(RichText::new(title).size(FS_SECTION).semibold());
}

/// A few words under a loader's name on its card.
fn loader_blurb(l: Loader) -> &'static str {
    match l {
        Loader::Vanilla => "No mods",
        Loader::Fabric => "Light, fast",
        Loader::Quilt => "Fabric-like",
        Loader::Forge => "The classic",
        Loader::NeoForge => "Newer Forge",
    }
}

/// The narrowest a loader card gets before the row wraps.
const LOADER_CARD_W: f32 = 118.0;
/// One mod loader to choose, as a card: its letter in a tile of its colour, the name and a few words. It lifts a
/// little under the pointer; the chosen one takes the brand tint and outline and a check in the corner.
fn loader_card(ui: &mut Ui, l: Loader, selected: bool, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 60.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let t = motion::hover(ui, resp.id, resp.hovered());
        let s = motion::toward(ui.ctx(), resp.id.with("on"), selected, motion::MEDIUM);
        let card = rect.translate(vec2(0.0, -1.5 * t));
        let p = ui.painter();
        if t > 0.0 {
            let shadow = egui::Shadow {
                offset: [0, 6],
                blur: 18,
                spread: 0,
                color: Color32::from_black_alpha(((if is_dark() { 110.0 } else { 26.0 }) * t) as u8),
            };
            p.add(shadow.as_shape(card, CornerRadius::same(R_CARD)));
        }
        let fill = motion::mix(motion::mix(c_field(), c_field_hover(), t), c_accent_soft(), s);
        let line = motion::mix(motion::mix(c_border(), c_control().gamma_multiply(0.8), t), c_accent(), s);
        p.rect(card, CornerRadius::same(R_CARD), fill, Stroke::new(1.0 + 0.5 * s, line), egui::StrokeKind::Inside);
        glass_edge(p, card, R_CARD);
        // the loader's letter on a pale tint of its colour, like the loader tags elsewhere; on the chosen one the tile
        // turns into a check in the brand gradient
        let hue = loader_color(l.label());
        let tile = egui::Rect::from_min_size(egui::pos2(card.left() + 12.0, card.center().y - 12.0), vec2(24.0, 24.0));
        let r = CornerRadius::same(R_TAG + 1);
        p.rect_filled(tile, r, hue.gamma_multiply(0.18 * (1.0 - s)));
        let letter = l.label().chars().next().unwrap_or('?').to_string();
        p.text(
            tile.center(),
            egui::Align2::CENTER_CENTER,
            letter,
            super::fonts::semibold(FS_BODY),
            hue.gamma_multiply(1.0 - s),
        );
        if s > 0.0 {
            motion::gradient_rect(p, tile, R_TAG + 1, c_accent().gamma_multiply(s), c_accent2().gamma_multiply(s));
            super::discover::paint_check(p, tile.center(), 12.0, c_on_accent().gamma_multiply(s));
        }
        // the name and the few words beside the tile
        let left = tile.right() + 8.0;
        let room = card.right() - left - 8.0;
        let name = p.layout(l.label().to_string(), super::fonts::semibold(FS_BODY), c_text(), room);
        p.galley(egui::pos2(left, card.center().y - 17.0), name, c_text());
        let mut job = egui::text::LayoutJob::single_section(
            loader_blurb(l).to_string(),
            egui::TextFormat::simple(egui::FontId::proportional(FS_META), c_muted()),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        p.galley(egui::pos2(left, card.center().y + 2.0), p.layout_job(job), c_muted());
    }
    focus_ring(ui, &resp, rect, R_CARD);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, l.label()));
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
        heading(ui, "New instance");
        muted(ui, "Pick a Minecraft version and a mod loader, then BlockForge downloads it all.");
        ui.add_space(14.0);
        let mut changed_version = false;
        let mut create = false;
        let mut enter = false;
        let area = ui.available_rect_before_wrap();
        // a wide page has the summary and the button in a panel beside the form; a narrower one has them in a bar
        // under it. Either way the button stays in view: only the form scrolls
        let beside = area.width() >= FORM_MIN_BESIDE + SIDE_GAP + SIDE_W;
        if beside {
            // a form reads best in a column of its own, not stretched across a wide window. Its scroll bar sits in
            // the gap before the panel
            let form_w = (area.width() - SIDE_W - SIDE_GAP).min(FORM_W);
            let form_rect = egui::Rect::from_min_size(area.min, vec2(form_w + 14.0, area.height()));
            let side_rect = egui::Rect::from_min_size(
                egui::pos2(area.left() + form_w + SIDE_GAP, area.top()),
                vec2(SIDE_W, area.height()),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(form_rect), |ui| {
                ScrollArea::vertical().id_salt("create-form").auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_max_width(form_w);
                    self.create_form(ui, area.height(), &mut changed_version, &mut enter);
                });
            });
            ui.scope_builder(egui::UiBuilder::new().max_rect(side_rect), |ui| {
                create = self.create_side_panel(ui);
            });
            ui.advance_cursor_after_rect(area);
        } else {
            let form_h = (ui.available_height() - 84.0).max(160.0);
            ScrollArea::vertical().id_salt("create-form").auto_shrink([false, true]).max_height(form_h).show(
                ui,
                |ui| {
                    ui.set_max_width((ui.available_width() - 14.0).min(FORM_W));
                    self.create_form(ui, form_h, &mut changed_version, &mut enter);
                },
            );
            create = self.create_bottom_bar(ui);
        }
        if changed_version {
            self.request_loader_versions();
        }
        if enter && self.create_blocker().is_none() {
            create = true;
        }
        if create {
            self.start_create();
        }
    }

    /// Whether the instance being made is still on its way.
    fn create_running(&self) -> bool {
        self.create.task_label.as_ref().is_some_and(|l| self.tasks.iter().any(|t| &t.label == l))
    }

    /// What Create waits for, said when the greyed-out button is pointed at; `None` when it can be pressed.
    fn create_blocker(&self) -> Option<String> {
        let c = &self.create;
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

    /// The steps of the form, one card each. `room` is the height the form has, so the grid of versions can show more
    /// of them on a tall window.
    fn create_form(&mut self, ui: &mut Ui, room: f32, changed_version: &mut bool, enter: &mut bool) {
        ui.spacing_mut().item_spacing.y = 8.0;

        // name
        card(ui, |ui| {
            // the name is one line: its field sits beside the step's title
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                step_title(ui, 1, "Name");
                ui.add_space(14.0);
                let c = &mut self.create;
                // the hint is the name the instance gets when the field is left empty
                let hint = if c.mc.is_empty() { "My instance".to_string() } else { default_name(&c.mc, c.loader) };
                let name_field = ui.add_sized(
                    [ui.available_width().min(420.0), 32.0],
                    egui::TextEdit::singleline(&mut c.name)
                        .hint_text(RichText::new(hint).color(c_faint()))
                        .margin(egui::Margin::symmetric(10, 7))
                        .background_color(c_field()),
                );
                // Enter in the name makes the instance, like the Create button
                *enter = name_field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            });
        });

        // Minecraft version
        ui.add_space(6.0);
        card(ui, |ui| self.version_step(ui, room, changed_version));

        // loader
        ui.add_space(6.0);
        card(ui, |ui| self.loader_step(ui, changed_version));

        // the options that can be left as they are
        ui.add_space(6.0);
        self.more_options(ui);
        ui.add_space(8.0);
    }

    /// Step 2: the Minecraft version, from a grid that can be searched and narrowed to the kinds wanted.
    fn version_step(&mut self, ui: &mut Ui, room: f32, changed_version: &mut bool) {
        let manifest = self.manifest.clone();
        let latest_release = manifest.as_ref().and_then(|m| m.latest.get("release").cloned()).unwrap_or_default();
        let latest_snapshot = manifest.as_ref().and_then(|m| m.latest.get("snapshot").cloned()).unwrap_or_default();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            step_title(ui, 2, "Minecraft version");
            let picked = self.create.mc.clone();
            if !picked.is_empty() {
                ui.label(RichText::new(&picked).color(c_muted()));
            }
            // the newest release is marked, and one click away when another version is picked
            if !latest_release.is_empty() {
                if picked == latest_release {
                    soft_tag(ui, "Latest release");
                } else if ghost(ui, &format!("Use the latest release ({latest_release})")).clicked() {
                    self.create.mc = latest_release.clone();
                    self.create.kinds.release = true;
                    *changed_version = true;
                }
            }
        });
        ui.add_space(4.0);
        match (&manifest, &self.manifest_error) {
            (Some(m), _) => {
                let counts = kind_counts(&m.versions, &self.create.filter);
                // the search and the kinds to show, each kind with how many versions it has (that match the search)
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let mut f = std::mem::take(&mut self.create.filter);
                    search_field(ui, &mut f, "Search versions", 190.0);
                    self.create.filter = f;
                    ui.add_space(6.0);
                    for kind in VersionKind::ALL {
                        let on = self.create.kinds.shows(kind);
                        let text = format!("{}  {}", kind.label(), counts[kind.index()]);
                        let resp = chip(ui, &text, on);
                        let resp = if kind == VersionKind::Old {
                            resp.on_hover_text("The betas and alphas from before Minecraft 1.0")
                        } else {
                            resp
                        };
                        if resp.clicked() {
                            self.create.kinds.toggle(kind);
                        }
                    }
                });
                let versions = shown_versions(&m.versions, self.create.kinds, &self.create.filter);
                // the grid sits a step into the card: a darker well on the dark look, a tinted one on the light
                ui.add_space(2.0);
                egui::Frame::new()
                    .fill(if is_dark() { c_bg() } else { c_surface() })
                    .stroke(Stroke::new(1.0, c_border()))
                    .corner_radius(CornerRadius::same(R_CONTROL))
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if versions.is_empty() {
                            ui.add_space(4.0);
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
                            small_muted(ui, text);
                            ui.add_space(4.0);
                            return;
                        }
                        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                        // the versions fill the width in a grid, a few columns of short names; a tall window shows
                        // more rows of them
                        let (cols, cell) = grid_columns(ui.available_width() - 12.0, 84.0, 4.0);
                        let rows = ((room - 430.0) / 32.0).floor().clamp(4.5, 8.5);
                        ScrollArea::vertical()
                            .id_salt("versions")
                            .max_height(rows * 32.0)
                            .auto_shrink([false, true])
                            .show_rows(ui, 28.0, versions.len().div_ceil(cols), |ui, range| {
                                for r in range {
                                    ui.horizontal(|ui| {
                                        for v in versions.iter().skip(r * cols).take(cols) {
                                            let selected = self.create.mc == v.id;
                                            let is_release = v.id == latest_release;
                                            let is_snapshot = v.id == latest_snapshot && !is_release;
                                            let resp = version_cell(ui, &v.id, selected, is_release || is_snapshot, cell)
                                                .on_hover_text(version_tooltip(v, is_release, is_snapshot));
                                            if resp.clicked() && !selected {
                                                self.create.mc = v.id.clone();
                                                *changed_version = true;
                                            }
                                        }
                                    });
                                }
                            });
                    });
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

    /// Step 3: the mod loader, its version, and the performance mods it can come with.
    fn loader_step(&mut self, ui: &mut Ui, changed_version: &mut bool) {
        step_head(ui, 3, "Mod loader", None);
        // the loaders as cards in a row that ends flush with the card's edge, wrapping in a narrow window
        let gap = 10.0;
        let (cols, w) = grid_columns(ui.available_width(), LOADER_CARD_W, gap);
        for row in Loader::ALL.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for l in row {
                    let on = *l == self.create.loader;
                    if loader_card(ui, *l, on, w).clicked() && !on {
                        self.create.loader = *l;
                        *changed_version = true;
                    }
                }
            });
        }
        if self.create.loader == Loader::Vanilla {
            return;
        }
        // the loader's version, once there is a list to pick from (or while it is looked up)
        let has_list = self.create.loading || !self.create.loader_versions.is_empty();
        if has_list {
            ui.add_space(8.0);
        }
        ui.horizontal_wrapped(|ui| {
            if !has_list {
                return;
            }
            filter_label(ui, &format!("{} version", self.create.loader.label()));
            if self.create.loading {
                spinner_sized(ui, 20.0);
                small_muted(ui, "Looking up the versions");
                return;
            }
            let Some(recommended) = recommended_loader(&self.create.loader_versions).map(str::to_string) else {
                return;
            };
            let newest = self.create.loader_versions[0].clone();
            let chosen = chosen_loader_version(&self.create).unwrap_or(&recommended).to_string();
            // what each version is: the one used unless another is picked, the newest, a test build
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
            egui::ComboBox::from_id_salt("loader-version").selected_text(describe(&chosen)).width(240.0).show_ui(
                ui,
                |ui| {
                    for v in &self.create.loader_versions {
                        if ui.selectable_label(chosen == *v, describe(v)).clicked() {
                            // the recommended one is kept as "no choice", so it follows the list
                            self.create.loader_version = if *v == recommended { String::new() } else { v.clone() };
                        }
                    }
                },
            );
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
            } else if self.create.loader_versions.is_empty() && !self.create.mc.is_empty() {
                let text = format!("{} isn't available for Minecraft {}.", self.create.loader.label(), self.create.mc);
                notice(ui, NoticeKind::Warn, &text, None);
            }
        }
        if matches!(self.create.loader, Loader::Forge | Loader::NeoForge) {
            small_muted(
                ui,
                "Forge and NeoForge run their own installer, so this takes a little longer the first time.",
            );
        }
        let key = self.create.loader.modrinth_name();
        if performance::offered_for(key) {
            ui.add_space(8.0);
            let hint = format!(
                "{} from Modrinth, for a smoother game. Anything not made for this Minecraft version yet is left out.",
                performance::describe(key)
            );
            setting_row(ui, "Add performance mods", &hint, |ui| {
                toggle(ui, &mut self.create.performance);
            });
        }
    }

    /// Step 4, folded until opened: a picture, the memory and a description, put into the instance right after it is
    /// made. The head says what was changed, so it is seen without opening it.
    fn more_options(&mut self, ui: &mut Ui) {
        let id = egui::Id::unique("create-more-options");
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
        card(ui, |ui| {
            let head = ui
                .scope(|ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        step_title(ui, 4, "More options");
                        let brief = self.create.extras.brief();
                        let text = if brief.is_empty() { "Picture, memory, description".to_string() } else { brief };
                        // cut with an ellipsis where the chevron's room at the right starts
                        let mut job = egui::text::LayoutJob::single_section(
                            text,
                            egui::TextFormat::simple(egui::FontId::proportional(FS_BODY), c_muted()),
                        );
                        job.wrap = egui::text::TextWrapping::truncate_at_width((ui.available_width() - 34.0).max(40.0));
                        let galley = ui.painter().layout_job(job);
                        let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
                        ui.painter().galley(rect.min, galley, c_muted());
                    });
                })
                .response
                .rect;
            // the head reaches across the card so the chevron sits at its right edge
            let head = egui::Rect::from_min_max(head.min, egui::pos2(ui.max_rect().right(), head.max.y)).expand(6.0);
            let resp = ui.interact(head, id.with("head"), egui::Sense::click());
            if resp.clicked() {
                state.toggle(ui);
            }
            let t = motion::hover(ui, resp.id, resp.hovered());
            if t > 0.0 {
                ui.painter().rect_stroke(
                    head,
                    CornerRadius::same(R_CONTROL),
                    Stroke::new(1.0, c_border().gamma_multiply(t)),
                    egui::StrokeKind::Inside,
                );
            }
            // the chevron points right when folded and down when open
            let c = egui::pos2(head.right() - 18.0, head.center().y);
            let a = state.openness(ui.ctx()) * std::f32::consts::FRAC_PI_2;
            let rot = |x: f32, y: f32| c + vec2(x * a.cos() - y * a.sin(), x * a.sin() + y * a.cos());
            ui.painter().add(egui::Shape::line(
                vec![rot(-2.5, -5.0), rot(2.5, 0.0), rot(-2.5, 5.0)],
                Stroke::new(1.6, motion::mix(c_muted(), c_text(), t)),
            ));
            focus_ring(ui, &resp, head, R_CONTROL);
            let is_open = state.is_open();
            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::DisclosureTriangle, true, is_open, "More options")
            });
            resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            state.show_body_unindented(ui, |ui| {
                ui.add_space(12.0);
                rule(ui);
                ui.add_space(12.0);
                self.picture_row(ui);
                ui.add_space(10.0);
                rule(ui);
                ui.add_space(10.0);
                self.memory_row(ui);
                ui.add_space(10.0);
                rule(ui);
                ui.add_space(10.0);
                ui.label(RichText::new("Description").semibold());
                small_muted(ui, "A few words about this instance, shown on its page. It can be changed later.");
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
        });
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
                let loader = (self.create.loader != Loader::Vanilla).then(|| self.create.loader.label());
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
                "As in Settings: picked automatically each time the game starts. A change there applies here too."
                    .to_string()
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
        super::settings::wide_row(ui, "Memory", &hint, 300.0, |ui| {
            let labels: Vec<&str> = choices.iter().map(|c| c.1).collect();
            let current = choices.iter().position(|c| c.0 == e.memory).unwrap_or(0);
            if let Some(i) = segmented(ui, &labels, current) {
                e.memory = choices[i].0;
                if e.memory == Memory::Custom && e.memory_mb == 0 {
                    e.memory_mb = launcher_mb.max(1024);
                }
            }
        });
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

    /// The panel beside the form on a wide window: the instance as it will be (picture, name, version, loader,
    /// memory) and the button that makes it.
    fn create_side_panel(&mut self, ui: &mut Ui) -> bool {
        let mut create = false;
        let blocker = self.create_blocker();
        let creating = self.create_running();
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(RichText::new("What will be made").size(FS_SECTION).semibold());
            ui.add_space(6.0);
            let c = &self.create;
            let name = if !c.name.trim().is_empty() {
                c.name.trim().to_string()
            } else if c.mc.is_empty() {
                "My instance".to_string()
            } else {
                default_name(&c.mc, c.loader)
            };
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let (rect, _) = ui.allocate_exact_size(vec2(52.0, 52.0), egui::Sense::hover());
                self.paint_create_picture(ui, rect);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(egui::Label::new(RichText::new(&name).semibold()).wrap());
                    let line = if c.mc.is_empty() { "No version picked yet".to_string() } else { summary(c) };
                    ui.add(egui::Label::new(RichText::new(line).size(FS_META).color(c_muted())).wrap());
                });
            });
            ui.add_space(6.0);
            rule(ui);
            ui.add_space(2.0);
            let loader = match (c.loader, chosen_loader_version(c)) {
                (Loader::Vanilla, _) => "None".to_string(),
                (l, Some(v)) => format!("{} {v}", l.label()),
                (l, None) => l.label().to_string(),
            };
            let mut rows: Vec<(&str, String)> = vec![
                ("Minecraft", if c.mc.is_empty() { "\u{2014}".to_string() } else { c.mc.clone() }),
                ("Mod loader", loader),
                ("Memory", memory_summary(&c.extras)),
            ];
            if performance::offered_for(c.loader.modrinth_name()) {
                rows.push(("Performance mods", if c.performance { "Added" } else { "None" }.to_string()));
            }
            rows.push(("Picture", if c.extras.picture.is_some() { "Chosen" } else { "Default" }.to_string()));
            for (key, value) in rows {
                summary_row(ui, key, &value);
            }
            ui.add_space(8.0);
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
            let resp = primary_enabled(ui, "Create instance", blocker.is_none());
            if let Some(why) = &blocker {
                if !creating {
                    ui.add(egui::Label::new(RichText::new(why).size(FS_META).color(c_muted())).wrap());
                }
            }
            if resp.clicked() {
                create = true;
            }
        });
        create
    }

    /// The bar under the form on a narrower window: what is about to be made, and the button that makes it.
    fn create_bottom_bar(&mut self, ui: &mut Ui) -> bool {
        let mut create = false;
        let blocker = self.create_blocker();
        let creating = self.create_running();
        let summary = summary(&self.create);
        ui.scope(|ui| {
            // the button sits under the form, not at the far edge of a wide window
            ui.set_max_width((ui.available_width() - 14.0).min(FORM_W));
            ui.add_space(4.0);
            // a bar of glass under the form: what is about to be made, and the button that makes it
            card_frame().inner_margin(egui::Margin::symmetric(18, 12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                super::discover::dialog_footer(
                    ui,
                    "create",
                    |ui| {
                        if creating {
                            spinner(ui);
                            muted(ui, "Working on it, you can follow along in the sidebar");
                        } else {
                            small_muted(ui, summary);
                        }
                    },
                    |ui| {
                        let resp = primary_enabled(ui, "Create instance", blocker.is_none());
                        let resp = match &blocker {
                            Some(why) => resp.on_disabled_hover_text(why),
                            None => resp,
                        };
                        if resp.clicked() {
                            create = true;
                        }
                    },
                );
            });
        });
        create
    }

    /// Make the instance as the form says, in a sidebar task, and go to the library where it shows up.
    fn start_create(&mut self) {
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
            let mut ready = format!("\"{}\" is ready", inst.name());
            // the options of "More options": the instance is there already, so one that can't be applied is said
            // in the message rather than undoing it all
            if extras.changes_file() {
                extras.apply(&mut inst.file.launcher);
                if let Err(e) = inst.save() {
                    ready.push_str(&format!(". Its memory and description weren't saved: {e:#}"));
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
        c.extras = Extras::default();
        c.picture_preview = None;
        c.picture_error = None;
        self.page = Page::Library;
    }
}

/// One line of the summary panel: what, in grey, and its value at the right.
fn summary_row(ui: &mut Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).color(c_muted()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(RichText::new(value).font(super::fonts::medium(FS_BODY))).truncate());
        });
    });
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
        assert_eq!(e.brief(), "");
        let mut l = LauncherBlock { maximum_memory: Some(4096), ..Default::default() };
        e.apply(&mut l);
        // left as they are, nothing is touched
        assert_eq!((l.maximum_memory, l.automatic_memory, l.description.as_str()), (Some(4096), None, ""));
        e.description = "  With friends  ".into();
        e.memory = Memory::Custom;
        e.memory_mb = 6144;
        e.picture = Some(PathBuf::from("/pictures/castle.png"));
        assert!(e.changes_file());
        assert_eq!(e.brief(), "6 GB memory, picture, description");
        e.apply(&mut l);
        assert_eq!((l.maximum_memory, l.automatic_memory, l.description.as_str()), (Some(6144), None, "With friends"));
        e.memory = Memory::Automatic;
        e.apply(&mut l);
        assert_eq!((l.maximum_memory, l.automatic_memory), (None, Some(true)));
        assert_eq!(memory_summary(&e), "Automatic");
        // a picture alone needs no change to the file: it becomes instance.png
        let only_picture = Extras { picture: Some(PathBuf::from("a.png")), ..Default::default() };
        assert!(!only_picture.changes_file());
    }

    #[test]
    fn a_version_tooltip_says_what_and_when() {
        let v = version("1.21.1", "release");
        assert_eq!(version_tooltip(&v, true, false), "Release, 8 August 2024\nThe latest release");
        let b = ManifestVersion { release_time: "not a date".into(), ..version("b1.7.3", "old_beta") };
        assert_eq!(version_tooltip(&b, false, false), "Old beta");
    }
}
