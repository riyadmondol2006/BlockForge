//! The export window: name, version, author, format, where to save, and a tree of the instance's folders and
//! files to include. Afterwards, the list of files Modrinth's reviewers will ask permission for.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{CornerRadius, RichText, ScrollArea, Stroke, Ui};

use super::App;
use super::theme::*;
use super::widgets::*;
use crate::cf_seam::MinimalCurseForge;
use crate::export::{self, ExportFormat, ExportOptions, ExportOutcome, Selection, TreeEntry};

/// Answers from background work, with the folder of the instance whose window asked: the window may have been
/// closed and opened for another instance since.
enum Event {
    Done(String, Result<ExportOutcome, String>),
    Picked(String, Option<PathBuf>),
}

/// The state of one open export window.
struct Dialog {
    folder: String,
    title: String,
    name: String,
    version: String,
    author: String,
    format: ExportFormat,
    save_to: String,
    selection: Selection,
    expanded: HashSet<String>,
    children: HashMap<String, Vec<TreeEntry>>,
    running: bool,
    error: Option<String>,
    /// Shown after Modrinth exports that had to pack mods into the archive.
    note: Option<String>,
    written: Vec<PathBuf>,
    picking: bool,
    /// What the selection holds, worked out again only when the selection changes: the selection it is for, and
    /// the counts.
    contents: Option<(Vec<String>, Contents)>,
}

pub struct ExportState {
    dialog: Option<Dialog>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Default for ExportState {
    fn default() -> Self {
        let (tx, rx) = channel();
        ExportState { dialog: None, tx, rx }
    }
}

/// How far each level of the tree is moved in.
const INDENT: f32 = 16.0;
/// The room the arrow in front of a folder takes.
const CHEVRON: f32 = 22.0;

/// The small arrow in front of a folder: points right when closed, down when open.
fn chevron(ui: &mut Ui, open: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(CHEVRON, CHEVRON), egui::Sense::click());
    let c = rect.center();
    let t = super::motion::hover(ui, resp.id, resp.hovered());
    if t > 0.0 {
        ui.painter().rect_filled(rect, CornerRadius::same(R_TAG), c_hover().gamma_multiply(t));
    }
    let color = super::motion::mix(c_muted(), c_text(), t);
    // the chevron turns from pointing right (folded) to pointing down (open)
    let a =
        super::motion::toward(ui.ctx(), resp.id.with("open"), open, super::motion::FAST) * std::f32::consts::FRAC_PI_2;
    let rot = |x: f32, y: f32| c + egui::vec2(x * a.cos() - y * a.sin(), x * a.sin() + y * a.cos());
    let pts = vec![rot(-2.0, -4.0), rot(2.5, 0.0), rot(-2.0, 4.0)];
    ui.painter().add(egui::Shape::line(pts, Stroke::new(1.6, color)));
    focus_ring(ui, &resp, rect, R_TAG);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, if open { "Collapse" } else { "Expand" }));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// What each format makes, in a few words beside the choice.
fn format_blurb(f: ExportFormat) -> &'static str {
    match f {
        ExportFormat::CurseForge => "A .zip for the CurseForge app",
        ExportFormat::Modrinth => "An .mrpack for Modrinth",
        ExportFormat::CurseForgeAndModrinth => "A CurseForge .zip and a Modrinth .mrpack",
        ExportFormat::MultiMc => "A .zip for MultiMC and Prism",
    }
}

/// The name of a format on its segment, short enough for the four to sit on one row.
fn format_short(f: ExportFormat) -> &'static str {
    match f {
        ExportFormat::CurseForgeAndModrinth => "Both",
        f => f.label(),
    }
}

/// The least room the tree of files gets, so what is exported can always be seen and changed.
const TREE_MIN: f32 = 180.0;
/// The labels in front of the fields, as wide as each other so the fields line up.
const LABEL_W: f32 = 58.0;
/// The narrowest window the name, version and author fit side by side in; a narrower one has them one under the
/// other.
const COMPACT_MIN: f32 = 520.0;

/// What an export takes from the instance's folder: the files in all, the folders at the top they come from, their
/// size, and whether worlds or the game's settings are among them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Contents {
    files: usize,
    folders: usize,
    bytes: u64,
    worlds: bool,
    settings: bool,
}

/// The game's own settings files at the top of an instance: options, keys and video, and the list of servers.
fn is_settings_file(rel: &str) -> bool {
    let name = rel.to_ascii_lowercase();
    (name.starts_with("options") && name.ends_with(".txt")) || name == "servers.dat"
}

/// What `selection` takes from the instance in `root` (see [`Contents`]).
fn contents(root: &std::path::Path, selection: &Selection) -> Contents {
    let mut c = Contents::default();
    for e in export::list_dir(root, "") {
        let checked = selection.is_checked(&e.rel);
        if !checked && !selection.is_partial(&e.rel) {
            continue;
        }
        if e.rel.eq_ignore_ascii_case("saves") {
            c.worlds = true;
        }
        if !e.is_dir {
            c.files += 1;
            c.bytes += e.size;
            c.settings |= is_settings_file(&e.rel);
            continue;
        }
        c.folders += 1;
        count_under(root, &e.rel, selection, checked, &mut c);
    }
    c
}

/// Add the files under the folder `rel` to `c`: all of them when the folder is ticked (`all`), otherwise the ticked
/// ones.
fn count_under(root: &std::path::Path, rel: &str, selection: &Selection, all: bool, c: &mut Contents) {
    for e in export::list_dir(root, rel) {
        let checked = all || selection.is_checked(&e.rel);
        if !checked && !selection.is_partial(&e.rel) {
            continue;
        }
        if e.is_dir {
            count_under(root, &e.rel, selection, checked, c);
        } else {
            c.files += 1;
            c.bytes += e.size;
        }
    }
}

/// The line beside Export: how much of the instance goes in, and whether that includes worlds or settings.
fn contents_text(c: &Contents) -> String {
    let files = if c.files == 1 { "1 file".to_string() } else { format!("{} files", c.files) };
    let mut text = match (c.files, c.folders) {
        (0, _) => "Only the mod list".to_string(),
        (_, 0) => format!("{files} \u{00B7} {}", human_size(c.bytes)),
        (_, 1) => format!("{files} from 1 folder \u{00B7} {}", human_size(c.bytes)),
        (_, n) => format!("{files} from {n} folders \u{00B7} {}", human_size(c.bytes)),
    };
    let includes: Vec<&str> = [(c.worlds, "worlds"), (c.settings, "settings")]
        .into_iter()
        .filter_map(|(on, what)| on.then_some(what))
        .collect();
    if !includes.is_empty() {
        text.push_str(&format!(" \u{00B7} Includes {}", includes.join(" and ")));
    }
    text
}

/// The width a label takes in the body font.
fn text_width(ui: &Ui, text: &str) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(FS_BODY), c_muted()).size().x
}

/// A label in front of a field, centred on the field's height and `width` wide.
fn field_label(ui: &mut Ui, text: &str, width: f32) {
    ui.allocate_ui_with_layout(egui::vec2(width, 32.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(width);
        muted(ui, text);
    });
}

/// A one-line field of the export window.
fn field(ui: &mut Ui, text: &mut String, hint: &str, width: f32) {
    ui.add_sized(
        [width, 32.0],
        egui::TextEdit::singleline(text)
            .hint_text(RichText::new(hint).color(c_faint()))
            .margin(egui::Margin::symmetric(10, 7))
            .background_color(c_field()),
    );
}

impl App {
    /// Open the export window for the instance in `folder`.
    pub fn open_export(&mut self, folder: &str) {
        let Some(inst) = self.instance(folder).cloned() else { return };
        if !export::can_be_exported(&inst) {
            self.toast_err(
                "This instance has no mod loader, so it can't be exported. Add Fabric, Forge, NeoForge or Quilt first.",
            );
            return;
        }
        let account = self.current_account().map(|a| a.name().to_string());
        let o = export::default_options(&inst, account.as_deref(), &self.settings);
        // the window was closed during an export of this instance: that export still runs
        let running = self.tasks.iter().any(|t| t.label == format!("Exporting {}", inst.name()));
        self.export.dialog = Some(Dialog {
            folder: folder.to_string(),
            title: format!("Export {}", inst.name()),
            name: o.name,
            version: o.version,
            author: o.author,
            format: o.format,
            save_to: o.save_to.to_string_lossy().to_string(),
            selection: Selection::new(o.overrides),
            expanded: HashSet::new(),
            children: HashMap::new(),
            running,
            error: None,
            note: None,
            written: Vec::new(),
            picking: false,
            contents: None,
        });
    }

    fn poll_export(&mut self) {
        while let Ok(ev) = self.export.rx.try_recv() {
            let folder = match &ev {
                Event::Done(f, _) | Event::Picked(f, _) => f.clone(),
            };
            let Some(d) = self.export.dialog.as_mut().filter(|d| d.folder == folder) else {
                // the window is gone or shows another instance; the sidebar task already told how it went
                continue;
            };
            match ev {
                Event::Picked(_, p) => {
                    d.picking = false;
                    if let Some(p) = p {
                        d.save_to = p.to_string_lossy().to_string();
                    }
                }
                Event::Done(_, Ok(outcome)) => {
                    d.running = false;
                    d.error = None;
                    d.written = outcome.paths.clone();
                    if !outcome.overrides_note.is_empty() {
                        d.note = Some(outcome.overrides_note.clone());
                    }
                    let shown = outcome.path.clone();
                    self.reload_instances();
                    self.toast("Exported the instance");
                    if let Some(dir) = shown.parent() {
                        let _ = open::that(dir);
                    }
                }
                Event::Done(_, Err(e)) => {
                    d.running = false;
                    d.error = Some(e);
                }
            }
        }
    }

    fn start_export(&mut self) {
        let Some(d) = self.export.dialog.as_mut() else { return };
        let Some(inst) = self.instances.iter().find(|i| i.folder == d.folder).cloned() else { return };
        d.running = true;
        d.error = None;
        d.note = None;
        let opts = ExportOptions {
            name: d.name.trim().to_string(),
            version: d.version.trim().to_string(),
            author: d.author.trim().to_string(),
            format: d.format,
            save_to: PathBuf::from(d.save_to.trim()),
            overrides: d.selection.to_overrides(),
        };
        let (dirs, settings, tx, ctx, folder) =
            (self.dirs.clone(), self.settings.clone(), self.export.tx.clone(), self.ctx.clone(), d.folder.clone());
        let check_modrinth = !settings.extras.get("dontCheckModsOnModrinth").and_then(|v| v.as_bool()).unwrap_or(false);
        let lock = super::mods::instance_lock(&inst.root);
        self.start_task(format!("Exporting {}", inst.name()), move |progress, _| async move {
            // looking up unknown mods saves what it learns into the instance, so it waits for other changes
            let _turn = lock.lock().await;
            let mut inst = crate::instances::load_one(&inst.root).unwrap_or(inst);
            let cf = MinimalCurseForge;
            progress.stage("Checking the mod list");
            let result: anyhow::Result<ExportOutcome> = async {
                let _ = export::scan_missing_mods(&mut inst, Some(&cf), check_modrinth).await;
                export::export(&dirs, &settings, &mut inst, &opts, &cf, &progress).await
            }
            .await;
            let reply = result.as_ref().map(|o| o.clone()).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Done(folder, reply));
            ctx.request_repaint();
            result.map(|o| format!("Exported to {}", o.path.display()))
        });
    }

    /// One row of the tree and, when expanded, its children.
    fn tree_rows(&mut self, ui: &mut Ui, root: &std::path::Path, rel: &str, depth: usize) {
        let entries = {
            let Some(d) = self.export.dialog.as_mut() else { return };
            d.children.entry(rel.to_string()).or_insert_with(|| export::list_dir(root, rel)).clone()
        };
        for e in entries {
            let Some(d) = self.export.dialog.as_mut() else { return };
            let mut checked = d.selection.is_checked(&e.rel);
            let partial = d.selection.is_partial(&e.rel);
            let open = d.expanded.contains(&e.rel);
            let mut toggle_open = false;
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.add_space(depth as f32 * INDENT);
                if e.is_dir {
                    if chevron(ui, open).clicked() {
                        toggle_open = true;
                    }
                } else {
                    ui.add_space(CHEVRON);
                }
                let label = if e.is_dir { format!("{}/", e.name) } else { e.name.clone() };
                let text = if partial { RichText::new(format!("{label}  (some)")) } else { RichText::new(label) };
                changed = ui.checkbox(&mut checked, text).changed();
                if !e.is_dir {
                    small_muted(ui, human_size(e.size));
                }
            });
            if changed {
                d.selection.set(root, &e.rel, checked);
            }
            if toggle_open {
                if open {
                    d.expanded.remove(&e.rel);
                } else {
                    d.expanded.insert(e.rel.clone());
                }
            }
            if e.is_dir && self.export.dialog.as_ref().is_some_and(|d| d.expanded.contains(&e.rel)) {
                self.tree_rows(ui, root, &e.rel, depth + 1);
            }
        }
    }

    #[allow(deprecated)]
    /// The export window. Call it every frame.
    pub(super) fn export_modal(&mut self, ctx: &egui::Context) {
        self.poll_export();
        let Some(folder) = self.export.dialog.as_ref().map(|d| d.folder.clone()) else { return };
        let Some(root) = self.instance(&folder).map(|i| i.root.clone()) else {
            self.export.dialog = None;
            return;
        };
        let mut close = false;
        let mut start = false;
        let mut pick = false;
        let mut copy_note: Option<String> = None;
        let mut reset_dir = false;

        // a short window loses the line under the title, so the files to include keep their room
        let short = ctx.content_rect().height() < 650.0;
        let (width, room) = super::curseforge::modal_size(ctx, 640.0, if short { 200.0 } else { 250.0 });
        let modal = egui::Modal::new(egui::Id::new("export-modal")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            use super::discover::*;
            ui.set_width(width);
            let Some(title) = self.export.dialog.as_ref().map(|d| d.title.clone()) else { return };
            if dialog_title(ui, &title) {
                close = true;
            }
            if !short {
                muted(ui, "Make a modpack file other people can import.");
            }
            ui.add_space(if short { 8.0 } else { 16.0 });

            // one scrolling part, so the buttons below it never leave the window
            ScrollArea::vertical().id_salt("export-body").max_height(room).auto_shrink([false, true]).show(ui, |ui| {
                let body_top = ui.cursor().top();
                let Some(d) = self.export.dialog.as_mut() else { return };
                if let Some(note) = d.note.clone() {
                    // Modrinth: which files were packed into the archive
                    let text = "Mods that are not on Modrinth were put in the overrides folder. If you upload this to Modrinth, make sure you have permission to distribute them, or your modpack will be denied.";
                    if notice(ui, NoticeKind::Info, text, Some("Copy list")).is_some_and(|r| r.clicked()) {
                        copy_note = Some(note.clone());
                    }
                    egui::CollapsingHeader::new("Show the files").id_salt("override-files").show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(&note).monospace()).wrap());
                    });
                    ui.add_space(6.0);
                }
                for p in &d.written {
                    ui.horizontal_wrapped(|ui| {
                        check_label(ui, "Saved");
                        ui.label(p.display().to_string());
                    });
                }
                if !d.written.is_empty() {
                    ui.add_space(6.0);
                }

                // the name, version and author on one row and where to save on the next, so the files to include
                // keep their room; one field a row in a narrow window
                if ui.available_width() >= COMPACT_MIN {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let (version_w, author_w) = (96.0, 150.0);
                        let labels = text_width(ui, "Version") + text_width(ui, "Author");
                        let name_w = (ui.available_width() - LABEL_W - labels - version_w - author_w - 5.0 * 8.0).max(120.0);
                        field_label(ui, "Name", LABEL_W - 8.0);
                        field(ui, &mut d.name, "The name of the pack", name_w);
                        muted(ui, "Version");
                        field(ui, &mut d.version, "1.0", version_w);
                        muted(ui, "Author");
                        field(ui, &mut d.author, "Your name", author_w);
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        field_label(ui, "Save to", LABEL_W - 8.0);
                        let buttons = remembered_width(ui, ui.scope_id().with("save-to-buttons"), 150.0);
                        let w = (ui.available_width() - buttons - 8.0).max(120.0);
                        field(ui, &mut d.save_to, "", w);
                        let used = ui
                            .horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                if secondary(ui, "Browse").clicked() {
                                    pick = true;
                                }
                                if ghost(ui, "Reset").clicked() {
                                    reset_dir = true;
                                }
                            })
                            .response
                            .rect
                            .width();
                        remember_width(ui, ui.scope_id().with("save-to-buttons"), used);
                    });
                } else {
                    egui::Grid::new("export-fields").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                        let w = (ui.available_width() - LABEL_W).max(120.0);
                        for (label, text, hint) in [
                            ("Name", &mut d.name, "The name of the pack"),
                            ("Version", &mut d.version, "1.0"),
                            ("Author", &mut d.author, "Your name"),
                        ] {
                            muted(ui, label);
                            field(ui, text, hint, w);
                            ui.end_row();
                        }
                        muted(ui, "Save to");
                        ui.horizontal_wrapped(|ui| {
                            field(ui, &mut d.save_to, "", (w - 150.0).max(120.0));
                            if secondary(ui, "Browse").clicked() {
                                pick = true;
                            }
                            if ghost(ui, "Reset").clicked() {
                                reset_dir = true;
                            }
                        });
                        ui.end_row();
                    });
                }
                if d.picking {
                    small_muted(ui, "Waiting for the folder chooser");
                }

                // the four formats as one row of segments, what the chosen one makes beside them
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    field_label(ui, "Format", LABEL_W - 8.0);
                    let labels = ExportFormat::ALL.map(format_short);
                    let now = ExportFormat::ALL.iter().position(|f| *f == d.format).unwrap_or(usize::MAX);
                    if let Some(i) = segmented(ui, &labels, now) {
                        d.format = ExportFormat::ALL[i];
                    }
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(RichText::new(format_blurb(d.format)).size(FS_META).color(c_muted())).truncate());
                });

                ui.add_space(10.0);
                // the title of the tree, with how much of the instance is in, and buttons for all or nothing
                d.children.entry(String::new()).or_insert_with(|| export::list_dir(&root, ""));
                let top: Vec<String> = d.children.get("").map(|c| c.iter().map(|e| e.rel.clone()).collect()).unwrap_or_default();
                let included = top.iter().filter(|r| d.selection.is_checked(r) || d.selection.is_partial(r)).count();
                let mut all = false;
                let mut none = false;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new("Folders and files to include").semibold()).on_hover_text(
                        "Mods that Modrinth or CurseForge know are listed by id, so they don't make the file bigger.",
                    );
                    if !top.is_empty() {
                        small_muted(ui, format!("{included} of {} in the pack", top.len()));
                        let id = ui.scope_id().with("tree-buttons");
                        end_aligned(ui, id, |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if ghost(ui, "Select all").on_hover_text("Include every folder and file").clicked() {
                                all = true;
                            }
                            if ghost(ui, "Select none").on_hover_text("Include only the mod list").clicked() {
                                none = true;
                            }
                        });
                    }
                });
                if all {
                    d.selection = Selection::new(top.iter().cloned());
                }
                if none {
                    d.selection = Selection::new(Vec::new());
                }
                ui.add_space(4.0);
                // the tree on a well of its own, scrolling on its own in the room that is left (never less than
                // TREE_MIN), so its rows are in sight when the window opens
                let used = ui.cursor().top() - body_top;
                let error_room = if d.error.is_some() { 60.0 } else { 0.0 };
                let tree_h = (room - used - error_room - 22.0).max(TREE_MIN);
                egui::Frame::new().fill(c_card()).stroke(Stroke::new(1.0, c_border())).corner_radius(CornerRadius::same(R_CARD)).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ScrollArea::vertical().id_salt("export-tree").max_height(tree_h).auto_shrink([false, false]).show(ui, |ui| {
                        self.tree_rows(ui, &root, "", 0);
                        if self.export.dialog.as_ref().is_some_and(|d| d.children.get("").is_some_and(Vec::is_empty)) {
                            small_muted(ui, "This instance has no files of its own to add yet, only its mod list.");
                        }
                    });
                });
                if let Some(e) = self.export.dialog.as_ref().and_then(|d| d.error.clone()) {
                    ui.add_space(8.0);
                    notice(ui, NoticeKind::Error, &e, None);
                }
            });

            let Some(d) = self.export.dialog.as_mut() else { return };
            ui.add_space(if short { 10.0 } else { 16.0 });
            let ready = !d.running && !d.name.trim().is_empty() && !d.save_to.trim().is_empty();
            let reason = if d.running {
                ""
            } else if d.name.trim().is_empty() {
                "Give the pack a name"
            } else if d.save_to.trim().is_empty() {
                "Choose where to save it"
            } else {
                ""
            };
            // what goes in, worked out again when the selection changes
            let overrides = d.selection.to_overrides();
            if d.contents.as_ref().is_none_or(|(for_, _)| *for_ != overrides) {
                d.contents = Some((overrides, contents(&root, &d.selection)));
            }
            let summary = d.contents.as_ref().map(|(_, c)| contents_text(c)).unwrap_or_default();
            let running = d.running;
            dialog_footer(
                ui,
                "export",
                |ui| {
                    if running {
                        spinner(ui);
                        muted(ui, "Working on it, you can follow along in the sidebar");
                    } else if !reason.is_empty() {
                        small_muted(ui, reason);
                    } else {
                        ui.label(RichText::new(&summary).size(FS_META).color(c_muted()));
                    }
                },
                |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if secondary(ui, "Cancel").clicked() {
                        close = true;
                    }
                    if primary_enabled(ui, if running { "Exporting" } else { "Export" }, ready).clicked() {
                        start = true;
                    }
                },
            );
        });
        if modal.should_close() {
            close = true;
        }
        if let Some(text) = copy_note {
            ctx.copy_text(text);
            self.toast("Copied");
        }
        if reset_dir {
            if let Some(d) = self.export.dialog.as_mut() {
                d.save_to = root.to_string_lossy().to_string();
            }
        }
        if pick {
            if let Some(d) = self.export.dialog.as_mut() {
                if !d.picking {
                    d.picking = true;
                    let (tx, c, folder) = (self.export.tx.clone(), self.ctx.clone(), d.folder.clone());
                    self.rt.spawn_blocking(move || {
                        let _ = tx.send(Event::Picked(folder, super::import::pick_path_blocking(true)));
                        c.request_repaint();
                    });
                }
            }
        }
        if start {
            self.start_export();
        }
        if close {
            self.export.dialog = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("bf-export-contents-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (rel, bytes) in [
            ("mods/a.jar", 100),
            ("mods/b.jar", 50),
            ("config/a.toml", 10),
            ("saves/World/level.dat", 1000),
            ("options.txt", 5),
            ("notes.md", 1),
        ] {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, vec![0u8; bytes]).unwrap();
        }
        root
    }

    #[test]
    fn the_counts_beside_export_follow_the_selection() {
        let root = instance("counts");
        let mods_and_config = Selection::new(["mods".to_string(), "config".to_string()]);
        let c = contents(&root, &mods_and_config);
        assert_eq!((c.files, c.folders, c.bytes, c.worlds, c.settings), (3, 2, 160, false, false));
        assert_eq!(contents_text(&c), format!("3 files from 2 folders \u{00B7} {}", human_size(160)));

        let mut everything = Selection::new(["mods", "config", "saves", "options.txt", "notes.md"].map(String::from));
        let c = contents(&root, &everything);
        assert_eq!((c.files, c.folders, c.worlds, c.settings), (6, 3, true, true));
        assert!(contents_text(&c).ends_with("Includes worlds and settings"));

        // one file left out of a ticked folder
        everything.set(&root, "mods/b.jar", false);
        assert_eq!(contents(&root, &everything).files, 5);

        let nothing = contents(&root, &Selection::new(Vec::new()));
        assert_eq!(contents_text(&nothing), "Only the mod list");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_game_settings_files_are_told_apart() {
        assert!(
            is_settings_file("options.txt") && is_settings_file("optionsof.txt") && is_settings_file("servers.dat")
        );
        assert!(!is_settings_file("config") && !is_settings_file("options.json"));
    }
}
