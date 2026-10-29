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
    let color = if resp.hovered() { c_text() } else { c_muted() };
    let pts: [egui::Pos2; 3] = if open {
        [c + egui::vec2(-4.0, -2.0), c + egui::vec2(0.0, 2.5), c + egui::vec2(4.0, -2.0)]
    } else {
        [c + egui::vec2(-2.0, -4.0), c + egui::vec2(2.5, 0.0), c + egui::vec2(-2.0, 4.0)]
    };
    ui.painter().add(egui::Shape::line(pts.to_vec(), Stroke::new(1.6, color)));
    focus_ring(ui, &resp, rect, R_TAG);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, if open { "Collapse" } else { "Expand" }));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
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

        let (width, room) = super::curseforge::modal_size(ctx, 640.0, 250.0);
        let modal = egui::Modal::new(egui::Id::new("export-modal")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            use super::discover::*;
            ui.set_width(width);
            let Some(title) = self.export.dialog.as_ref().map(|d| d.title.clone()) else { return };
            if dialog_title(ui, &title) {
                close = true;
            }
            muted(ui, "Make a modpack file other people can import.");
            ui.add_space(12.0);

            // one scrolling part, so the buttons below it never leave the window
            ScrollArea::vertical().id_salt("export-body").max_height(room).auto_shrink([false, true]).show(ui, |ui| {
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

                egui::Grid::new("export-fields").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                    let field_width = (width - 90.0).clamp(160.0, 420.0);
                    let field = |ui: &mut Ui, label: &str, text: &mut String, hint: &str| {
                        muted(ui, label);
                        ui.add_sized([field_width, 32.0], egui::TextEdit::singleline(text).hint_text(RichText::new(hint).color(c_faint())).margin(egui::Margin::symmetric(10, 7)).background_color(c_field()));
                        ui.end_row();
                    };
                    field(ui, "Name", &mut d.name, "The name of the pack");
                    field(ui, "Version", &mut d.version, "1.0");
                    field(ui, "Author", &mut d.author, "Your name");
                    muted(ui, "Format");
                    let labels: Vec<&str> = ExportFormat::ALL.iter().map(|f| f.label()).collect();
                    let current = ExportFormat::ALL.iter().position(|f| *f == d.format).unwrap_or(0);
                    if let Some(i) = segmented(ui, &labels, current) {
                        d.format = ExportFormat::ALL[i];
                    }
                    ui.end_row();
                    muted(ui, "Save to");
                    ui.horizontal(|ui| {
                        ui.add_sized([(field_width - 170.0).max(120.0), 32.0], egui::TextEdit::singleline(&mut d.save_to).margin(egui::Margin::symmetric(10, 7)).background_color(c_field()));
                        if secondary(ui, "Browse").clicked() {
                            pick = true;
                        }
                        if ghost(ui, "Reset").clicked() {
                            reset_dir = true;
                        }
                    });
                    ui.end_row();
                });
                if d.picking {
                    small_muted(ui, "Waiting for the folder chooser");
                }

                ui.add_space(12.0);
                section(ui, "Folders and files to include");
                small_muted(ui, "Mods that Modrinth or CurseForge know are listed by id, so they don't make the file bigger.");
                egui::Frame::new().fill(c_field()).stroke(Stroke::new(1.0, c_border())).corner_radius(CornerRadius::same(R_CONTROL)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.tree_rows(ui, &root, "", 0);
                    if self.export.dialog.as_ref().is_some_and(|d| d.children.get("").is_some_and(Vec::is_empty)) {
                        small_muted(ui, "This instance has no files of its own to add yet, only its mod list.");
                    }
                });
                if let Some(e) = self.export.dialog.as_ref().and_then(|d| d.error.clone()) {
                    ui.add_space(8.0);
                    notice(ui, NoticeKind::Error, &e, None);
                }
            });

            let Some(d) = self.export.dialog.as_mut() else { return };
            ui.add_space(12.0);
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
                    }
                },
                |ui| {
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
