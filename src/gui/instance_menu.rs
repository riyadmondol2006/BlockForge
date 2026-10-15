//! The instance menu and the small dialogs behind it (description, play offline as another name, mod loader),
//! shared by the cards, the home page and the instance page.

use std::path::PathBuf;

use egui::{RichText, Ui};

use super::theme::*;
use super::widgets::*;
use super::{App, InstTab, Msg, Page};
use crate::instances::{self, Instance};
use crate::launch::LaunchOptions;
use crate::loaders::{self, Loader};

/// What the player asked an instance to do.
pub(super) enum Act {
    Open(String),
    Play(String),
    PlayOffline(String),
    Stop(String),
    Folder(String),
    Mods(String),
    AddMods(String),
    Settings(String),
    Rename(String),
    ChangeImage(String),
    ChangeDescription(String),
    Duplicate(String),
    Export(String),
    ChangeLoader(String),
    Delete(String),
}

/// What a file chooser was opened for.
#[derive(Clone, Debug)]
pub enum PickPurpose {
    InstanceImage(String),
    JavaFolder,
}

pub struct LoaderDialog {
    folder: String,
    loader: Loader,
    versions: Vec<String>,
    /// Empty means the newest.
    version: String,
    loading: bool,
    requested: Option<(Loader, String)>,
    error: Option<String>,
}

impl App {
    pub(super) fn apply(&mut self, act: Act) {
        match act {
            Act::Open(f) => self.open_instance(&f),
            Act::Play(f) => self.play(&f, LaunchOptions::default()),
            Act::PlayOffline(f) => self.offline_play = Some((f, String::new(), false)),
            Act::Stop(f) => self.stop(&f),
            Act::Folder(f) => {
                if let Some(i) = self.instance(&f) {
                    let _ = open::that(&i.root);
                }
            }
            Act::Mods(f) => {
                self.open_instance(&f);
                self.tab = InstTab::Mods;
                self.mods_cache = None;
            }
            Act::AddMods(f) => {
                self.discover.kind = crate::modrinth::Kind::Mod;
                self.discover.target = Some(f);
                self.page = Page::Discover;
                self.discover_search(true);
            }
            Act::Settings(f) => {
                self.open_instance(&f);
                self.tab = InstTab::Settings;
            }
            Act::Rename(f) => {
                if let Some(i) = self.instance(&f) {
                    self.rename_target = Some((f.clone(), i.name().to_string()));
                }
            }
            Act::ChangeImage(f) => self.pick_file(PickPurpose::InstanceImage(f)),
            Act::ChangeDescription(f) => {
                if let Some(i) = self.instance(&f) {
                    self.desc_edit = Some((f.clone(), i.file.launcher.description.clone()));
                }
            }
            Act::Duplicate(f) => {
                if let Some(inst) = self.instance(&f).cloned() {
                    let name = format!("{} copy", inst.name());
                    let dirs = self.dirs.clone();
                    self.start_task(format!("Copying {}", inst.name()), move |_, _| async move {
                        tokio::task::spawn_blocking(move || instances::duplicate(&dirs, &inst, &name)).await??;
                        Ok("Instance copied".to_string())
                    });
                }
            }
            Act::ChangeLoader(f) => {
                if let Some(i) = self.instance(&f) {
                    let loader = i.loader().and_then(|(k, _)| Loader::parse(k)).unwrap_or(Loader::Vanilla);
                    self.loader_dialog = Some(LoaderDialog {
                        folder: f.clone(),
                        loader,
                        versions: Vec::new(),
                        version: String::new(),
                        loading: false,
                        requested: None,
                        error: None,
                    });
                    self.loader_dialog_request();
                }
            }
            Act::Export(f) => self.open_export(&f),
            Act::Delete(f) => self.confirm_delete = Some(f),
        }
    }

    /// The items of the "more" menu, in the Java edition's order. Returns what was clicked.
    pub(super) fn instance_menu(&self, ui: &mut Ui, inst: &Instance) -> Option<Act> {
        let f = inst.folder.clone();
        let mut act = None;
        ui.set_min_width(220.0);
        let mut item = |ui: &mut Ui, label: &str, make: fn(String) -> Act| {
            if ui.button(label).clicked() {
                act = Some(make(f.clone()));
                ui.close();
            }
        };
        if !self.accounts.is_empty() {
            item(ui, "Play offline with another name", Act::PlayOffline);
            ui.separator();
        }
        item(ui, "Settings", Act::Settings);
        item(ui, "Rename", Act::Rename);
        item(ui, "Clone", Act::Duplicate);
        item(ui, "Export", Act::Export);
        item(ui, "Change image", Act::ChangeImage);
        item(ui, "Change description", Act::ChangeDescription);
        ui.separator();
        item(ui, "Mod loader", Act::ChangeLoader);
        item(ui, "Add mods", Act::AddMods);
        item(ui, "Edit mods", Act::Mods);
        ui.separator();
        item(ui, "Open folder", Act::Folder);
        ui.separator();
        if ui.button(RichText::new("Delete instance").color(c_danger())).clicked() {
            act = Some(Act::Delete(inst.folder.clone()));
            ui.close();
        }
        act
    }

    /// The instance's picture: its own `instance.png` if it has one, otherwise a coloured letter.
    pub(super) fn instance_icon(&self, ui: &mut Ui, inst: &Instance, size: f32) {
        let custom = inst.root.join("instance.png");
        if custom.is_file() {
            let uri = format!("file://{}", custom.to_string_lossy().replace('\\', "/"));
            ui.add(
                egui::Image::new(uri)
                    .fit_to_exact_size(egui::vec2(size, size))
                    .corner_radius(egui::CornerRadius::same((size * 0.22) as u8)),
            );
        } else {
            avatar(ui, inst.name(), size);
        }
    }

    /// Open a file (or folder) chooser without freezing the window.
    pub fn pick_file(&mut self, purpose: PickPurpose) {
        let n = self.notifier();
        self.rt.spawn_blocking(move || {
            let dialog = rfd::FileDialog::new();
            let path = match &purpose {
                PickPurpose::InstanceImage(_) => {
                    dialog.add_filter("Images", &["png", "jpg", "jpeg", "webp"]).pick_file()
                }
                PickPurpose::JavaFolder => dialog.pick_folder(),
            };
            n.send(Msg::Picked(purpose, path));
        });
    }

    pub(super) fn picked(&mut self, purpose: PickPurpose, path: Option<PathBuf>) {
        let Some(path) = path else { return };
        match purpose {
            PickPurpose::InstanceImage(folder) => {
                let Some(inst) = self.instance(&folder) else { return };
                let target = inst.root.join("instance.png");
                let result = image::open(&path).map_err(anyhow::Error::from).and_then(|img| {
                    img.resize(256, 256, image::imageops::FilterType::Lanczos3)
                        .save_with_format(&target, image::ImageFormat::Png)
                        .map_err(anyhow::Error::from)
                });
                match result {
                    Ok(()) => {
                        self.ctx.forget_image(&format!("file://{}", target.to_string_lossy().replace('\\', "/")));
                        self.toast("Image changed");
                    }
                    Err(e) => self.toast_err(format!("Couldn't use that image: {e:#}")),
                }
            }
            PickPurpose::JavaFolder => {
                self.settings.java_path = path.to_string_lossy().to_string();
                self.settings.using_custom_java_path = true;
                self.save_settings();
            }
        }
    }

    fn loader_dialog_request(&mut self) {
        let Some(d) = self.loader_dialog.as_mut() else { return };
        d.version.clear();
        if d.loader == Loader::Vanilla {
            d.versions.clear();
            d.requested = None;
            d.loading = false;
            return;
        }
        let Some(mc) = self.instances.iter().find(|i| i.folder == d.folder).map(|i| i.mc_version().to_string()) else {
            return;
        };
        let key = (d.loader, mc);
        if d.requested.as_ref() == Some(&key) {
            return;
        }
        d.requested = Some(key.clone());
        d.loading = true;
        d.versions.clear();
        let (dirs, n) = (self.dirs.clone(), self.notifier());
        self.rt.spawn(async move {
            let r = loaders::versions(&dirs, key.0, &key.1).await.map_err(|e| format!("{e:#}"));
            n.send(Msg::DialogLoaderVersions { loader: key.0, mc: key.1, result: r });
        });
    }

    pub(super) fn dialog_loader_versions(&mut self, loader: Loader, mc: &str, result: Result<Vec<String>, String>) {
        let Some(d) = self.loader_dialog.as_mut() else { return };
        if d.requested.as_ref() != Some(&(loader, mc.to_string())) {
            return;
        }
        d.loading = false;
        match result {
            Ok(v) => {
                d.versions = v;
                d.error = None;
            }
            Err(e) => d.error = Some(e),
        }
    }

    /// The three small dialogs of the menu.
    pub(super) fn instance_dialogs_ui(&mut self, ctx: &egui::Context) {
        // description
        if let Some((folder, mut text)) = self.desc_edit.clone() {
            let (mut save, mut close) = (false, false);
            egui::Modal::new(egui::Id::unique("desc")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                ui.set_width(440.0);
                ui.label(RichText::new("Change description").size(19.0).strong());
                ui.add_space(6.0);
                ui.add_sized(
                    [ui.available_width(), 90.0],
                    egui::TextEdit::multiline(&mut text)
                        .margin(egui::Margin::symmetric(12, 10))
                        .background_color(c_field()),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if primary(ui, "Save").clicked() {
                        save = true;
                    }
                    if secondary(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            });
            self.desc_edit = Some((folder.clone(), text.clone()));
            if save {
                if let Some(idx) = self.instances.iter().position(|i| i.folder == folder) {
                    self.instances[idx].file.launcher.description = text;
                    if let Err(e) = self.instances[idx].save() {
                        self.toast_err(format!("{e:#}"));
                    }
                }
                close = true;
            }
            if close {
                self.desc_edit = None;
            }
        }

        // play offline with another name
        if let Some((folder, mut name, mut bad)) = self.offline_play.clone() {
            let (mut go, mut close) = (false, false);
            egui::Modal::new(egui::Id::unique("offline")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                ui.set_width(400.0);
                ui.label(RichText::new("Play offline with another name").size(19.0).strong());
                muted(ui, "3-16 letters, numbers or underscores. No account needed.");
                ui.add_space(6.0);
                let r = ui.add_sized(
                    [ui.available_width(), 38.0],
                    egui::TextEdit::singleline(&mut name)
                        .hint_text("Player name")
                        .margin(egui::Margin::symmetric(12, 9))
                        .background_color(c_field()),
                );
                if bad {
                    ui.label(RichText::new("Use 3-16 letters, numbers or underscores.").color(c_danger()));
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if primary(ui, "Play").clicked()
                        || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        go = true;
                    }
                    if secondary(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            });
            if go {
                if super::whoplays::valid_player_name(name.trim()) {
                    let acc = crate::accounts::Account::offline(name.trim());
                    self.play_as(&folder, LaunchOptions::default(), Some(acc));
                    close = true;
                } else {
                    bad = true;
                }
            }
            self.offline_play = if close { None } else { Some((folder, name, bad)) };
        }

        // mod loader
        if self.loader_dialog.is_some() {
            let (mut apply, mut close, mut changed) = (false, false, false);
            let mut d = self.loader_dialog.take().unwrap();
            let current = self.instance(&d.folder).and_then(|i| i.loader().map(|(k, v)| format!("{k} {v}")));
            egui::Modal::new(egui::Id::unique("loader")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                ui.set_width(440.0);
                ui.label(RichText::new("Mod loader").size(19.0).strong());
                muted(ui, format!("Now: {}", current.clone().unwrap_or_else(|| "None (vanilla)".into())));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for l in Loader::ALL {
                        let label = if l == Loader::Vanilla { "None" } else { l.label() };
                        if chip(ui, label, d.loader == l).clicked() && d.loader != l {
                            d.loader = l;
                            changed = true;
                        }
                    }
                });
                if d.loader != Loader::Vanilla {
                    ui.horizontal(|ui| {
                        muted(ui, "Version");
                        if d.loading {
                            ui.spinner();
                        } else if let Some(e) = &d.error {
                            ui.label(RichText::new(e).color(c_danger()));
                        } else if d.versions.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "{} isn't available for this Minecraft version",
                                    d.loader.label()
                                ))
                                .color(c_warn()),
                            );
                        } else {
                            let newest = d.versions[0].clone();
                            let shown =
                                if d.version.is_empty() { format!("Latest ({newest})") } else { d.version.clone() };
                            egui::ComboBox::from_id_salt("dlg-loader-version")
                                .selected_text(shown)
                                .width(220.0)
                                .show_ui(ui, |ui| {
                                    if ui.selectable_label(d.version.is_empty(), format!("Latest ({newest})")).clicked()
                                    {
                                        d.version.clear();
                                    }
                                    for v in d.versions.iter().take(60) {
                                        if ui.selectable_label(&d.version == v, v).clicked() {
                                            d.version = v.clone();
                                        }
                                    }
                                });
                        }
                    });
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let ready = d.loader == Loader::Vanilla || (!d.loading && !d.versions.is_empty());
                    if primary_enabled(ui, "Apply", ready).clicked() {
                        apply = true;
                    }
                    if secondary(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            });
            if apply {
                if let Some(inst) = self.instance(&d.folder).cloned() {
                    let (dirs, loader) = (self.dirs.clone(), d.loader);
                    let version = Some(d.version.clone()).filter(|v| !v.is_empty());
                    self.start_task(format!("Changing the mod loader of {}", inst.name()), move |p, _| async move {
                        let mut inst = inst;
                        instances::set_loader(&dirs, &mut inst, loader, version.as_deref(), &p).await?;
                        Ok(format!("{} now uses {}", inst.name(), loader.label()))
                    });
                }
                close = true;
            }
            self.loader_dialog = if close { None } else { Some(d) };
            if changed {
                self.loader_dialog_request();
            }
        }
    }
}
