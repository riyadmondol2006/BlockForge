//! Packs: browse ATLauncher's own modpacks and Technic's, pick a version, choose optional mods and install, and
//! update or reinstall an installed pack.

use std::sync::Arc;
use std::time::Instant;

use egui::{Align, Layout, RichText, ScrollArea, Ui};

use super::discover::*;
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
    /// When the Technic search field was last typed in and its search has not started yet.
    edited: Option<Instant>,
    /// The window that asks for a pack code is open.
    code_open: bool,
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
            edited: None,
            code_open: false,
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
        if let Some(inst) = existing.as_ref().filter(|i| self.is_playing(&i.folder)) {
            self.toast_err(format!("Close Minecraft in {} first, then update it.", inst.name()));
            return;
        }
        let label = if reinstall { format!("Updating {title}") } else { format!("Installing {title}") };
        // an update changes the instance: one change at a time, so a mod switched meanwhile isn't written over
        let lock = existing.as_ref().map(|i| super::mods::instance_lock(&i.root));
        self.start_task(label, move |progress, _n| async move {
            let _turn = match &lock {
                Some(l) => Some(l.lock().await),
                None => None,
            };
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

    /// For the screenshot tour: a catalog's list still on its way (`atl-loading`, `technic-loading`) or failed to
    /// come (`atl-error`), as the page shows it.
    pub(super) fn shot_packs_state(&mut self, state: &str) {
        let p = &mut self.packs;
        p.detail = None;
        match state {
            "atl-loading" => {
                p.tab = PackTab::Atl;
                (p.atl, p.atl_view, p.atl_loading, p.atl_error) = (None, None, true, None);
            }
            "atl-error" => {
                p.tab = PackTab::Atl;
                (p.atl, p.atl_view, p.atl_loading) = (None, None, false);
                p.atl_error = Some(
                    "error sending request for url (https://cdn.atlauncher.com/launcher/json/packsnew.json)".into(),
                );
            }
            "technic-loading" => {
                p.tab = PackTab::Technic;
                // an answer already on its way is for an older search
                p.tech_seq += 1;
                (p.tech_loading, p.tech_started, p.tech_error) = (true, true, None);
                p.tech.clear();
            }
            _ => {}
        }
    }

    /// Open the page on ATLauncher's packs, or with `technic` on Technic's (searched the first time).
    pub(super) fn packs_show(&mut self, technic: bool) {
        self.packs.tab = if technic { PackTab::Technic } else { PackTab::Atl };
        self.packs.detail = None;
        if technic && !self.packs.tech_started {
            self.packs_search_technic();
        }
        self.go(Page::Packs);
    }

    pub(super) fn packs_ui(&mut self, ui: &mut Ui) {
        if self.packs.atl.is_none() && !self.packs.atl_loading && self.packs.atl_error.is_none() {
            self.packs_load_atl();
        }
        if self.packs.detail.is_some() {
            self.packs_detail_ui(ui);
            return;
        }
        // the platforms have modpacks only; another kind opens on a site that has it
        if self.explore_header(ui) {
            return;
        }
        match self.packs.tab {
            PackTab::Atl => self.packs_atl_ui(ui),
            PackTab::Technic => self.packs_technic_ui(ui),
        }
        if self.packs.code_open {
            self.packs_code_dialog(ui.ctx());
        }
    }

    /// The small window that asks for a pack code.
    fn packs_code_dialog(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut add = false;
        let (width, _) = super::curseforge::modal_size(ctx, 420.0, 200.0);
        let modal = egui::Modal::new(egui::Id::unique("pack-code")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(width);
            ui.label(RichText::new("Add a private pack").size(FS_SECTION).semibold());
            ui.add_space(2.0);
            muted(ui, "A pack code opens a pack that is not listed for everyone.");
            ui.add_space(14.0);
            let r = ui.add_sized(
                [ui.available_width(), 34.0],
                egui::TextEdit::singleline(&mut self.packs.code)
                    .hint_text(RichText::new("Pack code").color(c_faint()))
                    .margin(egui::Margin::symmetric(12, 8))
                    .background_color(c_field()),
            );
            focus_when_shown(ui, &r);
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                add = true;
            }
            ui.add_space(16.0);
            dialog_footer(
                ui,
                "pack-code",
                |_| {},
                |ui| {
                    if secondary(ui, "Cancel").clicked() {
                        close = true;
                    }
                    if primary_enabled(ui, "Add", !self.packs.code.trim().is_empty()).clicked() {
                        add = true;
                    }
                },
            );
        });
        if modal.should_close() {
            close = true;
        }
        if add {
            self.packs_add_code();
            // the code is cleared when it opened a pack; when it did not, the window stays to try another
            close = self.packs.code.is_empty();
        }
        if close {
            self.packs.code_open = false;
        }
    }

    fn packs_atl_ui(&mut self, ui: &mut Ui) {
        let page_w = ui.available_width();
        search_bar(
            ui,
            "atl",
            &mut self.packs,
            |ui, packs, width| {
                // the whole list is here already, so it is searched on every key
                search_input(ui, &mut packs.query, "Search ATLauncher packs", width);
            },
            |ui, packs| {
                let value = match packs.order {
                    PackOrder::Position => "Featured",
                    PackOrder::NameAscending => "Name A to Z",
                    PackOrder::NameDescending => "Name Z to A",
                };
                dropdown(ui, "pack-order", "Order", value, false, false, |ui| {
                    for (o, l) in [
                        (PackOrder::Position, "Featured order"),
                        (PackOrder::NameAscending, "Name A to Z"),
                        (PackOrder::NameDescending, "Name Z to A"),
                    ] {
                        ui.selectable_value(&mut packs.order, o, l);
                    }
                });
            },
            |_, _| {},
        );
        ui.add_space(6.0);
        if let Some(e) = self.packs.atl_error.clone() {
            if fetch_error(
                ui,
                "Couldn't load the ATLauncher pack list. Check your internet connection, or try again in a few minutes.",
                &e,
            ) {
                // the page asks again once the error is gone
                self.packs.atl_error = None;
            }
            return;
        }
        let Some(list) = self.packs.atl.clone() else {
            result_skeleton(ui, 4, "Loading ATLauncher packs\u{2026}");
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
            let query = self.packs.query.trim().to_string();
            let text = if query.is_empty() {
                "No packs to show. Try adding a pack code.".to_string()
            } else {
                format!("No packs match \u{201C}{query}\u{201D}.")
            };
            if empty_state(ui, &text, (!query.is_empty()).then_some("Clear search")).is_some_and(|r| r.clicked()) {
                self.packs.query.clear();
            }
            self.packs.atl_view = Some((key, packs));
            return;
        }
        let (cards, mut flip) = (self.results_as_cards(), None);
        let count = results_text(packs.len() as u64, "pack", "packs");
        let mut add_code = false;
        context_line_with(
            ui,
            Some(count),
            true,
            |ui| {
                // a pack code opens a pack that is not listed for everyone
                add_code = ghost_icon(ui, super::icons::Icon::Plus, "Add a private pack code").clicked();
            },
            |ui| flip = view_switch(ui, cards, page_w),
        );
        self.set_results_view(flip);
        if add_code {
            self.packs.code_open = true;
        }
        ui.add_space(4.0);
        let mut open: Option<Pack> = None;
        let mut card = |this: &mut App, ui: &mut Ui, (pack, description): &(Pack, String), width: Option<f32>| {
            // a pack that is not listed for everyone says so on a pill in front of its figures
            let mut pills = Vec::new();
            if pack.is_private() {
                pills.push(("Private".to_string(), c_warn()));
            } else if pack.is_semi_public() {
                pills.push(("Code".to_string(), c_warn()));
            }
            // the Minecraft version and the pack's own version of its newest release, and how many there are
            let mut compat = Vec::new();
            if let Some(v) = pack.latest_version() {
                compat.push((v.minecraft.to_string(), c_muted()));
                compat.push((pack_version(&v.version), c_muted()));
            }
            compat.push((results_text(pack.versions.len() as u64, "version", "versions"), c_faint()));
            let view = ResultView {
                key: format!("atl:{}", pack.name),
                title: &pack.name,
                by: None,
                summary: description,
                pills,
                compat,
                sites: Vec::new(),
                tags: Vec::new(),
                figure: None,
            };
            result_item(ui, width, &view, |ui, slot| match slot {
                Slot::Icon(size) => this.result_icon(ui, Some(&pack.image_url()), &pack.name, size),
                Slot::Actions => {
                    // the version, the loader and the optional mods are chosen on the pack's page
                    if install_button(ui, "Install").on_hover_text("Choose the version, then install").clicked() {
                        open = Some(pack.clone());
                    }
                }
            });
        };
        // hundreds of packs: only the rows (or lines of cards) in view are laid out
        let list_id = egui::Id::unique("atl-packs");
        if cards && cards_fit(page_w) {
            lazy_cards(ui, list_id.with("cards"), &packs, |ui, item, width| card(self, ui, item, Some(width)));
        } else {
            super::mods::lazy_list(ui, list_id, &packs, ROW_H, |ui, item| card(self, ui, item, None));
        }
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
        let page_w = ui.available_width();
        search_bar(
            ui,
            "technic",
            &mut (&mut self.packs, &mut search),
            |ui, (packs, search), width| {
                let mut q = std::mem::take(&mut packs.tech_query);
                let mut edited = packs.edited.take();
                if live_search_w(ui, &mut q, "Search Technic packs", &mut edited, width) {
                    **search = true;
                }
                packs.tech_query = q;
                packs.edited = edited;
            },
            |_, _| {},
            |_, _| {},
        );
        let trending = self.packs.tech_query.trim().is_empty();
        if trending || !self.packs.tech.is_empty() {
            let count =
                (!self.packs.tech.is_empty()).then(|| results_text(self.packs.tech.len() as u64, "pack", "packs"));
            let (cards, mut flip) = (self.results_as_cards(), None);
            context_line_with(
                ui,
                count,
                false,
                |ui| {
                    if trending {
                        small_muted(ui, "Showing what's trending");
                    }
                },
                |ui| flip = view_switch(ui, cards, page_w),
            );
            self.set_results_view(flip);
        }
        ui.add_space(6.0);
        if let Some(e) = self.packs.tech_error.clone() {
            if fetch_error(
                ui,
                "Couldn't reach Technic. Check your internet connection, or try again in a few minutes.",
                &e,
            ) && !self.packs.tech_loading
            {
                search = true;
            }
        }
        if search {
            self.packs_search_technic();
        }
        if self.packs.tech_loading && self.packs.tech.is_empty() {
            let status = if trending { "Loading Technic packs\u{2026}" } else { "Searching Technic\u{2026}" };
            result_skeleton(ui, 4, status);
        } else if self.packs.tech.is_empty() && self.packs.tech_error.is_none() && self.packs.tech_started {
            let query = self.packs.tech_query.trim().to_string();
            let text = if query.is_empty() {
                "No packs to show.".to_string()
            } else {
                format!("No packs match \u{201C}{query}\u{201D}.")
            };
            if empty_state(ui, &text, (!query.is_empty()).then_some("Clear search")).is_some_and(|r| r.clicked()) {
                self.packs.tech_query.clear();
                self.packs_search_technic();
            }
        }
        let results = std::mem::take(&mut self.packs.tech);
        let mut open: Option<Slim> = None;
        // a new search starts at the top of its list
        let list_id = ("tech-list", self.packs.tech_query.trim().to_string());
        let cards = self.results_as_cards();
        ScrollArea::vertical().id_salt(list_id).auto_shrink(false).show(ui, |ui| {
            result_list(ui, cards, &results, |ui, m, width| {
                let view = ResultView {
                    key: format!("tech:{}", m.slug),
                    title: &m.name,
                    by: None,
                    summary: "",
                    pills: Vec::new(),
                    compat: vec![(m.slug.clone(), c_muted())],
                    sites: Vec::new(),
                    tags: Vec::new(),
                    figure: None,
                };
                result_item(ui, width, &view, |ui, slot| match slot {
                    Slot::Icon(size) => self.result_icon(ui, m.icon_url.as_deref(), &m.name, size),
                    Slot::Actions => {
                        if install_button(ui, "Install").on_hover_text("Choose the version, then install").clicked() {
                            open = Some(m.clone());
                        }
                    }
                });
            });
            if self.packs.tech_loading {
                ui.add_space(8.0);
                spinner(ui);
            }
            ui.add_space(8.0);
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
        let mut title = String::new();
        let mut facts: Vec<(String, egui::Color32)> = Vec::new();
        if let Some(d) = &self.packs.detail {
            title = d.title.clone();
            if let Some(i) = &d.icon {
                icon = Some((i.clone(), d.title.clone()));
            }
            website = d.website.clone();
            if let Some(p) = &d.prepared {
                facts.push((p.json.minecraft.to_string(), c_muted()));
                if let Some(k) = d.loader_kind {
                    facts.push((k.type_name().to_string(), loader_color(k.type_name())));
                }
                if p.json.memory > 0 {
                    facts.push((format!("{} MB", p.json.memory), c_muted()));
                }
                if let Some(j) = &p.json.java {
                    facts.push((j.describe(), c_muted()));
                }
            }
        }
        let java_major = self.java_list.iter().filter_map(|j| j.major).max();
        let mut open_website = false;
        let back_to = if matches!(self.packs.detail.as_ref().map(|d| &d.source), Some(Source::Technic(_))) {
            "Technic"
        } else {
            "ATLauncher"
        };
        if back_link(ui, back_to).clicked() {
            back = true;
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            if let Some((url, name)) = &icon {
                self.result_icon(ui, Some(url), name, 96.0);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                heading(ui, &title);
                // what the version needs, on the monospace tags of the result rows: the loader in its own colour
                if !facts.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(5.0, 5.0);
                        for (text, color) in &facts {
                            mono_tag(ui, text, *color).on_hover_text(match text {
                                t if t.ends_with(" MB") => "The memory the pack asks for",
                                t if t == &facts[0].0 => "Minecraft version",
                                _ => "",
                            });
                        }
                    });
                }
            });
            if website.is_some() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ghost(ui, "Open website").clicked() {
                        open_website = true;
                    }
                });
            }
        });
        ui.add_space(14.0);
        let Some(d) = self.packs.detail.as_mut() else { return };
        ScrollArea::vertical().auto_shrink(false).max_height((ui.available_height() - 64.0).max(160.0)).show(ui, |ui| {
            // text is easiest to read in a column of its own
            ui.set_max_width(ui.available_width().min(DETAIL_W));
            ui.spacing_mut().item_spacing.y = 8.0;
            if !d.description.is_empty() {
                description_ui(ui, &plain_text(&d.description));
                ui.add_space(16.0);
            }
            // what to install, on a card of its own
            card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            if d.reinstall.is_none() {
                section(ui, "Name");
                ui.add_sized(
                    [ui.available_width().min(360.0), 32.0],
                    egui::TextEdit::singleline(&mut d.name).margin(egui::Margin::symmetric(10, 7)).background_color(c_field()),
                );
                ui.add_space(8.0);
            } else {
                small_muted(ui, format!("Updating {}", d.name));
            }
            section(ui, "Version");
            let current = d.versions.get(d.selected).map(|v| v.label.clone()).unwrap_or_else(|| if d.loading { "Loading".into() } else { "No versions".into() });
            let before = d.selected;
            egui::ComboBox::from_id_salt("pack-version").selected_text(current).width(320.0).show_ui(ui, |ui| {
                for (i, v) in d.versions.iter().enumerate() {
                    let text = if v.recommended { format!("{}  (recommended)", v.label) } else { v.label.clone() };
                    ui.selectable_value(&mut d.selected, i, text);
                }
            });
            if d.selected != before {
                reprepare = true;
            }
            if let Some(p) = &d.prepared {
                // the Java edition warns before launching when the Java in use does not fit; say so early
                if let (Some(j), Some(have)) = (&p.json.java, java_major) {
                    if !j.conforms(have as i64) {
                        notice(
                            ui,
                            NoticeKind::Warn,
                            &format!("This version needs {}. The newest Java found here is {}.", j.describe(), have),
                            None,
                        );
                    }
                }
            }
            if !d.loader_versions.is_empty() {
                ui.add_space(8.0);
                section(ui, &format!("{} version", d.loader_kind.map(|k| k.type_name()).unwrap_or("Loader")));
                let current = d.loader_versions.get(d.loader_pick).map(|l| l.version.clone()).unwrap_or_default();
                egui::ComboBox::from_id_salt("pack-loader").selected_text(current).width(320.0).show_ui(ui, |ui| {
                    for (i, l) in d.loader_versions.iter().enumerate() {
                        ui.selectable_value(&mut d.loader_pick, i, &l.version);
                    }
                });
            }
            if d.reinstall.is_some() {
                ui.add_space(8.0);
                setting_row(
                    ui,
                    "Keep my mods",
                    "If the update changes the Minecraft version, mods you added may stop working. Keep them, or leave this off to remove them.",
                    |ui| {
                        toggle_named(ui, &mut d.save_mods, "Keep my mods");
                    },
                );
            }
            if d.loading {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    spinner(ui);
                    muted(ui, "Reading this version");
                });
            }
            if let Some(e) = &d.error {
                ui.add_space(8.0);
                notice(ui, NoticeKind::Error, &format!("Couldn't read this version: {e}"), None);
            }
            if let Some(msg) = d.prepared.as_ref().and_then(|p| p.json.messages.clone()) {
                if let Some(text) = if d.reinstall.is_some() { msg.update } else { msg.install } {
                    ui.add_space(8.0);
                    small_muted(ui, "Message from the pack");
                    notice(ui, NoticeKind::Info, &plain_text(&text), None);
                }
            }
            if let (Some(choices), Some(p)) = (d.choices.as_mut(), d.prepared.clone()) {
                if choices.needs_asking() {
                    ui.add_space(12.0);
                    choices_ui(ui, choices, &p);
                }
            }
            });
            ui.add_space(16.0);
        });
        // what installing does, and the one button for it
        let summary = match (&d.prepared, d.loading) {
            (Some(p), _) => {
                let mut parts = Vec::new();
                if let Some(c) = &d.choices {
                    let n = c.choices.iter().filter(|c| c.selected).count();
                    if n > 0 {
                        parts.push(format!("Installs {n} mods"));
                    }
                }
                if let Some(j) = &p.json.java {
                    parts.push(format!("needs {}", j.describe()));
                }
                parts.join("  \u{00B7}  ")
            }
            (None, true) => "Reading this version".to_string(),
            (None, false) => String::new(),
        };
        let ready = d.prepared.is_some() && !d.loading && (d.reinstall.is_some() || !d.name.trim().is_empty());
        let label = if d.reinstall.is_some() { "Update" } else { "Install" };
        ui.scope(|ui| {
            ui.set_max_width(ui.available_width().min(DETAIL_W));
            rule(ui);
            ui.add_space(8.0);
            dialog_footer(
                ui,
                "pack-detail",
                |ui| {
                    small_muted(ui, summary);
                },
                |ui| {
                    if primary_enabled(ui, label, ready).clicked() {
                        install = true;
                    }
                },
            );
        });
        if open_website {
            if let Some(w) = website {
                let _ = open::that(w);
            }
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
    section(ui, "Optional mods");
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
                small_muted(ui, "pick one");
            }
        });
        // the Java edition asks before a mod with a warning is switched on; here the warning stays under the
        // box for as long as it is ticked
        if c.selected {
            if let Some(w) = choices.warning_for(i, &prepared.json.warnings) {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(indent + 26.0);
                    ui.label(RichText::new(plain_text(&w)).color(c_warn()).size(FS_META));
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
}

/// A pack's own version as its tag shows it: "v1.2.0" for a number, a name as it is.
pub(super) fn pack_version(version: &str) -> String {
    let v = version.trim();
    if v.starts_with(|c: char| c.is_ascii_digit()) { format!("v{v}") } else { v.to_string() }
}

/// The widest the text of a pack's page gets.
const DETAIL_W: f32 = 720.0;

/// Pack descriptions come with HTML and line breaks; show them as plain text. The end of a paragraph or a heading is
/// a blank line and a list item a line of its own with a bullet, so [`description_blocks`] still finds them.
fn plain_text(s: &str) -> String {
    let mut s = s.replace("<br/>", "\n").replace("<br>", "\n").replace("<br />", "\n").replace("<li>", "\n\u{2022} ");
    for end in ["</p>", "</div>", "</ul>", "</ol>", "</h1>", "</h2>", "</h3>", "</h4>", "</h5>", "</h6>"] {
        s = s.replace(end, "\n\n");
    }
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&nbsp;", " ").trim().to_string()
}

/// A part of a pack's description as its page lays it out.
#[derive(Clone, Debug, PartialEq)]
enum Block {
    /// A short line of its own that names what comes after it ("Included Mods").
    Heading(String),
    /// Lines that belong together (their own line breaks kept).
    Paragraph(String),
    /// An item of a list, without the character it was marked with: the page draws its own bullet.
    Item(String),
}

/// What a line of a description is a list item of, when it is one: it starts with a bullet-like mark ("•", "-", "☑",
/// "✔", "➤"…) and a space. Returns the rest of the line.
fn list_item(line: &str) -> Option<&str> {
    let mut chars = line.char_indices();
    let (_, first) = chars.next()?;
    let marker = matches!(first, '-' | '*' | '+')
        || (!first.is_ascii()
            && !first.is_alphanumeric()
            && !matches!(
                first,
                '\u{201C}' | '\u{2018}' | '\u{00AB}' | '\u{300C}' | '\u{300E}' | '\u{FF08}' | '\u{2026}'
            ));
    if !marker {
        return None;
    }
    // a mark may be drawn as an emoji (a variation selector after it); a run of marks ("⚠︎⚠︎ Note") is not a bullet
    let mut rest = &line[first.len_utf8()..];
    rest = rest.trim_start_matches(['\u{FE0E}', '\u{FE0F}']);
    let item = rest.strip_prefix(char::is_whitespace)?.trim();
    (!item.is_empty()).then_some(item)
}

/// Whether a paragraph of one line is a heading: short, not a sentence (no full stop at its end), and written like a
/// title, every word capitalised ("Included Mods", "SETUP & INSTALLATION", "Features:").
fn is_heading(line: &str) -> bool {
    const SMALL: [&str; 12] = ["a", "an", "and", "the", "of", "for", "to", "in", "on", "with", "&", "or"];
    let line = line.trim_end_matches(':');
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.is_empty() || words.len() > 6 || line.chars().count() > 48 || line.ends_with(['.', ',', ';', '!', '?']) {
        return false;
    }
    if line.contains("://") {
        return false;
    }
    words.iter().enumerate().all(|(i, w)| {
        let first = w.chars().next().unwrap_or(' ');
        first.is_uppercase() || first.is_ascii_digit() || (i > 0 && SMALL.contains(&w.to_lowercase().as_str()))
    })
}

/// A pack's description (already [`plain_text`]) as headings, paragraphs and list items: blank lines end a
/// paragraph, a line with a bullet-like mark is an item, and a paragraph of one title-like line before more text is a
/// heading.
fn description_blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let flush = |para: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Paragraph(para.join("\n")));
            para.clear();
        }
    };
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            flush(&mut para, &mut out);
        } else if let Some(item) = list_item(line) {
            flush(&mut para, &mut out);
            out.push(Block::Item(item.to_string()));
        } else {
            para.push(line);
        }
    }
    flush(&mut para, &mut out);
    // a one-line paragraph that reads as a title is a heading, when text or a list comes after it (not another
    // heading: two title-like lines in a row are the end of one thing and the name of the next)
    for i in (0..out.len().saturating_sub(1)).rev() {
        let next_is_heading = matches!(out[i + 1], Block::Heading(_));
        if let Block::Paragraph(p) = &out[i] {
            if !next_is_heading && !p.contains('\n') && is_heading(p) {
                out[i] = Block::Heading(p.trim_end_matches(':').to_string());
            }
        }
    }
    out
}

/// A pack's description laid out to be read: headings in the section size, paragraphs with room between them and
/// between their lines, list items behind a drawn bullet.
fn description_ui(ui: &mut Ui, text: &str) {
    let blocks = description_blocks(text);
    let body = |text: &str, width: f32, color: egui::Color32| {
        let mut job = egui::text::LayoutJob::default();
        job.append(
            text,
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::proportional(FS_BODY),
                color,
                line_height: Some(20.0),
                ..Default::default()
            },
        );
        job.wrap.max_width = width;
        job
    };
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (i, b) in blocks.iter().enumerate() {
            let after_item = i > 0 && matches!(blocks[i - 1], Block::Item(_));
            match b {
                Block::Heading(t) => {
                    if i > 0 {
                        ui.add_space(18.0);
                    }
                    ui.add(egui::Label::new(RichText::new(t).size(FS_SECTION).semibold().color(c_text())).wrap());
                    ui.add_space(8.0);
                }
                Block::Paragraph(t) => {
                    if i > 0 && !matches!(blocks[i - 1], Block::Heading(_)) {
                        ui.add_space(12.0);
                    }
                    let job = body(t, ui.available_width(), c_muted());
                    ui.add(egui::Label::new(job));
                }
                Block::Item(t) => {
                    // items of one list sit close together; a list starts with the room of a paragraph
                    if i > 0 {
                        ui.add_space(if after_item { 4.0 } else { 10.0 });
                    }
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let (dot, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                        ui.painter().circle_filled(egui::pos2(dot.left() + 7.0, dot.center().y), 2.5, c_faint());
                        let job = body(t, ui.available_width(), c_muted());
                        ui.add(egui::Label::new(job));
                    });
                }
            }
        }
    });
}

#[cfg(test)]
mod description_tests {
    use super::*;

    #[test]
    fn a_description_falls_into_headings_paragraphs_and_items() {
        let text = "Welcome to the pack.\nIt has a lot.\n\nIncluded Mods\n\n\u{2611} Create\n\n\u{2611} Mekanism\n\nAnd so much more.\n\nSETUP & INSTALLATION\n\nForge works.";
        assert_eq!(
            description_blocks(text),
            [
                Block::Paragraph("Welcome to the pack.\nIt has a lot.".into()),
                Block::Heading("Included Mods".into()),
                Block::Item("Create".into()),
                Block::Item("Mekanism".into()),
                Block::Paragraph("And so much more.".into()),
                Block::Heading("SETUP & INSTALLATION".into()),
                Block::Paragraph("Forge works.".into()),
            ]
        );
    }

    #[test]
    fn list_marks_are_found_and_other_first_characters_are_not() {
        for line in
            ["- Create", "* Create", "\u{2022} Create", "\u{2714}\u{FE0F} Create", "\u{27A4} Create", "\u{2611} Create"]
        {
            assert_eq!(list_item(line), Some("Create"), "{line}");
        }
        // a warning made of marks, a quote, a word, a mark with nothing after it
        for line in
            ["\u{26A0}\u{FE0E}\u{26A0}\u{FE0E} Note", "\u{201C}Quoted\u{201D} text", "Create", "-", "-5 degrees"]
        {
            assert_eq!(list_item(line), None, "{line}");
        }
    }

    #[test]
    fn headings_are_short_title_lines_and_never_the_last_block() {
        assert!(is_heading("Included Mods"));
        assert!(is_heading("Features:"));
        assert!(is_heading("Lots of Building Blocks"));
        assert!(!is_heading("And so much more."));
        assert!(!is_heading("this is a sentence without a stop"));
        assert!(!is_heading("https://example.com/Pack"));
        // a last line on its own is the end of the text, not the name of what follows
        assert_eq!(
            description_blocks("Some text.\n\nThe End"),
            [Block::Paragraph("Some text.".into()), Block::Paragraph("The End".into())]
        );
    }

    #[test]
    fn html_lists_and_paragraphs_become_lines() {
        let text = plain_text("<p>Hello</p><h3>Mods</h3><ul><li>Create</li><li>Mekanism</li></ul>");
        assert_eq!(
            description_blocks(&text),
            [
                Block::Paragraph("Hello".into()),
                Block::Heading("Mods".into()),
                Block::Item("Create".into()),
                Block::Item("Mekanism".into()),
            ]
        );
    }
}
