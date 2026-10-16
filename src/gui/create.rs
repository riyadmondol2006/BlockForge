//! Create: pick a Minecraft version and a loader, then download it all.

use egui::{CornerRadius, RichText, ScrollArea, Stroke, Ui};

use super::theme::*;
use super::widgets::*;
use super::{App, Msg, Page};
use crate::instances;
use crate::loaders::{self, Loader};
use crate::mods::performance;

pub struct CreateState {
    pub name: String,
    pub mc: String,
    pub show_snapshots: bool,
    pub filter: String,
    pub loader: Loader,
    pub loader_versions: Vec<String>,
    /// Empty means "the newest".
    pub loader_version: String,
    pub loading: bool,
    requested: Option<(Loader, String)>,
    pub error: Option<String>,
    /// Add Sodium and the other performance mods once the instance is made.
    pub performance: bool,
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

impl Default for CreateState {
    fn default() -> Self {
        CreateState {
            name: String::new(),
            mc: String::new(),
            show_snapshots: false,
            filter: String::new(),
            loader: Loader::Vanilla,
            loader_versions: Vec::new(),
            loader_version: String::new(),
            loading: false,
            requested: None,
            error: None,
            performance: false,
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

    pub(super) fn create_ui(&mut self, ui: &mut Ui) {
        heading(ui, "Create an instance");
        muted(ui, "A separate copy of Minecraft with its own worlds, mods and settings.");
        ui.add_space(8.0);
        let mut changed_version = false;
        let mut create = false;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            card(ui, |ui| {
                ui.label(RichText::new("Name").strong());
                let c = &mut self.create;
                // the hint is the name the instance gets when the field is left empty
                let hint = if c.mc.is_empty() { "My instance".to_string() } else { default_name(&c.mc, c.loader) };
                ui.add_sized(
                    [360.0f32.min(ui.available_width()), 36.0],
                    egui::TextEdit::singleline(&mut c.name)
                        .hint_text(hint)
                        .margin(egui::Margin::symmetric(12, 8))
                        .background_color(c_field()),
                );
            });
            ui.add_space(6.0);
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Minecraft version").strong());
                    ui.label(RichText::new(&self.create.mc).color(c_accent()).strong());
                });
                match (&self.manifest, &self.manifest_error) {
                    (Some(m), _) => {
                        ui.horizontal(|ui| {
                            let mut f = std::mem::take(&mut self.create.filter);
                            search_field(ui, &mut f, "Filter versions", 200.0);
                            self.create.filter = f;
                            ui.checkbox(&mut self.create.show_snapshots, "Show snapshots");
                        });
                        let filter = self.create.filter.to_lowercase();
                        let versions: Vec<&str> = m
                            .versions
                            .iter()
                            .filter(|v| self.create.show_snapshots || v.kind == "release")
                            .filter(|v| filter.is_empty() || v.id.to_lowercase().contains(&filter))
                            .map(|v| v.id.as_str())
                            .collect();
                        egui::Frame::new()
                            .fill(c_field())
                            .stroke(Stroke::new(1.0, c_border()))
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(egui::Margin::same(6))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ScrollArea::vertical()
                                    .id_salt("versions")
                                    .max_height(190.0)
                                    .auto_shrink([false, true])
                                    .show_rows(ui, 28.0, versions.len(), |ui, range| {
                                        for v in &versions[range] {
                                            let selected = self.create.mc == *v;
                                            if ui.selectable_label(selected, *v).clicked() && !selected {
                                                self.create.mc = v.to_string();
                                                changed_version = true;
                                            }
                                        }
                                    });
                            });
                    }
                    (None, Some(e)) => {
                        ui.label(RichText::new(format!("Couldn't load the version list: {e}")).color(c_danger()));
                        if secondary(ui, "Try again").clicked() {
                            self.manifest_error = None;
                            self.refresh_manifest();
                        }
                    }
                    (None, None) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            muted(ui, "Loading versions");
                        });
                    }
                }
            });
            ui.add_space(6.0);
            card(ui, |ui| {
                ui.label(RichText::new("Mod loader").strong());
                ui.horizontal(|ui| {
                    for l in Loader::ALL {
                        if chip(ui, l.label(), self.create.loader == l).clicked() && self.create.loader != l {
                            self.create.loader = l;
                            changed_version = true;
                        }
                    }
                });
                if self.create.loader != Loader::Vanilla {
                    ui.horizontal(|ui| {
                        muted(ui, "Version");
                        if self.create.loading {
                            ui.spinner();
                        } else if let Some(e) = self.create.error.clone() {
                            ui.label(RichText::new(format!("Couldn't get the versions: {e}")).color(c_danger()));
                            if secondary(ui, "Try again").clicked() {
                                // forget the failed request so it is made again
                                self.create.requested = None;
                                self.create.error = None;
                                changed_version = true;
                            }
                        } else if self.create.loader_versions.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "{} isn't available for Minecraft {}",
                                    self.create.loader.label(),
                                    self.create.mc
                                ))
                                .color(c_warn()),
                            );
                        } else {
                            let newest = self.create.loader_versions[0].clone();
                            let shown = if self.create.loader_version.is_empty() {
                                format!("Latest ({newest})")
                            } else {
                                self.create.loader_version.clone()
                            };
                            egui::ComboBox::from_id_salt("loader-version").selected_text(shown).width(220.0).show_ui(
                                ui,
                                |ui| {
                                    if ui
                                        .selectable_label(
                                            self.create.loader_version.is_empty(),
                                            format!("Latest ({newest})"),
                                        )
                                        .clicked()
                                    {
                                        self.create.loader_version.clear();
                                    }
                                    for v in self.create.loader_versions.iter().take(60) {
                                        if ui.selectable_label(&self.create.loader_version == v, v).clicked() {
                                            self.create.loader_version = v.clone();
                                        }
                                    }
                                },
                            );
                        }
                    });
                    if matches!(self.create.loader, Loader::Forge | Loader::NeoForge) {
                        small_muted(
                            ui,
                            "Forge and NeoForge run their own installer, so this takes a little longer the first time.",
                        );
                    }
                    let key = self.create.loader.modrinth_name();
                    if performance::offered_for(key) {
                        ui.add_space(4.0);
                        ui.checkbox(&mut self.create.performance, "Add performance mods");
                        small_muted(
                            ui,
                            format!(
                                "{} from Modrinth, for a smoother game. Anything not made for this Minecraft version yet is left out.",
                                performance::describe(key)
                            ),
                        );
                    }
                }
            });
            ui.add_space(8.0);
            let ready = !self.create.mc.is_empty()
                && !self.tasks.iter().any(|t| t.label.starts_with("Creating"))
                && (self.create.loader == Loader::Vanilla || !self.create.loader_versions.is_empty());
            ui.horizontal(|ui| {
                if primary_enabled(ui, "Create instance", ready).clicked() {
                    create = true;
                }
                if self.tasks.iter().any(|t| t.label.starts_with("Creating")) {
                    ui.spinner();
                    muted(ui, "Working on it, you can follow along in the sidebar");
                }
            });
        });
        if changed_version {
            self.request_loader_versions();
        }
        if create {
            let c = &self.create;
            let loader = c.loader;
            let mc = c.mc.clone();
            let name = if c.name.trim().is_empty() { default_name(&mc, loader) } else { c.name.trim().to_string() };
            let version = Some(c.loader_version.clone()).filter(|v| !v.is_empty());
            let performance = c.performance && performance::offered_for(loader.modrinth_name());
            let (dirs, settings) = (self.dirs.clone(), self.settings.clone());
            let label = format!("Creating {name}");
            self.start_task(label, move |p, _| async move {
                let mut inst = instances::create(&dirs, &name, &mc, loader, version.as_deref(), &p).await?;
                let ready = format!("\"{}\" is ready", inst.name());
                if !performance {
                    return Ok(ready);
                }
                let ctx = crate::mods::ModContext::build(&dirs, &inst, &settings).await;
                let r = crate::mods::install::install_performance_pack(&mut inst, &ctx, &p).await;
                Ok(performance_note(ready, &r.installed, &r.skipped))
            });
            self.create.name.clear();
            self.page = Page::Library;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // performance mods are offered where they exist, not for plain Minecraft
        assert!(performance::offered_for(Loader::Fabric.modrinth_name()));
        assert!(!performance::offered_for(Loader::Vanilla.modrinth_name()));
    }
}
