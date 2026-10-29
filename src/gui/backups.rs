//! Backups for the instance page: make a backup in one of the three modes, and restore or delete earlier ones.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::{RichText, Ui};

use super::App;
use super::instance_menu::{SplitRow, secondary_menu};
use super::theme::*;
use super::widgets::*;
use crate::backups::{self, Backup, BackupMode};
use crate::instances::Instance;
use crate::svcsettings::ServiceSettings;

/// How many backups the list shows before "Show all".
const SHOWN: usize = 4;
/// The width of the buttons at the end of a row: Restore and the menu.
const TRAIL_W: f32 = 112.0;
/// The list is read again when it is older than this.
const REFRESH_AFTER: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct BackupsState {
    folder: String,
    list: Vec<Backup>,
    /// The backup settings when the list was read (the folder and the automatic backup switch).
    svc: Option<ServiceSettings>,
    loaded: Option<Instant>,
    stale: bool,
    show_all: bool,
    /// The backup to restore and the folder of the instance it goes back into (the one whose card was clicked).
    restore: Option<(String, PathBuf)>,
    delete: Option<PathBuf>,
}

/// The label of the job that restores into an instance, also used to see whether one is running.
fn restore_label(name: &str) -> String {
    format!("Restoring {name}")
}

fn backup_label(name: &str) -> String {
    format!("Backing up {name}")
}

impl BackupsState {
    /// Read the list again at the next draw (after a backup was made or removed).
    pub fn mark_stale(&mut self) {
        self.stale = true;
    }
}

fn when(b: &Backup) -> String {
    b.created.format("%d %b %Y, %H:%M").to_string()
}

impl App {
    /// The card on the instance page's overview. Call it with the instance being shown.
    pub(super) fn backups_card(&mut self, ui: &mut Ui, inst: &Instance) {
        let st = &mut self.svc.backups;
        if st.folder != inst.folder || st.stale || st.loaded.is_none_or(|t| t.elapsed() > REFRESH_AFTER) {
            if st.folder != inst.folder {
                st.show_all = false;
            }
            let svc = ServiceSettings::from_settings(&self.settings);
            st.folder = inst.folder.clone();
            st.list = backups::list(&self.dirs, &svc, inst.name());
            st.svc = Some(svc);
            st.loaded = Some(Instant::now());
            st.stale = false;
        }
        // the list is handed back after drawing, so it isn't copied on every frame
        let list: Vec<Backup> = std::mem::take(&mut st.list);
        let svc = st.svc.clone().unwrap_or_default();
        let show_all = st.show_all;
        let playing = self.is_playing(&inst.folder);
        let working = {
            let (restoring, backing_up) = (restore_label(inst.name()), backup_label(inst.name()));
            self.tasks.iter().any(|t| t.label == restoring || t.label == backing_up)
        };
        let mut make: Option<BackupMode> = None;
        let mut restore = None;
        let mut delete = None;
        let mut toggle_all = false;
        let mut open_folder = false;
        let mut show: Option<PathBuf> = None;

        // the title, what happens by itself, and the way to make one
        let mut head = SplitRow::new(ui, egui::Id::unique("backups-head"), 32.0);
        head.left(ui, |ui| {
            ui.label(RichText::new("Backups").size(FS_SECTION).semibold());
            if svc.enable_automatic_backup_after_launch {
                ui.label(RichText::new("Automatic after playing").size(FS_META).color(c_faint()));
            }
        });
        head.right(ui, |ui| {
            if secondary_icon(ui, super::icons::Icon::Folder, "Open backups folder").clicked() {
                open_folder = true;
            }
            if working {
                // a backup or a restore of this instance is running: another one now would read half-written files
                spinner_sized(ui, 14.0);
                muted(ui, "A backup or restore of this instance is running\u{2026}");
            } else {
                let (_, picked) = secondary_menu(ui, "Back up now", |ui| {
                    let mut mode = None;
                    for m in BackupMode::ALL {
                        if ui.button(m.label()).clicked() {
                            mode = Some(m);
                            ui.close();
                        }
                    }
                    mode
                });
                if let Some(Some(mode)) = picked {
                    make = Some(mode);
                }
            }
        });
        head.finish(ui);
        small_muted(
            ui,
            "A backup is a zip file of this instance. Worlds and settings are small; adding the mods or everything makes it bigger.",
        );
        ui.add_space(4.0);
        if list.is_empty() {
            small_muted(ui, "No backups of this instance yet. Back up now makes the first one.");
        }
        let shown = list.iter().take(if show_all { usize::MAX } else { SHOWN });
        for (i, b) in shown.enumerate() {
            if i > 0 {
                rule(ui);
            }
            // the fill is put in behind the row once it is known whether the row is pointed at
            let fill = ui.painter().add(egui::Shape::Noop);
            let mut line = SplitRow::fixed(ui, 56.0, TRAIL_W);
            let row = line.rect();
            let t = super::motion::hover(ui, egui::Id::unique(("backup-row", &b.path)), ui.rect_contains_pointer(row));
            if t > 0.0 {
                let r = row.expand2(egui::vec2(8.0, 0.0));
                ui.painter().set(fill, egui::Shape::rect_filled(r, R_CONTROL, c_hover().gamma_multiply(t)));
            }
            line.left(ui, |ui| {
                // a small tile for the archive, then the date and the file
                let (tile, _) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::hover());
                ui.painter().rect_filled(tile, R_CONTROL, c_field());
                ui.painter().rect_stroke(tile, R_CONTROL, egui::Stroke::new(1.0, c_border()), egui::StrokeKind::Inside);
                super::icons::paint(ui.painter(), super::icons::Icon::Package, tile.center(), 16.0, c_muted());
                ui.add_space(4.0);
                // a long file name is cut short next to the buttons instead of running under them
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add_space(8.0);
                    ui.add(egui::Label::new(RichText::new(when(b)).font(super::fonts::medium(FS_BODY))).truncate());
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{}  \u{00B7}  {}", human_size(b.size), b.file_name))
                                .color(c_muted())
                                .size(FS_META),
                        )
                        .truncate(),
                    );
                });
            });
            line.right(ui, |ui| {
                // said before the question, not refused after it
                let why = if playing { "Close Minecraft first" } else { "Wait for the backup that is being made" };
                ui.add_enabled_ui(!working && !playing, |ui| {
                    if ghost(ui, "Restore").on_disabled_hover_text(why).clicked() {
                        restore = Some(b.path.clone());
                    }
                });
                if let Some(Some(act)) = kebab_menu(ui, |ui| {
                    let mut act = None;
                    if ui.button("Show in folder").clicked() {
                        act = Some(false);
                        ui.close();
                    }
                    ui.separator();
                    if ui.add_enabled(!working, egui::Button::new(RichText::new("Delete").color(c_danger()))).clicked()
                    {
                        act = Some(true);
                        ui.close();
                    }
                    act
                }) {
                    if act {
                        delete = Some(b.path.clone());
                    } else {
                        show = Some(b.path.clone());
                    }
                }
            });
            line.finish(ui);
        }
        if list.len() > SHOWN {
            let label = if show_all { "Show fewer".to_string() } else { format!("Show all {}", list.len()) };
            if ghost(ui, &label).clicked() {
                toggle_all = true;
            }
        }

        self.svc.backups.list = list;
        if open_folder {
            self.open_folder(&backups::backups_dir(&self.dirs, &svc));
        }
        if toggle_all {
            self.svc.backups.show_all = !show_all;
        }
        if let Some(dir) = show.as_deref().and_then(|p| p.parent()) {
            self.open_folder(dir);
        }
        if let Some(path) = restore {
            self.svc.backups.restore = Some((inst.folder.clone(), path));
        }
        if let Some(path) = delete {
            self.svc.backups.delete = Some(path);
        }
        if let Some(mode) = make {
            self.start_backup(inst, mode);
        }
    }

    /// Make a backup in the background.
    pub(super) fn start_backup(&mut self, inst: &Instance, mode: BackupMode) {
        let (dirs, svc) = (self.dirs.clone(), ServiceSettings::from_settings(&self.settings));
        let (root, name) = (inst.root.clone(), inst.name().to_string());
        self.svc.backups.mark_stale();
        self.start_task(backup_label(&name), move |_p, _n| async move {
            let made = tokio::task::spawn_blocking(move || backups::create(&dirs, &svc, &root, &name, mode)).await??;
            Ok(format!(
                "Backup is complete ({})",
                made.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
            ))
        });
    }

    /// The confirmation windows for restoring and deleting.
    pub(super) fn backups_overlays(&mut self, ctx: &egui::Context) {
        if let Some((folder, path)) = self.svc.backups.restore.clone() {
            // the instance whose card was clicked, even if another one is selected by now
            let Some(inst) = self.instance(&folder).cloned() else {
                self.svc.backups.restore = None;
                return;
            };
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let body = format!(
                "{name} will be put back into \"{}\". What it holds (for example worlds and settings) replaces what is there now. What is replaced is saved as a new backup first, so nothing is lost.",
                inst.name()
            );
            match super::services::confirm_modal(ctx, "restore-backup", "Restore this backup?", &body, "Restore", false)
            {
                Some(true) => {
                    self.svc.backups.restore = None;
                    if self.is_playing(&inst.folder) {
                        self.toast_err("Close Minecraft first, it is using these files");
                        return;
                    }
                    let (restoring, backing_up) = (restore_label(inst.name()), backup_label(inst.name()));
                    if self.tasks.iter().any(|t| t.label == restoring || t.label == backing_up) {
                        self.toast_err("Wait for the backup or restore of this instance to finish first");
                        return;
                    }
                    let (dirs, svc) = (self.dirs.clone(), ServiceSettings::from_settings(&self.settings));
                    self.svc.backups.mark_stale();
                    self.start_task(restoring, move |_p, _n| async move {
                        let label = inst.name().to_string();
                        tokio::task::spawn_blocking(move || {
                            backups::restore(&dirs, &svc, &path, &inst.root, inst.name(), true)
                        })
                        .await??;
                        Ok(format!("Restored {label}"))
                    });
                }
                Some(false) => self.svc.backups.restore = None,
                None => {}
            }
        }
        if let Some(path) = self.svc.backups.delete.clone() {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            match super::services::confirm_modal(
                ctx,
                "delete-backup",
                "Delete this backup?",
                &format!("{name} will be removed from this computer. This can't be undone."),
                "Delete",
                true,
            ) {
                Some(true) => {
                    self.svc.backups.delete = None;
                    let svc = ServiceSettings::from_settings(&self.settings);
                    match backups::delete(&self.dirs, &svc, &path) {
                        Ok(()) => self.toast("Backup deleted"),
                        Err(e) => self.toast_err(format!("{e:#}")),
                    }
                    self.svc.backups.mark_stale();
                }
                Some(false) => self.svc.backups.delete = None,
                None => {}
            }
        }
    }
}
