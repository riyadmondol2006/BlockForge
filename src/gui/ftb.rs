//! The FTB page: browse and search Feed The Beast's public modpacks, read about one, pick a version and install
//! it, and update the FTB instances that have a newer version.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Align, Layout, RichText, ScrollArea, Ui};

use super::App;
use super::discover::*;
use super::theme::*;
use super::widgets::*;
use crate::cf_seam::MinimalCurseForge;
use crate::ftb::{self, ManualDownload, PackManifest, PackVersion, Sort, TargetType};

enum Event {
    Ids { seq: u64, result: Result<Vec<i64>, String> },
    Page { seq: u64, asked: usize, packs: Vec<PackManifest> },
    Target { version: i64, text: String, ok: bool },
    Updates(HashMap<String, PackVersion>),
    Manual { pack: i64, version: i64, name: Option<String>, files: Vec<ManualDownload> },
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
    /// version id -> "Minecraft 1.20.1, Fabric 0.14.21"
    targets: HashMap<i64, String>,
    /// Versions that couldn't be read; asked again when picked again.
    targets_failed: HashSet<i64>,
    /// instance folder -> the newer version
    pub updates: HashMap<String, PackVersion>,
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
            targets: HashMap::new(),
            targets_failed: HashSet::new(),
            updates: HashMap::new(),
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
        f.packs.clear();
        f.ids.clear();
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
                    }
                },
                Event::Page { seq, asked, packs } if seq == self.ftb.seq => {
                    self.ftb.loading = false;
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
        let label = format!("Updating {}", inst.name());
        let folder_s = folder.to_string();
        self.ftb.updates.remove(&folder_s);
        self.start_task(label, move |p, _| async move {
            let updated = ftb::update(&dirs, &inst, latest.id, &opts, &MinimalCurseForge, &p).await?;
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
        heading(ui, "FTB Modpacks");
        ui.add_space(8.0);
        let mut q = std::mem::take(&mut self.ftb.query);
        if big_search(ui, &mut q, "Search FTB modpacks") {
            search = true;
        }
        self.ftb.query = q;
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            filter_label(ui, "Sort by");
            egui::ComboBox::from_id_salt("ftb-sort").selected_text(self.ftb.sort.label()).width(160.0).show_ui(
                ui,
                |ui| {
                    for s in Sort::ALL {
                        if ui.selectable_value(&mut self.ftb.sort, s, s.label()).clicked() {
                            search = true;
                        }
                    }
                },
            );
        });
        if search {
            self.ftb_search();
        }
        ui.add_space(8.0);

        // updates for installed FTB packs
        let updates: Vec<(String, String, PackVersion)> = self
            .instances
            .iter()
            .filter_map(|i| {
                self.ftb.updates.get(&i.folder).map(|v| (i.folder.clone(), i.name().to_string(), v.clone()))
            })
            .collect();
        let mut update_folder: Option<String> = None;
        if !updates.is_empty() {
            card(ui, |ui| {
                ui.label(RichText::new("Updates available").size(15.0).semibold());
                ui.add_space(4.0);
                for (folder, name, v) in &updates {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(name).semibold());
                        small_muted(ui, format!("\u{2192} {}", v.label()));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if secondary(ui, "Update").clicked() {
                                update_folder = Some(folder.clone());
                            }
                        });
                    });
                }
            });
            ui.add_space(8.0);
        }
        if let Some(f) = update_folder {
            self.ftb_update_instance(&f);
        }

        self.ftb_manual_card(ui);

        if let Some(e) = self.ftb.error.clone() {
            if super::discover::fetch_error_card(ui, "Couldn't reach Feed The Beast", &e) && !self.ftb.loading {
                if self.ftb.ids.is_empty() {
                    self.ftb_search();
                } else {
                    self.ftb_more();
                }
            }
        }

        let mut open: Option<PackManifest> = None;
        let mut install: Option<(i64, i64, Option<String>, String)> = None;
        let mut more = false;
        let mut close_details = false;
        let mut version_change: Option<i64> = None;

        if self.ftb.selected.is_some() {
            let targets = self.ftb.targets.clone();
            // a second click while the first install runs would make a second instance
            let installing = self
                .ftb
                .selected
                .as_ref()
                .is_some_and(|s| self.tasks.iter().any(|t| t.label == format!("Installing {}", s.pack.name)));
            let sel = self.ftb.selected.as_mut();
            if let Some(sel) = sel {
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&sel.pack.name).size(18.0).semibold());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if secondary(ui, "Close").clicked() {
                                close_details = true;
                            }
                            if secondary(ui, "Website").clicked() {
                                let _ = open::that(sel.pack.website_url());
                            }
                        });
                    });
                    if !sel.pack.authors.is_empty() {
                        small_muted(
                            ui,
                            format!(
                                "by {}",
                                sel.pack.authors.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
                            ),
                        );
                    }
                    if let Some(s) = &sel.pack.synopsis {
                        ui.add_space(4.0);
                        ui.scope(|ui| {
                            ui.set_max_width(ui.available_width().min(720.0));
                            ui.add(egui::Label::new(RichText::new(s.clone()).color(c_muted())).wrap());
                        });
                    }
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        for t in sel.pack.tags.iter().take(8) {
                            tag(ui, &t.name, c_muted());
                        }
                    });
                    ui.add_space(8.0);
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
                        small_muted(ui, targets.get(&sel.version).cloned().unwrap_or_default());
                    });
                    ui.horizontal_wrapped(|ui| {
                        filter_label(ui, "Name");
                        ui.add_sized(
                            [300.0, 32.0],
                            egui::TextEdit::singleline(&mut sel.name)
                                .margin(egui::Margin::symmetric(10, 7))
                                .background_color(c_field()),
                        );
                        if installing {
                            ui.spinner();
                            muted(ui, "Installing");
                        } else if primary(ui, "Install").clicked() {
                            install = Some((
                                sel.pack.id,
                                sel.version,
                                Some(sel.name.trim().to_string()).filter(|n| !n.is_empty()),
                                sel.pack.name.clone(),
                            ));
                        }
                    });
                });
            }
            ui.add_space(8.0);
        }
        if let Some(v) = version_change {
            self.ftb_load_target(v);
        }
        if close_details {
            self.ftb.selected = None;
        }

        if self.ftb.packs.is_empty() && self.ftb.loading {
            skeleton_rows(ui, 6, 76.0);
        } else if self.ftb.packs.is_empty() && self.ftb.error.is_none() {
            let query = self.ftb.query.trim().to_string();
            let title = if query.is_empty() {
                "No packs to show".to_string()
            } else {
                format!("No packs match \u{201C}{query}\u{201D}")
            };
            if empty_state(ui, &title, "Try a different search.", (!query.is_empty()).then_some("Clear search")) {
                self.ftb.query.clear();
                self.ftb_search();
            }
        }
        let packs = std::mem::take(&mut self.ftb.packs);
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
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
                            ui.label(RichText::new(&pack.name).size(15.0).semibold());
                            ui.add(
                                egui::Label::new(
                                    RichText::new(pack.synopsis.clone().unwrap_or_default())
                                        .color(c_muted())
                                        .size(13.0),
                                )
                                .truncate(),
                            );
                            let tags: Vec<(&str, egui::Color32)> =
                                pack.tags.iter().take(4).map(|t| (t.name.as_str(), c_muted())).collect();
                            let figure =
                                (!wide).then(|| format!("{} installs", human_count(pack.installs.max(0) as u64)));
                            meta_line(ui, figure, &tags, false);
                        });
                        row_actions(ui, &rkey, |ui| {
                            if secondary(ui, "Details").clicked() {
                                open = Some(pack.clone());
                            }
                            if wide {
                                ui.add_space(8.0);
                                stat_column(ui, &human_count(pack.installs.max(0) as u64), "installs");
                            }
                        });
                    });
                });
            }
            ui.add_space(8.0);
            if self.ftb.loading {
                if !packs.is_empty() {
                    ui.spinner();
                }
            } else if self.ftb.ids.len() > self.ftb.page * ftb::PAGE_SIZE && secondary(ui, "Load more").clicked() {
                more = true;
            }
            ui.add_space(8.0);
        });
        self.ftb.packs = packs;
        if let Some(p) = open {
            self.ftb_select(p);
        }
        if let Some((pack, version, name, title)) = install {
            self.ftb_install(pack, version, name, title);
        }
        if more {
            self.ftb_more();
        }
    }

    /// Mods CurseForge only gives out through its website: open each page, then try again.
    fn ftb_manual_card(&mut self, ui: &mut Ui) {
        let Some((pack, version, name, files)) = self.ftb.manual.clone() else { return };
        let mut retry = false;
        let mut dismiss = false;
        card(ui, |ui| {
            ui.label(RichText::new("Some mods need to be downloaded by hand").semibold().color(c_warn()));
            small_muted(
                ui,
                "The authors of these mods don't allow other launchers to download them. Open each page, save the file to your Downloads folder, then press Try again.",
            );
            let downloads: PathBuf = crate::packutil::downloads_dir();
            for f in &files {
                let present =
                    downloads.join(&f.file_name).is_file() || downloads.join(f.file_name.replace(' ', "+")).is_file();
                ui.horizontal(|ui| {
                    ui.label(if present {
                        RichText::new("\u{2713}").color(c_accent())
                    } else {
                        RichText::new("\u{2022}").color(c_muted())
                    });
                    ui.label(&f.file_name);
                    small_muted(ui, format!("({})", f.mod_name));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if secondary(ui, "Open page").clicked() {
                            let _ = open::that(&f.url);
                        }
                    });
                });
            }
            ui.horizontal(|ui| {
                if primary(ui, "Try again").clicked() {
                    retry = true;
                }
                if secondary(ui, "Open Downloads folder").clicked() {
                    let _ = open::that(&downloads);
                }
                if secondary(ui, "Cancel").clicked() {
                    dismiss = true;
                }
            });
        });
        ui.add_space(6.0);
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
