//! The FTB page: browse and search Feed The Beast's public modpacks, read about one, pick a version and install
//! it, and update the FTB instances that have a newer version.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use egui::{RichText, ScrollArea, Ui};

use super::App;
use super::discover::*;
use super::theme::*;
use super::widgets::*;
use crate::cf_seam::MinimalCurseForge;
use crate::ftb::{self, ManualDownload, PackManifest, PackVersion, Sort, TargetType};

enum Event {
    Ids {
        seq: u64,
        result: Result<Vec<i64>, String>,
    },
    Page {
        seq: u64,
        asked: usize,
        packs: Vec<PackManifest>,
    },
    Target {
        version: i64,
        target: Target,
    },
    Updates(HashMap<String, PackVersion>),
    Manual {
        pack: i64,
        version: i64,
        name: Option<String>,
        files: Vec<ManualDownload>,
    },
    /// An update of an instance ended; `done` is false when it failed (the offer stays).
    Updated {
        folder: String,
        done: bool,
    },
}

/// What a version of a pack needs, as its details window shows it. Install waits for it: it says which Minecraft,
/// loader and Java the instance gets.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Target {
    /// Being read from FTB.
    Loading,
    Ready {
        /// "1.20.1"
        minecraft: String,
        /// "Fabric 0.14.21", when it has a loader.
        loader: Option<String>,
        /// "Java 17", when the version names one.
        java: Option<String>,
        mods: usize,
        /// The memory it recommends, in MB (0: it doesn't say).
        memory: i64,
    },
    /// It couldn't be read: why.
    Failed(String),
}

impl Target {
    pub(super) fn of(m: &ftb::VersionManifest) -> Target {
        let minecraft = m.target(TargetType::Game).map(|t| t.version.clone()).unwrap_or_default();
        let loader = m.target(TargetType::Modloader).map(|t| format!("{} {}", loader_display(&t.name), t.version));
        // Java is a target too, of a type the manifest's reader doesn't name
        let java = m.targets.iter().find(|t| t.name.eq_ignore_ascii_case("java")).map(|t| java_label(&t.version));
        let mods = m.files.iter().filter(|f| f.is_mod()).count();
        Target::Ready { minecraft, loader, java, mods, memory: m.specs.as_ref().map(|s| s.recommended).unwrap_or(0) }
    }
}

/// A Java version as a player names it: "17.0.2+8" is Java 17, "1.8.0_312" Java 8.
pub(super) fn java_label(version: &str) -> String {
    let mut parts = version.split(['.', '_', '+', '-']);
    let first = parts.next().unwrap_or_default();
    let major = if first == "1" { parts.next().unwrap_or(first) } else { first };
    format!("Java {major}")
}

/// The pack whose details are open.
struct Selected {
    pack: PackManifest,
    version: i64,
    name: String,
}

pub struct FtbState {
    pub query: String,
    pub sort: Sort,
    ids: Vec<i64>,
    packs: Vec<PackManifest>,
    page: usize,
    loading: bool,
    loaded_once: bool,
    seq: u64,
    error: Option<String>,
    selected: Option<Selected>,
    /// When the search field was last typed in and its search has not started yet.
    edited: Option<Instant>,
    /// The window that lists the packs with updates is open.
    review_open: bool,
    /// version id -> what it needs (a version that couldn't be read is asked again when picked again)
    targets: HashMap<i64, Target>,
    /// instance folder -> the newer version
    pub updates: HashMap<String, PackVersion>,
    /// The instances being updated right now.
    updating: HashSet<String>,
    /// Counts the result lists that replaced the one before, so a new search starts at the top of the list.
    list_gen: u64,
    updates_checked: bool,
    manual: Option<(i64, i64, Option<String>, Vec<ManualDownload>)>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Default for FtbState {
    fn default() -> Self {
        let (tx, rx) = channel();
        FtbState {
            query: String::new(),
            sort: Sort::Installs,
            ids: Vec::new(),
            packs: Vec::new(),
            page: 0,
            loading: false,
            loaded_once: false,
            seq: 0,
            error: None,
            selected: None,
            edited: None,
            review_open: false,
            targets: HashMap::new(),
            updates: HashMap::new(),
            updating: HashSet::new(),
            list_gen: 0,
            updates_checked: false,
            manual: None,
            tx,
            rx,
        }
    }
}

impl App {
    /// Start a new search / listing from the first page.
    fn ftb_search(&mut self) {
        let f = &mut self.ftb;
        f.seq += 1;
        f.loading = true;
        f.loaded_once = true;
        // the old rows stay until the first page of the new listing arrives
        f.page = 0;
        f.error = None;
        let (seq, query, sort, dirs, tx, ctx) =
            (f.seq, f.query.clone(), f.sort, self.dirs.clone(), f.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let result = if query.trim().is_empty() {
                ftb::list_ids(&dirs, sort).await
            } else {
                ftb::search_ids(&dirs, &query).await
            };
            let _ = tx.send(Event::Ids { seq, result: result.map_err(|e| format!("{e:#}")) });
            ctx.request_repaint();
        });
    }

    /// Load the next page of packs.
    fn ftb_more(&mut self) {
        let f = &mut self.ftb;
        f.page += 1;
        f.loading = true;
        f.error = None;
        // how many packs the page asks for: none of them arriving means FTB couldn't be reached
        let asked = f.ids.len().saturating_sub((f.page - 1) * ftb::PAGE_SIZE).min(ftb::PAGE_SIZE);
        let (seq, page, ids, dirs, tx, ctx) =
            (f.seq, f.page, f.ids.clone(), self.dirs.clone(), f.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let packs = ftb::page(&dirs, &ids, page).await;
            let _ = tx.send(Event::Page { seq, asked, packs });
            ctx.request_repaint();
        });
    }

    /// Look for newer versions of the FTB instances (call at start-up, and when the page opens).
    pub fn ftb_check_updates(&mut self) {
        self.ftb.updates_checked = true;
        let all: Vec<_> = self.instances.iter().filter(|i| ftb::is_ftb_instance(i)).cloned().collect();
        if all.is_empty() {
            return;
        }
        let (dirs, tx, ctx) = (self.dirs.clone(), self.ftb.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let found = ftb::check_updates(&dirs, &all).await;
            let _ = tx.send(Event::Updates(found));
            ctx.request_repaint();
        });
    }

    fn ftb_select(&mut self, pack: PackManifest) {
        let version = pack
            .newest_release()
            .or_else(|| pack.versions_newest_first().into_iter().next())
            .map(|v| v.id)
            .unwrap_or(0);
        let name = pack.name.clone();
        self.ftb.selected = Some(Selected { pack, version, name });
        self.ftb_load_target(version);
    }

    /// Find out which Minecraft and loader a version needs, for the details card.
    fn ftb_load_target(&mut self, version: i64) {
        let Some(sel) = self.ftb.selected.as_ref() else { return };
        if version == 0 || self.ftb.targets.get(&version).is_some_and(|t| !matches!(t, Target::Failed(_))) {
            return;
        }
        let (pack_id, dirs, tx, ctx) = (sel.pack.id, self.dirs.clone(), self.ftb.tx.clone(), self.ctx.clone());
        self.ftb.targets.insert(version, Target::Loading);
        self.rt.spawn(async move {
            let target = match ftb::version_manifest(&dirs, pack_id, version).await {
                Ok(m) => Target::of(&m),
                Err(e) => Target::Failed(format!("{e:#}")),
            };
            let _ = tx.send(Event::Target { version, target });
            ctx.request_repaint();
        });
    }

    /// For the screenshot tour: the list still on its way (`loading`), or the open pack's version being read, read,
    /// or failed to read (`version-loading`, `version-ready`, `version-error`).
    pub(super) fn shot_ftb_state(&mut self, state: &str) {
        let f = &mut self.ftb;
        let version = f.selected.as_ref().map_or(0, |s| s.version);
        let target = match state {
            "loading" => {
                // an answer already on its way is for an older listing
                f.seq += 1;
                (f.loading, f.loaded_once, f.error) = (true, true, None);
                f.packs.clear();
                f.ids.clear();
                return;
            }
            "version-loading" => Target::Loading,
            "version-ready" => Target::Ready {
                minecraft: "1.21.1".into(),
                loader: Some("NeoForge 21.1.172".into()),
                java: Some("Java 21".into()),
                mods: 384,
                memory: 8192,
            },
            "version-error" => Target::Failed("FTB answered 503 Service Unavailable".into()),
            _ => return,
        };
        f.targets.insert(version, target);
    }

    /// Takes in what background work found. Called on every frame (not only while the page shows), so update
    /// checks reach the library and a failed install's hand-download list is waiting when the page opens.
    pub(super) fn poll_ftb(&mut self) {
        while let Ok(ev) = self.ftb.rx.try_recv() {
            match ev {
                Event::Ids { seq, result } if seq == self.ftb.seq => match result {
                    Ok(ids) => {
                        self.ftb.ids = ids;
                        self.ftb_more();
                    }
                    Err(e) => {
                        self.ftb.loading = false;
                        self.ftb.error = Some(e);
                        // the old rows no longer match what was asked for
                        self.ftb.packs.clear();
                        self.ftb.ids.clear();
                    }
                },
                Event::Page { seq, asked, packs } if seq == self.ftb.seq => {
                    self.ftb.loading = false;
                    if self.ftb.page == 1 {
                        // the first page of a new listing takes the place of the old rows, even when it failed:
                        // those rows were for another search
                        self.ftb.packs.clear();
                        self.ftb.list_gen += 1;
                    }
                    if packs.is_empty() && asked > 0 {
                        // every pack of the page failed to arrive: the list can't be trusted to be empty
                        self.ftb.page = self.ftb.page.saturating_sub(1);
                        self.ftb.error = Some("None of the packs on this page arrived.".into());
                    }
                    self.ftb.packs.extend(packs);
                }
                Event::Target { version, target } => {
                    self.ftb.targets.insert(version, target);
                }
                Event::Updates(u) => self.ftb.updates = u,
                Event::Manual { pack, version, name, files } => self.ftb.manual = Some((pack, version, name, files)),
                Event::Updated { folder, done } => {
                    self.ftb.updating.remove(&folder);
                    if done {
                        self.ftb.updates.remove(&folder);
                    }
                }
                _ => {}
            }
        }
    }

    /// Install a version of a pack as a new instance.
    fn ftb_install(&mut self, pack_id: i64, version: i64, name: Option<String>, title: String) {
        let dirs = self.dirs.clone();
        let opts = ftb::InstallOptions::from_settings(&self.settings, name.clone());
        let (tx, ctx) = (self.ftb.tx.clone(), self.ctx.clone());
        self.start_task(format!("Installing {title}"), move |p, _| async move {
            match ftb::install(&dirs, pack_id, version, &opts, &MinimalCurseForge, &p).await {
                Ok(inst) => Ok(format!("\"{}\" is ready", inst.name())),
                Err(e) => {
                    if let Some(m) = e.downcast_ref::<ftb::ManualDownloadsNeeded>() {
                        let _ = tx.send(Event::Manual { pack: pack_id, version, name, files: m.0.clone() });
                        ctx.request_repaint();
                    }
                    Err(e)
                }
            }
        });
    }

    /// Move an FTB instance to its pack's newest version.
    pub fn ftb_update_instance(&mut self, folder: &str) {
        let Some(inst) = self.instance(folder).cloned() else { return };
        let Some(latest) = self.ftb.updates.get(folder).cloned() else { return };
        let dirs = self.dirs.clone();
        let opts = ftb::InstallOptions::from_settings(&self.settings, None);
        if self.is_playing(folder) {
            self.toast_err(format!("Close Minecraft in {} first, then update it.", inst.name()));
            return;
        }
        if !self.ftb.updating.insert(folder.to_string()) {
            return;
        }
        let label = format!("Updating {}", inst.name());
        let (folder_s, tx, ctx) = (folder.to_string(), self.ftb.tx.clone(), self.ctx.clone());
        // one change to the instance at a time: a mod switched in the Mods tab meanwhile would be written over
        let lock = super::mods::instance_lock(&inst.root);
        self.start_task(label, move |p, _| async move {
            let _turn = lock.lock().await;
            // the offer stays until the update worked, so a failed one can be tried again
            let r = ftb::update(&dirs, &inst, latest.id, &opts, &MinimalCurseForge, &p).await;
            let _ = tx.send(Event::Updated { folder: folder_s, done: r.is_ok() });
            ctx.request_repaint();
            let updated = r?;
            Ok(format!("\"{}\" is now on {}", updated.name(), latest.label()))
        });
    }

    pub(super) fn ftb_ui(&mut self, ui: &mut Ui) {
        self.poll_ftb();
        if !self.ftb.loaded_once {
            self.ftb_search();
        }
        if !self.ftb.updates_checked {
            self.ftb_check_updates();
        }
        let mut search = false;
        // the platforms have modpacks only; another kind opens on a site that has it
        if self.explore_header(ui) {
            return;
        }
        let page_w = ui.available_width();
        search_bar(
            ui,
            "ftb",
            &mut (&mut self.ftb, &mut search),
            |ui, (f, search), width| {
                let mut q = std::mem::take(&mut f.query);
                let mut edited = f.edited.take();
                if live_search_w(ui, &mut q, "Search FTB modpacks", &mut edited, width) {
                    **search = true;
                }
                f.query = q;
                f.edited = edited;
            },
            |ui, (f, search)| {
                dropdown(ui, "ftb-sort", "Sort", f.sort.label(), false, false, |ui| {
                    for s in Sort::ALL {
                        if ui.selectable_value(&mut f.sort, s, s.label()).clicked() {
                            **search = true;
                        }
                    }
                });
            },
            |_, _| {},
        );
        if search {
            self.ftb_search();
        }
        ui.add_space(8.0);

        // updates for installed FTB packs: one line, the list is in a window
        let waiting = self.instances.iter().filter(|i| self.ftb.updates.contains_key(&i.folder)).count();
        if waiting > 0 {
            let text = if waiting == 1 {
                "1 FTB pack has an update".to_string()
            } else {
                format!("{waiting} FTB packs have updates")
            };
            if notice(ui, NoticeKind::Info, &text, Some("Review")).is_some_and(|r| r.clicked()) {
                self.ftb.review_open = true;
            }
            ui.add_space(8.0);
        }
        if waiting == 0 {
            self.ftb.review_open = false;
        }
        if self.ftb.review_open {
            self.ftb_review_dialog(ui.ctx());
        }

        self.ftb_manual_dialog(ui.ctx());

        if let Some(e) = self.ftb.error.clone() {
            let text = "Couldn't reach Feed The Beast. Check your internet connection, or try again in a few minutes.";
            if fetch_error(ui, text, &e) && !self.ftb.loading {
                if self.ftb.ids.is_empty() {
                    self.ftb_search();
                } else {
                    self.ftb_more();
                }
            }
        }

        let mut open: Option<PackManifest> = None;
        let mut more = false;
        self.ftb_details_dialog(ui.ctx());

        if !self.ftb.packs.is_empty() {
            let (count, cards) = (results_text(self.ftb.ids.len() as u64, "pack", "packs"), self.results_as_cards());
            let mut flip = None;
            context_line_with(ui, Some(count), false, |_| {}, |ui| flip = view_switch(ui, cards, page_w));
            self.set_results_view(flip);
        }
        if self.ftb.packs.is_empty() && self.ftb.loading {
            result_skeleton(ui, 4, "Loading Feed The Beast packs\u{2026}");
        } else if self.ftb.packs.is_empty() && self.ftb.error.is_none() {
            let query = self.ftb.query.trim().to_string();
            let text = if query.is_empty() {
                "No packs to show.".to_string()
            } else {
                format!("No packs match \u{201C}{query}\u{201D}.")
            };
            if empty_state(ui, &text, (!query.is_empty()).then_some("Clear search")).is_some_and(|r| r.clicked()) {
                self.ftb.query.clear();
                self.ftb_search();
            }
        }
        // a new search over rows that are already shown: a thin bar says it is on its way
        if self.ftb.loading && self.ftb.page <= 1 && !self.ftb.packs.is_empty() {
            progress_bar(ui, None, 2.0);
        } else {
            ui.add_space(2.0);
        }
        let packs = std::mem::take(&mut self.ftb.packs);
        let cards = self.results_as_cards();
        ScrollArea::vertical().id_salt(("ftb-list", self.ftb.list_gen)).auto_shrink(false).show(ui, |ui| {
            result_list(ui, cards, &packs, |ui, pack, width| {
                let view = ResultView {
                    key: format!("ftb:{}", pack.id),
                    title: &pack.name,
                    by: None,
                    summary: pack.synopsis.as_deref().unwrap_or_default(),
                    pills: Vec::new(),
                    compat: pack
                        .versions_newest_first()
                        .first()
                        .map(|v| vec![(super::packs::pack_version(&v.label()), c_muted())])
                        .unwrap_or_default(),
                    sites: Vec::new(),
                    tags: pack.tags.iter().map(|t| t.name.clone()).collect(),
                    figure: Some((human_count(pack.installs.max(0) as u64), "installs")),
                };
                result_item(ui, width, &view, |ui, slot| match slot {
                    Slot::Icon(size) => {
                        self.result_icon(ui, pack.square_art().map(|a| a.url.as_str()), &pack.name, size);
                    }
                    Slot::Actions => {
                        // the version and the name are chosen in the pack's window
                        if install_button(ui, "Install").on_hover_text("Choose the version, then install").clicked() {
                            open = Some(pack.clone());
                        }
                    }
                });
            });
            ui.add_space(8.0);
            let shown = packs.len();
            let total = self.ftb.ids.len();
            if self.ftb.loading {
                if !packs.is_empty() {
                    ui.vertical_centered(|ui| {
                        spinner(ui);
                    });
                }
            } else if total > self.ftb.page * ftb::PAGE_SIZE {
                ui.vertical_centered(|ui| {
                    small_faint(ui, format!("Showing {} of {}", thousands(shown as u64), thousands(total as u64)));
                    let next = total.saturating_sub(shown).min(ftb::PAGE_SIZE);
                    if ghost(ui, &format!("Load {next} more")).clicked() {
                        more = true;
                    }
                });
            }
            ui.add_space(8.0);
        });
        self.ftb.packs = packs;
        if let Some(p) = open {
            self.ftb_select(p);
        }
        if more {
            self.ftb_more();
        }
    }

    /// The packs with a newer version, each with its Update button.
    fn ftb_review_dialog(&mut self, ctx: &egui::Context) {
        let rows: Vec<(String, String, PackVersion)> = self
            .instances
            .iter()
            .filter_map(|i| {
                self.ftb.updates.get(&i.folder).map(|v| (i.folder.clone(), i.name().to_string(), v.clone()))
            })
            .collect();
        let mut close = false;
        let mut update: Option<String> = None;
        let (width, list_height) = super::curseforge::modal_size(ctx, 560.0, 220.0);
        let modal = egui::Modal::new(egui::Id::unique("ftb-updates")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(width);
            ui.label(RichText::new("FTB pack updates").size(FS_SECTION).semibold());
            ui.add_space(12.0);
            well(ui, |ui| {
                ScrollArea::vertical().max_height(list_height.min(340.0)).id_salt("ftb-updates-list").show(ui, |ui| {
                    for (n, (folder, name, v)) in rows.iter().enumerate() {
                        if n > 0 {
                            rule(ui);
                        }
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.set_max_width((ui.available_width() - 120.0).max(160.0));
                                ui.spacing_mut().item_spacing.y = 0.0;
                                ui.add(egui::Label::new(RichText::new(name).semibold()).truncate());
                                small_muted(ui, format!("\u{2192} {}", v.label()));
                            });
                            let id = ui.scope_id().with(("ftb-update-row", folder));
                            end_aligned(ui, id, |ui| {
                                if self.ftb.updating.contains(folder) {
                                    spinner(ui).on_hover_text("Updating");
                                    return;
                                }
                                let clicked =
                                    if rows.len() == 1 { primary(ui, "Update") } else { secondary(ui, "Update") };
                                if clicked.clicked() {
                                    update = Some(folder.clone());
                                }
                            });
                        });
                        ui.add_space(6.0);
                    }
                })
            });
            ui.add_space(12.0);
            dialog_footer(
                ui,
                "ftb-updates",
                |_| {},
                |ui| {
                    // the safe answer has the focus when the window opens, so Enter or Space never confirms by accident
                    let safe = secondary(ui, "Close");
                    focus_when_shown(ui, &safe);
                    if safe.clicked() {
                        close = true;
                    }
                },
            );
        });
        if modal.should_close() {
            close = true;
        }
        if let Some(f) = update {
            self.ftb_update_instance(&f);
        }
        if close {
            self.ftb.review_open = false;
        }
    }

    /// One pack: who made it, what it is, the version to take and the name of the instance.
    fn ftb_details_dialog(&mut self, ctx: &egui::Context) {
        if self.ftb.selected.is_none() {
            return;
        }
        let targets = self.ftb.targets.clone();
        // a second click while the first install runs would make a second instance
        let installing = self
            .ftb
            .selected
            .as_ref()
            .is_some_and(|s| self.tasks.iter().any(|t| t.label == format!("Installing {}", s.pack.name)));
        let mut close = false;
        let mut install: Option<(i64, i64, Option<String>, String)> = None;
        let mut version_change: Option<i64> = None;
        let mut retry = false;
        let (width, body_height) = super::curseforge::modal_size(ctx, 620.0, 260.0);
        let Some(mut sel) = self.ftb.selected.take() else { return };
        let modal = egui::Modal::new(egui::Id::unique("ftb-details")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(width);
            // the pack's picture beside its name and who made it, the cross at the right
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                self.result_icon(ui, sel.pack.square_art().map(|a| a.url.as_str()), &sel.pack.name, 44.0);
                let text_w = (ui.available_width() - 44.0).max(80.0);
                ui.vertical(|ui| {
                    ui.set_max_width(text_w);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(egui::Label::new(RichText::new(&sel.pack.name).size(FS_SECTION).semibold()).truncate());
                    if !sel.pack.authors.is_empty() {
                        let by = sel.pack.authors.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ");
                        ui.add(
                            egui::Label::new(RichText::new(format!("by {by}")).size(FS_META).color(c_muted()))
                                .truncate(),
                        );
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if close_button(ui).clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(14.0);
            ScrollArea::vertical().id_salt("ftb-details-body").max_height(body_height).auto_shrink([false, true]).show(
                ui,
                |ui| {
                    if let Some(s) = &sel.pack.synopsis {
                        ui.add(egui::Label::new(RichText::new(s.clone()).color(c_muted())).wrap());
                        ui.add_space(4.0);
                    }
                    // the newest version and the tags, as on the pack's row
                    if !sel.pack.tags.is_empty() || !sel.pack.versions.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(5.0, 5.0);
                            if let Some(v) = sel.pack.versions_newest_first().first() {
                                mono_tag(ui, &super::packs::pack_version(&v.label()), c_muted())
                                    .on_hover_text("The newest version");
                            }
                            for t in sel.pack.tags.iter().take(8) {
                                soft_chip(ui, &t.name, chip_hue(&t.name));
                            }
                        });
                        ui.add_space(10.0);
                    }
                    well(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            field_label(ui, "Version");
                            let current = sel
                                .pack
                                .versions
                                .iter()
                                .find(|v| v.id == sel.version)
                                .map(|v| v.label())
                                .unwrap_or_default();
                            egui::ComboBox::from_id_salt("ftb-version").selected_text(current).width(200.0).show_ui(
                                ui,
                                |ui| {
                                    for v in sel.pack.versions_newest_first().into_iter().take(60) {
                                        if ui.selectable_label(sel.version == v.id, v.label()).clicked()
                                            && sel.version != v.id
                                        {
                                            sel.version = v.id;
                                            version_change = Some(v.id);
                                        }
                                    }
                                },
                            );
                        });
                        // what the version needs, once it is read: Install waits for it
                        ui.add_space(2.0);
                        if version_needs(ui, targets.get(&sel.version)) {
                            retry = true;
                        }
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            field_label(ui, "Name");
                            ui.add_sized(
                                [(ui.available_width() - 4.0).clamp(160.0, 300.0), 34.0],
                                egui::TextEdit::singleline(&mut sel.name)
                                    .margin(egui::Margin::symmetric(12, 8))
                                    .background_color(c_field()),
                            );
                        });
                    });
                },
            );
            ui.add_space(12.0);
            let website = sel.pack.website_url();
            // a version picked this frame has nothing read yet, so it can't be installed until it has
            let ready = version_change.is_none() && matches!(targets.get(&sel.version), Some(Target::Ready { .. }));
            dialog_footer(
                ui,
                "ftb-details",
                |ui| {
                    if ghost(ui, "Website").clicked() {
                        let _ = open::that(&website);
                    }
                },
                |ui| {
                    if installing {
                        spinner(ui);
                        muted(ui, "Installing");
                    } else if !ready {
                        small_muted(ui, "Install waits for the version\u{2019}s details");
                    }
                    // the safe answer has the focus when the window opens, so Enter or Space never confirms by accident
                    let safe = secondary(ui, "Close");
                    focus_when_shown(ui, &safe);
                    if safe.clicked() {
                        close = true;
                    }
                    if primary_enabled(ui, "Install", ready && !installing).clicked() {
                        install = Some((
                            sel.pack.id,
                            sel.version,
                            Some(sel.name.trim().to_string()).filter(|n| !n.is_empty()),
                            sel.pack.name.clone(),
                        ));
                    }
                },
            );
        });
        self.ftb.selected = Some(sel);
        if modal.should_close() {
            close = true;
        }
        if let Some(v) = version_change {
            self.ftb_load_target(v);
        } else if retry {
            let v = self.ftb.selected.as_ref().map_or(0, |s| s.version);
            self.ftb_load_target(v);
        }
        if let Some((pack, version, name, title)) = install {
            self.ftb_install(pack, version, name, title);
        }
        if close {
            self.ftb.selected = None;
        }
    }

    /// Mods CurseForge only gives out through its website: open each page, then try again.
    fn ftb_manual_dialog(&mut self, ctx: &egui::Context) {
        let Some((pack, version, name, files)) = self.ftb.manual.clone() else { return };
        let mut retry = false;
        let mut dismiss = false;
        let downloads: PathBuf = crate::packutil::downloads_dir();
        let (width, list_height) = super::curseforge::modal_size(ctx, 620.0, 300.0);
        let modal = egui::Modal::new(egui::Id::unique("ftb-manual")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(width);
            ui.label(RichText::new("Some mods need to be downloaded by hand").size(FS_SECTION).semibold());
            muted(
                ui,
                "The authors of these mods don't allow other launchers to download them. Open each page, save the file to your Downloads folder, then press Try again.",
            );
            ui.add_space(12.0);
            well(ui, |ui| ScrollArea::vertical().max_height(list_height.min(320.0)).id_salt("ftb-manual-list").show(ui, |ui| {
                for (n, f) in files.iter().enumerate() {
                    if n > 0 {
                        rule(ui);
                    }
                    let present = downloads.join(&f.file_name).is_file()
                        || downloads.join(f.file_name.replace(' ', "+")).is_file();
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if present {
                            check_label(ui, "In Downloads");
                        } else {
                            small_muted(ui, "\u{2022}");
                        }
                        ui.vertical(|ui| {
                            ui.set_max_width((ui.available_width() - 130.0).max(160.0));
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.add(egui::Label::new(&f.file_name).truncate());
                            small_muted(ui, &f.mod_name);
                        });
                        let id = ui.scope_id().with(("ftb-manual-row", &f.file_name));
                        end_aligned(ui, id, |ui| {
                            if secondary(ui, "Open page").clicked() {
                                let _ = open::that(&f.url);
                            }
                        });
                    });
                    ui.add_space(6.0);
                }
            }));
            ui.add_space(12.0);
            dialog_footer(
                ui,
                "ftb-manual",
                |ui| {
                    if ghost(ui, "Open Downloads folder").clicked() {
                        let _ = open::that(&downloads);
                    }
                },
                |ui| {
                    // the safe answer has the focus when the window opens, so Enter or Space never confirms by accident
                    let safe = secondary(ui, "Cancel");
                    focus_when_shown(ui, &safe);
                    if safe.clicked() {
                        dismiss = true;
                    }
                    if primary(ui, "Try again").clicked() {
                        retry = true;
                    }
                },
            );
        });
        if modal.should_close() {
            dismiss = true;
        }
        if retry {
            self.ftb.manual = None;
            let title =
                self.ftb.selected.as_ref().map(|s| s.pack.name.clone()).unwrap_or_else(|| format!("pack {pack}"));
            self.ftb_install(pack, version, name, title);
        }
        if dismiss {
            self.ftb.manual = None;
        }
    }
}

/// What a version needs under its picker: a turning arc while it is read, its Minecraft, loader and Java on tags once
/// it is (with how many mods and how much memory), or why it couldn't be read with a way to try again. Returns true
/// when Try again was pressed.
fn version_needs(ui: &mut Ui, target: Option<&Target>) -> bool {
    match target {
        None | Some(Target::Loading) => {
            ui.horizontal(|ui| {
                spinner_sized(ui, 16.0);
                let r = small_muted_response(ui, "Loading version details\u{2026}");
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Status, true, "Loading version details"));
            });
            false
        }
        Some(Target::Ready { minecraft, loader, java, mods, memory }) => {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(5.0, 5.0);
                if !minecraft.is_empty() {
                    mono_tag(ui, &format!("Minecraft {minecraft}"), c_muted());
                }
                if let Some(l) = loader {
                    mono_tag(ui, l, tag_color(l.split(' ').next().unwrap_or_default()));
                }
                if let Some(j) = java {
                    mono_tag(ui, j, c_muted());
                }
                ui.add_space(4.0);
                let mut rest = vec![if *mods == 1 { "1 mod".to_string() } else { format!("{mods} mods") }];
                if *memory > 0 {
                    rest.push(format!("about {memory} MB of memory"));
                }
                small_muted(ui, rest.join("  \u{00B7}  "));
            });
            false
        }
        Some(Target::Failed(e)) => {
            notice(
                ui,
                NoticeKind::Error,
                &format!("Couldn\u{2019}t read this version, so it can\u{2019}t be installed yet: {e}"),
                None,
            );
            ui.add_space(2.0);
            secondary(ui, "Try again").clicked()
        }
    }
}

/// [`small_muted`] that hands back its response.
fn small_muted_response(ui: &mut Ui, text: &str) -> egui::Response {
    ui.label(RichText::new(text).size(FS_META).color(c_muted()))
}

/// The width of the labels in front of the fields of a pack's window, so the fields line up.
const LABEL_W: f32 = 56.0;

/// A label in the column in front of the fields.
fn field_label(ui: &mut Ui, text: &str) {
    ui.allocate_ui_with_layout(egui::vec2(LABEL_W, 34.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(LABEL_W);
        filter_label(ui, text);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_versions_are_named_as_players_name_them() {
        assert_eq!(java_label("17.0.2+8"), "Java 17");
        assert_eq!(java_label("1.8.0_312"), "Java 8");
        assert_eq!(java_label("21"), "Java 21");
    }

    #[test]
    fn a_versions_needs_are_read_from_its_targets() {
        let target = |name: &str, version: &str, kind| ftb::ManifestTarget {
            version: version.into(),
            id: -1,
            name: name.into(),
            kind,
            updated: 0,
        };
        let m = ftb::VersionManifest {
            targets: vec![
                target("minecraft", "1.18.2", Some(TargetType::Game)),
                target("forge", "40.1.84", Some(TargetType::Modloader)),
                target("java", "17.0.2+8", None),
            ],
            ..Default::default()
        };
        assert_eq!(
            Target::of(&m),
            Target::Ready {
                minecraft: "1.18.2".into(),
                loader: Some("Forge 40.1.84".into()),
                java: Some("Java 17".into()),
                mods: 0,
                memory: 0,
            }
        );
    }
}
