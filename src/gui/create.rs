//! Create: pick a Minecraft version and a loader, then download it all.

use egui::{CornerRadius, RichText, ScrollArea, Stroke, Ui};

use super::discover::filter_label;
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
    /// The sidebar label of the instance being made, so Create waits for that job (and not for any other whose
    /// label happens to start the same way, such as "Creating a support bundle").
    task_label: Option<String>,
}

/// The widest the form gets.
const FORM_W: f32 = 720.0;

/// One version in the grid of versions: plain until pointed at, a tone step up when it is the chosen one.
fn version_cell(ui: &mut Ui, text: &str, selected: bool, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 28.0), egui::Sense::click());
    let bg = if selected {
        c_selection()
    } else if resp.hovered() {
        c_hover()
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), bg);
    let font = if selected { super::fonts::medium(FS_BODY) } else { egui::FontId::proportional(FS_BODY) };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, font, c_text());
    focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, text));
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

/// What is about to be made, in a line: "Minecraft 1.21.1 with Fabric 0.16.5".
fn summary(c: &CreateState) -> String {
    if c.mc.is_empty() {
        return String::new();
    }
    if c.loader == Loader::Vanilla {
        return format!("Minecraft {}", c.mc);
    }
    let version = if c.loader_version.is_empty() { c.loader_versions.first() } else { Some(&c.loader_version) };
    match version {
        Some(v) => format!("Minecraft {} with {} {v}", c.mc, c.loader.label()),
        None => format!("Minecraft {} with {}", c.mc, c.loader.label()),
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
            task_label: None,
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
        heading(ui, "New instance");
        ui.add_space(8.0);
        let mut changed_version = false;
        let mut create = false;
        let mut enter = false;
        // the button stays in view under the form: the form scrolls when the window is short
        let form_h = (ui.available_height() - 64.0).max(160.0);
        ScrollArea::vertical().auto_shrink([false, true]).max_height(form_h).show(ui, |ui| {
            // a form reads best in a column of its own, not stretched across a wide window
            ui.set_max_width((ui.available_width() - 14.0).min(FORM_W));
            ui.spacing_mut().item_spacing.y = 8.0;

            // name
            section(ui, "Name");
            let c = &mut self.create;
            // the hint is the name the instance gets when the field is left empty
            let hint = if c.mc.is_empty() { "My instance".to_string() } else { default_name(&c.mc, c.loader) };
            let name_field = ui.add_sized(
                [ui.available_width().min(360.0), 32.0],
                egui::TextEdit::singleline(&mut c.name)
                    .hint_text(RichText::new(hint).color(c_faint()))
                    .margin(egui::Margin::symmetric(10, 7))
                    .background_color(c_field()),
            );
            // Enter in the name makes the instance, like the button under the form
            enter = name_field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

            // Minecraft version
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Minecraft version").size(FS_SECTION).semibold());
                if !self.create.mc.is_empty() {
                    ui.label(RichText::new(&self.create.mc).color(c_muted()));
                }
            });
            match (&self.manifest, &self.manifest_error) {
                (Some(m), _) => {
                    ui.horizontal(|ui| {
                        if let Some(i) = segmented(ui, &["Releases", "Snapshots too"], usize::from(self.create.show_snapshots)) {
                            self.create.show_snapshots = i == 1;
                        }
                        ui.add_space(8.0);
                        let mut f = std::mem::take(&mut self.create.filter);
                        search_field(ui, &mut f, "Filter versions", 180.0);
                        self.create.filter = f;
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
                        .corner_radius(CornerRadius::same(R_CONTROL))
                        .inner_margin(egui::Margin::same(6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            if versions.is_empty() {
                                ui.add_space(4.0);
                                small_muted(ui, format!("No version matches \u{201C}{}\u{201D}.", self.create.filter.trim()));
                                ui.add_space(4.0);
                                return;
                            }
                            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                            // the versions fill the width in a grid, a few columns of short names
                            let (cols, cell) = grid_columns(ui.available_width() - 12.0, 84.0, 4.0);
                            ScrollArea::vertical()
                                .id_salt("versions")
                                .max_height(176.0)
                                .auto_shrink([false, true])
                                .show_rows(ui, 28.0, versions.len().div_ceil(cols), |ui, range| {
                                    for r in range {
                                        ui.horizontal(|ui| {
                                            for v in versions.iter().skip(r * cols).take(cols) {
                                                let selected = self.create.mc == *v;
                                                if version_cell(ui, v, selected, cell).clicked() && !selected {
                                                    self.create.mc = v.to_string();
                                                    changed_version = true;
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

            // loader
            ui.add_space(16.0);
            section(ui, "Mod loader");
            let labels: Vec<&str> = Loader::ALL.iter().map(|l| l.label()).collect();
            let current = Loader::ALL.iter().position(|l| *l == self.create.loader).unwrap_or(0);
            if let Some(i) = segmented(ui, &labels, current) {
                if Loader::ALL[i] != self.create.loader {
                    self.create.loader = Loader::ALL[i];
                    changed_version = true;
                }
            }
            if self.create.loader != Loader::Vanilla {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    filter_label(ui, &format!("{} version", self.create.loader.label()));
                    if self.create.loading {
                        ui.spinner();
                    } else if !self.create.loader_versions.is_empty() {
                        let newest = self.create.loader_versions[0].clone();
                        let shown = if self.create.loader_version.is_empty() {
                            format!("Latest ({newest})")
                        } else {
                            self.create.loader_version.clone()
                        };
                        egui::ComboBox::from_id_salt("loader-version").selected_text(shown).width(240.0).show_ui(
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
                if !self.create.loading {
                    if let Some(e) = self.create.error.clone() {
                        let text = format!("Couldn't get the versions: {e}");
                        if notice(ui, NoticeKind::Error, &text, Some("Try again")).is_some_and(|r| r.clicked()) {
                            // forget the failed request so it is made again
                            self.create.requested = None;
                            self.create.error = None;
                            changed_version = true;
                        }
                    } else if self.create.loader_versions.is_empty() {
                        let text = format!(
                            "{} isn't available for Minecraft {}.",
                            self.create.loader.label(),
                            self.create.mc
                        );
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

            ui.add_space(8.0);
        });
        let creating = self.create.task_label.as_ref().is_some_and(|l| self.tasks.iter().any(|t| &t.label == l));
        let ready = !self.create.mc.is_empty()
            && !creating
            && (self.create.loader == Loader::Vanilla || !self.create.loader_versions.is_empty());
        let summary = summary(&self.create);
        // what a greyed-out Create waits for, said when it is pointed at
        let why_not = if self.create.mc.is_empty() {
            "Pick a Minecraft version first".to_string()
        } else if creating {
            "The instance is being made. Its progress is in the sidebar".to_string()
        } else if self.create.loading {
            format!("Looking up the {} versions", self.create.loader.label())
        } else {
            format!("{} isn't available for this Minecraft version", self.create.loader.label())
        };
        ui.scope(|ui| {
            // the button sits under the form, not at the far edge of a wide window
            ui.set_max_width(ui.available_width().min(FORM_W));
            rule(ui);
            ui.add_space(8.0);
            super::discover::dialog_footer(
                ui,
                "create",
                |ui| {
                    if creating {
                        ui.spinner();
                        muted(ui, "Working on it, you can follow along in the sidebar");
                    } else {
                        small_muted(ui, summary);
                    }
                },
                |ui| {
                    if primary_enabled(ui, "Create instance", ready).on_disabled_hover_text(&why_not).clicked() {
                        create = true;
                    }
                },
            );
        });
        if changed_version {
            self.request_loader_versions();
        }
        if enter && ready {
            create = true;
        }
        if create {
            let c = &self.create;
            let loader = c.loader;
            let mc = c.mc.clone();
            let wanted = if c.name.trim().is_empty() { default_name(&mc, loader) } else { c.name.trim().to_string() };
            // two instances of one name would share their backups (they are found by name): the second gets a number
            let name = unique_instance_name(&self.instances, &wanted);
            let version = Some(c.loader_version.clone()).filter(|v| !v.is_empty());
            let performance = c.performance && performance::offered_for(loader.modrinth_name());
            let (dirs, settings) = (self.dirs.clone(), self.settings.clone());
            let label = format!("Creating {name}");
            self.create.task_label = Some(label.clone());
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
        // performance mods are offered where they exist, not for plain Minecraft
        assert!(performance::offered_for(Loader::Fabric.modrinth_name()));
        assert!(!performance::offered_for(Loader::Vanilla.modrinth_name()));
    }
}
