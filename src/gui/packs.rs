//! Packs: browse ATLauncher's own modpacks and Technic's, pick a version, choose optional mods and install, and
//! update or reinstall an installed pack.

use std::sync::Arc;

use egui::{Align, CornerRadius, Layout, RichText, ScrollArea, Stroke, Ui};

use super::theme::*;
use super::widgets::*;
use super::{App, Msg, Page};
use crate::instances::LoaderVersion;
use crate::packs::atlauncher::{self, InstanceOrigin, Pack, PackList, PackOrder, UserCtx};
use crate::packs::choose::ModChoices;
use crate::packs::engine::{self, Ctx, Prepared, Quiet, Request, Source, VersionInfo};
use crate::packs::loader::{self, Kind};
use crate::packs::technic::{self, Build, Modpack, Slim};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PackTab {
    Atl,
    Technic,
}

/// What the background work reports back.
pub enum PacksMsg {
    AtlList(Result<Arc<PackList>, String>),
    Technic {
        seq: u64,
        result: Result<Vec<Slim>, String>,
    },
    TechnicPack {
        token: u64,
        result: Result<(Modpack, Vec<Build>), String>,
    },
    /// `prep` tells which reading of the version this answers: picking another version starts a new one.
    Prepared {
        token: u64,
        prep: u64,
        result: Result<Arc<Prepared>, String>,
    },
    LoaderVersions {
        token: u64,
        prep: u64,
        result: Result<Vec<LoaderVersion>, String>,
    },
}

/// One version in the picker.
#[derive(Clone)]
struct VersionRow {
    label: String,
    info: VersionInfo,
    recommended: bool,
}

/// The details view of one pack: version picker, loader, optional mods.
struct Detail {
    token: u64,
    source: Source,
    title: String,
    description: String,
    website: Option<String>,
    icon: Option<String>,
    name: String,
    versions: Vec<VersionRow>,
    selected: usize,
    /// Counts the readings of the selected version, so an answer for a version picked earlier is ignored.
    prep: u64,
    prepared: Option<Arc<Prepared>>,
    choices: Option<ModChoices>,
    loading: bool,
    error: Option<String>,
    loader_kind: Option<Kind>,
    loader_versions: Vec<LoaderVersion>,
    loader_pick: usize,
    /// The instance being updated or reinstalled (folder name).
    reinstall: Option<String>,
    save_mods: bool,
    is_server: bool,
}

pub struct PacksState {
    pub tab: PackTab,
    atl: Option<Arc<PackList>>,
    atl_loading: bool,
    atl_error: Option<String>,
    query: String,
    order: PackOrder,
    code: String,
    tech_query: String,
    tech: Vec<Slim>,
    tech_loading: bool,
    tech_seq: u64,
    tech_error: Option<String>,
    tech_started: bool,
    detail: Option<Detail>,
    token: u64,
    /// An update asked for before the pack list had arrived.
    pending_update: Option<String>,
    /// The packs the list shows with their descriptions as plain text, for the search, order and account they
    /// were made for. Searching and cleaning up hundreds of descriptions on every frame is wasted work.
    atl_view: Option<(String, Vec<(Pack, String)>)>,
}

impl Default for PacksState {
    fn default() -> Self {
        PacksState {
            tab: PackTab::Atl,
            atl: None,
            atl_loading: false,
            atl_error: None,
            query: String::new(),
            order: PackOrder::Position,
            code: String::new(),
            tech_query: String::new(),
            tech: Vec::new(),
            tech_loading: false,
            tech_seq: 0,
            tech_error: None,
            tech_started: false,
            detail: None,
            token: 0,
            pending_update: None,
            atl_view: None,
        }
    }
}

impl App {
    fn packs_user(&self) -> UserCtx {
        UserCtx::from_settings(&self.settings, self.current_account().map(|a| a.name()))
    }

    fn packs_ctx(&self) -> Ctx {
        Ctx::from_settings(&self.dirs, &self.settings)
    }

    /// Read the pack list (from the server, or the saved copy offline). Called when the page first opens.
    pub(super) fn packs_load_atl(&mut self) {
        if self.packs.atl_loading {
            return;
        }
        self.packs.atl_loading = true;
        let (dirs, n) = (self.dirs.clone(), self.notifier());
        self.rt.spawn(async move {
            let r = PackList::load(&dirs).await.map(Arc::new).map_err(|e| format!("{e:#}"));
            n.send(Msg::Packs(PacksMsg::AtlList(r)));
        });
    }

    fn packs_search_technic(&mut self) {
        self.packs.tech_seq += 1;
        self.packs.tech_loading = true;
        self.packs.tech_started = true;
        let (seq, q, dirs, n) =
            (self.packs.tech_seq, self.packs.tech_query.clone(), self.dirs.clone(), self.notifier());
        self.rt.spawn(async move {
            let r = if q.trim().is_empty() { technic::trending(&dirs).await } else { technic::search(&dirs, &q).await };
            n.send(Msg::Packs(PacksMsg::Technic { seq, result: r.map_err(|e| format!("{e:#}")) }));
        });
    }

    pub(super) fn packs_msg(&mut self, msg: PacksMsg) {
        match msg {
            PacksMsg::AtlList(r) => {
                self.packs.atl_loading = false;
                match r {
                    Ok(list) => {
                        self.packs.atl = Some(list);
                        self.packs.atl_view = None;
                        self.packs.atl_error = None;
                        if let Some(folder) = self.packs.pending_update.take() {
                            self.packs_open_update(&folder);
                        }
                    }
                    Err(e) => self.packs.atl_error = Some(e),
                }
            }
            PacksMsg::Technic { seq, result } => {
                if seq != self.packs.tech_seq {
                    return;
                }
                self.packs.tech_loading = false;
                match result {
                    Ok(list) => {
                        self.packs.tech = list;
                        self.packs.tech_error = None;
                    }
                    Err(e) => self.packs.tech_error = Some(e),
                }
            }
            PacksMsg::TechnicPack { token, result } => {
                let Some(d) = self.packs.detail.as_mut().filter(|d| d.token == token) else { return };
                match result {
                    Ok((pack, builds)) => {
                        d.title = pack.title().to_string();
                        d.description = pack.description.clone().unwrap_or_default();
                        d.website = pack.platform_url.clone();
                        d.icon = pack.icon_url().map(str::to_string);
                        if d.name.is_empty() {
                            d.name = pack.title().to_string();
                        }
                        d.versions = builds
                            .iter()
                            .map(|b| VersionRow {
                                label: b.label(),
                                info: VersionInfo { version: b.version.clone(), hash: None, is_dev: false },
                                recommended: b.recommended,
                            })
                            .collect();
                        // an update starts on the newest build; a new install on the recommended one
                        d.selected = technic::default_build(&builds).unwrap_or(0);
                        d.source = Source::Technic(pack);
                        d.loading = false;
                        self.packs_prepare_selected();
                    }
                    Err(e) => {
                        d.loading = false;
                        d.error = Some(e);
                    }
                }
            }
            PacksMsg::Prepared { token, prep, result } => {
                let Some(d) = self.packs.detail.as_mut().filter(|d| d.token == token && d.prep == prep) else { return };
                d.loading = false;
                match result {
                    Ok(p) => {
                        d.choices = Some(p.choices.clone());
                        d.loader_kind = p.loader_kind;
                        d.prepared = Some(p);
                        d.error = None;
                        self.packs_load_loader_versions();
                    }
                    Err(e) => {
                        d.prepared = None;
                        d.choices = None;
                        d.error = Some(e);
                    }
                }
            }
            PacksMsg::LoaderVersions { token, prep, result } => {
                let Some(d) = self.packs.detail.as_mut().filter(|d| d.token == token && d.prep == prep) else { return };
                if let Ok(list) = result {
                    d.loader_pick = 0;
                    d.loader_versions = list;
                }
            }
        }
    }

    /// Read the selected version's definition: its mods, loader and messages.
    fn packs_prepare_selected(&mut self) {
        let ctx = self.packs_ctx();
        let existing =
            self.packs.detail.as_ref().and_then(|d| d.reinstall.clone()).and_then(|f| self.instance(&f).cloned());
        let n = self.notifier();
        let Some(d) = self.packs.detail.as_mut() else { return };
        let Some(row) = d.versions.get(d.selected).cloned() else { return };
        d.prep += 1;
        d.loading = true;
        d.error = None;
        d.prepared = None;
        d.choices = None;
        d.loader_versions.clear();
        let (token, prep, source, is_server) = (d.token, d.prep, d.source.clone(), d.is_server);
        self.rt.spawn(async move {
            let r = engine::prepare(&ctx, &source, &row.info, is_server, existing.as_ref())
                .await
                .map(Arc::new)
                .map_err(|e| format!("{e:#}"));
            n.send(Msg::Packs(PacksMsg::Prepared { token, prep, result: r }));
        });
    }

    /// If the pack lets the player choose the loader version, fetch the choices.
    fn packs_load_loader_versions(&mut self) {
        let (dirs, n) = (self.dirs.clone(), self.notifier());
        let Some(d) = self.packs.detail.as_ref() else { return };
        let (Some(kind), Some(p)) = (d.loader_kind, d.prepared.clone()) else { return };
        if !p.json.loader.as_ref().is_some_and(|l| l.choose) {
            return;
        }
        let (token, prep, mc) = (d.token, d.prep, p.json.minecraft.clone());
        self.rt.spawn(async move {
            let r = loader::choosable(&dirs, kind, &mc).await.map_err(|e| format!("{e:#}"));
            n.send(Msg::Packs(PacksMsg::LoaderVersions { token, prep, result: r }));
        });
    }

    fn packs_open_atl(&mut self, pack: &Pack) {
        let user = self.packs_user();
        let manifest = self.manifest.clone();
        let versions: Vec<VersionRow> = pack
            .pickable_versions(&user, false, &|_| true)
            .into_iter()
            .map(|v| {
                let kind = manifest
                    .as_ref()
                    .and_then(|m| m.versions.iter().find(|x| x.id == v.minecraft))
                    .map(|x| x.kind.clone());
                VersionRow {
                    label: format!("{}{}", v.label(kind.as_deref()), if v.is_dev() { " (dev)" } else { "" }),
                    recommended: v.is_recommended && !v.is_dev(),
                    info: VersionInfo::from(&v),
                }
            })
            .collect();
        let selected = pack.default_version(&pack.pickable_versions(&user, false, &|_| true)).unwrap_or(0);
        self.packs.token += 1;
        self.packs.detail = Some(Detail {
            token: self.packs.token,
            source: Source::Atl(pack.clone()),
            title: pack.name.clone(),
            description: pack.description.clone(),
            website: pack.website_url.clone(),
            icon: Some(pack.image_url()),
            name: pack.name.clone(),
            versions,
            selected,
            prep: 0,
            prepared: None,
            choices: None,
            loading: false,
            error: None,
            loader_kind: None,
            loader_versions: Vec::new(),
            loader_pick: 0,
            reinstall: None,
            save_mods: false,
            is_server: false,
        });
        self.packs_prepare_selected();
    }

    fn packs_open_technic(&mut self, slug: &str, reinstall: Option<String>, name: String) {
        self.packs.token += 1;
        let token = self.packs.token;
        self.packs.detail = Some(Detail {
            token,
            source: Source::Technic(Modpack { name: slug.to_string(), ..Default::default() }),
            title: slug.to_string(),
            description: String::new(),
            website: None,
            icon: None,
            name,
            versions: Vec::new(),
            selected: 0,
            prep: 0,
            prepared: None,
            choices: None,
            loading: true,
            error: None,
            loader_kind: None,
            loader_versions: Vec::new(),
            loader_pick: 0,
            reinstall,
            save_mods: false,
            is_server: false,
        });
        let (dirs, n, slug) = (self.dirs.clone(), self.notifier(), slug.to_string());
        self.rt.spawn(async move {
            let r = async {
                let pack = technic::modpack(&dirs, &slug).await?;
                let builds = technic::builds(&dirs, &pack).await?;
                anyhow::Ok((pack, builds))
            }
            .await
            .map_err(|e| format!("{e:#}"));
            n.send(Msg::Packs(PacksMsg::TechnicPack { token, result: r }));
        });
    }

    /// Open the update view for an installed pack (the Update button on the library and instance pages). Packs
    /// from ATLauncher's list and from Technic are handled here; for other instances nothing happens.
    pub fn packs_open_update(&mut self, folder: &str) {
        let Some(inst) = self.instance(folder).cloned() else { return };
        self.page = Page::Packs;
        match atlauncher::origin_of(&inst) {
            InstanceOrigin::Atl => {
                let Some(list) = self.packs.atl.clone() else {
                    self.packs.pending_update = Some(folder.to_string());
                    self.packs_load_atl();
                    return;
                };
                let Some(pack) = atlauncher::pack_of(&list, &inst).cloned() else {
                    self.toast_err("That pack isn't in ATLauncher's list any more");
                    return;
                };
                self.packs_open_atl(&pack);
                let newest = atlauncher::update_for(&list, &inst).map(|u| u.version.version);
                if let Some(d) = self.packs.detail.as_mut() {
                    d.reinstall = Some(folder.to_string());
                    d.name = inst.file.launcher.name.clone();
                    let want = newest.unwrap_or_else(|| inst.file.launcher.version.clone());
                    if let Some(i) = d.versions.iter().position(|v| v.info.version.eq_ignore_ascii_case(&want)) {
                        d.selected = i;
                    }
                }
                self.packs_prepare_selected();
            }
            InstanceOrigin::Technic | InstanceOrigin::TechnicSolder => match technic::installed_pack(&inst) {
                Some(m) => self.packs_open_technic(&m.name, Some(folder.to_string()), inst.file.launcher.name.clone()),
                None => self.toast_err("Couldn't tell which Technic pack this instance was made from"),
            },
            _ => self.toast("This instance doesn't come from an ATLauncher or Technic pack"),
        }
    }

    fn packs_install(&mut self) {
        let ctx = self.packs_ctx();
        let Some(d) = self.packs.detail.as_mut() else { return };
        let Some(row) = d.versions.get(d.selected).cloned() else { return };
        let existing = d.reinstall.clone().and_then(|f| self.instances.iter().find(|i| i.folder == f).cloned());
        let choice = d
            .loader_versions
            .get(d.loader_pick)
            .filter(|_| d.loader_kind.is_some())
            .map(|l| loader::Choice { version: l.version.clone(), raw_version: l.raw_version.clone() });
        let title = d.title.clone();
        let reinstall = existing.is_some();
        let source = d.source.clone();
        let choices = d.choices.clone();
        let save_mods = d.save_mods;
        let name = d.name.clone();
        let solder = d.prepared.as_ref().and_then(|p| p.solder.clone());
        let label = if reinstall { format!("Updating {title}") } else { format!("Installing {title}") };
        self.start_task(label, move |progress, _n| async move {
            let mut req = Request::new(&name, source, row.info, Arc::new(Quiet::new(progress.clone())));
            req.is_reinstall = reinstall;
            req.existing = existing;
            req.save_mods = save_mods;
            req.loader_choice = choice;
            req.choices = choices;
            req.solder = solder;
            let inst = engine::install(&ctx, req, &progress).await?;
            Ok(format!("{} {}", if reinstall { "Updated" } else { "Installed" }, inst.name()))
        });
        self.packs.detail = None;
        self.page = Page::Library;
    }

    pub(super) fn packs_ui(&mut self, ui: &mut Ui) {
        if self.packs.atl.is_none() && !self.packs.atl_loading && self.packs.atl_error.is_none() {
            self.packs_load_atl();
        }
        if self.packs.detail.is_some() {
            self.packs_detail_ui(ui);
            return;
        }
        ui.horizontal(|ui| {
            heading(ui, "Packs");
            muted(ui, "ATLauncher and Technic modpacks");
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if chip(ui, "ATLauncher", self.packs.tab == PackTab::Atl).clicked() {
                self.packs.tab = PackTab::Atl;
            }
            if chip(ui, "Technic", self.packs.tab == PackTab::Technic).clicked() {
                self.packs.tab = PackTab::Technic;
                if !self.packs.tech_started {
                    self.packs_search_technic();
                }
            }
        });
        ui.add_space(4.0);
        match self.packs.tab {
            PackTab::Atl => self.packs_atl_ui(ui),
            PackTab::Technic => self.packs_technic_ui(ui),
        }
    }

    fn packs_atl_ui(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            let mut q = std::mem::take(&mut self.packs.query);
            search_field(ui, &mut q, "Search ATLauncher packs", 320.0f32.min(ui.available_width()));
            self.packs.query = q;
            egui::ComboBox::from_id_salt("pack-order")
                .selected_text(match self.packs.order {
                    PackOrder::Position => "Featured order",
                    PackOrder::NameAscending => "Name A to Z",
                    PackOrder::NameDescending => "Name Z to A",
                })
                .show_ui(ui, |ui| {
                    for (o, l) in [
                        (PackOrder::Position, "Featured order"),
                        (PackOrder::NameAscending, "Name A to Z"),
                        (PackOrder::NameDescending, "Name Z to A"),
                    ] {
                        ui.selectable_value(&mut self.packs.order, o, l);
                    }
                });
            // a pack code opens a pack that is not listed for everyone
            let mut code = std::mem::take(&mut self.packs.code);
            search_field(ui, &mut code, "Pack code", 140.0);
            self.packs.code = code;
            if secondary(ui, "Add code").clicked() {
                self.packs_add_code();
            }
        });
        if let Some(e) = self.packs.atl_error.clone() {
            if super::discover::fetch_error_card(ui, "Couldn't load the ATLauncher pack list", &e) {
                // the page asks again once the error is gone
                self.packs.atl_error = None;
            }
            return;
        }
        let Some(list) = self.packs.atl.clone() else {
            ui.horizontal(|ui| {
                ui.spinner();
                muted(ui, "Getting the pack list");
            });
            return;
        };
        let user = self.packs_user();
        let key = format!(
            "{}\u{1f}{:?}\u{1f}{:?}\u{1f}{}",
            self.packs.query,
            self.packs.order,
            user.username,
            user.added_codes.join(",")
        );
        if self.packs.atl_view.as_ref().is_none_or(|(k, _)| *k != key) {
            let packs = list
                .search(&user, &self.packs.query, self.packs.order)
                .into_iter()
                .map(|p| (p.clone(), plain_text(&p.description)))
                .collect();
            self.packs.atl_view = Some((key, packs));
        }
        let Some((key, packs)) = self.packs.atl_view.take() else { return };
        if packs.is_empty() {
            card(ui, |ui| muted(ui, "No packs found."));
            self.packs.atl_view = Some((key, packs));
            return;
        }
        let mut open: Option<Pack> = None;
        super::mods::lazy_list(ui, egui::Id::unique("atl-packs"), &packs, 104.0, |ui, (pack, description)| {
            pack_card(ui, |ui| {
                ui.horizontal(|ui| {
                    self.remote_image(ui, &pack.image_url(), 60.0, &pack.name);
                    ui.vertical(|ui| {
                        ui.set_max_width((ui.available_width() - 150.0).max(120.0));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&pack.name).strong().size(16.0));
                            if pack.is_private() {
                                tag(ui, "Private", c_warn());
                            } else if pack.is_semi_public() {
                                tag(ui, "Code", c_warn());
                            }
                        });
                        ui.add(egui::Label::new(RichText::new(description).color(c_muted())).truncate());
                        ui.horizontal(|ui| {
                            if let Some(v) = pack.latest_version() {
                                small_muted(ui, format!("{} \u{00B7} Minecraft {}", v.version, v.minecraft));
                            }
                            small_muted(ui, format!("{} versions", pack.versions.len()));
                        });
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary(ui, "Install").clicked() {
                            open = Some(pack.clone());
                        }
                    });
                });
            });
        });
        self.packs.atl_view = Some((key, packs));
        if let Some(p) = open {
            self.packs_open_atl(&p);
        }
    }

    fn packs_add_code(&mut self) {
        let code = self.packs.code.trim().to_string();
        if code.is_empty() {
            return;
        }
        let Some(list) = self.packs.atl.clone() else { return };
        let user = self.packs_user();
        match list.add_code(&code, &user) {
            Some(added) => {
                let mut codes = user.added_codes.clone();
                codes.push(added);
                self.settings.extras.insert("addedPacks".into(), serde_json::json!(codes));
                self.save_settings();
                self.packs.code.clear();
                self.toast("Pack added");
            }
            None => self.toast_err("That code doesn't open a new pack"),
        }
    }

    fn packs_technic_ui(&mut self, ui: &mut Ui) {
        let mut search = false;
        ui.horizontal_wrapped(|ui| {
            let mut q = std::mem::take(&mut self.packs.tech_query);
            let r = search_field(ui, &mut q, "Search Technic packs", 340.0f32.min(ui.available_width()));
            self.packs.tech_query = q;
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                search = true;
            }
            if primary(ui, "Search").clicked() {
                search = true;
            }
        });
        if self.packs.tech_query.trim().is_empty() {
            small_muted(ui, "Showing what's trending");
        }
        if let Some(e) = self.packs.tech_error.clone() {
            if super::discover::fetch_error_card(ui, "Couldn't reach Technic", &e) && !self.packs.tech_loading {
                search = true;
            }
        }
        if search {
            self.packs_search_technic();
        }
        if self.packs.tech_loading {
            ui.spinner();
        } else if self.packs.tech.is_empty() && self.packs.tech_error.is_none() && self.packs.tech_started {
            card(ui, |ui| muted(ui, "No packs found. Try other words."));
        }
        let results = std::mem::take(&mut self.packs.tech);
        let mut open: Option<Slim> = None;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for m in &results {
                pack_card(ui, |ui| {
                    ui.horizontal(|ui| {
                        match &m.icon_url {
                            Some(u) if !u.is_empty() => self.remote_image(ui, u, 60.0, &m.name),
                            _ => {
                                avatar(ui, &m.name, 60.0);
                            }
                        }
                        ui.vertical(|ui| {
                            ui.set_max_width((ui.available_width() - 150.0).max(120.0));
                            ui.label(RichText::new(&m.name).strong().size(16.0));
                            small_muted(ui, &m.slug);
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if primary(ui, "Install").clicked() {
                                open = Some(m.clone());
                            }
                        });
                    });
                });
            }
        });
        self.packs.tech = results;
        if let Some(m) = open {
            self.packs_open_technic(&m.slug, None, m.name.clone());
        }
    }

    fn packs_detail_ui(&mut self, ui: &mut Ui) {
        let mut back = false;
        let mut install = false;
        let mut reprepare = false;
        let mut website: Option<String> = None;
        let mut icon: Option<(String, String)> = None;
        if let Some(d) = &self.packs.detail {
            if let Some(i) = &d.icon {
                icon = Some((i.clone(), d.title.clone()));
            }
        }
        let java_major = self.java_list.iter().filter_map(|j| j.major).max();
        ui.horizontal(|ui| {
            if secondary(ui, "Back").clicked() {
                back = true;
            }
            if let Some((url, title)) = &icon {
                self.remote_image(ui, url, 44.0, title);
            }
            if let Some(d) = &self.packs.detail {
                heading(ui, &d.title);
            }
        });
        ui.add_space(6.0);
        let Some(d) = self.packs.detail.as_mut() else { return };
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if !d.description.is_empty() {
                card(ui, |ui| {
                    ui.label(plain_text(&d.description));
                    if let Some(w) = &d.website {
                        if secondary(ui, "Open website").clicked() {
                            website = Some(w.clone());
                        }
                    }
                });
                ui.add_space(6.0);
            }
            card(ui, |ui| {
                if d.reinstall.is_none() {
                    ui.label(RichText::new("Name").strong());
                    ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(400.0));
                    ui.add_space(8.0);
                } else {
                    small_muted(ui, format!("Updating {}", d.name));
                }
                ui.label(RichText::new("Version").strong());
                let current = d.versions.get(d.selected).map(|v| v.label.clone()).unwrap_or_else(|| if d.loading { "Loading".into() } else { "No versions".into() });
                let before = d.selected;
                egui::ComboBox::from_id_salt("pack-version").selected_text(current).width(400.0).show_ui(ui, |ui| {
                    for (i, v) in d.versions.iter().enumerate() {
                        let text = if v.recommended { format!("{}  (recommended)", v.label) } else { v.label.clone() };
                        ui.selectable_value(&mut d.selected, i, text);
                    }
                });
                if d.selected != before {
                    reprepare = true;
                }
                if let Some(p) = &d.prepared {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        tag(ui, &format!("Minecraft {}", p.json.minecraft), c_accent());
                        if let Some(k) = d.loader_kind {
                            tag(ui, k.type_name(), loader_color(k.type_name()));
                        }
                        if p.json.memory > 0 {
                            tag(ui, &format!("{} MB memory", p.json.memory), c_muted());
                        }
                        if let Some(j) = &p.json.java {
                            tag(ui, &j.describe(), c_muted());
                        }
                    });
                    // the Java edition warns before launching when the Java in use does not fit; say so early
                    if let (Some(j), Some(have)) = (&p.json.java, java_major) {
                        if !j.conforms(have as i64) {
                            ui.label(RichText::new(format!("This version needs {}. The newest Java found here is {}.", j.describe(), have)).color(c_warn()).size(12.5));
                        }
                    }
                }
                if !d.loader_versions.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new(format!("{} version", d.loader_kind.map(|k| k.type_name()).unwrap_or("Loader"))).strong());
                    let current = d.loader_versions.get(d.loader_pick).map(|l| l.version.clone()).unwrap_or_default();
                    egui::ComboBox::from_id_salt("pack-loader").selected_text(current).width(400.0).show_ui(ui, |ui| {
                        for (i, l) in d.loader_versions.iter().enumerate() {
                            ui.selectable_value(&mut d.loader_pick, i, &l.version);
                        }
                    });
                }
                if d.reinstall.is_some() {
                    ui.add_space(8.0);
                    ui.checkbox(&mut d.save_mods, "Keep my mods");
                    small_muted(ui, "If the update changes the Minecraft version, mods you added may stop working. Keep them, or leave this off to remove them.");
                }
            });
            if d.loading {
                ui.add_space(8.0);
                ui.spinner();
            }
            if let Some(e) = &d.error {
                ui.add_space(6.0);
                card(ui, |ui| {
                    ui.label(RichText::new("Couldn't read this version").color(c_danger()).strong());
                    muted(ui, e.clone());
                });
            }
            if let Some(msg) = d.prepared.as_ref().and_then(|p| p.json.messages.clone()) {
                if let Some(text) = if d.reinstall.is_some() { msg.update } else { msg.install } {
                    ui.add_space(6.0);
                    card(ui, |ui| {
                        ui.label(RichText::new("Message from the pack").strong());
                        muted(ui, plain_text(&text));
                    });
                }
            }
            if let (Some(choices), Some(p)) = (d.choices.as_mut(), d.prepared.clone()) {
                if choices.needs_asking() {
                    ui.add_space(6.0);
                    choices_ui(ui, choices, &p);
                }
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ready = d.prepared.is_some() && !d.loading && (d.reinstall.is_some() || !d.name.trim().is_empty());
                let label = if d.reinstall.is_some() { "Update" } else { "Install" };
                if primary_enabled(ui, label, ready).clicked() {
                    install = true;
                }
                if secondary(ui, "Cancel").clicked() {
                    back = true;
                }
            });
        });
        if let Some(w) = website {
            let _ = open::that(w);
        }
        if reprepare {
            self.packs_prepare_selected();
        }
        if install {
            self.packs_install();
        } else if back {
            self.packs.detail = None;
        }
    }
}

/// The optional-mod boxes: required mods on one side, optional on the other, with the Java window's buttons.
fn choices_ui(ui: &mut Ui, choices: &mut ModChoices, prepared: &Prepared) {
    card(ui, |ui| {
        ui.label(RichText::new("Optional mods").strong().size(16.0));
        ui.horizontal(|ui| {
            let label = if choices.has_recommended() { "Select recommended" } else { "Select all" };
            if secondary(ui, label).clicked() {
                choices.select_recommended();
            }
            if secondary(ui, "Clear all").clicked() {
                choices.clear_all();
            }
        });
        ui.add_space(4.0);
        let order = choices.alphabetical();
        let mut toggled: Option<(usize, bool)> = None;
        for &i in &order {
            let c = &choices.choices[i];
            if !c.optional || !c.visible {
                continue;
            }
            let mut on = c.selected;
            let indent = if c.pack_mod.has_linked() { 22.0 } else { 0.0 };
            ui.horizontal(|ui| {
                ui.add_space(indent);
                let r = ui.add_enabled(c.enabled, egui::Checkbox::new(&mut on, &c.pack_mod.name));
                if let Some(desc) = c.pack_mod.description.as_deref().filter(|d| !d.is_empty()) {
                    r.on_hover_text(desc);
                }
                if !c.pack_mod.version.is_empty() {
                    small_muted(ui, &c.pack_mod.version);
                }
                if c.pack_mod.has_group() {
                    tag(ui, "pick one", c_muted());
                }
            });
            // the Java edition asks before a mod with a warning is switched on; here the warning stays under the
            // box for as long as it is ticked
            if c.selected {
                if let Some(w) = choices.warning_for(i, &prepared.json.warnings) {
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(indent + 26.0);
                        ui.label(RichText::new(plain_text(&w)).color(c_warn()).size(12.5));
                    });
                }
            }
            if on != c.selected {
                toggled = Some((i, on));
            }
        }
        if let Some((i, on)) = toggled {
            choices.toggle(i, on);
            ui.ctx().request_repaint();
        }
        let required = choices.choices.iter().filter(|c| !c.optional && c.visible).count();
        small_muted(ui, format!("{required} required mods are always installed"));
    });
}

fn pack_card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let r = egui::Frame::new()
        .fill(c_card())
        .stroke(Stroke::new(1.0, c_border()))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner;
    ui.add_space(2.0);
    r
}

/// Pack descriptions come with HTML and line breaks; show them as plain text.
fn plain_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.replace("<br/>", "\n").replace("<br>", "\n").chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&nbsp;", " ").trim().to_string()
}
