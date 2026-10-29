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
        text: String,
        ok: bool,
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
    /// version id -> "Minecraft 1.20.1, Fabric 0.14.21"
    targets: HashMap<i64, String>,
    /// Versions that couldn't be read; asked again when picked again.
    targets_failed: HashSet<i64>,
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
            targets_failed: HashSet::new(),
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
        if version == 0 || (self.ftb.targets.contains_key(&version) && !self.ftb.targets_failed.contains(&version)) {
            return;
        }
        self.ftb.targets_failed.remove(&version);
        let (pack_id, dirs, tx, ctx) = (sel.pack.id, self.dirs.clone(), self.ftb.tx.clone(), self.ctx.clone());
        self.ftb.targets.insert(version, "Reading the version".into());
        self.rt.spawn(async move {
            let (text, ok) = match ftb::version_manifest(&dirs, pack_id, version).await {
                Ok(m) => {
                    let t = |k| m.target(k).map(|t| format!("{} {}", t.name, t.version));
                    let mods = m.files.iter().filter(|f| f.is_mod()).count();
                    let text = format!(
                        "{}, {}  \u{00B7}  {} mods  \u{00B7}  needs about {} MB of memory",
                        t(TargetType::Game).unwrap_or_default().replace("minecraft", "Minecraft"),
                        t(TargetType::Modloader).unwrap_or_default(),
                        mods,
                        m.specs.map(|s| s.recommended).unwrap_or(0)
                    );
                    (text, true)
                }
                Err(e) => (format!("Couldn't read this version: {e:#}"), false),
            };
            let _ = tx.send(Event::Target { version, text, ok });
            ctx.request_repaint();
        });
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
                Event::Target { version, text, ok } => {
                    if !ok {
                        self.ftb.targets_failed.insert(version);
                    }
                    self.ftb.targets.insert(version, text);
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
        heading(ui, "FTB");
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            let mut q = std::mem::take(&mut self.ftb.query);
            let mut edited = self.ftb.edited.take();
            if live_search(ui, &mut q, "Search FTB modpacks", &mut edited) {
                search = true;
            }
            self.ftb.query = q;
            self.ftb.edited = edited;
            egui::ComboBox::from_id_salt("ftb-sort")
                .selected_text(format!("Sort: {}", self.ftb.sort.label()))
                .width(190.0)
                .show_ui(ui, |ui| {
                    for s in Sort::ALL {
                        if ui.selectable_value(&mut self.ftb.sort, s, s.label()).clicked() {
                            search = true;
                        }
                    }
                });
        });
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
            context_line(ui, Some(format!("{} packs", thousands(self.ftb.ids.len() as u64))), false, |_| {});
        }
        if self.ftb.packs.is_empty() && self.ftb.loading {
            skeleton_rows(ui, 6, ROW_H);
        } else if self.ftb.packs.is_empty() && self.ftb.error.is_none() {
            let query = self.ftb.query.trim().to_string();
            let text = if query.is_empty() {
                "No packs to show.".to_string()
            } else {
                format!("No packs match \u{201C}{query}\u{201D}.")
            };
            if empty_row(ui, &text, (!query.is_empty()).then_some("Clear search")).is_some_and(|r| r.clicked()) {
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
        ScrollArea::vertical().id_salt(("ftb-list", self.ftb.list_gen)).auto_shrink(false).show(ui, |ui| {
            for pack in &packs {
                let rkey = format!("ftb:{}", pack.id);
                row_frame(ui, |ui| {
                    let wide = ui.available_width() >= 560.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        match pack.square_art() {
                            Some(art) => self.remote_image(ui, &art.url, 48.0, &pack.name),
                            None => {
                                avatar(ui, &pack.name, 48.0);
                            }
                        }
                        let text_w = row_text_width(ui, &rkey);
                        ui.vertical(|ui| {
                            ui.set_max_width(text_w);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            name_line(ui, &pack.name, None);
                            summary_line(ui, pack.synopsis.as_deref().unwrap_or_default());
                            let mut parts = Vec::new();
                            if !wide {
                                parts.push((
                                    format!("{} installs", human_count(pack.installs.max(0) as u64)),
                                    c_faint(),
                                ));
                            }
                            parts.extend(pack.tags.iter().take(4).map(|t| (t.name.clone(), c_faint())));
                            meta_line(ui, &parts);
                        });
                        row_actions(ui, &rkey, |ui| {
                            action_cell(ui, &rkey, |ui| {
                                if secondary(ui, "Details").clicked() {
                                    open = Some(pack.clone());
                                }
                            });
                            if wide {
                                ui.add_space(8.0);
                                stat_column(ui, &human_count(pack.installs.max(0) as u64), "installs");
                            }
                        });
                    });
                });
            }
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
            ui.add_space(8.0);
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
                            let clicked = if rows.len() == 1 { primary(ui, "Update") } else { secondary(ui, "Update") };
                            if clicked.clicked() {
                                update = Some(folder.clone());
                            }
                        });
                    });
                    ui.add_space(6.0);
                }
            });
            ui.add_space(8.0);
            dialog_footer(
                ui,
                "ftb-updates",
                |_| {},
                |ui| {
                    if secondary(ui, "Close").clicked() {
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
        let (width, body_height) = super::curseforge::modal_size(ctx, 620.0, 260.0);
        let Some(sel) = self.ftb.selected.as_mut() else { return };
        let modal = egui::Modal::new(egui::Id::unique("ftb-details")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(width);
            if dialog_title(ui, &sel.pack.name) {
                close = true;
            }
            if !sel.pack.authors.is_empty() {
                small_muted(
                    ui,
                    format!("by {}", sel.pack.authors.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")),
                );
            }
            ui.add_space(8.0);
            ScrollArea::vertical().id_salt("ftb-details-body").max_height(body_height).auto_shrink([false, true]).show(
                ui,
                |ui| {
                    if let Some(s) = &sel.pack.synopsis {
                        ui.add(egui::Label::new(RichText::new(s.clone()).color(c_muted())).wrap());
                        ui.add_space(4.0);
                    }
                    let tags: Vec<(String, egui::Color32)> =
                        sel.pack.tags.iter().take(8).map(|t| (t.name.clone(), c_faint())).collect();
                    if !tags.is_empty() {
                        ui.add(egui::Label::new(tag_line(&tags)).wrap());
                        ui.add_space(8.0);
                    }
                    ui.horizontal_wrapped(|ui| {
                        filter_label(ui, "Version");
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
                    if let Some(text) = targets.get(&sel.version).filter(|t| !t.is_empty()) {
                        small_muted(ui, text.clone());
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        filter_label(ui, "Name");
                        ui.add_sized(
                            [300.0, 32.0],
                            egui::TextEdit::singleline(&mut sel.name)
                                .margin(egui::Margin::symmetric(10, 7))
                                .background_color(c_field()),
                        );
                    });
                },
            );
            ui.add_space(12.0);
            let website = sel.pack.website_url();
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
                    }
                    if secondary(ui, "Close").clicked() {
                        close = true;
                    }
                    if primary_enabled(ui, "Install", !installing).clicked() {
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
        if modal.should_close() {
            close = true;
        }
        if let Some(v) = version_change {
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
            ui.add_space(8.0);
            ScrollArea::vertical().max_height(list_height.min(320.0)).id_salt("ftb-manual-list").show(ui, |ui| {
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
            });
            ui.add_space(8.0);
            dialog_footer(
                ui,
                "ftb-manual",
                |ui| {
                    if ghost(ui, "Open Downloads folder").clicked() {
                        let _ = open::that(&downloads);
                    }
                },
                |ui| {
                    if secondary(ui, "Cancel").clicked() {
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

/// Words divided by dots, as one line of faint text (the tags of a pack).
fn tag_line(parts: &[(String, egui::Color32)]) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let format =
        |color| egui::text::TextFormat { font_id: egui::FontId::proportional(FS_META), color, ..Default::default() };
    for (i, (text, color)) in parts.iter().enumerate() {
        if i > 0 {
            job.append(" \u{00B7} ", 0.0, format(c_faint()));
        }
        job.append(text, 0.0, format(*color));
    }
    job
}
