//! CurseForge in the window: the Modrinth / CurseForge switch on Discover, CurseForge project cards, the file and
//! dependency chooser, the "download this one in your browser" window, and the update lists.
//!
//! Everything here is `impl App`, like the other pages. It keeps its own small message channel (`CfState`) so the
//! window's main `Msg` enum does not need new variants; the lead only has to call [`App::cf_windows`] once per frame.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use egui::{Id, RichText, ScrollArea, Ui};

use super::App;
use super::theme::*;
use super::widgets::*;
use crate::curseforge::api::{self, SearchPage, SearchQuery, Section, Sort};
use crate::curseforge::browser::{BrowserHub, ItemState};
use crate::curseforge::filter::{self, Target};
use crate::curseforge::install::{self, Dependency, ModUpdate};
use crate::curseforge::model::{CategoryForGame, File, GameVersionLatestFiles, Project};
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

/// The file chooser for one project ("Installing X" in the Java edition; "Add X" here, as its button says).
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
    /// When the search field was last typed in and its search has not started yet.
    edited: Option<Instant>,
    import_text: String,
    dialog: Option<FileDialog>,
    next_dialog: u64,
    /// One per install that may need files downloaded by hand; the window shows the first with files waiting.
    browsers: Vec<Arc<BrowserHub>>,
    confirm_skip: bool,
    /// Whether the launcher's configuration allows CurseForge, read once rather than on every frame.
    pub(super) enabled: Option<bool>,
    target_info: Option<TargetInfo>,
    notice_shown: bool,
    updates: Option<UpdateWin>,
    /// A mod waiting for "Remove" to be confirmed, with the instance folder it is in.
    confirm_remove: Option<(Project, String)>,
    /// Counts the result lists that replaced the one before, so a new search starts at the top of the list.
    list_gen: u64,
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
            edited: None,
            import_text: String::new(),
            dialog: None,
            next_dialog: 1,
            browsers: Vec::new(),
            confirm_skip: false,
            enabled: None,
            target_info: None,
            notice_shown: false,
            updates: None,
            confirm_remove: None,
            list_gen: 0,
            tx,
            rx,
        }
    }
}

/// What a project card asked for.
enum CardAct {
    Install(Project, Option<String>),
    Reinstall(Project, Option<String>),
    /// Put a newer file of an installed project into the instance.
    Update(Project, String),
    Remove(Project, String),
    View(Project),
}

impl App {
    pub(super) fn cf_settings(&self) -> CfSettings {
        CfSettings::from_settings(&self.settings)
    }

    /// Done once: pick the platform from the `defaultModPlatform` setting and hand the API key to the client.
    pub(super) fn cf_init(&mut self) {
        if self.cf.inited {
            return;
        }
        self.cf.inited = true;
        curseforge::configure(&self.settings);
        self.cf.active = self.cf_settings().prefer_curseforge;
    }

    /// Whether the launcher's configuration allows CurseForge, read once until the platform is chosen again.
    pub(super) fn cf_enabled(&mut self) -> bool {
        *self.cf.enabled.get_or_insert_with(|| curseforge::platform_enabled(&self.dirs, "modsEnabled"))
    }

    /// The section the CurseForge page shows.
    pub(super) fn cf_section(&self) -> Section {
        self.cf.section
    }

    /// Show `section` on the CurseForge page, searched afresh the next time the page is drawn.
    pub(super) fn cf_show_section(&mut self, section: Section) {
        if self.cf.section != section {
            self.cf.section = section;
            self.cf.searched = false;
        }
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
            // the old rows stay on screen until the new ones arrive (see `CfMsg::Search`)
            cf.page = 0;
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
        } else {
            // without an instance, the version and loader of the filters (shared with Modrinth's page)
            if cf.section.uses_game_version() {
                q.game_version = self.discover.mc_filter.clone();
            }
            if cf.section.uses_loader() {
                let loader = self.discover.loader_filter.as_deref();
                q.mod_loaders = loader.map(super::discover::cf_loaders_for).unwrap_or_default();
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
        use super::discover::*;
        self.cf_init();
        if !self.cf.active {
            return false;
        }
        // a Horizon instance isn't the target of what it can't take
        let section = self.cf.section;
        if self
            .discover
            .target
            .as_ref()
            .and_then(|f| self.instance(f))
            .is_some_and(|i| !super::edition::takes(i, section.into()))
        {
            self.discover.target = None;
        }
        let ctx = ui.ctx().clone();
        let enabled = *self.cf.enabled.get_or_insert_with(|| curseforge::platform_enabled(&self.dirs, "modsEnabled"));
        if self.explore_header(ui) {
            // another kind or source was picked in the head: it is drawn from the next frame
            return true;
        }
        if !enabled {
            notice(
                ui,
                NoticeKind::Info,
                "CurseForge is switched off. The launcher's configuration doesn't allow CurseForge.",
                None,
            );
            return true;
        }
        let mut search = !self.cf.searched;
        self.cf_load_categories(&ctx);
        let page_w = ui.available_width();
        let hint = format!("Search {}", self.cf.section.label().to_lowercase());
        let mut st = (&mut *self, &mut search);
        search_bar(
            ui,
            "curseforge",
            &mut st,
            |ui, (this, search), width| {
                let mut q = std::mem::take(&mut this.cf.query);
                let mut edited = this.cf.edited.take();
                if live_search_w(ui, &mut q, &hint, &mut edited, width) {
                    **search = true;
                }
                this.cf.query = q;
                this.cf.edited = edited;
            },
            |ui, (this, search)| {
                if this.cf.section != Section::Modpacks {
                    this.target_dropdown(ui, search);
                }
            },
            |ui, (this, search)| {
                this.cf_filter_controls(ui, search);
                this.cf_sort_dropdown(ui, search);
            },
        );
        if self.cf_chip_row(ui) {
            search = true;
        }

        self.cf_context_line(ui, &mut search, page_w);
        if search {
            self.cf_search(&ctx, true);
        }
        // a new search over rows that are already shown: a thin bar says it is on its way
        if self.cf.loading && self.cf.page == 0 && !self.cf.projects.is_empty() {
            progress_bar(ui, None, 2.0);
        } else {
            ui.add_space(2.0);
        }

        if let Some(e) = self.cf.error.clone() {
            let text = "Couldn't reach CurseForge. Check your internet connection, or try again in a few minutes.";
            if fetch_error(ui, text, &e) && !self.cf.loading {
                // the first page again, or the page that failed to load
                self.cf_search(&ctx, self.cf.projects.is_empty());
            }
        }
        if self.cf.projects.is_empty() && self.cf.loading {
            result_skeleton(ui, 4, "Searching CurseForge\u{2026}");
            return true;
        }
        if self.cf.projects.is_empty() && self.cf.error.is_none() && self.cf.searched {
            let query = self.cf.query.trim().to_string();
            let filtered = !self.cf_chips().is_empty();
            let text = match (query.is_empty(), filtered) {
                (true, false) => "Nothing to show. Try another section, or pick a different instance.".to_string(),
                (true, true) => "Nothing on CurseForge matches these filters.".to_string(),
                (false, false) => format!("No results for \u{201C}{query}\u{201D} on CurseForge."),
                (false, true) => format!("No results for \u{201C}{query}\u{201D} on CurseForge with these filters."),
            };
            let action = if filtered { Some("Clear filters") } else { (!query.is_empty()).then_some("Clear search") };
            if empty_state(ui, &text, action).is_some_and(|r| r.clicked()) {
                if filtered {
                    self.cf_clear_filters();
                } else {
                    self.cf.query.clear();
                }
                self.cf_search(&ctx, true);
            }
            return true;
        }

        let installed: HashMap<i64, i64> = self.cf_target_info().map(|t| t.installed.clone()).unwrap_or_default();
        // what the instance runs: its version and loader on every row, and what a newer file has to fit
        let fits = self.cf_target_info().map(|t| (t.target.mc_version.clone(), t.target.search_loaders()));
        let target_compat = self
            .discover
            .target
            .as_ref()
            .and_then(|f| self.instance(f))
            .filter(|_| self.cf.section != Section::Modpacks)
            .map(|i| super::discover::instance_compat(i, self.cf.section.uses_loader()));
        let projects = std::mem::take(&mut self.cf.projects);
        let section = self.cf.section;
        let target = self.discover.target.clone();
        // a Horizon instance only for what it can take
        let instance_choices: Vec<(String, String)> = self
            .instances
            .iter()
            .filter(|i| super::edition::takes(i, section.into()))
            .map(|i| (i.folder.clone(), super::discover::instance_choice(i)))
            .collect();
        let mut act: Option<CardAct> = None;
        let mut more = false;
        let installing: HashSet<String> =
            self.tasks.iter().filter_map(|t| t.label.strip_prefix("Installing ").map(str::to_string)).collect();
        let cards = self.results_as_cards();
        ScrollArea::vertical().id_salt(("cf-list", self.cf.list_gen)).auto_shrink(false).show(ui, |ui| {
            result_list(ui, cards, &projects, |ui, p, width| {
                let is_installed = installed.contains_key(&p.id) && section != Section::Modpacks;
                // a newer release than the one installed that fits the instance
                let newer = is_installed
                    .then_some(fits.as_ref())
                    .flatten()
                    .and_then(|(mc, loaders)| newer_file(&p.latest_files_indexes, mc, loaders, installed[&p.id]));
                let busy = installing.contains(&p.name);
                let mut pills = Vec::new();
                if p.allow_mod_distribution == Some(false) {
                    pills.push(("Download in browser".to_string(), c_warn()));
                }
                let authors = p.author_names();
                let view = ResultView {
                    key: format!("cf:{}", p.id),
                    title: &p.name,
                    by: Some(&authors),
                    summary: &p.summary,
                    pills,
                    compat: target_compat.clone().unwrap_or_else(|| project_compat(p, section)),
                    sites: Vec::new(),
                    tags: p.categories.iter().map(|c| c.name.clone()).collect(),
                    figure: Some((human_count(p.download_count.max(0.0) as u64), "downloads")),
                };
                result_item(ui, width, &view, |ui, slot| {
                    if let Slot::Icon(size) = slot {
                        self.result_icon(ui, p.logo_url(), &p.name, size);
                        return;
                    }
                    if busy {
                        // a second click would open the file chooser again (for a modpack, a second instance)
                        installing_mark(ui);
                    } else if let (Some(_), Some(t)) = (newer, &target) {
                        if update_button(ui).on_hover_text("A newer version fits this instance").clicked() {
                            act = Some(CardAct::Update(p.clone(), t.clone()));
                        }
                    } else if is_installed {
                        installed_mark(ui);
                    } else if section == Section::Modpacks {
                        if install_button(ui, "Install").clicked() {
                            act = Some(CardAct::Install(p.clone(), None));
                        }
                    } else if let Some(t) = &target {
                        if install_button(ui, "Install").clicked() {
                            act = Some(CardAct::Install(p.clone(), Some(t.clone())));
                        }
                    } else {
                        let r = install_button(ui, "Install");
                        egui::Popup::menu(&r).show(|ui| {
                            ui.set_min_width(180.0);
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
                    // the three dots hold what is rarely needed: installing again, the project's page, and
                    // removing it
                    kebab_menu(ui, |ui| {
                        if is_installed {
                            if let Some(t) = &target {
                                if ui.button("Reinstall").clicked() {
                                    act = Some(CardAct::Reinstall(p.clone(), Some(t.clone())));
                                    ui.close();
                                }
                            }
                        }
                        if ui.button("View on CurseForge").clicked() {
                            act = Some(CardAct::View(p.clone()));
                            ui.close();
                        }
                        if is_installed {
                            if let Some(t) = &target {
                                if ui.button(RichText::new("Remove").color(c_danger())).clicked() {
                                    act = Some(CardAct::Remove(p.clone(), t.clone()));
                                    ui.close();
                                }
                            }
                        }
                    });
                });
            });
            ui.add_space(8.0);
            let shown = projects.len() as u64;
            if self.cf.has_more {
                ui.vertical_centered(|ui| {
                    if self.cf.total > shown {
                        small_faint(ui, format!("Showing {} of {}", thousands(shown), thousands(self.cf.total)));
                    }
                    if self.cf.loading {
                        spinner(ui);
                    } else {
                        let next = if self.cf.total > shown {
                            (self.cf.total - shown).min(u64::from(curseforge::PAGE_SIZE)).to_string()
                        } else {
                            curseforge::PAGE_SIZE.to_string()
                        };
                        if ghost(ui, &format!("Load {next} more")).clicked() {
                            more = true;
                        }
                    }
                });
            } else if self.cf.loading {
                ui.vertical_centered(|ui| {
                    spinner(ui);
                });
            }
            ui.add_space(8.0);
        });
        self.cf.projects = projects;
        match act {
            Some(CardAct::Install(p, folder)) => self.cf_open_dialog(&ctx, p, folder, None, false),
            Some(CardAct::Reinstall(p, folder)) => {
                let installed_file = installed_file_of(folder.as_ref().and_then(|f| self.instance(f)), p.id);
                self.cf_open_dialog(&ctx, p, folder, installed_file, true);
            }
            Some(CardAct::Update(p, folder)) => {
                // the newest file is chosen in the window; the one installed is named there
                let installed_file = installed_file_of(self.instance(&folder), p.id);
                self.cf_open_dialog(&ctx, p, Some(folder), installed_file, false);
            }
            Some(CardAct::Remove(p, folder)) => self.cf.confirm_remove = Some((p, folder)),
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

    /// The filters of the CurseForge page's search bar, after the field, the instance and the order: the version and
    /// loader while none is chosen, and the category.
    fn cf_filter_controls(&mut self, ui: &mut Ui, search: &mut bool) {
        use super::discover::*;
        let section = self.cf.section;
        if self.cf_target_info().is_none() {
            if section.uses_game_version() {
                self.version_dropdown(ui, search);
            }
            if section.uses_loader() {
                self.loader_dropdown(ui, search);
            }
        }
        let cf = &mut self.cf;
        let current = cf
            .category
            .and_then(|id| cf.categories.iter().find(|c| c.id == id))
            .map(|c| clip(&c.name, 22))
            .unwrap_or_else(|| "Any".into());
        dropdown(ui, "cf-category", "Category", &current, cf.category.is_some(), false, |ui| {
            ui.set_min_width(220.0);
            if ui.selectable_label(cf.category.is_none(), "All categories").clicked() {
                cf.category = None;
                *search = true;
            }
            ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                for c in &cf.categories {
                    if ui.selectable_label(cf.category == Some(c.id), &c.name).clicked() {
                        cf.category = Some(c.id);
                        *search = true;
                    }
                }
            });
        });
    }

    /// The order of the CurseForge page's list.
    fn cf_sort_dropdown(&mut self, ui: &mut Ui, search: &mut bool) {
        let sort = &mut self.cf.sort;
        super::discover::dropdown(ui, "cf-sort", "Sort", sort.label(), false, false, |ui| {
            for s in Sort::ALL {
                if ui.selectable_value(sort, s, s.label()).clicked() {
                    *search = true;
                }
            }
        });
    }

    /// The CurseForge page's filters that are on: the version and loader (while no instance is chosen) and the
    /// category. Each with what taking it off clears.
    fn cf_chips(&mut self) -> Vec<(String, u8)> {
        let section = self.cf.section;
        let mut chips = Vec::new();
        if self.cf_target_info().is_none() {
            if let Some(v) = self.discover.mc_filter.as_ref().filter(|_| section.uses_game_version()) {
                chips.push((format!("Minecraft {v}"), 0));
            }
            if let Some(l) = self.discover.loader_filter.as_deref().filter(|_| section.uses_loader()) {
                let label = super::discover::LOADERS.iter().find(|(n, _)| *n == l).map_or(l, |(_, label)| *label);
                chips.push((label.to_string(), 1));
            }
        }
        if let Some(c) = self.cf.category.and_then(|id| self.cf.categories.iter().find(|c| c.id == id)) {
            chips.push((c.name.clone(), 2));
        }
        chips
    }

    /// Takes the CurseForge page's filters off: the version, the loader and the category.
    fn cf_clear_filters(&mut self) {
        self.discover.mc_filter = None;
        self.discover.loader_filter = None;
        self.cf.category = None;
    }

    /// The chips of the filters that are on, under the search bar. Returns true when one was taken off.
    fn cf_chip_row(&mut self, ui: &mut Ui) -> bool {
        use super::discover::{ChipAct, chip_row};
        let chips = self.cf_chips();
        let labels: Vec<String> = chips.iter().map(|(l, _)| l.clone()).collect();
        match chip_row(ui, &labels, None) {
            Some(ChipAct::Remove(i)) => {
                match chips[i].1 {
                    0 => self.discover.mc_filter = None,
                    1 => self.discover.loader_filter = None,
                    _ => self.cf.category = None,
                }
                true
            }
            Some(ChipAct::Clear) => {
                self.cf_clear_filters();
                true
            }
            None => false,
        }
    }

    /// Under the filters: what the chosen instance runs and the update check, the number of results, and the loader
    /// helper (Fabric API and friends) when it is missing.
    fn cf_context_line(&mut self, ui: &mut Ui, search: &mut bool, page_w: f32) {
        use super::discover::*;
        let section = self.cf.section;
        let target = if section == Section::Modpacks { None } else { self.discover.target.clone() };
        let limited_to = target.as_ref().and_then(|f| self.instance(f)).map(|inst| {
            format!(
                "Showing what works with Minecraft {}{}",
                inst.mc_version(),
                inst.loader().map(|(k, _)| format!(" and {k}")).unwrap_or_default()
            )
        });
        let count = (self.cf.total > 0).then(|| results_text(self.cf.total, "result", "results"));
        let mut show_everything = false;
        let (cards, mut flip) = (self.results_as_cards(), None);
        if limited_to.is_some() || count.is_some() {
            context_line_with(
                ui,
                count,
                limited_to.is_some() && section != Section::Modpacks,
                |ui| {
                    if let (Some(text), Some(folder)) = (limited_to, target) {
                        small_muted(ui, text);
                        if ghost(ui, "Show everything").clicked() {
                            show_everything = true;
                        }
                        if ghost(ui, "Check for updates").clicked() {
                            self.cf_check_mod_updates(ui.ctx(), &folder);
                        }
                    }
                },
                |ui| flip = view_switch(ui, cards, page_w),
            );
        }
        self.set_results_view(flip);
        if show_everything {
            self.discover.target = None;
            *search = true;
        }
        if section == Section::Modpacks {
            self.cf_modpack_tools(ui);
            return;
        }
        // Fabric API and friends, which most mods of that loader need
        let missing = self.cf_target_info().and_then(|t| Some((t.folder.clone(), t.missing_api?)));
        if let Some((folder, (id, name))) = missing {
            let loader = self.instance(&folder).and_then(|i| i.loader()).map(|l| l.0).unwrap_or("this loader");
            let text = format!("{name} is not installed. Most mods for {loader} need it.");
            let label = format!("Installing {name}");
            let running = self.tasks.iter().any(|t| t.label == label);
            ui.add_space(6.0);
            if running {
                notice(ui, NoticeKind::Warn, &text, None);
            } else if notice(ui, NoticeKind::Warn, &text, Some(&format!("Install {name}"))).is_some_and(|r| r.clicked())
            {
                self.cf_install_by_id(id, folder, label);
            }
        }
    }

    /// The row under the section tabs for modpacks: import an address or a downloaded zip, and look for updates.
    fn cf_modpack_tools(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let mut text = std::mem::take(&mut self.cf.import_text);
            search_field(
                ui,
                &mut text,
                "Paste a CurseForge modpack address, or the path of a downloaded .zip",
                (ui.available_width() - 260.0).clamp(220.0, 480.0),
            );
            self.cf.import_text = text;
            if secondary(ui, "Import").clicked() && !self.cf.import_text.trim().is_empty() {
                let t = self.cf.import_text.trim().trim_matches('"').to_string();
                self.cf.import_text.clear();
                self.cf_import_text(t);
            }
            if ghost(ui, "Check my modpacks for updates").clicked() {
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

    /// When CurseForge is the side Discover shows, open it on `kind` with a fresh search (the Mods tab's "Add").
    pub(super) fn cf_show_for(&mut self, kind: crate::mods::metadata::ContentKind) {
        use crate::mods::metadata::ContentKind;
        if !self.cf.active {
            return;
        }
        self.cf.section = match kind {
            ContentKind::Mods => Section::Mods,
            ContentKind::ResourcePacks => Section::ResourcePacks,
            ContentKind::Shaders => Section::ShaderPacks,
        };
        self.cf.searched = false;
    }

    /// Remove every file of a CurseForge project from an instance, the way the Mods tab removes one: in the
    /// background, one change to the instance at a time, and to the recycle bin when that setting is on.
    fn cf_remove(&mut self, folder: &str, project: &Project) {
        if self.is_playing(folder) {
            self.toast_err(format!("Close Minecraft first, then remove {}.", project.name));
            return;
        }
        let (id, name) = (project.id, project.name.clone());
        let settings = crate::mods::ModSettings::from_settings(&self.settings);
        self.mods_job(folder, "Removing", move |mut inst, _| async move {
            tokio::task::spawn_blocking(move || {
                let keys: Vec<crate::mods::files::ModKey> = inst
                    .mod_records()
                    .iter()
                    .filter(|m| m.curse_forge_project_id == Some(id))
                    .map(|m| crate::mods::files::ModKey { file: m.file.clone(), kind: m.kind.clone() })
                    .collect();
                if keys.is_empty() {
                    anyhow::bail!("{name} isn't in this instance any more");
                }
                for k in &keys {
                    crate::mods::files::remove(&mut inst, k, &settings)?;
                }
                Ok(format!("Removed {name}"))
            })
            .await?
        });
        self.mods_cache = None;
        self.cf.target_info = None;
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
        if self.is_playing(folder) {
            self.toast_err(format!("Close Minecraft in {} first, then update it.", inst.name()));
            return;
        }
        let (dirs, cfs, hub) = (self.dirs.clone(), self.cf_settings(), self.cf_new_hub());
        let lock = super::mods::instance_lock(&inst.root);
        self.start_task(format!("Updating {}", inst.name()), move |p, _| async move {
            let _turn = lock.lock().await;
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
        if let Some((project, folder)) = self.cf.confirm_remove.clone() {
            let use_bin = crate::mods::ModSettings::from_settings(&self.settings).use_recycle_bin;
            let body = if use_bin {
                "Its files go to the Recycle Bin (Trash). You can add it again at any time."
            } else {
                "Its files are deleted from this instance. You can add it again at any time."
            };
            match super::services::confirm_modal(
                ctx,
                "cf-confirm-remove",
                &format!("Remove {}?", project.name),
                body,
                "Remove",
                true,
            ) {
                Some(true) => {
                    self.cf.confirm_remove = None;
                    self.cf_remove(&folder, &project);
                }
                Some(false) => self.cf.confirm_remove = None,
                None => {}
            }
        }
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
                            self.cf.list_gen += 1;
                        }
                        self.cf.projects.extend(page.projects);
                        self.cf.total = page.total;
                        self.cf.has_more = page.has_more;
                        self.cf.error = None;
                    }
                    Err(e) => {
                        self.cf.error = Some(e);
                        if reset {
                            // the old rows no longer match what was asked for
                            self.cf.projects.clear();
                        } else {
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

        let modal = egui::Modal::new(Id::unique("cf-file-dialog")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(width);
            // the project's picture beside what is being installed and where it goes
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                self.result_icon(ui, d.project.logo_url(), &d.project.name, 44.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    // the verb of the button below: nothing is installing until it is pressed
                    let title = format!("{} {}", if is_pack { "Install" } else { "Add" }, d.project.name);
                    ui.add(egui::Label::new(RichText::new(title).size(FS_SECTION).semibold()).wrap());
                    if let Some((_, name)) = &target {
                        small_muted(ui, format!("into {name}"));
                    } else if is_pack {
                        small_muted(ui, "as a new instance");
                    } else if d.folder.is_some() {
                        ui.label(RichText::new("That instance isn't there any more.").color(c_danger()));
                    }
                });
            });
            ui.add_space(14.0);
            // one scrolling part: the sections inside it are plain, so nothing scrolls inside the scroll
            let body = ScrollArea::vertical().id_salt("cf-file-body").max_height(body_height).auto_shrink([false, true]);
            body.show(ui, |ui| {
                if d.loading {
                    ui.horizontal(|ui| {
                        spinner(ui);
                        muted(ui, "Getting the files");
                    });
                }
                if let Some(e) = &d.error {
                    notice(ui, NoticeKind::Warn, e, None);
                    ui.add_space(8.0);
                }
                if !d.files.is_empty() {
                    // the version and what is known about it, set into the dialog as one group
                    super::discover::well(ui, |ui| {
                    if let Some(installed) = d.installed_file.and_then(|id| d.all_files.iter().find(|f| f.id == id)) {
                        small_muted(ui, format!("The version currently installed is {}", installed.label()));
                    }
                    ui.horizontal(|ui| {
                        super::discover::filter_label(ui, "Version to install");
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
                            ui.spacing_mut().item_spacing.x = 6.0;
                            // how finished the file is, on a pill: a test version in the colour that warns
                            match f.release_type {
                                Some(2) => super::discover::pill(ui, "Beta", c_warn()),
                                Some(3) => super::discover::pill(ui, "Alpha", c_danger()),
                                _ => super::discover::pill(ui, "Release", c_accent()),
                            };
                            // what the file is for on the monospace tags of the result rows, then its size and date
                            for v in f.game_versions.iter().take(6) {
                                let color = if super::discover::LOADERS.iter().any(|(_, l)| l == v) { loader_color(v) } else { c_muted() };
                                super::discover::mono_tag(ui, v, color);
                            }
                            let mut facts: Vec<String> = Vec::new();
                            facts.push(human_size(f.file_length.max(0) as u64));
                            facts.push(f.file_date.chars().take(10).collect::<String>());
                            facts.retain(|t| !t.is_empty());
                            ui.label(RichText::new(facts.join("  \u{00B7}  ")).font(super::fonts::mono(11.5)).color(c_muted()));
                        });
                    }
                    });
                    if let Some(f) = d.files.get(d.selected) {
                        if f.download_url.is_none() {
                            ui.add_space(4.0);
                            notice(
                                ui,
                                NoticeKind::Warn,
                                "The author of this file does not let launchers download it. Your browser will open so you can download it yourself; BlockForge then picks the file up from your Downloads folder.",
                                None,
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
                            super::discover::filter_label(ui, "Instance name");
                            ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(320.0).background_color(c_field()));
                        });
                    }

                    // dependencies
                    if target.is_some() {
                        if d.deps_loading {
                            ui.horizontal(|ui| {
                                spinner(ui);
                                muted(ui, "Checking what this file needs");
                            });
                        } else if let Some(e) = &d.deps_error {
                            ui.add_space(4.0);
                            if notice(
                                ui,
                                NoticeKind::Warn,
                                &format!("Couldn't check which other mods this one needs: {e}"),
                                Some("Try again"),
                            )
                            .is_some_and(|r| r.clicked())
                            {
                                retry_deps = true;
                            }
                        } else if !d.deps.is_empty() {
                            ui.add_space(10.0);
                            section(ui, "These mods go with it");
                            small_muted(ui, "Required ones are ticked. Tick the others to add them too.");
                            ui.add_space(4.0);
                            super::discover::well(ui, |ui| {
                                for (n, (dep, on)) in d.deps.iter_mut().enumerate() {
                                    if n > 0 {
                                        rule(ui);
                                    }
                                    ui.horizontal(|ui| {
                                        let usable = dep.file.is_some();
                                        ui.add_enabled(usable, egui::Checkbox::new(on, ""));
                                        ui.vertical(|ui| {
                                            ui.spacing_mut().item_spacing.y = 0.0;
                                            ui.label(RichText::new(&dep.project.name).semibold());
                                            let what = if dep.is_required() { "Required" } else { "Optional" };
                                            match &dep.file {
                                                Some(f) => small_muted(ui, format!("{what}  \u{00B7}  {}", f.label())),
                                                None => small_muted(ui, format!("{what}  \u{00B7}  No file fits this instance")),
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
                                ui.add_space(10.0);
                                section(ui, "What changed");
                                ui.add_space(4.0);
                                super::discover::well(ui, |ui| {
                                    ui.add(egui::Label::new(RichText::new(text.as_str()).color(c_muted())).wrap());
                                });
                            }
                            Some((id, Err(e))) if *id == fid => {
                                small_muted(ui, format!("Couldn't get the changelog: {e}"));
                            }
                            _ => {
                                if d.changelog_loading {
                                    spinner(ui);
                                } else if super::discover::text_link(ui, "Show what changed in this version", FS_BODY).clicked() {
                                    want_changelog = true;
                                }
                            }
                        }
                    }
                }
            });
            ui.add_space(12.0);
            // wait for the list of what the file needs, or the mod would go in without it
            let can = !d.files.is_empty() && (is_pack || (target.is_some() && !d.deps_loading));
            let reason = if can {
                ""
            } else if d.files.is_empty() {
                if d.loading { "Getting the files" } else { "No file to install" }
            } else if !is_pack && target.is_none() {
                "There is no instance to add it to"
            } else {
                "Checking what this file needs"
            };
            let file = d.files.get(d.selected).map(|f| d.project.file_page_url(f));
            super::discover::dialog_footer(
                ui,
                "cf-file",
                |ui| {
                    if ghost(ui, "View page").clicked() {
                        if let Some(url) = d.project.website_url() {
                            let _ = open::that(url);
                        }
                    }
                    if let Some(url) = &file {
                        if ghost(ui, "View file").clicked() {
                            let _ = open::that(url);
                        }
                    }
                },
                |ui| {
                    if !reason.is_empty() {
                        small_muted(ui, reason);
                    }
                    // the safe answer has the focus when the window opens, so Enter or Space never confirms by accident
                    let safe = secondary(ui, "Cancel");
                    focus_when_shown(ui, &safe);
                    if safe.clicked() {
                        close = true;
                    }
                    if primary_enabled(ui, if is_pack { "Install" } else { "Add" }, can).clicked() {
                        install_now = true;
                    }
                },
            );
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
        use super::discover::*;
        let mut open_all = false;
        let mut skip_remaining = false;
        let mut cancel = false;
        let mut skip_one: Option<u64> = None;
        let mut confirm = self.cf.confirm_skip;

        let (width, list_height) = modal_size(ctx, 660.0, 300.0);
        egui::Modal::new(Id::unique("cf-browser-dialog")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(width);
            ui.label(RichText::new("Download these in your browser").size(FS_SECTION).semibold());
            ui.add_space(6.0);
            muted(ui, "The authors of these files do not let launchers download them. Open each one in your browser and save it to your Downloads folder; BlockForge notices it there and carries on by itself.");
            small_faint(ui, format!("Watching: {}", user_folder.display()));
            ui.add_space(12.0);
            well(ui, |ui| ScrollArea::vertical().max_height(list_height.min(320.0)).id_salt("cf-browser-list").show(ui, |ui| {
                for (n, it) in items.iter().enumerate() {
                    if n > 0 {
                        rule(ui);
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.set_max_width((ui.available_width() - 270.0).max(120.0));
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.add(egui::Label::new(RichText::new(&it.request.label).semibold()).truncate());
                            ui.add(egui::Label::new(RichText::new(&it.request.file_name).size(FS_META).color(c_muted())).truncate());
                        });
                        let id = ui.scope_id().with(("cf-browser-row", it.id));
                        end_aligned(ui, id, |ui| match &it.state {
                            ItemState::Waiting => {
                                if ghost(ui, "Skip").clicked() {
                                    skip_one = Some(it.id);
                                }
                                if ghost(ui, "Copy link").clicked() {
                                    ui.ctx().copy_text(it.request.url.clone());
                                }
                                if secondary(ui, "Open").clicked() {
                                    let _ = open::that(&it.request.url);
                                }
                            }
                            ItemState::Found(_) => {
                                check_label(ui, "Download complete");
                            }
                            ItemState::Skipped => {
                                small_muted(ui, "Skipped");
                            }
                        });
                    });
                    ui.add_space(6.0);
                }
            }));
            ui.add_space(12.0);
            if confirm {
                notice(ui, NoticeKind::Warn, "Skipping the remaining downloads may cause issues launching the modpack. Are you sure?", None);
                ui.add_space(8.0);
                dialog_footer(
                    ui,
                    "cf-browser-confirm",
                    |_| {},
                    |ui| {
                        if secondary(ui, "Keep waiting").clicked() {
                            confirm = false;
                        }
                        if danger(ui, "Skip them").clicked() {
                            skip_remaining = true;
                            confirm = false;
                        }
                    },
                );
            } else {
                dialog_footer(
                    ui,
                    "cf-browser",
                    |ui| {
                        if ghost(ui, "Open folder").clicked() {
                            let _ = open::that(&user_folder);
                        }
                    },
                    |ui| {
                        if danger(ui, "Cancel install").clicked() {
                            cancel = true;
                        }
                        if secondary(ui, "Skip remaining").clicked() {
                            confirm = true;
                        }
                        if primary(ui, "Open all in browser").clicked() {
                            open_all = true;
                        }
                    },
                );
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
        let modal = egui::Modal::new(Id::unique("cf-updates")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            use super::discover::*;
            ui.set_width(width);
            ui.label(RichText::new(&w.title).size(FS_SECTION).semibold());
            ui.add_space(10.0);
            if w.loading {
                ui.horizontal(|ui| {
                    spinner(ui);
                    muted(ui, "Checking CurseForge");
                });
            }
            if let Some(e) = &w.error {
                notice(ui, NoticeKind::Error, e, None);
            }
            if !w.loading && w.error.is_none() && w.mods.is_empty() && w.packs.is_empty() {
                muted(ui, "Everything is up to date.");
            }
            if !w.mods.is_empty() || !w.packs.is_empty() {
                ui.add_space(4.0);
                well(ui, |ui| {
                    ScrollArea::vertical().max_height(list_height.min(340.0)).id_salt("cf-updates-list").show(
                        ui,
                        |ui| {
                            let only_one = w.mods.len() + w.packs.len() == 1;
                            let mut first = true;
                            for u in &w.mods {
                                if !std::mem::take(&mut first) {
                                    rule(ui);
                                }
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.set_max_width((ui.available_width() - 120.0).max(160.0));
                                        ui.spacing_mut().item_spacing.y = 0.0;
                                        ui.label(RichText::new(&u.project.name).semibold());
                                        small_muted(ui, format!("{}  ->  {}", u.installed_file, u.latest.label()));
                                    });
                                    let id = ui.scope_id().with(("cf-update-row", u.project.id));
                                    end_aligned(ui, id, |ui| {
                                        let clicked = if only_one {
                                            primary(ui, "Update").clicked()
                                        } else {
                                            secondary(ui, "Update").clicked()
                                        };
                                        if clicked {
                                            apply_mod = Some(u.clone());
                                        }
                                    });
                                });
                                ui.add_space(6.0);
                            }
                            for (folder, name, u) in &w.packs {
                                if !std::mem::take(&mut first) {
                                    rule(ui);
                                }
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.set_max_width((ui.available_width() - 200.0).max(160.0));
                                        ui.spacing_mut().item_spacing.y = 0.0;
                                        ui.label(RichText::new(name).semibold());
                                        small_muted(ui, format!("New version: {}", u.latest.label()));
                                        if u.ignored {
                                            small_muted(ui, "You chose to ignore this update");
                                        }
                                    });
                                    let id = ui.scope_id().with(("cf-pack-row", folder));
                                    end_aligned(ui, id, |ui| {
                                        if !u.ignored && ghost(ui, "Ignore").clicked() {
                                            ignore_pack = Some((folder.clone(), u.latest.id));
                                        }
                                        let clicked = if only_one {
                                            primary(ui, "Update").clicked()
                                        } else {
                                            secondary(ui, "Update").clicked()
                                        };
                                        if clicked {
                                            apply_pack = Some((folder.clone(), u.clone()));
                                        }
                                    });
                                });
                                ui.add_space(6.0);
                            }
                        },
                    )
                });
            }
            ui.add_space(12.0);
            dialog_footer(
                ui,
                "cf-updates",
                |_| {},
                |ui| {
                    // the safe answer has the focus when the window opens, so Enter or Space never confirms by accident
                    let safe = secondary(ui, "Close");
                    focus_when_shown(ui, &safe);
                    if safe.clicked() {
                        close = true;
                    }
                    if w.mods.len() > 1 && primary(ui, "Update all").clicked() {
                        update_all = true;
                    }
                },
            );
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
/// The file of a CurseForge project an instance has, if it has one.
fn installed_file_of(inst: Option<&crate::instances::Instance>, project: i64) -> Option<i64> {
    inst?.mod_records().iter().find(|r| r.curse_forge_project_id == Some(project)).and_then(|r| r.curse_forge_file_id)
}

/// The newest full release among a project's newest files that is made for Minecraft `mc` and one of `loaders` (any
/// loader, when none is asked for), when it is newer than the file `installed`. CurseForge numbers files in the order
/// they were uploaded, so a higher number is a newer file.
fn newer_file(indexes: &[GameVersionLatestFiles], mc: &str, loaders: &[i64], installed: i64) -> Option<i64> {
    indexes
        .iter()
        .filter(|f| f.game_version == mc && f.release_type == 1)
        .filter(|f| loaders.is_empty() || loaders.contains(&f.mod_loader))
        .map(|f| f.file_id)
        .max()
        .filter(|id| *id > installed && installed > 0)
}

/// What a project works with, for its row while no instance is chosen: the newest release it has a file for and
/// the loaders of its files.
fn project_compat(p: &Project, section: Section) -> Vec<(String, egui::Color32)> {
    use crate::curseforge::{MODLOADER_FABRIC, MODLOADER_FORGE, MODLOADER_NEOFORGE, MODLOADER_QUILT};
    let mut out = Vec::new();
    if let Some(v) = super::discover::newest_release(p.latest_files_indexes.iter().map(|f| f.game_version.as_str())) {
        out.push((v, c_muted()));
    }
    if section.uses_loader() {
        for (id, name) in [
            (MODLOADER_FABRIC, "Fabric"),
            (MODLOADER_QUILT, "Quilt"),
            (MODLOADER_FORGE, "Forge"),
            (MODLOADER_NEOFORGE, "NeoForge"),
        ] {
            if p.latest_files_indexes.iter().any(|f| f.mod_loader == id) {
                out.push((name.to_string(), loader_color(name)));
            }
        }
    }
    out
}

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
    use super::{GameVersionLatestFiles, html_to_text, modal_size, newer_file};

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
    fn an_update_is_a_newer_release_for_the_instance() {
        let f = |mc: &str, loader: i64, id: i64, release_type: i64| GameVersionLatestFiles {
            game_version: mc.into(),
            mod_loader: loader,
            file_id: id,
            release_type,
            ..Default::default()
        };
        let files = [f("1.21.1", 4, 900, 1), f("1.21.1", 4, 950, 2), f("1.21.1", 6, 990, 1), f("1.20.1", 4, 999, 1)];
        // the newest Fabric release for 1.21.1 is 900: the beta, NeoForge's file and 1.20.1's don't count
        assert_eq!(newer_file(&files, "1.21.1", &[4], 800), Some(900));
        assert_eq!(newer_file(&files, "1.21.1", &[4], 900), None);
        // no loader asked for: any loader's release
        assert_eq!(newer_file(&files, "1.21.1", &[], 900), Some(990));
        // a file the launcher doesn't know is never "older"
        assert_eq!(newer_file(&files, "1.21.1", &[4], 0), None);
        assert_eq!(newer_file(&files, "1.19.2", &[4], 1), None);
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
