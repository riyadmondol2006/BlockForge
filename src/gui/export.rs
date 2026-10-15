//! The export window: name, version, author, format, where to save, and a tree of the instance's folders and
//! files to include. Afterwards, the list of files Modrinth's reviewers will ask permission for.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Align, CornerRadius, Layout, RichText, ScrollArea, Stroke, Ui};

use super::App;
use super::theme::*;
use super::widgets::*;
use crate::cf_seam::MinimalCurseForge;
use crate::export::{self, ExportFormat, ExportOptions, ExportOutcome, Selection, TreeEntry};

enum Event {
    Done(Result<ExportOutcome, String>),
    Picked(Option<PathBuf>),
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
            running: false,
            error: None,
            note: None,
            written: Vec::new(),
            picking: false,
        });
    }

    fn poll_export(&mut self) {
        while let Ok(ev) = self.export.rx.try_recv() {
            let Some(d) = self.export.dialog.as_mut() else { continue };
            match ev {
                Event::Picked(p) => {
                    d.picking = false;
                    if let Some(p) = p {
                        d.save_to = p.to_string_lossy().to_string();
                    }
                }
                Event::Done(Ok(outcome)) => {
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
                Event::Done(Err(e)) => {
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
        let (dirs, settings, tx, ctx) =
            (self.dirs.clone(), self.settings.clone(), self.export.tx.clone(), self.ctx.clone());
        let check_modrinth = !settings.extras.get("dontCheckModsOnModrinth").and_then(|v| v.as_bool()).unwrap_or(false);
        self.start_task(format!("Exporting {}", inst.name()), move |progress, _| async move {
            let mut inst = inst;
            let cf = MinimalCurseForge;
            progress.stage("Checking the mod list");
            let result: anyhow::Result<ExportOutcome> = async {
                let _ = export::scan_missing_mods(&mut inst, Some(&cf), check_modrinth).await;
                export::export(&dirs, &settings, &mut inst, &opts, &cf, &progress).await
            }
            .await;
            let reply = result.as_ref().map(|o| o.clone()).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Done(reply));
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
                ui.add_space(depth as f32 * 18.0);
                if e.is_dir {
                    if ui.small_button(if open { "\u{25BE}" } else { "\u{25B8}" }).clicked() {
                        toggle_open = true;
                    }
                } else {
                    ui.add_space(22.0);
                }
                let label = if e.is_dir { format!("{}/", e.name) } else { e.name.clone() };
                let text = if partial {
                    RichText::new(format!("{label}  (some)")).color(c_warn())
                } else {
                    RichText::new(label)
                };
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

        let modal = egui::Modal::new(egui::Id::new("export-modal")).show(ctx, |ui| {
            ui.set_width(600.0);
            let Some(d) = self.export.dialog.as_mut() else { return };
            ui.horizontal(|ui| {
                ui.label(RichText::new(&d.title).size(21.0).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if secondary(ui, "Close").clicked() {
                        close = true;
                    }
                });
            });
            muted(ui, "Make a modpack file other people can import.");
            ui.add_space(6.0);

            if d.note.is_some() {
                // Modrinth: which files were packed into the archive
                card(ui, |ui| {
                    ui.label(RichText::new("Overrides included").strong());
                    small_muted(ui, "Your exported instance contains mods not on Modrinth, so they were put in the overrides folder. If you upload this to Modrinth, make sure you have permission to distribute the mods below, or your modpack will be denied.");
                    let note = d.note.clone().unwrap_or_default();
                    ScrollArea::vertical().id_salt("override-note").max_height(140.0).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(&note).monospace().size(12.0)).wrap());
                    });
                    if secondary(ui, "Copy").clicked() {
                        copy_note = Some(note);
                    }
                });
                ui.add_space(6.0);
            }
            if !d.written.is_empty() {
                for p in &d.written {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Saved").color(c_accent()));
                        ui.label(p.display().to_string());
                    });
                }
                ui.add_space(6.0);
            }

            egui::Grid::new("export-fields").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                let field = |ui: &mut Ui, label: &str, text: &mut String, hint: &str| {
                    muted(ui, label);
                    ui.add_sized([420.0, 32.0], egui::TextEdit::singleline(text).hint_text(hint).margin(egui::Margin::symmetric(10, 6)).background_color(c_field()));
                    ui.end_row();
                };
                field(ui, "Name", &mut d.name, "The name of the pack");
                field(ui, "Version", &mut d.version, "1.0");
                field(ui, "Author", &mut d.author, "Your name");
                muted(ui, "Format");
                ui.horizontal(|ui| {
                    for f in ExportFormat::ALL {
                        if chip(ui, f.label(), d.format == f).clicked() {
                            d.format = f;
                        }
                    }
                });
                ui.end_row();
                muted(ui, "Save to");
                ui.horizontal(|ui| {
                    ui.add_sized([250.0, 32.0], egui::TextEdit::singleline(&mut d.save_to).margin(egui::Margin::symmetric(10, 6)).background_color(c_field()));
                    if secondary(ui, "Browse").clicked() {
                        pick = true;
                    }
                    if secondary(ui, "Reset").clicked() {
                        reset_dir = true;
                    }
                });
                ui.end_row();
            });
            if d.picking {
                small_muted(ui, "Waiting for the folder chooser");
            }

            ui.add_space(6.0);
            ui.label(RichText::new("Folders and files to include").strong());
            small_muted(ui, "Mods that Modrinth or CurseForge know are listed by id, so they don't make the file bigger.");
            egui::Frame::new().fill(c_field()).stroke(Stroke::new(1.0, c_border())).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ScrollArea::vertical().id_salt("export-tree").max_height(230.0).auto_shrink([false, true]).show(ui, |ui| {
                    self.tree_rows(ui, &root, "", 0);
                });
            });

            let Some(d) = self.export.dialog.as_mut() else { return };
            if let Some(e) = &d.error {
                ui.add_space(4.0);
                ui.label(RichText::new(e).color(c_danger()));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let ready = !d.running && !d.name.trim().is_empty() && !d.save_to.trim().is_empty();
                if primary_enabled(ui, if d.running { "Exporting" } else { "Export" }, ready).clicked() {
                    start = true;
                }
                if d.running {
                    ui.spinner();
                    muted(ui, "Working on it, you can follow along in the sidebar");
                }
            });
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
                    let (tx, c) = (self.export.tx.clone(), self.ctx.clone());
                    self.rt.spawn_blocking(move || {
                        let _ = tx.send(Event::Picked(super::import::pick_path_blocking(true)));
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
