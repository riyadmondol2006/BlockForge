//! CurseForge in the window: the Modrinth / CurseForge switch on Discover, CurseForge project cards, the file and
//! dependency chooser, the "download this one in your browser" window, and the update lists.
//!
//! Everything here is `impl App`, like the other pages. It keeps its own small message channel (`CfState`) so the
//! window's main `Msg` enum does not need new variants; the lead only has to call [`App::cf_windows`] once per frame.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Align, CornerRadius, Id, Layout, RichText, ScrollArea, Stroke, Ui};

use super::App;
use super::theme::*;
use super::widgets::*;
use crate::curseforge::api::{self, SearchPage, SearchQuery, Section, Sort};
use crate::curseforge::browser::{BrowserHub, ItemState};
use crate::curseforge::filter::{self, Target};
use crate::curseforge::install::{self, Dependency, ModUpdate};
use crate::curseforge::model::{CategoryForGame, File, Project};
use crate::curseforge::pack::{self, PackSource, PackUpdate};
use crate::curseforge::{self, CfSettings};

/// Answers from background work, delivered to the window through `CfState`'s own channel.
enum CfMsg {
    Search { seq: u64, reset: bool, result: Result<SearchPage, String> },
    Categories { section: Section, result: Result<Vec<CategoryForGame>, String> },
    Files { dialog: u64, result: Result<Vec<File>, String> },
    Deps { dialog: u64, file: i64, result: Result<Vec<Dependency>, String> },
    Changelog { dialog: u64, file: i64, result: Result<String, String> },
    ModUpdates { folder: String, result: Result<Vec<ModUpdate>, String> },
    PackUpdates { result: Result<Vec<(String, String, PackUpdate)>, String> },
    Project { result: Result<Project, String>, folder: Option<String>, installed_file: Option<i64>, reinstall: bool },
}

/// The file chooser for one project ("Installing X" in the Java edition).
struct FileDialog {
    id: u64,
    project: Project,
    /// The instance the file goes into; `None` for a modpack (it becomes a new instance).
    folder: Option<String>,
    /// The file the instance has now, when reinstalling or updating.
    installed_file: Option<i64>,
    /// Preselect the installed file (reinstall) instead of the newest one.
    reinstall: bool,
    all_files: Vec<File>,
    files: Vec<File>,
    show_every_file: bool,
    selected: usize,
    loading: bool,
    error: Option<String>,
    deps: Vec<(Dependency, bool)>,
    deps_for: Option<i64>,
    deps_loading: bool,
    /// Why the dependencies couldn't be worked out, when they couldn't.
    deps_error: Option<String>,
    changelog: Option<(i64, Result<String, String>)>,
    changelog_loading: bool,
    /// The name of the new instance (modpacks).
    name: String,
    /// What the files are matched against, and the instance's name; worked out once when the dialog opens.
    target: Option<(Target, String)>,
}

/// What Discover needs to know about the instance it installs into. Reading it means parsing the whole mod list
/// and the launcher's config file, so it is kept until the instance's file changes.
struct TargetInfo {
    folder: String,
    stamp: Option<(std::time::SystemTime, u64)>,
    target: Target,
    /// CurseForge project id -> installed file id.
    installed: HashMap<i64, i64>,
    /// Fabric API or the like, when the loader's mods need it and it is missing.
    missing_api: Option<(i64, &'static str)>,
}

fn file_stamp(path: &std::path::Path) -> Option<(std::time::SystemTime, u64)> {
    std::fs::metadata(path).ok().and_then(|m| Some((m.modified().ok()?, m.len())))
}

/// A dialog's width and the height its scrolling middle part may take, so the dialog fits a small window (or a
/// large interface scale) with its buttons still on screen.
pub(super) fn modal_size(ctx: &egui::Context, width: f32, reserved: f32) -> (f32, f32) {
    let screen = ctx.content_rect();
    ((screen.width() - 48.0).clamp(240.0, width), (screen.height() - reserved).max(120.0))
}

/// The list of newer mod files / modpack files.
struct UpdateWin {
    folder: Option<String>,
    title: String,
    loading: bool,
    error: Option<String>,
    mods: Vec<ModUpdate>,
    packs: Vec<(String, String, PackUpdate)>,
}

pub struct CfState {
    inited: bool,
    /// Discover shows CurseForge rather than Modrinth.
    pub active: bool,
    section: Section,
    query: String,
    sort: Sort,
    category: Option<i64>,
    categories: Vec<CategoryForGame>,
    categories_for: Option<Section>,
    projects: Vec<Project>,
    total: u64,
    has_more: bool,
    page: u32,
    loading: bool,
    seq: u64,
    error: Option<String>,
    searched: bool,
    import_text: String,
    dialog: Option<FileDialog>,
    next_dialog: u64,
    /// One per install that may need files downloaded by hand; the window shows the first with files waiting.
    browsers: Vec<Arc<BrowserHub>>,
    confirm_skip: bool,
    /// Whether the launcher's configuration allows CurseForge, read once rather than on every frame.
    enabled: Option<bool>,
    target_info: Option<TargetInfo>,
    notice_shown: bool,
    updates: Option<UpdateWin>,
    tx: Sender<CfMsg>,
    rx: Receiver<CfMsg>,
}

impl Default for CfState {
    fn default() -> Self {
        let (tx, rx) = channel();
        CfState {
            inited: false,
            active: false,
            section: Section::Mods,
            query: String::new(),
            sort: Sort::Popularity,
            category: None,
            categories: Vec::new(),
            categories_for: None,
            projects: Vec::new(),
            total: 0,
            has_more: false,
            page: 0,
            loading: false,
            seq: 0,
            error: None,
            searched: false,
            import_text: String::new(),
            dialog: None,
            next_dialog: 1,
            browsers: Vec::new(),
            confirm_skip: false,
            enabled: None,
            target_info: None,
            notice_shown: false,
            updates: None,
            tx,
            rx,
        }
    }
}

/// What a project card asked for.
enum CardAct {
    Install(Project, Option<String>),
    Reinstall(Project, Option<String>),
    Remove(Project, String),
    View(Project),
}

impl App {
    fn cf_settings(&self) -> CfSettings {
        CfSettings::from_settings(&self.settings)
    }

    /// Done once: pick the platform from the `defaultModPlatform` setting and hand the API key to the client.
    fn cf_init(&mut self) {
        if self.cf.inited {
            return;
        }
        self.cf.inited = true;
        curseforge::configure(&self.settings);
        self.cf.active = self.cf_settings().prefer_curseforge;
    }

    /// The "Modrinth | CurseForge" switch. Returns `true` when it was changed.
    pub(super) fn cf_platform_chips(&mut self, ui: &mut Ui) -> bool {
        self.cf_init();
        let mut changed = false;
        if chip(ui, "Modrinth", !self.cf.active).clicked() && self.cf.active {
            self.cf.active = false;
            changed = true;
        }
        if chip(ui, "CurseForge", self.cf.active).clicked() && !self.cf.active {
            self.cf.active = true;
            // the launcher's configuration may have changed since it was last read
            self.cf.enabled = None;
            changed = true;
        }
        changed
    }

    /// The target instance's details for the CurseForge page, read again only when its file changed.
    fn cf_target_info(&mut self) -> Option<&TargetInfo> {
        if self.cf.section == Section::Modpacks {
            return None;
        }
        let folder = self.discover.target.clone()?;
        let inst = self.instance(&folder)?;
        let stamp = file_stamp(&inst.instance_file());
        let fresh = self.cf.target_info.as_ref().is_some_and(|t| t.folder == folder && t.stamp == stamp);
        if !fresh {
            let target = Target::from_instance(inst, &self.dirs, &self.cf_settings());
            let records = inst.mod_records();
            let installed = records
                .iter()
                .filter_map(|r| Some((r.curse_forge_project_id?, r.curse_forge_file_id.unwrap_or(0))))
                .collect();
            let missing_api = target.missing_loader_api(&records);
            self.cf.target_info = Some(TargetInfo { folder, stamp, target, installed, missing_api });
        }
        self.cf.target_info.as_ref()
    }

    // -----------------------------------------------------------------------------------------------------
    // searching

    fn cf_search(&mut self, ctx: &egui::Context, reset: bool) {
        let cfs = self.cf_settings();
        let target = self.cf_target_info().map(|t| t.target.clone());
        let cf = &mut self.cf;
        if reset {
            cf.page = 0;
            cf.projects.clear();
        } else {
            cf.page += 1;
        }
        cf.seq += 1;
        cf.loading = true;
        cf.searched = true;
        let mut q = SearchQuery::new(cf.section);
        q.text = cf.query.trim().to_string();
        q.sort = cf.sort;
        q.category_id = cf.category;
        q.page = cf.page;
        if let Some(t) = &target {
            if cf.section.uses_game_version() {
                q.game_version = t.search_game_version(&cfs);
            }
            if cf.section.uses_loader() {
                q.mod_loaders = t.search_loaders();
            }
        }
        let (seq, tx, ctx) = (cf.seq, cf.tx.clone(), ctx.clone());
        self.rt.spawn(async move {
            let result = api::search(&q).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(CfMsg::Search { seq, reset, result });
            ctx.request_repaint();
        });
    }

    fn cf_load_categories(&mut self, ctx: &egui::Context) {
        let section = self.cf.section;
        if self.cf.categories_for == Some(section) {
            return;
        }
        self.cf.categories_for = Some(section);
        self.cf.categories.clear();
        self.cf.category = None;
        let (tx, ctx) = (self.cf.tx.clone(), ctx.clone());
        self.rt.spawn(async move {
            let result = api::categories_for(section).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(CfMsg::Categories { section, result });
            ctx.request_repaint();
        });
    }

    // -----------------------------------------------------------------------------------------------------
    // the Discover page

    /// Draws Discover for CurseForge when that platform is selected. Returns `false` (drawing nothing) when
    /// Modrinth is selected, so the Modrinth page carries on.
    pub(super) fn cf_discover_ui(&mut self, ui: &mut Ui) -> bool {
        self.cf_init();
        if !self.cf.active {
            return false;
        }
        let ctx = ui.ctx().clone();
        let enabled = *self.cf.enabled.get_or_insert_with(|| curseforge::platform_enabled(&self.dirs, "modsEnabled"));
        if !enabled {
            ui.horizontal_wrapped(|ui| {
                heading(ui, "Discover");
                muted(ui, "from CurseForge");
                self.cf_platform_chips(ui);
            });
            card(ui, |ui| muted(ui, "CurseForge has been switched off by the launcher's configuration."));
            return true;
        }
        self.cf_load_categories(&ctx);
        let mut search = !self.cf.searched;
        ui.horizontal_wrapped(|ui| {
            heading(ui, "Discover");
            muted(ui, "from CurseForge");
            ui.add_space(8.0);
            self.cf_platform_chips(ui);
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for s in Section::ALL {
                if chip(ui, s.label(), self.cf.section == s).clicked() && self.cf.section != s {
                    self.cf.section = s;
                    search = true;
                    self.cf_load_categories(&ctx);
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            let mut q = std::mem::take(&mut self.cf.query);
            let r = search_field(
                ui,
                &mut q,
                &format!("Search {}", self.cf.section.label().to_lowercase()),
                320.0f32.min(ui.available_width()),
            );
            self.cf.query = q;
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                search = true;
            }
            if primary(ui, "Search").clicked() {
                search = true;
            }
            egui::ComboBox::from_id_salt("cf-sort").selected_text(self.cf.sort.label()).show_ui(ui, |ui| {
                for s in Sort::ALL {
                    if ui.selectable_value(&mut self.cf.sort, s, s.label()).clicked() {
                        search = true;
                    }
                }
            });
            let current = self
                .cf
                .category
                .and_then(|id| self.cf.categories.iter().find(|c| c.id == id))
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "All categories".into());
            egui::ComboBox::from_id_salt("cf-cat").selected_text(current).width(190.0).show_ui(ui, |ui| {
                if ui.selectable_label(self.cf.category.is_none(), "All categories").clicked() {
                    self.cf.category = None;
                    search = true;
                }
                let cats = self.cf.categories.clone();
                for c in &cats {
                    if ui.selectable_label(self.cf.category == Some(c.id), &c.name).clicked() {
                        self.cf.category = Some(c.id);
                        search = true;
                    }
                }
            });
        });

        if self.cf.section == Section::Modpacks {
            self.cf_modpack_tools(ui);
        } else {
            self.cf_target_row(ui, &mut search);
        }
        if search {
            self.cf_search(&ctx, true);
        }
        ui.add_space(6.0);

        if let Some(e) = self.cf.error.clone() {
            if super::discover::fetch_error_card(ui, "Couldn't reach CurseForge", &e) && !self.cf.loading {
                // the first page again, or the page that failed to load
                self.cf_search(&ctx, self.cf.projects.is_empty());
            }
        }
        if self.cf.projects.is_empty() && !self.cf.loading && self.cf.error.is_none() && self.cf.searched {
            card(ui, |ui| {
                muted(
                    ui,
                    if self.cf.query.trim().is_empty() {
                        "No results."
                    } else {
                        "No results. Remove your search words and try again."
                    },
                )
            });
            return true;
        }

        let installed: HashMap<i64, i64> = self.cf_target_info().map(|t| t.installed.clone()).unwrap_or_default();
        let projects = std::mem::take(&mut self.cf.projects);
        let section = self.cf.section;
        let target = self.discover.target.clone();
        let instance_choices: Vec<(String, String)> =
            self.instances.iter().map(|i| (i.folder.clone(), i.name().to_string())).collect();
        let mut act: Option<CardAct> = None;
        let mut more = false;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for p in &projects {
                egui::Frame::new()
                    .fill(c_card())
                    .stroke(Stroke::new(1.0, c_border()))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(egui::Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            match p.logo_url() {
                                Some(url) => self.remote_image(ui, url, 60.0, &p.name),
                                None => {
                                    avatar(ui, &p.name, 60.0);
                                }
                            }
                            ui.vertical(|ui| {
                                ui.set_max_width((ui.available_width() - 190.0).max(120.0));
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new(&p.name).strong().size(16.0));
                                    small_muted(ui, format!("by {}", p.author_names()));
                                });
                                ui.add(egui::Label::new(RichText::new(&p.summary).color(c_muted())).truncate());
                                ui.horizontal_wrapped(|ui| {
                                    small_muted(
                                        ui,
                                        format!("{} downloads", human_count(p.download_count.max(0.0) as u64)),
                                    );
                                    for c in p.categories.iter().take(3) {
                                        tag(ui, &c.name, c_muted());
                                    }
                                    if installed.contains_key(&p.id) {
                                        tag(ui, "Installed", c_accent());
                                    }
                                    if p.allow_mod_distribution == Some(false) {
                                        tag(ui, "Download in browser", c_warn());
                                    }
                                });
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if secondary(ui, "View").clicked() {
                                    act = Some(CardAct::View(p.clone()));
                                }
                                let is_installed = installed.contains_key(&p.id) && section != Section::Modpacks;
                                if is_installed {
                                    if let Some(t) = &target {
                                        if danger(ui, "Remove").clicked() {
                                            act = Some(CardAct::Remove(p.clone(), t.clone()));
                                        }
                                        if secondary(ui, "Reinstall").clicked() {
                                            act = Some(CardAct::Reinstall(p.clone(), Some(t.clone())));
                                        }
                                    }
                                } else if section == Section::Modpacks {
                                    if primary(ui, "Install").clicked() {
                                        act = Some(CardAct::Install(p.clone(), None));
                                    }
                                } else if let Some(t) = &target {
                                    if primary(ui, "Install").clicked() {
                                        act = Some(CardAct::Install(p.clone(), Some(t.clone())));
                                    }
                                } else {
                                    ui.menu_button("Install", |ui| {
                                        if instance_choices.is_empty() {
                                            ui.label("Create an instance first");
                                        }
                                        for (folder, name) in &instance_choices {
                                            if ui.button(name).clicked() {
                                                act = Some(CardAct::Install(p.clone(), Some(folder.clone())));
                                                ui.close();
                                            }
                                        }
                                    });
                                }
                            });
                        });
                    });
                ui.add_space(2.0);
            }
            if self.cf.has_more {
                ui.horizontal(|ui| {
                    if self.cf.loading {
                        ui.spinner();
                    } else if secondary(ui, "Load more").clicked() {
                        more = true;
                    }
                });
            } else if self.cf.loading {
                ui.spinner();
            }
        });
        self.cf.projects = projects;
        match act {
            Some(CardAct::Install(p, folder)) => self.cf_open_dialog(&ctx, p, folder, None, false),
            Some(CardAct::Reinstall(p, folder)) => {
                let installed_file = folder.as_ref().and_then(|f| self.instance(f)).and_then(|i| {
                    i.mod_records()
                        .iter()
                        .find(|r| r.curse_forge_project_id == Some(p.id))
                        .and_then(|r| r.curse_forge_file_id)
                });
                self.cf_open_dialog(&ctx, p, folder, installed_file, true);
            }
            Some(CardAct::Remove(p, folder)) => self.cf_remove(&folder, &p),
            Some(CardAct::View(p)) => {
                if let Some(url) = p.website_url() {
                    let _ = open::that(url);
                }
            }
            None => {}
        }
        if more {
            self.cf_search(&ctx, false);
        }
        true
    }

    /// "Install into" row for mods and the like, with the loader helper and the update check.
    fn cf_target_row(&mut self, ui: &mut Ui, search: &mut bool) {
        ui.horizontal_wrapped(|ui| {
            muted(ui, "Install into");
            let current = self
                .discover
                .target
                .as_ref()
                .and_then(|f| self.instance(f))
                .map(|i| i.name().to_string())
                .unwrap_or_else(|| "Choose an instance".into());
            let names: Vec<(String, String)> =
                self.instances.iter().map(|i| (i.folder.clone(), i.name().to_string())).collect();
            egui::ComboBox::from_id_salt("cf-target").selected_text(current).width(240.0).show_ui(ui, |ui| {
                if ui.selectable_label(self.discover.target.is_none(), "Any (show everything)").clicked() {
                    self.discover.target = None;
                    *search = true;
                }
                for (folder, name) in &names {
                    if ui.selectable_label(self.discover.target.as_ref() == Some(folder), name).clicked() {
                        self.discover.target = Some(folder.clone());
                        *search = true;
                    }
                }
            });
            if let Some(folder) = self.discover.target.clone() {
                if let Some(inst) = self.instance(&folder) {
                    small_muted(
                        ui,
                        format!(
                            "Showing what works with Minecraft {}{}",
                            inst.mc_version(),
                            inst.loader().map(|(k, _)| format!(" and {k}")).unwrap_or_default()
                        ),
                    );
                }
                if secondary(ui, "Check for updates").clicked() {
                    self.cf_check_mod_updates(ui.ctx(), &folder);
                }
            }
        });
        // Fabric API and friends, which most mods of that loader need
        let missing = self.cf_target_info().and_then(|t| Some((t.folder.clone(), t.missing_api?)));
        if let Some((folder, (id, name))) = missing {
            let loader = self.instance(&folder).and_then(|i| i.loader()).map(|l| l.0).unwrap_or("this loader");
            let text = format!("{name} is not installed. Most mods for {loader} need it.");
            let label = format!("Installing {name}");
            let running = self.tasks.iter().any(|t| t.label == label);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(text).color(c_warn()));
                if running {
                    ui.spinner();
                } else if secondary(ui, &format!("Install {name}")).clicked() {
                    self.cf_install_by_id(id, folder, label);
                }
            });
        }
    }

    /// The row under the section chips for modpacks: import an address or a downloaded zip, and look for updates.
    fn cf_modpack_tools(&mut self, ui: &mut Ui) {
        self.other_modpack_sources(ui);
        ui.horizontal_wrapped(|ui| {
            let mut text = std::mem::take(&mut self.cf.import_text);
            search_field(
                ui,
                &mut text,
                "Paste a CurseForge modpack address, or the path of a downloaded .zip",
                520.0f32.min(ui.available_width()),
            );
            self.cf.import_text = text;
            if secondary(ui, "Import").clicked() && !self.cf.import_text.trim().is_empty() {
                let t = self.cf.import_text.trim().trim_matches('"').to_string();
                self.cf.import_text.clear();
                self.cf_import_text(t);
            }
            if secondary(ui, "Check my modpacks for updates").clicked() {
                self.cf_check_pack_updates(ui.ctx());
            }
        });
    }

    // -----------------------------------------------------------------------------------------------------
    // installing

    /// Open the file chooser for `project`. `folder` is the instance to install into (none for a modpack).
    pub fn cf_open_dialog(
        &mut self,
        ctx: &egui::Context,
        project: Project,
        folder: Option<String>,
        installed_file: Option<i64>,
        reinstall: bool,
    ) {
        let id = self.cf.next_dialog;
        self.cf.next_dialog += 1;
        let name = project.name.clone();
        let folder = if project.class_id == curseforge::CLASS_MODPACKS { None } else { folder };
        let cfs = self.cf_settings();
        let target = folder
            .as_deref()
            .and_then(|f| self.instance(f))
            .map(|i| (Target::from_instance(i, &self.dirs, &cfs), i.name().to_string()));
        self.cf.dialog = Some(FileDialog {
            id,
            project: project.clone(),
            folder,
            target,
            deps_error: None,
            installed_file,
            reinstall,
            all_files: Vec::new(),
            files: Vec::new(),
            show_every_file: false,
            selected: 0,
            loading: true,
            error: None,
            deps: Vec::new(),
            deps_for: None,
            deps_loading: false,
            changelog: None,
            changelog_loading: false,
            name,
        });
        let (tx, ctx) = (self.cf.tx.clone(), ctx.clone());
        self.rt.spawn(async move {
            let result = api::files(project.id).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(CfMsg::Files { dialog: id, result });
            ctx.request_repaint();
        });
    }

    /// Open the file chooser for a project known only by its id (from the mod manager's "Reinstall" and similar).
    pub fn cf_open_project(
        &mut self,
        ctx: &egui::Context,
        project_id: i64,
        folder: Option<String>,
        installed_file: Option<i64>,
        reinstall: bool,
    ) {
        let (tx, ctx) = (self.cf.tx.clone(), ctx.clone());
        self.rt.spawn(async move {
            let result = api::project(project_id).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(CfMsg::Project { result, folder, installed_file, reinstall });
            ctx.request_repaint();
        });
    }

    fn cf_new_hub(&mut self) -> Arc<BrowserHub> {
        let hub = BrowserHub::new(&self.dirs, self.cf_settings().custom_downloads_path.as_deref());
        // installs that ended let go of theirs; another install still running keeps its own
        self.cf.browsers.retain(|h| Arc::strong_count(h) > 1);
        self.cf.browsers.push(hub.clone());
        hub
    }

    /// Install `file` (and the chosen dependencies) into the instance in the background.
    pub fn cf_install_file(&mut self, folder: &str, project: Project, file: File, include: Vec<i64>) {
        let Some(inst) = self.instance(folder).cloned() else { return };
        let (dirs, cfs, hub) = (self.dirs.clone(), self.cf_settings(), self.cf_new_hub());
        let lock = super::mods::instance_lock(&inst.root);
        self.start_task(format!("Installing {}", project.name), move |p, _| async move {
            let _turn = lock.lock().await;
            let report =
                install::install_with_dependencies(&dirs, &cfs, &inst, &project, &file, &include, &p, Some(&hub))
                    .await?;
            Ok(format!("{} ({})", report.summary(), inst.name()))
        });
        self.mods_cache = None;
    }

    /// Install the best file of a project that fits the instance, with everything it requires.
    pub fn cf_install_by_id(&mut self, project_id: i64, folder: String, label: String) {
        let Some(inst) = self.instance(&folder).cloned() else { return };
        let (dirs, cfs, hub) = (self.dirs.clone(), self.cf_settings(), self.cf_new_hub());
        let lock = super::mods::instance_lock(&inst.root);
        self.start_task(label, move |p, _| async move {
            let _turn = lock.lock().await;
            let project = api::project(project_id).await?;
            let report = install::install_latest(&dirs, &cfs, &inst, &project, &p, Some(&hub)).await?;
            Ok(format!("{} ({})", report.summary(), inst.name()))
        });
        self.mods_cache = None;
    }

    fn cf_remove(&mut self, folder: &str, project: &Project) {
        let Some(inst) = self.instance(folder).cloned() else { return };
        match install::remove_project(&inst, project.id) {
            Ok(_) => self.toast(format!("Removed {}", project.name)),
            Err(e) => self.toast_err(format!("{e:#}")),
        }
        self.mods_cache = None;
        self.reload_instances();
    }

    /// Install a modpack as a new instance.
    pub fn cf_install_pack(&mut self, project: Project, file: Option<File>, name: String) {
        let (dirs, cfs, hub) = (self.dirs.clone(), self.cf_settings(), self.cf_new_hub());
        let title = project.name.clone();
        self.start_task(format!("Installing {title}"), move |p, _| async move {
            let inst =
                pack::install_pack(&dirs, &cfs, PackSource::Project { project, file }, Some(name), &p, Some(&hub))
                    .await?;
            Ok(format!("Created {}", inst.name()))
        });
    }

    /// Make an instance from a downloaded CurseForge `.zip` (also used when one is dropped on the window).
    pub fn cf_import_zip(&mut self, path: PathBuf) {
        let (dirs, cfs, hub) = (self.dirs.clone(), self.cf_settings(), self.cf_new_hub());
        let label = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.start_task(format!("Importing {label}"), move |p, _| async move {
            let inst = pack::install_pack(&dirs, &cfs, PackSource::Zip(path), None, &p, Some(&hub)).await?;
            Ok(format!("Created {}", inst.name()))
        });
    }

    fn cf_import_text(&mut self, text: String) {
        if text.starts_with("http") {
            let (dirs, cfs, hub) = (self.dirs.clone(), self.cf_settings(), self.cf_new_hub());
            self.start_task("Installing from CurseForge", move |p, _| async move {
                let inst = pack::install_from_url(&dirs, &cfs, &text, None, &p, Some(&hub)).await?;
                Ok(format!("Created {}", inst.name()))
            });
        } else {
            self.cf_import_zip(PathBuf::from(text));
        }
    }

    // -----------------------------------------------------------------------------------------------------
    // updates

    /// Look for newer files of the CurseForge mods in an instance and show them in a list.
    pub fn cf_check_mod_updates(&mut self, ctx: &egui::Context, folder: &str) {
        let Some(inst) = self.instance(folder).cloned() else { return };
        self.cf.updates = Some(UpdateWin {
            folder: Some(folder.to_string()),
            title: format!("Updates for {}", inst.name()),
            loading: true,
            error: None,
            mods: Vec::new(),
            packs: Vec::new(),
        });
        let (dirs, cfs, tx, ctx, folder) =
            (self.dirs.clone(), self.cf_settings(), self.cf.tx.clone(), ctx.clone(), folder.to_string());
        let lock = super::mods::instance_lock(&inst.root);
        self.rt.spawn(async move {
            // mods the launcher has not identified yet are looked up by fingerprint first (which saves what it
            // learns, so it waits its turn with other changes to the instance)
            {
                let _turn = lock.lock().await;
                let _ = install::identify_unknown_mods(&cfs, &inst).await;
            }
            let fresh = crate::instances::load_one(&inst.root).unwrap_or(inst);
            let result = install::check_updates(&dirs, &cfs, &fresh).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(CfMsg::ModUpdates { folder, result });
            ctx.request_repaint();
        });
    }

    /// Look for newer files of every CurseForge modpack instance.
    pub fn cf_check_pack_updates(&mut self, ctx: &egui::Context) {
        self.cf.updates = Some(UpdateWin {
            folder: None,
            title: "Modpack updates".into(),
            loading: true,
            error: None,
            mods: Vec::new(),
            packs: Vec::new(),
        });
        let (dirs, cfs, instances, tx, ctx) =
            (self.dirs.clone(), self.cf_settings(), self.instances.clone(), self.cf.tx.clone(), ctx.clone());
        self.rt.spawn(async move {
            let result =
                pack::check_pack_updates(&dirs, &cfs, &instances).await.map_err(|e| format!("{e:#}")).map(|map| {
                    let mut list: Vec<(String, String, PackUpdate)> = map
                        .into_iter()
                        .map(|(folder, u)| {
                            (
                                instances
                                    .iter()
                                    .find(|i| i.folder == folder)
                                    .map(|i| i.name().to_string())
                                    .unwrap_or_else(|| folder.clone()),
                                folder,
                                u,
                            )
                        })
                        .map(|(n, f, u)| (f, n, u))
                        .collect();
                    list.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
                    list
                });
            let _ = tx.send(CfMsg::PackUpdates { result });
            ctx.request_repaint();
        });
    }

    /// Update mods of one instance, one after the other in one task: each update rewrites the instance's mod list,
    /// so running them side by side could lose some of the changes.
    fn cf_apply_mod_updates(&mut self, folder: &str, updates: Vec<ModUpdate>) {
        let Some(inst) = self.instance(folder).cloned() else { return };
        let label = match updates.as_slice() {
            [] => return,
            [one] => format!("Updating {}", one.installed_name),
            many => format!("Updating {} mods", many.len()),
        };
        let (dirs, cfs, hub) = (self.dirs.clone(), self.cf_settings(), self.cf_new_hub());
        let lock = super::mods::instance_lock(&inst.root);
        self.start_task(label, move |p, _| async move {
            let _turn = lock.lock().await;
            let mut done = Vec::new();
            let mut failed = Vec::new();
            for u in &updates {
                match install::apply_update(&dirs, &cfs, &inst, u, &p, Some(&hub)).await {
                    Ok(_) => done.push(u.project.name.clone()),
                    Err(e) => failed.push(format!("{} ({e:#})", u.project.name)),
                }
                if hub.is_cancelled() {
                    break;
                }
            }
            if failed.is_empty() {
                Ok(format!("Updated {}", done.join(", ")))
            } else {
                anyhow::bail!("Updated {} of {}. Couldn't update {}", done.len(), updates.len(), failed.join(", "))
            }
        });
        self.mods_cache = None;
    }

    fn cf_apply_pack_update(&mut self, folder: &str, update: PackUpdate) {
        let Some(inst) = self.instance(folder).cloned() else { return };
        let (dirs, cfs, hub) = (self.dirs.clone(), self.cf_settings(), self.cf_new_hub());
        self.start_task(format!("Updating {}", inst.name()), move |p, _| async move {
            let updated = pack::update_pack(&dirs, &cfs, &inst, &update.latest, &p, Some(&hub)).await?;
            Ok(format!("Updated {} to {}", updated.name(), update.latest.display()))
        });
    }

    // -----------------------------------------------------------------------------------------------------
    // windows

    /// Call once per frame: delivers background answers and draws whatever CurseForge window is open.
    pub(super) fn cf_windows(&mut self, ctx: &egui::Context) {
        // pick up a changed API key from the settings
        curseforge::configure(&self.settings);
        while let Ok(m) = self.cf.rx.try_recv() {
            self.cf_handle(ctx, m);
        }
        self.cf_dialog_ui(ctx);
        self.cf_browser_ui(ctx);
        self.cf_updates_ui(ctx);
    }

    fn cf_handle(&mut self, ctx: &egui::Context, msg: CfMsg) {
        match msg {
            CfMsg::Search { seq, reset, result } => {
                if seq != self.cf.seq {
                    return;
                }
                self.cf.loading = false;
                match result {
                    Ok(page) => {
                        if reset {
                            self.cf.projects.clear();
                        }
                        self.cf.projects.extend(page.projects);
                        self.cf.total = page.total;
                        self.cf.has_more = page.has_more;
                        self.cf.error = None;
                    }
                    Err(e) => {
                        self.cf.error = Some(e);
                        if !reset {
                            self.cf.page = self.cf.page.saturating_sub(1);
                        }
                    }
                }
            }
            CfMsg::Categories { section, result } => {
                if self.cf.categories_for == Some(section) {
                    self.cf.categories = result.unwrap_or_default();
                }
            }
            CfMsg::Files { dialog, result } => {
                let cfs = self.cf_settings();
                let Some(d) = self.cf.dialog.as_mut().filter(|d| d.id == dialog) else { return };
                let target = d.target.as_ref().map(|(t, _)| t.clone());
                d.loading = false;
                match result {
                    Ok(files) => {
                        d.all_files = files;
                        Self::cf_refilter(d, target.as_ref(), &cfs);
                        if d.files.is_empty() && !d.all_files.is_empty() && target.is_some() {
                            // nothing fits: say so and let the player see every file
                            d.error = Some("No file of this project fits the instance. You can still choose from all of them below.".into());
                            d.show_every_file = true;
                            Self::cf_refilter(d, target.as_ref(), &cfs);
                        }
                        if d.all_files.is_empty() {
                            d.error = Some("No files found for this project. CurseForge may be down or having issues; try again in a few minutes.".into());
                        }
                    }
                    Err(e) => d.error = Some(e),
                }
                self.cf_request_deps(ctx);
            }
            CfMsg::Deps { dialog, file, result } => {
                if let Some(d) = self.cf.dialog.as_mut().filter(|d| d.id == dialog && d.deps_for == Some(file)) {
                    d.deps_loading = false;
                    let list = match result {
                        Ok(list) => list,
                        Err(e) => {
                            d.deps_error = Some(e);
                            Vec::new()
                        }
                    };
                    d.deps = list
                        .into_iter()
                        .map(|dep| {
                            let on = dep.is_required() && dep.file.is_some();
                            (dep, on)
                        })
                        .collect();
                }
            }
            CfMsg::Changelog { dialog, file, result } => {
                if let Some(d) = self.cf.dialog.as_mut().filter(|d| d.id == dialog) {
                    d.changelog_loading = false;
                    // turned into text once here, not on every frame it is shown
                    d.changelog = Some((file, result.map(|html| html_to_text(&html))));
                }
            }
            CfMsg::ModUpdates { folder, result } => {
                if let Some(w) = self.cf.updates.as_mut().filter(|w| w.folder.as_deref() == Some(&folder)) {
                    w.loading = false;
                    match result {
                        Ok(list) => w.mods = list,
                        Err(e) => w.error = Some(e),
                    }
                }
            }
            CfMsg::PackUpdates { result } => {
                if let Some(w) = self.cf.updates.as_mut().filter(|w| w.folder.is_none()) {
                    w.loading = false;
                    match result {
                        Ok(list) => w.packs = list,
                        Err(e) => w.error = Some(e),
                    }
                }
            }
            CfMsg::Project { result, folder, installed_file, reinstall } => match result {
                Ok(p) => self.cf_open_dialog(ctx, p, folder, installed_file, reinstall),
                Err(e) => self.toast_err(e),
            },
        }
    }

    /// Choose the files to list and the one to preselect.
    fn cf_refilter(d: &mut FileDialog, target: Option<&Target>, cfs: &CfSettings) {
        d.files = match (target, d.show_every_file) {
            (Some(t), false) => filter::files_for_target(&d.project, &d.all_files, t, cfs),
            _ => {
                let mut all = d.all_files.clone();
                all.sort_by(|a, b| b.id.cmp(&a.id));
                all
            }
        };
        let installed_at = d.installed_file.and_then(|id| d.files.iter().position(|f| f.id == id));
        d.selected = match installed_at {
            Some(i) if d.reinstall => i,
            _ => filter::pick_latest(&d.files, cfs.allow_alpha_beta)
                .and_then(|f| d.files.iter().position(|x| x.id == f.id))
                .unwrap_or(0),
        };
        d.deps_for = None;
        d.deps.clear();
        d.deps_error = None;
    }

    /// Ask for the dependencies of the file that is selected in the chooser.
    fn cf_request_deps(&mut self, ctx: &egui::Context) {
        let Some(d) = self.cf.dialog.as_mut() else { return };
        let Some(file) = d.files.get(d.selected).cloned() else { return };
        if d.deps_for == Some(file.id) {
            return;
        }
        let Some(inst) = d.folder.clone().and_then(|f| self.instances.iter().find(|i| i.folder == f).cloned()) else {
            return;
        };
        d.deps_for = Some(file.id);
        d.deps_loading = true;
        d.deps_error = None;
        d.deps.clear();
        let (dialog, dirs, cfs, tx, ctx) =
            (d.id, self.dirs.clone(), CfSettings::from_settings(&self.settings), self.cf.tx.clone(), ctx.clone());
        self.rt.spawn(async move {
            let result = install::dependencies_of(&dirs, &cfs, &inst, &file).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(CfMsg::Deps { dialog, file: file.id, result });
            ctx.request_repaint();
        });
    }

    /// The chooser: version, dependencies, changelog, Add.
    fn cf_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.cf.dialog.take() else { return };
        let cfs = self.cf_settings();
        // the instance is gone (deleted while the dialog was open): nothing can be added to it
        let target = d.target.clone().filter(|_| d.folder.as_deref().is_some_and(|f| self.instance(f).is_some()));
        let is_pack = d.project.class_id == curseforge::CLASS_MODPACKS;
        let mut close = false;
        let mut install_now = false;
        let mut want_changelog = false;
        let mut reselect = false;
        let mut retry_deps = false;
        let before = d.selected;
        let (width, body_height) = modal_size(ctx, 600.0, 200.0);

        let modal = egui::Modal::new(Id::unique("cf-file-dialog")).show(ctx, |ui| {
            ui.set_width(width);
            ui.add(egui::Label::new(RichText::new(format!("Installing {}", d.project.name)).size(19.0).strong()).wrap());
            if let Some((_, name)) = &target {
                small_muted(ui, format!("into {name}"));
            } else if is_pack {
                small_muted(ui, "as a new instance");
            } else if d.folder.is_some() {
                ui.label(RichText::new("That instance isn't there any more.").color(c_danger()));
            }
            ui.add_space(6.0);
            let body = ScrollArea::vertical().id_salt("cf-file-body").max_height(body_height).auto_shrink([false, true]);
            body.show(ui, |ui| {
                if d.loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        muted(ui, "Getting the files");
                    });
                }
                if let Some(e) = &d.error {
                    ui.label(RichText::new(e).color(c_warn()));
                }
                if !d.files.is_empty() {
                    if let Some(installed) = d.installed_file.and_then(|id| d.all_files.iter().find(|f| f.id == id)) {
                        ui.label(RichText::new(format!("The version currently installed is {}", installed.label())).color(c_accent()));
                    }
                    ui.horizontal(|ui| {
                        muted(ui, "Version to install");
                        let selected_text = d.files.get(d.selected).map(|f| f.label()).unwrap_or_default();
                        let combo_width = (ui.available_width() - 8.0).clamp(160.0, 420.0);
                        egui::ComboBox::from_id_salt("cf-file-version").selected_text(selected_text).width(combo_width).show_ui(ui, |ui| {
                            for (i, f) in d.files.iter().enumerate() {
                                let mut label = f.label();
                                if d.installed_file == Some(f.id) {
                                    label.push_str("  (installed)");
                                }
                                ui.selectable_value(&mut d.selected, i, label);
                            }
                        });
                    });
                    if let Some(f) = d.files.get(d.selected) {
                        ui.horizontal_wrapped(|ui| {
                            match f.release_type {
                                Some(2) => tag(ui, "Beta", c_warn()),
                                Some(3) => tag(ui, "Alpha", c_danger()),
                                _ => tag(ui, "Release", c_accent()),
                            }
                            for g in f.game_versions.iter().take(8) {
                                tag(ui, g, c_muted());
                            }
                            small_muted(ui, human_size(f.file_length.max(0) as u64));
                            small_muted(ui, f.file_date.chars().take(10).collect::<String>());
                        });
                        if f.download_url.is_none() {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new("The author of this file does not let launchers download it. Your browser will open so you can download it yourself; BlockForge then picks the file up from your Downloads folder.")
                                    .color(c_warn()),
                            );
                        }
                    }
                    if d.show_every_file || d.error.is_some() || target.is_some() {
                        if ui.checkbox(&mut d.show_every_file, "Show files that do not match this instance").changed() {
                            reselect = true;
                        }
                    }
                    if is_pack {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            muted(ui, "Instance name");
                            ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(320.0).background_color(c_field()));
                        });
                    }

                    // dependencies
                    if target.is_some() {
                        if d.deps_loading {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                muted(ui, "Checking what this file needs");
                            });
                        } else if let Some(e) = &d.deps_error {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(format!("Couldn't check which other mods this one needs: {e}"))
                                        .color(c_warn()),
                                );
                                if secondary(ui, "Try again").clicked() {
                                    retry_deps = true;
                                }
                            });
                        } else if !d.deps.is_empty() {
                            ui.add_space(6.0);
                            ui.label(RichText::new("These mods go with it").strong());
                            small_muted(ui, "Required ones are ticked. Tick the others to add them too.");
                            ScrollArea::vertical().max_height(170.0).id_salt("cf-deps").show(ui, |ui| {
                                for (dep, on) in d.deps.iter_mut() {
                                    ui.horizontal(|ui| {
                                        let usable = dep.file.is_some();
                                        ui.add_enabled(usable, egui::Checkbox::new(on, ""));
                                        ui.vertical(|ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(RichText::new(&dep.project.name).strong());
                                                if dep.is_required() {
                                                    tag(ui, "Required", c_accent());
                                                } else {
                                                    tag(ui, "Optional", c_muted());
                                                }
                                            });
                                            match &dep.file {
                                                Some(f) => small_muted(ui, f.label()),
                                                None => small_muted(ui, "No file fits this instance"),
                                            }
                                        });
                                    });
                                }
                            });
                        }
                    }

                    // changelog
                    if let Some(f) = d.files.get(d.selected) {
                        let fid = f.id;
                        match &d.changelog {
                            Some((id, Ok(text))) if *id == fid => {
                                ui.add_space(6.0);
                                ui.label(RichText::new("What changed").strong());
                                ScrollArea::vertical().max_height(150.0).id_salt("cf-changelog").show(ui, |ui| {
                                    ui.label(text.as_str());
                                });
                            }
                            Some((id, Err(e))) if *id == fid => {
                                small_muted(ui, format!("Couldn't get the changelog: {e}"));
                            }
                            _ => {
                                if d.changelog_loading {
                                    ui.spinner();
                                } else if ui.link("Show what changed in this version").clicked() {
                                    want_changelog = true;
                                }
                            }
                        }
                    }
                }
            });
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                // wait for the list of what the file needs, or the mod would go in without it
                let can = !d.files.is_empty() && (is_pack || (target.is_some() && !d.deps_loading));
                if primary_enabled(ui, if is_pack { "Install" } else { "Add" }, can).clicked() {
                    install_now = true;
                }
                if secondary(ui, "View page").clicked() {
                    if let Some(url) = d.project.website_url() {
                        let _ = open::that(url);
                    }
                }
                if let Some(f) = d.files.get(d.selected) {
                    if secondary(ui, "View file").clicked() {
                        let _ = open::that(d.project.file_page_url(f));
                    }
                }
                if secondary(ui, "Cancel").clicked() {
                    close = true;
                }
            });
        });
        if modal.should_close() {
            close = true;
        }

        if reselect {
            let target_only = target.as_ref().map(|(t, _)| t.clone());
            Self::cf_refilter(&mut d, target_only.as_ref(), &cfs);
        }
        if want_changelog {
            if let Some(f) = d.files.get(d.selected) {
                d.changelog_loading = true;
                let (dialog, pid, fid, tx, c) = (d.id, d.project.id, f.id, self.cf.tx.clone(), ctx.clone());
                self.rt.spawn(async move {
                    let result = api::changelog(pid, fid).await.map_err(|e| format!("{e:#}"));
                    let _ = tx.send(CfMsg::Changelog { dialog, file: fid, result });
                    c.request_repaint();
                });
            }
        }
        if install_now {
            if let Some(file) = d.files.get(d.selected).cloned() {
                let include: Vec<i64> = d
                    .deps
                    .iter()
                    .filter(|(dep, on)| *on && dep.file.is_some())
                    .map(|(dep, _)| dep.project.id)
                    .collect();
                if is_pack {
                    let name =
                        if d.name.trim().is_empty() { d.project.name.clone() } else { d.name.trim().to_string() };
                    self.cf_install_pack(d.project.clone(), Some(file), name);
                } else if let Some(folder) = d.folder.clone() {
                    self.cf_install_file(&folder, d.project.clone(), file, include);
                }
                close = true;
            }
        }
        if close {
            return;
        }
        let changed_selection = d.selected != before || retry_deps;
        self.cf.dialog = Some(d);
        if changed_selection {
            if let Some(d) = self.cf.dialog.as_mut() {
                d.deps_for = None;
            }
        }
        self.cf_request_deps(ctx);
    }

    /// The window that lists files to download by hand and watches for them.
    fn cf_browser_ui(&mut self, ctx: &egui::Context) {
        // an install that ended (finished, failed, gave up waiting or was cancelled) has let go of its hub; its
        // files may still say "waiting", but nothing waits for them any more
        self.cf.browsers.retain(|h| Arc::strong_count(h) > 1);
        let Some(hub) = self.cf.browsers.iter().find(|h| h.waiting() && !h.is_cancelled()).cloned() else {
            self.cf.confirm_skip = false;
            return;
        };
        if !self.cf.notice_shown && !self.cf_settings().seen_distribution_notice {
            self.cf.notice_shown = true;
            CfSettings::mark_notice_seen(&mut self.settings);
            self.save_settings();
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(1000));
        let items = hub.items();
        let folder = hub.collect_dir.clone();
        let user_folder = hub.watch_dirs.last().cloned().unwrap_or_else(|| folder.clone());
        let mut open_all = false;
        let mut skip_remaining = false;
        let mut cancel = false;
        let mut skip_one: Option<u64> = None;
        let mut confirm = self.cf.confirm_skip;

        let (width, list_height) = modal_size(ctx, 660.0, 300.0);
        egui::Modal::new(Id::unique("cf-browser-dialog")).show(ctx, |ui| {
            ui.set_width(width);
            ui.label(RichText::new("Download these in your browser").size(19.0).strong());
            ui.add_space(4.0);
            ui.label("The authors of these files do not let launchers download them. Open each one in your browser and save it to your Downloads folder; BlockForge notices it there and carries on by itself.");
            small_muted(ui, format!("Watching: {}", user_folder.display()));
            ui.add_space(8.0);
            ScrollArea::vertical().max_height(list_height.min(320.0)).id_salt("cf-browser-list").show(ui, |ui| {
                for it in &items {
                    egui::Frame::new().fill(c_card()).stroke(Stroke::new(1.0, c_border())).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.set_max_width((ui.available_width() - 260.0).max(120.0));
                                ui.add(egui::Label::new(RichText::new(&it.request.label).strong()).truncate());
                                ui.add(egui::Label::new(RichText::new(&it.request.file_name).color(c_muted()).size(12.0)).truncate());
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| match &it.state {
                                ItemState::Waiting => {
                                    if secondary(ui, "Skip").clicked() {
                                        skip_one = Some(it.id);
                                    }
                                    if secondary(ui, "Copy link").clicked() {
                                        ui.ctx().copy_text(it.request.url.clone());
                                    }
                                    if primary(ui, "Open").clicked() {
                                        let _ = open::that(&it.request.url);
                                    }
                                }
                                ItemState::Found(_) => {
                                    ui.label(RichText::new("Download complete").color(c_accent()));
                                }
                                ItemState::Skipped => {
                                    ui.label(RichText::new("Skipped").color(c_muted()));
                                }
                            });
                        });
                    });
                }
            });
            ui.add_space(8.0);
            if confirm {
                ui.label(RichText::new("Skipping the remaining downloads may cause issues launching the modpack. Are you sure?").color(c_warn()));
                ui.horizontal(|ui| {
                    if danger(ui, "Skip them").clicked() {
                        skip_remaining = true;
                        confirm = false;
                    }
                    if secondary(ui, "Keep waiting").clicked() {
                        confirm = false;
                    }
                });
            } else {
                ui.horizontal_wrapped(|ui| {
                    if secondary(ui, "Open folder").clicked() {
                        let _ = open::that(&user_folder);
                    }
                    if primary(ui, "Open all in browser").clicked() {
                        open_all = true;
                    }
                    if secondary(ui, "Skip remaining").clicked() {
                        confirm = true;
                    }
                    if danger(ui, "Cancel install").clicked() {
                        cancel = true;
                    }
                });
            }
        });
        self.cf.confirm_skip = confirm;
        if let Some(id) = skip_one {
            hub.skip(id);
        }
        if open_all {
            for url in hub.waiting_urls() {
                let _ = open::that(url);
            }
        }
        if skip_remaining {
            hub.skip_remaining();
        }
        if cancel {
            hub.cancel();
        }
    }

    /// The list of newer files.
    fn cf_updates_ui(&mut self, ctx: &egui::Context) {
        let Some(w) = self.cf.updates.take() else { return };
        let mut w = w;
        let mut close = false;
        let mut apply_mod: Option<ModUpdate> = None;
        let mut apply_pack: Option<(String, PackUpdate)> = None;
        let mut ignore_pack: Option<(String, i64)> = None;
        let mut update_all = false;
        let (width, list_height) = modal_size(ctx, 620.0, 220.0);
        let modal = egui::Modal::new(Id::unique("cf-updates")).show(ctx, |ui| {
            ui.set_width(width);
            ui.label(RichText::new(&w.title).size(19.0).strong());
            ui.add_space(6.0);
            if w.loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    muted(ui, "Checking CurseForge");
                });
            }
            if let Some(e) = &w.error {
                ui.label(RichText::new(e).color(c_danger()));
            }
            if !w.loading && w.error.is_none() && w.mods.is_empty() && w.packs.is_empty() {
                muted(ui, "Everything is up to date.");
            }
            ScrollArea::vertical().max_height(list_height.min(340.0)).id_salt("cf-updates-list").show(ui, |ui| {
                for u in &w.mods {
                    card(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.set_max_width((ui.available_width() - 110.0).max(160.0));
                                ui.label(RichText::new(&u.project.name).strong());
                                small_muted(ui, format!("{}  ->  {}", u.installed_file, u.latest.label()));
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if primary(ui, "Update").clicked() {
                                    apply_mod = Some(u.clone());
                                }
                            });
                        });
                    });
                }
                for (folder, name, u) in &w.packs {
                    card(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.set_max_width((ui.available_width() - 170.0).max(160.0));
                                ui.label(RichText::new(name).strong());
                                small_muted(ui, format!("New version: {}", u.latest.label()));
                                if u.ignored {
                                    small_muted(ui, "You chose to ignore this update");
                                }
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if primary(ui, "Update").clicked() {
                                    apply_pack = Some((folder.clone(), u.clone()));
                                }
                                if !u.ignored && secondary(ui, "Ignore").clicked() {
                                    ignore_pack = Some((folder.clone(), u.latest.id));
                                }
                            });
                        });
                    });
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if w.mods.len() > 1 && primary(ui, "Update all").clicked() {
                    update_all = true;
                }
                if secondary(ui, "Close").clicked() {
                    close = true;
                }
            });
        });
        if modal.should_close() {
            close = true;
        }
        if let (Some(folder), Some(u)) = (w.folder.clone(), apply_mod) {
            w.mods.retain(|m| m.project.id != u.project.id);
            self.cf_apply_mod_updates(&folder, vec![u]);
        }
        if update_all {
            if let Some(folder) = w.folder.clone() {
                let all = std::mem::take(&mut w.mods);
                self.cf_apply_mod_updates(&folder, all);
            }
        }
        if let Some((folder, u)) = apply_pack {
            self.cf_apply_pack_update(&folder, u);
            w.packs.retain(|p| p.0 != folder);
        }
        if let Some((folder, id)) = ignore_pack {
            if let Some(inst) = self.instance(&folder).cloned() {
                if let Err(e) = pack::ignore_update(&inst, id) {
                    self.toast_err(format!("{e:#}"));
                }
            }
            for p in w.packs.iter_mut().filter(|p| p.0 == folder) {
                p.2.ignored = true;
            }
            self.reload_instances();
        }
        if !close {
            self.cf.updates = Some(w);
        }
    }
}

/// CurseForge's descriptions and changelogs are HTML; turn them into readable text.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            for t in chars.by_ref() {
                if t == '>' {
                    break;
                }
                tag.push(t);
            }
            let name = tag.trim_start_matches('/').split_whitespace().next().unwrap_or("").to_lowercase();
            let closing = tag.starts_with('/');
            match name.as_str() {
                "br" => out.push('\n'),
                "li" if !closing => out.push_str("\n\u{2022} "),
                "p" | "div" | "h1" | "h2" | "h3" | "h4" | "ul" | "ol" if closing => out.push('\n'),
                _ => {}
            }
        } else if c == '&' {
            let mut ent = String::new();
            while let Some(&n) = chars.peek() {
                if n == ';' || ent.len() > 8 {
                    break;
                }
                ent.push(n);
                chars.next();
            }
            if chars.peek() == Some(&';') {
                chars.next();
                match ent.as_str() {
                    "amp" => out.push('&'),
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    "quot" => out.push('"'),
                    "apos" | "#39" => out.push('\''),
                    "nbsp" => out.push(' '),
                    other => {
                        // &#39; and &#x27; alike
                        let code = match other.strip_prefix("#x").or_else(|| other.strip_prefix("#X")) {
                            Some(hex) => u32::from_str_radix(hex, 16).ok(),
                            None => other.strip_prefix('#').and_then(|n| n.parse::<u32>().ok()),
                        };
                        if let Some(c) = code.and_then(char::from_u32) {
                            out.push(c);
                        }
                    }
                }
            } else {
                out.push('&');
                out.push_str(&ent);
            }
        } else {
            out.push(c);
        }
    }
    // squeeze runs of blank lines
    let mut result = String::new();
    let mut blanks = 0;
    for line in out.lines() {
        if line.trim().is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        result.push_str(line.trim_end());
        result.push('\n');
    }
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::{html_to_text, modal_size};

    #[test]
    fn changelog_html_becomes_text() {
        let t = html_to_text("<p>Fixed &amp; improved</p><ul><li>one</li><li>two &lt;3</li></ul><br/>done");
        assert!(t.starts_with("Fixed & improved"));
        assert!(t.contains("\u{2022} one\n\u{2022} two <3"));
        assert!(t.ends_with("done"));
        assert!(!t.contains('<') || t.contains("<3"));
        assert_eq!(html_to_text("it&#39;s &#x27;fine&#X27; &#xZZ; &bogus"), "it's 'fine'  &bogus");
    }

    #[test]
    fn dialogs_fit_small_windows() {
        let ctx = egui::Context::default();
        let mut sizes = Vec::new();
        for (w, h) in [(1280.0, 720.0), (640.0, 360.0)] {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| sizes.push(modal_size(ui.ctx(), 600.0, 200.0)));
            out.textures_delta.clear();
        }
        assert_eq!(sizes[0], (600.0, 520.0));
        // narrower than the window, and the scrolling part never vanishes
        assert_eq!(sizes[1], (592.0, 160.0));
    }
}
