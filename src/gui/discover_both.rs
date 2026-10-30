//! Discover with every site searched together: Modrinth and CurseForge (and Smithed for data packs) in one list, a
//! project that is on more than one shown once, and each installed the way its own site's page installs it.

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use egui::{Color32, ScrollArea, Ui};

use super::discover::*;
use super::theme::*;
use super::widgets::*;
use super::{App, Msg};
use crate::curseforge::Project;
use crate::curseforge::api::{self, SearchPage, SearchQuery, Section, Sort as CfSort};
use crate::curseforge::filter::Target;
use crate::modrinth::{Hit, Kind, Sort};
use crate::smithed::Pack;

/// CurseForge's half of a search on both sites (Modrinth's half is Discover's own list).
#[derive(Default)]
pub struct CfSide {
    pub projects: Vec<Project>,
    pub total: u64,
    pub has_more: bool,
    page: u32,
    pub loading: bool,
    pub error: Option<String>,
    seq: u64,
    /// The CurseForge projects in the target instance, with the instance file's time they were read at.
    installed: Option<(String, Option<SystemTime>, HashSet<i64>)>,
}

/// Smithed's part of a search for data packs.
#[derive(Default)]
pub struct SmSide {
    pub packs: Vec<Pack>,
    pub total: u64,
    page: u32,
    pub loading: bool,
    pub error: Option<String>,
    seq: u64,
}

impl SmSide {
    fn has_more(&self) -> bool {
        (self.packs.len() as u64) < self.total
    }
}

/// One row of the combined list: where it is in each site's results, for the sites it is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub mr: Option<usize>,
    pub cf: Option<usize>,
    pub sm: Option<usize>,
}

impl Entry {
    #[cfg(test)]
    fn of(mr: Option<usize>, cf: Option<usize>, sm: Option<usize>) -> Entry {
        Entry { mr, cf, sm }
    }
}

/// Letters and digits only, in lower case: "Sodium Extra" and "sodium-extra" read the same.
fn norm(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Whether a Modrinth result and a CurseForge one are the same project: the same name, and the same slug or a shared
/// author as well (a name alone could be two different mods).
fn same(hit: &Hit, p: &Project) -> bool {
    let name = norm(&hit.title);
    if name.is_empty() || name != norm(&p.name) {
        return false;
    }
    let slug = norm(&hit.slug);
    let author = norm(&hit.author);
    (!slug.is_empty() && slug == norm(&p.slug))
        || (!author.is_empty() && p.authors.iter().any(|a| norm(&a.name) == author))
}

/// Whether a Smithed pack is the project of a row from the other sites: the same name, and the same slug or exactly
/// the same name as written.
fn same_pack(pack: &Pack, hit: Option<&Hit>, project: Option<&Project>) -> bool {
    let name = norm(&pack.name);
    if name.is_empty() {
        return false;
    }
    let id = norm(&pack.id);
    let rows = hit.map(|h| (&h.title, &h.slug)).into_iter().chain(project.map(|p| (&p.name, &p.slug)));
    rows.into_iter().any(|(title, slug)| {
        norm(title) == name && (norm(slug) == id || title.trim().eq_ignore_ascii_case(pack.name.trim()))
    })
}

/// Every site's results as one list. By downloads, the rows are ordered by their downloads (a project on more than
/// one site counts them all); in any other order each site's own order is kept and they take turns.
pub fn merge(mr: &[Hit], cf: &[Project], sm: &[Pack], by_downloads: bool) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::with_capacity(mr.len() + cf.len() + sm.len());
    // the row each site's result ended up in
    let (mut of_mr, mut of_cf, mut of_sm) = (vec![0; mr.len()], vec![usize::MAX; cf.len()], vec![0; sm.len()]);
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, p) in cf.iter().enumerate() {
        by_name.entry(norm(&p.name)).or_default().push(i);
    }
    for (j, hit) in mr.iter().enumerate() {
        let twin = by_name
            .get(&norm(&hit.title))
            .and_then(|c| c.iter().copied().find(|&i| of_cf[i] == usize::MAX && same(hit, &cf[i])));
        of_mr[j] = entries.len();
        if let Some(i) = twin {
            of_cf[i] = entries.len();
        }
        entries.push(Entry { mr: Some(j), cf: twin, sm: None });
    }
    for (i, row) in of_cf.iter_mut().enumerate() {
        if *row == usize::MAX {
            *row = entries.len();
            entries.push(Entry { cf: Some(i), ..Default::default() });
        }
    }
    for (k, pack) in sm.iter().enumerate() {
        let twin =
            entries.iter().position(|e| e.sm.is_none() && same_pack(pack, e.mr.map(|j| &mr[j]), e.cf.map(|i| &cf[i])));
        of_sm[k] = match twin {
            Some(row) => {
                entries[row].sm = Some(k);
                row
            }
            None => {
                entries.push(Entry { sm: Some(k), ..Default::default() });
                entries.len() - 1
            }
        };
    }
    if by_downloads {
        let downloads = |e: &Entry| {
            e.mr.map_or(0, |j| mr[j].downloads)
                + e.cf.map_or(0, |i| cf[i].download_count.max(0.0) as u64)
                + e.sm.map_or(0, |k| sm[k].downloads)
        };
        // a stable sort: equal counts keep the sites' own order
        entries.sort_by_key(|e| std::cmp::Reverse(downloads(e)));
        return entries;
    }
    let mut out = Vec::with_capacity(entries.len());
    let mut shown = vec![false; entries.len()];
    for n in 0..mr.len().max(cf.len()).max(sm.len()) {
        for &row in [of_mr.get(n), of_cf.get(n), of_sm.get(n)].into_iter().flatten() {
            if !shown[row] {
                shown[row] = true;
                out.push(entries[row]);
            }
        }
    }
    out
}

/// CurseForge's section for a Modrinth kind.
pub(super) fn section_for(kind: Kind) -> Section {
    match kind {
        Kind::Mod => Section::Mods,
        Kind::Modpack => Section::Modpacks,
        Kind::ResourcePack => Section::ResourcePacks,
        Kind::Shader => Section::ShaderPacks,
        Kind::DataPack => Section::DataPacks,
    }
}

/// Modrinth's kind for a CurseForge section; Modrinth has no worlds (and no plugins).
pub(super) fn kind_for(section: Section) -> Option<Kind> {
    Kind::ALL.into_iter().find(|k| section_for(*k) == section)
}

/// CurseForge's order for Modrinth's.
fn cf_sort(sort: Sort) -> CfSort {
    match sort {
        Sort::Relevance => CfSort::Popularity,
        Sort::Downloads => CfSort::TotalDownloads,
        Sort::Updated => CfSort::LastUpdated,
    }
}

pub(super) fn modrinth_color() -> Color32 {
    if is_dark() { Color32::from_rgb(0x1b, 0xd9, 0x6a) } else { Color32::from_rgb(0x0e, 0x7a, 0x3c) }
}

pub(super) fn curseforge_color() -> Color32 {
    if is_dark() { Color32::from_rgb(0xf1, 0x64, 0x36) } else { Color32::from_rgb(0xb8, 0x42, 0x1a) }
}

fn smithed_color() -> Color32 {
    if is_dark() { Color32::from_rgb(0x9a, 0x8c, 0xff) } else { Color32::from_rgb(0x4b, 0x3c, 0xc4) }
}

impl super::discover::DiscoverState {
    /// CurseForge's results for a search on both sites.
    pub fn cf_arrived(&mut self, seq: u64, reset: bool, result: Result<SearchPage, String>) {
        if seq != self.cf.seq {
            return;
        }
        self.cf.loading = false;
        match result {
            Ok(page) => {
                if reset {
                    self.cf.projects = page.projects;
                } else {
                    self.cf.projects.extend(page.projects);
                }
                self.cf.total = page.total;
                self.cf.has_more = page.has_more;
                self.cf.error = None;
            }
            Err(e) => {
                self.cf.error = Some(e);
                if reset {
                    self.cf.projects.clear();
                    self.cf.total = 0;
                    self.cf.has_more = false;
                }
            }
        }
        self.remerge();
    }

    /// Smithed's results for a search for data packs on every site.
    pub fn sm_arrived(&mut self, seq: u64, reset: bool, result: Result<(Vec<Pack>, u64), String>) {
        if seq != self.sm.seq {
            return;
        }
        self.sm.loading = false;
        match result {
            Ok((packs, total)) => {
                if reset {
                    self.sm.packs = packs;
                } else {
                    self.sm.packs.extend(packs);
                }
                self.sm.total = total;
                self.sm.error = None;
            }
            Err(e) => {
                self.sm.error = Some(e);
                if reset {
                    self.sm.packs.clear();
                    self.sm.total = 0;
                }
            }
        }
        self.remerge();
    }

    /// Whether Smithed is searched too: it only has data packs, and none of Modrinth's categories.
    pub fn with_smithed(&self) -> bool {
        self.both && self.kind == Kind::DataPack && !self.modrinth_only()
    }

    /// Leaves CurseForge out of a search on every site while a filter only Modrinth has is on: its rows go, and an
    /// answer still on its way is ignored.
    fn cf_left_out(&mut self) {
        let side = &mut self.cf;
        side.seq += 1;
        side.projects.clear();
        side.total = 0;
        side.has_more = false;
        side.page = 0;
        side.loading = false;
        side.error = None;
    }

    /// The same for Smithed.
    fn sm_left_out(&mut self) {
        let side = &mut self.sm;
        side.seq += 1;
        side.packs.clear();
        side.total = 0;
        side.page = 0;
        side.loading = false;
        side.error = None;
    }

    /// Builds the combined list again from every site's results.
    pub fn remerge(&mut self) {
        if self.both {
            let sm: &[Pack] = if self.with_smithed() { &self.sm.packs } else { &[] };
            self.merged = merge(&self.hits, &self.cf.projects, sm, self.sort == Sort::Downloads);
        }
    }
}

/// What a row of the combined list asked for.
enum Act {
    Modrinth(Hit, Option<String>),
    CurseForge(Project, Option<String>),
    /// a Modrinth data pack: the instance and its worlds
    ModrinthData(Hit, String, Vec<String>),
    Smithed(Pack, String, Vec<String>),
    Open(String),
}

/// A row's Install button: into the chosen instance, or a menu of instances when none is chosen; a modpack makes an
/// instance of its own. Gives the instance folder (none for a modpack) when pressed.
fn install_choice(
    ui: &mut Ui,
    modpack: bool,
    target: &Option<String>,
    choices: &[(String, String)],
) -> Option<Option<String>> {
    if modpack {
        return super::discover::install_button(ui, "Install").clicked().then_some(None);
    }
    if let Some(t) = target {
        return super::discover::install_button(ui, "Install").clicked().then(|| Some(t.clone()));
    }
    let r = super::discover::install_button(ui, "Install");
    let mut chosen = None;
    egui::Popup::menu(&r).show(|ui| {
        ui.set_min_width(180.0);
        if choices.is_empty() {
            ui.label("Create an instance first");
        }
        for (folder, name) in choices {
            if ui.button(name).clicked() {
                chosen = Some(Some(folder.clone()));
                ui.close();
            }
        }
    });
    chosen
}

fn modrinth_page(hit: &Hit) -> String {
    let slug = if hit.slug.is_empty() { &hit.project_id } else { &hit.slug };
    let kind = if hit.project_type.is_empty() { "project" } else { &hit.project_type };
    format!("https://modrinth.com/{kind}/{slug}")
}

impl App {
    /// Whether CurseForge may be used (the launcher's configuration can switch it off).
    pub(super) fn both_cf_allowed(&mut self) -> bool {
        self.cf_enabled()
    }

    /// CurseForge's half of a search on both sites: the same words, kind, instance and order as Modrinth's half.
    pub(super) fn both_search_cf(&mut self, reset: bool) {
        if self.discover.modrinth_only() {
            self.discover.cf_left_out();
            return;
        }
        if !self.both_cf_allowed() || (!reset && !self.discover.cf.has_more) {
            return;
        }
        let section = section_for(self.discover.kind);
        let cfs = self.cf_settings();
        let target = if section == Section::Modpacks {
            None
        } else {
            self.discover
                .target
                .as_ref()
                .and_then(|f| self.instance(f))
                .map(|inst| Target::from_instance(inst, &self.dirs, &cfs))
        };
        let mut q = SearchQuery::new(section);
        q.text = self.discover.query.trim().to_string();
        q.sort = cf_sort(self.discover.sort);
        if let Some(t) = &target {
            if section.uses_game_version() {
                q.game_version = t.search_game_version(&cfs);
            }
            if section.uses_loader() {
                q.mod_loaders = t.search_loaders();
            }
        } else {
            // without an instance (or for modpacks, which make their own), the version and loader of the filters
            if section.uses_game_version() {
                q.game_version = self.discover.mc_filter.clone();
            }
            if section.uses_loader() {
                q.mod_loaders = self.discover.loader_filter.as_deref().map(cf_loaders_for).unwrap_or_default();
            }
        }
        let side = &mut self.discover.cf;
        side.page = if reset { 0 } else { side.page + 1 };
        q.page = side.page;
        side.seq += 1;
        side.loading = true;
        let (seq, n) = (side.seq, self.notifier());
        self.rt.spawn(async move {
            let result = api::search(&q).await.map_err(|e| format!("{e:#}"));
            n.send(Msg::SearchCf { seq, reset, result });
        });
    }

    /// Smithed's part of a search for data packs: the same words, Minecraft version and order.
    pub(super) fn both_search_sm(&mut self, reset: bool) {
        if self.discover.modrinth_only() {
            self.discover.sm_left_out();
            return;
        }
        if !self.discover.with_smithed() || (!reset && !self.discover.sm.has_more()) {
            return;
        }
        let mc = match self.discover.target.as_ref().and_then(|f| self.instance(f)) {
            Some(inst) => Some(inst.mc_version().to_string()),
            None => self.discover.mc_filter.clone(),
        };
        let (text, sort) = (self.discover.query.trim().to_string(), self.discover.sort);
        let side = &mut self.discover.sm;
        side.page = if reset { 0 } else { side.page + 1 };
        side.seq += 1;
        side.loading = true;
        let (seq, page, n) = (side.seq, side.page, self.notifier());
        self.rt.spawn(async move {
            let result =
                crate::smithed::search(&text, mc.as_deref(), sort, page, 20).await.map_err(|e| format!("{e:#}"));
            n.send(Msg::SearchSmithed { seq, reset, result });
        });
    }

    /// Adds a Smithed pack to worlds of an instance.
    fn install_smithed(&mut self, pack: Pack, folder: String, worlds: Vec<String>) {
        let Some(inst) = self.instance(&folder) else {
            self.toast_err("That instance isn't there any more. Choose another one.");
            return;
        };
        let root = inst.root.clone();
        let lock = super::mods::instance_lock(&root);
        self.start_task(format!("Installing {}", pack.name), move |p, _| async move {
            let _turn = lock.lock().await;
            let mut inst = crate::instances::load_one(&root)
                .ok_or_else(|| anyhow::anyhow!("that instance can't be read any more"))?;
            crate::smithed::install(&mut inst, &pack, &worlds, &p).await?;
            Ok(format!("Added {} to {}", pack.name, worlds_text(&worlds)))
        });
        self.mods_cache = None;
    }

    /// The CurseForge projects in the target instance, read again only when the instance's file changes.
    fn both_refresh_installed_cf(&mut self) {
        let Some(inst) = self.discover.target.as_ref().and_then(|f| self.instance(f)) else {
            self.discover.cf.installed = None;
            return;
        };
        let stamp = std::fs::metadata(inst.instance_file()).and_then(|m| m.modified()).ok();
        let fresh =
            matches!(&self.discover.cf.installed, Some((folder, at, _)) if *folder == inst.folder && *at == stamp);
        if !fresh {
            let ids = inst.mod_records().iter().filter_map(|r| r.curse_forge_project_id).collect();
            self.discover.cf.installed = Some((inst.folder.clone(), stamp, ids));
        }
    }

    /// The results of a search on every site, under Discover's filters.
    pub(super) fn discover_both_list(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let allowed = self.both_cf_allowed();
        // a filter only Modrinth has leaves CurseForge out; the chips above say so
        let cf_on = allowed && !self.discover.modrinth_only();
        let smithed = self.discover.with_smithed();
        let loading = self.discover.loading || self.discover.cf.loading || (smithed && self.discover.sm.loading);
        // a new search over rows that are already shown: a thin bar says it is on its way
        if loading && !self.discover.merged.is_empty() && self.discover.cf.page == 0 && self.discover.sm.page == 0 {
            progress_bar(ui, None, 2.0);
        } else {
            ui.add_space(2.0);
        }
        if !allowed {
            notice(
                ui,
                NoticeKind::Info,
                "CurseForge is switched off in the launcher's configuration, so its results aren't shown.",
                None,
            );
            ui.add_space(6.0);
        }
        // the sites that couldn't be reached, and the ones that could
        let mr_error = self.discover.error.clone();
        let cf_error = self.discover.cf.error.clone().filter(|_| cf_on);
        let sm_error = self.discover.sm.error.clone().filter(|_| smithed);
        let mut sites = vec![("Modrinth", &mr_error)];
        if cf_on {
            sites.push(("CurseForge", &cf_error));
        }
        if smithed {
            sites.push(("Smithed", &sm_error));
        }
        let failed: Vec<&str> = sites.iter().filter(|(_, e)| e.is_some()).map(|(n, _)| *n).collect();
        let working: Vec<&str> = sites.iter().filter(|(_, e)| e.is_none()).map(|(n, _)| *n).collect();
        if !failed.is_empty() {
            let detail: Vec<String> =
                sites.iter().filter_map(|(n, e)| e.as_ref().map(|e| format!("{n}: {e}"))).collect();
            if working.is_empty() {
                let text = format!(
                    "Couldn't reach {}. Check your internet connection, or try again in a few minutes.",
                    join_or(&failed)
                );
                if fetch_error(ui, &text, &detail.join("\n")) && !loading {
                    self.discover_search(self.discover.merged.is_empty());
                }
            } else {
                let text = format!(
                    "{} couldn't be reached, so only results from {} are shown.",
                    join_and(&failed),
                    join_and(&working)
                );
                let r = ui.scope(|ui| notice(ui, NoticeKind::Warn, &text, Some("Try again")));
                r.response.on_hover_text(detail.join("\n"));
                if r.inner.is_some_and(|r| r.clicked()) && !loading {
                    self.discover_search(true);
                }
                ui.add_space(6.0);
            }
        }
        if self.discover.merged.is_empty() && loading {
            let sites = if self.discover.with_smithed() {
                "Modrinth, CurseForge and Smithed"
            } else {
                "Modrinth and CurseForge"
            };
            result_skeleton(ui, 4, &format!("Searching {sites}\u{2026}"));
            return;
        }
        if self.discover.merged.is_empty() && failed.is_empty() {
            self.discover_nothing(ui, &join_or(&working));
            return;
        }

        self.discover_refresh_installed();
        self.both_refresh_installed_cf();
        let kind = self.discover.kind;
        let modpack = kind == Kind::Modpack;
        let data_pack = kind == Kind::DataPack;
        let mr_installed: HashSet<String> =
            self.discover.installed_ids().filter(|_| !modpack && !data_pack).cloned().unwrap_or_default();
        let cf_installed: HashSet<i64> = self
            .discover
            .cf
            .installed
            .as_ref()
            .filter(|_| !modpack && !data_pack)
            .map(|(_, _, ids)| ids.clone())
            .unwrap_or_default();
        let installing_titles: HashSet<String> =
            self.tasks.iter().filter_map(|t| t.label.strip_prefix("Installing ").map(str::to_string)).collect();
        let hits = std::mem::take(&mut self.discover.hits);
        let projects = std::mem::take(&mut self.discover.cf.projects);
        let packs = std::mem::take(&mut self.discover.sm.packs);
        let merged = std::mem::take(&mut self.discover.merged);
        let target = self.discover.target.clone();
        let choices: Vec<(String, String)> =
            self.instances.iter().map(|i| (i.folder.clone(), instance_choice(i))).collect();
        let mut act: Option<Act> = None;
        let mut more = false;
        let cards = self.results_as_cards();
        let matched = self.discover.target.as_ref().and_then(|f| self.instance(f)).filter(|_| !modpack).cloned();
        ScrollArea::vertical().id_salt(("discover-both", self.discover.list_gen)).auto_shrink(false).show(ui, |ui| {
            result_list(ui, cards, &merged, |ui, entry, width| {
                let hit = entry.mr.and_then(|j| hits.get(j));
                let project = entry.cf.and_then(|i| projects.get(i));
                let pack = entry.sm.and_then(|k| packs.get(k));
                let key = match (hit, project, pack) {
                    (Some(h), _, _) => format!("mr:{}", h.project_id),
                    (None, Some(p), _) => format!("cf:{}", p.id),
                    (None, None, Some(p)) => format!("sm:{}", p.id),
                    _ => return,
                };
                let title = hit
                    .map(|h| h.title.as_str())
                    .or(project.map(|p| p.name.as_str()))
                    .or(pack.map(|p| p.name.as_str()))
                    .unwrap_or_default();
                let is_installed = hit.is_some_and(|h| mr_installed.contains(&h.project_id))
                    || project.is_some_and(|p| cf_installed.contains(&p.id));
                let busy = installing_titles.contains(title)
                    || project.is_some_and(|p| installing_titles.contains(&p.name))
                    || pack.is_some_and(|p| installing_titles.contains(&p.name));
                let downloads = hit.map_or(0, |h| h.downloads)
                    + project.map_or(0, |p| p.download_count.max(0.0) as u64)
                    + pack.map_or(0, |p| p.downloads);
                let icon = hit
                    .and_then(|h| h.icon_url.as_deref())
                    .filter(|u| !u.is_empty())
                    .or_else(|| project.and_then(|p| p.logo_url()))
                    .or_else(|| pack.and_then(|p| p.icon.as_deref()));
                let author = hit.map(|h| h.author.clone()).or_else(|| project.map(|p| p.author_names()));
                let summary = hit
                    .map(|h| h.description.as_str())
                    .or(project.map(|p| p.summary.as_str()))
                    .or(pack.map(|p| p.summary.as_str()));
                // the sites it is on, as a quiet dot and name each (after what it works with), and its categories
                let mut sites = Vec::new();
                if hit.is_some() {
                    sites.push(("Modrinth".to_string(), modrinth_color()));
                }
                if project.is_some() {
                    sites.push(("CurseForge".to_string(), curseforge_color()));
                }
                if pack.is_some() {
                    sites.push(("Smithed".to_string(), smithed_color()));
                }
                let mut pills = Vec::new();
                let tags = match (hit, project, pack) {
                    (Some(h), _, _) => hit_tags(h, kind, usize::MAX),
                    (None, Some(p), _) => p.categories.iter().map(|c| c.name.clone()).collect(),
                    (None, None, Some(p)) => split_tags(p.categories.iter().map(String::as_str), usize::MAX).1,
                    _ => Vec::new(),
                };
                // what it works with: the instance's version and loader, or what the Modrinth project says
                let compat = match (&matched, hit) {
                    (Some(inst), _) => instance_compat(inst, kind == Kind::Mod),
                    (None, Some(h)) => hit_compat(h, None, kind),
                    _ => Vec::new(),
                };
                if hit.is_none() && pack.is_none() && project.is_some_and(|p| p.allow_mod_distribution == Some(false)) {
                    pills.push(("Download in browser".to_string(), c_warn()));
                }
                let view = ResultView {
                    key,
                    title,
                    by: author.as_deref(),
                    summary: summary.unwrap_or_default(),
                    pills,
                    compat,
                    sites,
                    tags,
                    figure: Some((human_count(downloads), "downloads")),
                };
                result_item(ui, width, &view, |ui, slot| {
                    if let Slot::Icon(size) = slot {
                        self.result_icon(ui, icon, title, size);
                        return;
                    }
                    if busy {
                        installing_mark(ui);
                        return;
                    }
                    if data_pack && (hit.is_some() || pack.is_some()) {
                        // a data pack from Modrinth or Smithed goes into worlds of the instance
                        let have = |inst: &crate::instances::Instance| match (hit, pack) {
                            (Some(h), _) => worlds_with(inst, Some(&h.project_id), None),
                            (None, Some(p)) => worlds_with(inst, None, Some(&p.file_name())),
                            _ => HashSet::new(),
                        };
                        if let Some((folder, worlds)) = data_pack_button(ui, &self.instances, target.as_deref(), &have)
                        {
                            act = match (hit, pack) {
                                (Some(h), _) => Some(Act::ModrinthData(h.clone(), folder, worlds)),
                                (None, Some(p)) => Some(Act::Smithed(p.clone(), folder, worlds)),
                                _ => None,
                            };
                        }
                    } else if is_installed {
                        installed_mark(ui);
                    } else if let Some(folder) = install_choice(ui, modpack, &target, &choices) {
                        // a project on both sites comes from Modrinth: no key, no files to fetch by hand
                        act = match (hit, project) {
                            (Some(h), _) => Some(Act::Modrinth(h.clone(), folder)),
                            (None, Some(p)) => Some(Act::CurseForge(p.clone(), folder)),
                            _ => None,
                        };
                    }
                    kebab_menu(ui, |ui| {
                        if let (Some(_), Some(p), false, false) = (hit, project, is_installed, data_pack) {
                            if modpack || target.is_some() {
                                if ui.button("Install from CurseForge instead").clicked() {
                                    act = Some(Act::CurseForge(p.clone(), target.clone().filter(|_| !modpack)));
                                    ui.close();
                                }
                            } else {
                                ui.menu_button("Install from CurseForge instead", |ui| {
                                    if choices.is_empty() {
                                        ui.label("Create an instance first");
                                    }
                                    for (folder, name) in &choices {
                                        if ui.button(name).clicked() {
                                            act = Some(Act::CurseForge(p.clone(), Some(folder.clone())));
                                            ui.close();
                                        }
                                    }
                                });
                            }
                        }
                        if let Some(h) = hit {
                            if ui.button("Open on Modrinth").clicked() {
                                act = Some(Act::Open(modrinth_page(h)));
                                ui.close();
                            }
                        }
                        if let Some(url) = project.and_then(|p| p.website_url()) {
                            if ui.button("View on CurseForge").clicked() {
                                act = Some(Act::Open(url.to_string()));
                                ui.close();
                            }
                        }
                        if let Some(p) = pack {
                            if ui.button("Open on Smithed").clicked() {
                                act = Some(Act::Open(p.page_url()));
                                ui.close();
                            }
                        }
                    });
                });
            });
            ui.add_space(8.0);
            let mr_more = (hits.len() as u64) < self.discover.total;
            let cf_more = cf_on && self.discover.cf.has_more;
            let sm_more = smithed && (packs.len() as u64) < self.discover.sm.total;
            if mr_more || cf_more || sm_more {
                ui.vertical_centered(|ui| {
                    let total = self.discover.total
                        + if cf_on { self.discover.cf.total } else { 0 }
                        + if smithed { self.discover.sm.total } else { 0 };
                    if total > merged.len() as u64 {
                        small_faint(
                            ui,
                            format!("Showing {} of about {}", thousands(merged.len() as u64), thousands(total)),
                        );
                    }
                    if loading {
                        spinner(ui);
                    } else if ghost(ui, "Load more").clicked() {
                        more = true;
                    }
                });
            } else if loading {
                ui.vertical_centered(|ui| {
                    spinner(ui);
                });
            }
            ui.add_space(8.0);
        });
        self.discover.hits = hits;
        self.discover.cf.projects = projects;
        self.discover.sm.packs = packs;
        self.discover.merged = merged;
        match act {
            Some(Act::Modrinth(hit, folder)) => self.install_hit(&hit, folder),
            Some(Act::CurseForge(p, folder)) => self.cf_open_dialog(&ctx, p, folder, None, false),
            Some(Act::ModrinthData(hit, folder, worlds)) => self.install_data_pack_hit(&hit, &folder, worlds),
            Some(Act::Smithed(pack, folder, worlds)) => self.install_smithed(pack, folder, worlds),
            Some(Act::Open(url)) => {
                let _ = open::that(url);
            }
            None => {}
        }
        if more {
            self.discover_search(false);
        }
    }
}

/// "Modrinth", "Modrinth and CurseForge", "Modrinth, CurseForge and Smithed".
fn join_and(names: &[&str]) -> String {
    join_with(names, "and")
}

fn join_or(names: &[&str]) -> String {
    join_with(names, "or")
}

fn join_with(names: &[&str], word: &str) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} {word} {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(title: &str, slug: &str, author: &str, downloads: u64) -> Hit {
        serde_json::from_value(serde_json::json!({
            "project_id": format!("mr-{slug}"), "slug": slug, "title": title, "author": author, "downloads": downloads
        }))
        .unwrap()
    }

    fn project(id: i64, name: &str, slug: &str, author: &str, downloads: f64) -> Project {
        let mut p = Project::default();
        p.id = id;
        p.name = name.into();
        p.slug = slug.into();
        p.download_count = downloads;
        p.authors = vec![crate::curseforge::model::Author { name: author.into(), ..Default::default() }];
        p
    }

    #[test]
    fn a_project_on_both_sites_is_one_row() {
        let mr = [hit("Sodium", "sodium", "jellysquid3", 100), hit("Lithium", "lithium", "jellysquid3", 50)];
        let cf = [project(1, "Sodium", "sodium", "jellysquid_", 80.0), project(2, "JEI", "jei", "mezz", 300.0)];
        // turns, each site in its own order, the twin taking the first place either reaches
        assert_eq!(
            merge(&mr, &cf, &[], false),
            vec![Entry::of(Some(0), Some(0), None), Entry::of(Some(1), None, None), Entry::of(None, Some(1), None)]
        );
        // by downloads: a project on both counts both sites' downloads
        assert_eq!(
            merge(&mr, &cf, &[], true),
            vec![Entry::of(None, Some(1), None), Entry::of(Some(0), Some(0), None), Entry::of(Some(1), None, None)]
        );
    }

    #[test]
    fn the_same_name_by_someone_else_is_two_rows() {
        let mr = [hit("Backpacks", "backpacks-mr", "alice", 10)];
        let cf = [project(7, "Backpacks!", "backpacks-cf", "bob", 20.0)];
        assert_eq!(merge(&mr, &cf, &[], false), vec![Entry::of(Some(0), None, None), Entry::of(None, Some(0), None)]);
        // the same slug is enough when the authors' names differ between the sites
        let cf = [project(7, "Backpacks", "backpacks-mr", "bob", 20.0)];
        assert_eq!(merge(&mr, &cf, &[], false), vec![Entry::of(Some(0), Some(0), None)]);
    }

    #[test]
    fn one_site_alone_keeps_its_order() {
        let mr = [hit("A", "a", "x", 1), hit("B", "b", "x", 2)];
        assert_eq!(merge(&mr, &[], &[], false), vec![Entry::of(Some(0), None, None), Entry::of(Some(1), None, None)]);
        assert_eq!(merge(&[], &[project(1, "C", "c", "y", 5.0)], &[], true), vec![Entry::of(None, Some(0), None)]);
    }

    fn pack(id: &str, name: &str, downloads: u64) -> Pack {
        Pack { id: id.into(), name: name.into(), downloads, ..Default::default() }
    }

    #[test]
    fn a_smithed_pack_joins_the_row_of_the_same_project() {
        let mr = [hit("Terralith", "terralith", "Stardust Labs", 100)];
        let cf = [project(3, "Terralith", "terralith", "Stardust_Labs", 50.0)];
        let sm = [pack("terralith", "Terralith", 7), pack("myriad", "Myriad", 30)];
        // one row for the project on all three, the pack only on Smithed in a row of its own
        assert_eq!(
            merge(&mr, &cf, &sm, false),
            vec![Entry::of(Some(0), Some(0), Some(0)), Entry::of(None, None, Some(1))]
        );
        // by downloads the row counts all three sites
        assert_eq!(
            merge(&mr, &cf, &sm, true),
            vec![Entry::of(Some(0), Some(0), Some(0)), Entry::of(None, None, Some(1))]
        );
        let sm = [pack("myriad", "Myriad", 300)];
        assert_eq!(merge(&mr, &[], &sm, true), vec![Entry::of(None, None, Some(0)), Entry::of(Some(0), None, None)]);
    }

    #[test]
    fn a_smithed_pack_by_a_similar_name_stays_apart() {
        let mr = [hit("Tree-Capitator", "treecap-mod", "x", 1)];
        let sm = [pack("treecapitator", "Tree Capitator", 1)];
        assert_eq!(merge(&mr, &[], &sm, false), vec![Entry::of(Some(0), None, None), Entry::of(None, None, Some(0))]);
    }

    #[test]
    fn the_sites_are_named_in_a_sentence() {
        assert_eq!(join_and(&["Modrinth"]), "Modrinth");
        assert_eq!(join_or(&["Modrinth", "CurseForge"]), "Modrinth or CurseForge");
        assert_eq!(join_and(&["Modrinth", "CurseForge", "Smithed"]), "Modrinth, CurseForge and Smithed");
    }
}
