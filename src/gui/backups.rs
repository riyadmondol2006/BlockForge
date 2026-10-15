//! Backups for the instance page: make a backup in one of the three modes, and restore or delete earlier ones.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::{Align, Layout, RichText, Ui};

use super::App;
use super::theme::*;
use super::widgets::*;
use crate::backups::{self, Backup, BackupMode};
use crate::instances::Instance;
use crate::svcsettings::ServiceSettings;

/// How many backups the card lists before "Show all".
const SHOWN: usize = 6;
/// The list is read again when it is older than this.
const REFRESH_AFTER: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct BackupsState {
    folder: String,
    list: Vec<Backup>,
    loaded: Option<Instant>,
    stale: bool,
    show_all: bool,
    restore: Option<PathBuf>,
    delete: Option<PathBuf>,
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
        let svc = ServiceSettings::from_settings(&self.settings);
        let st = &mut self.svc.backups;
        if st.folder != inst.folder || st.stale || st.loaded.is_none_or(|t| t.elapsed() > REFRESH_AFTER) {
            if st.folder != inst.folder {
                st.show_all = false;
            }
            st.folder = inst.folder.clone();
            st.list = backups::list(&self.dirs, &svc, inst.name());
            st.loaded = Some(Instant::now());
            st.stale = false;
        }
        let list: Vec<Backup> = st.list.clone();
        let show_all = st.show_all;
        let mut make: Option<BackupMode> = None;
        let mut restore = None;
        let mut delete = None;
        let mut toggle_all = false;
        let folder_to_open = backups::backups_dir(&self.dirs, &svc);

        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Backups").strong().size(16.0));
                if svc.enable_automatic_backup_after_launch {
                    tag(ui, "Automatic after playing", c_accent());
                }
            });
            small_muted(
                ui,
                "A backup is a zip file of this instance. Worlds and settings are small; adding the mods or everything makes it bigger.",
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for mode in BackupMode::ALL {
                    if secondary(ui, mode.label()).clicked() {
                        make = Some(mode);
                    }
                }
            });
            ui.add_space(6.0);
            if list.is_empty() {
                small_muted(ui, "No backups of this instance yet.");
            }
            for b in list.iter().take(if show_all { usize::MAX } else { SHOWN }) {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(when(b));
                        small_muted(ui, format!("{}  \u{00B7}  {}", human_size(b.size), b.file_name));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if danger(ui, "Delete").clicked() {
                            delete = Some(b.path.clone());
                        }
                        if secondary(ui, "Restore").clicked() {
                            restore = Some(b.path.clone());
                        }
                    });
                });
                ui.separator();
            }
            ui.horizontal(|ui| {
                if list.len() > SHOWN && secondary(ui, if show_all { "Show fewer" } else { "Show all" }).clicked() {
                    toggle_all = true;
                }
                if secondary_icon(ui, super::icons::Icon::Folder, "Open backups folder").clicked() {
                    let _ = std::fs::create_dir_all(&folder_to_open);
                    let _ = open::that(&folder_to_open);
                }
            });
        });

        if toggle_all {
            self.svc.backups.show_all = !show_all;
        }
        if let Some(path) = restore {
            self.svc.backups.restore = Some(path);
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
        self.start_task(format!("Backing up {name}"), move |_p, _n| async move {
            let made = tokio::task::spawn_blocking(move || backups::create(&dirs, &svc, &root, &name, mode)).await??;
            Ok(format!(
                "Backup is complete ({})",
                made.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
            ))
        });
    }

    /// The confirmation windows for restoring and deleting.
    pub(super) fn backups_overlays(&mut self, ctx: &egui::Context) {
        if let Some(path) = self.svc.backups.restore.clone() {
            let Some(inst) = self.selected.as_deref().and_then(|f| self.instance(f)).cloned() else {
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
                    let (dirs, svc) = (self.dirs.clone(), ServiceSettings::from_settings(&self.settings));
                    self.svc.backups.mark_stale();
                    self.start_task(format!("Restoring {}", inst.name()), move |_p, _n| async move {
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
