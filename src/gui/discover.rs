//! Discover: search Modrinth and install into an instance.

use std::collections::HashSet;
use std::time::SystemTime;

use egui::{Align, CornerRadius, Layout, RichText, ScrollArea, Stroke, Ui};

use super::theme::*;
use super::widgets::*;
use super::{App, Msg, Page};
use crate::modrinth::{self, Hit, Kind, Sort};
use crate::mods::metadata::ContentKind;

pub struct DiscoverState {
    pub kind: Kind,
    pub query: String,
    pub sort: Sort,
    /// The instance mods are installed into (folder name).
    pub target: Option<String>,
    pub hits: Vec<Hit>,
    pub total: u64,
    pub loading: bool,
    pub seq: u64,
    pub error: Option<String>,
    pub installing: HashSet<String>,
    offset_pending: u32,
    /// A search has been started since the page first opened.
    searched: bool,
    /// The Modrinth projects in the target instance, with the instance file's time they were read at.
    installed: Option<(String, Option<SystemTime>, HashSet<String>)>,
}

impl Default for DiscoverState {
    fn default() -> Self {
        DiscoverState {
            kind: Kind::Mod,
            query: String::new(),
            sort: Sort::Downloads,
            target: None,
            hits: Vec::new(),
            total: 0,
            loading: false,
            seq: 0,
            error: None,
            installing: HashSet::new(),
            offset_pending: 0,
            searched: false,
            installed: None,
        }
    }
}

/// The Modrinth kind as the mod manager files it.
fn content_kind(kind: Kind) -> Option<ContentKind> {
    match kind {
        Kind::Mod => Some(ContentKind::Mods),
        Kind::ResourcePack => Some(ContentKind::ResourcePacks),
        Kind::Shader => Some(ContentKind::Shaders),
        Kind::Modpack => None,
    }
}

impl DiscoverState {
    pub fn arrived(&mut self, seq: u64, result: Result<(Vec<Hit>, u64), String>) {
        if seq != self.seq {
            return;
        }
        self.loading = false;
        match result {
            Ok((hits, total)) => {
                if self.offset_pending == 0 {
                    self.hits = hits;
                } else {
                    self.hits.extend(hits);
                }
                self.total = total;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
}

const PAGE: u32 = 20;

/// A card for a list that couldn't be fetched: what failed and what to do in plain words, the technical details
/// small underneath, and a button to try again. Returns whether "Try again" was pressed.
pub(super) fn fetch_error_card(ui: &mut Ui, title: &str, detail: &str) -> bool {
    let mut retry = false;
    card(ui, |ui| {
        ui.label(RichText::new(title).color(c_danger()).strong());
        muted(ui, "Check your internet connection, or try again in a few minutes.");
        ui.add(egui::Label::new(RichText::new(detail).color(c_muted()).size(11.5)).wrap());
        ui.add_space(4.0);
        retry = secondary(ui, "Try again").clicked();
    });
    retry
}

impl App {
    /// Start a search. `reset` begins again from the first result; otherwise the next page is added.
    pub(super) fn discover_search(&mut self, reset: bool) {
        // what the target instance runs: its Minecraft version, and every loader whose mods it can use
        let kind = self.discover.kind;
        let (mc, loaders) = match (self.discover.target.as_ref().and_then(|f| self.instance(f)), kind) {
            (Some(inst), Kind::Mod | Kind::ResourcePack | Kind::Shader) => {
                let loaders = if kind == Kind::Mod {
                    crate::mods::InstanceTarget::of(inst, &crate::mods::RemoteConfig::load(&self.dirs))
                        .accepted_loaders()
                } else {
                    Vec::new()
                };
                (Some(inst.mc_version().to_string()), loaders)
            }
            _ => (None, Vec::new()),
        };
        let d = &mut self.discover;
        if reset {
            d.hits.clear();
            d.total = 0;
        }
        d.searched = true;
        d.offset_pending = d.hits.len() as u32;
        d.seq += 1;
        d.loading = true;
        let (seq, query, sort, offset) = (d.seq, d.query.clone(), d.sort, d.offset_pending);
        let n = self.notifier();
        self.rt.spawn(async move {
            let r = modrinth::search_for_loaders(&query, kind, mc.as_deref(), &loaders, sort, offset, PAGE)
                .await
                .map_err(|e| format!("{e:#}"));
            n.send(Msg::Search { seq, result: r });
        });
    }

    fn install_hit(&mut self, hit: &Hit, folder: Option<String>) {
        let id = hit.project_id.clone();
        let title = hit.title.clone();
        let kind = self.discover.kind;
        let Some(content) = content_kind(kind) else {
            self.discover.installing.insert(id.clone());
            let dirs = self.dirs.clone();
            self.start_task(format!("Installing {title}"), move |p, _| async move {
                let inst = modrinth::install_modpack(&dirs, &id, &p).await?;
                Ok(format!("Created {}", inst.name()))
            });
            return;
        };
        let Some(root) = folder.as_deref().and_then(|f| self.instance(f)).map(|i| i.root.clone()) else {
            self.toast_err("That instance isn't there any more. Choose another one.");
            return;
        };
        self.discover.installing.insert(id.clone());
        // the mod manager's install: it records the file in the instance, adds what it needs, and says when what
        // it needs clashes with something already installed
        let (dirs, settings, lock) = (self.dirs.clone(), self.settings.clone(), super::mods::instance_lock(&root));
        self.start_task(format!("Installing {title}"), move |p, n| async move {
            let _turn = lock.lock().await;
            let mut inst = crate::instances::load_one(&root)
                .ok_or_else(|| anyhow::anyhow!("that instance can't be read any more"))?;
            let ctx = crate::mods::ModContext::build(&dirs, &inst, &settings).await;
            let r = crate::mods::install::install_project(&mut inst, &ctx, &id, content, &p).await?;
            if !r.warnings.is_empty() {
                n.send(Msg::Toast(format!("{title}: {}", r.warnings.join(" ")), true));
            }
            Ok(format!("Added {} to {}", r.installed.join(", "), inst.name()))
        });
        self.mods_cache = None;
    }

    /// Discover lists Modrinth's and CurseForge's modpacks; the other sources have pages of their own.
    pub(super) fn other_modpack_sources(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            small_muted(ui, "More modpacks:");
            if ui.link("ATLauncher and Technic").clicked() {
                self.page = Page::Packs;
            }
            small_muted(ui, "and");
            if ui.link("Feed The Beast").clicked() {
                self.page = Page::Ftb;
            }
        });
    }

    /// Keeps the list of Modrinth projects in the target instance current, reading it again only when the
    /// instance's file changes.
    fn discover_refresh_installed(&mut self) {
        let Some(inst) = self.discover.target.as_ref().and_then(|f| self.instance(f)) else {
            self.discover.installed = None;
            return;
        };
        let stamp = std::fs::metadata(inst.instance_file()).and_then(|m| m.modified()).ok();
        let fresh = matches!(&self.discover.installed, Some((folder, at, _)) if *folder == inst.folder && *at == stamp);
        if !fresh {
            let ids = crate::mods::files::installed_project_ids(inst);
            self.discover.installed = Some((inst.folder.clone(), stamp, ids));
        }
    }

    pub(super) fn discover_ui(&mut self, ui: &mut Ui) {
        // an instance that was deleted or renamed can't be the target any more
        if self.discover.target.as_ref().is_some_and(|f| self.instance(f).is_none()) {
            self.discover.target = None;
            self.discover.searched = false;
        }
        // the CurseForge page, when that platform is chosen
        if self.cf_discover_ui(ui) {
            return;
        }
        // the first visit shows the most downloaded right away
        let mut search = !self.discover.searched;
        ui.horizontal_wrapped(|ui| {
            heading(ui, "Discover");
            muted(ui, "from Modrinth");
            ui.add_space(8.0);
            self.cf_platform_chips(ui);
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for k in Kind::ALL {
                if chip(ui, k.label(), self.discover.kind == k).clicked() && self.discover.kind != k {
                    self.discover.kind = k;
                    search = true;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            let mut q = std::mem::take(&mut self.discover.query);
            let r = search_field(
                ui,
                &mut q,
                &format!("Search {}", self.discover.kind.label().to_lowercase()),
                340.0f32.min(ui.available_width()),
            );
            self.discover.query = q;
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                search = true;
            }
            if primary(ui, "Search").clicked() {
                search = true;
            }
            egui::ComboBox::from_id_salt("sort")
                .selected_text(match self.discover.sort {
                    Sort::Relevance => "Most relevant",
                    Sort::Downloads => "Most downloaded",
                    Sort::Updated => "Recently updated",
                })
                .show_ui(ui, |ui| {
                    for (s, l) in [
                        (Sort::Relevance, "Most relevant"),
                        (Sort::Downloads, "Most downloaded"),
                        (Sort::Updated, "Recently updated"),
                    ] {
                        if ui.selectable_value(&mut self.discover.sort, s, l).clicked() {
                            search = true;
                        }
                    }
                });
        });
        if self.discover.kind == Kind::Modpack {
            self.other_modpack_sources(ui);
        } else {
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
                egui::ComboBox::from_id_salt("target").selected_text(current).width(240.0).show_ui(ui, |ui| {
                    if ui.selectable_label(self.discover.target.is_none(), "Any (show everything)").clicked() {
                        self.discover.target = None;
                        search = true;
                    }
                    for (folder, name) in &names {
                        if ui.selectable_label(self.discover.target.as_ref() == Some(folder), name).clicked() {
                            self.discover.target = Some(folder.clone());
                            search = true;
                        }
                    }
                });
                if let Some(inst) = self.discover.target.as_ref().and_then(|f| self.instance(f)) {
                    small_muted(
                        ui,
                        format!(
                            "Showing only what works with Minecraft {}{}",
                            inst.mc_version(),
                            inst.loader().map(|(k, _)| format!(" and {k}")).unwrap_or_default()
                        ),
                    );
                }
            });
        }
        if search {
            self.discover_search(true);
        }
        ui.add_space(6.0);

        if let Some(e) = self.discover.error.clone() {
            if fetch_error_card(ui, "Couldn't reach Modrinth", &e) && !self.discover.loading {
                // the first page again, or the page that failed to load
                self.discover_search(self.discover.hits.is_empty());
            }
        }
        if self.discover.hits.is_empty() && !self.discover.loading && self.discover.error.is_none() {
            card(ui, |ui| muted(ui, "No results. Try a different search."));
            return;
        }

        self.discover_refresh_installed();
        let installed = self.discover.installed.take();
        let hits = std::mem::take(&mut self.discover.hits);
        let kind = self.discover.kind;
        let target = self.discover.target.clone();
        let mut install: Option<(Hit, Option<String>)> = None;
        let mut more = false;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for hit in &hits {
                egui::Frame::new()
                    .fill(c_card())
                    .stroke(Stroke::new(1.0, c_border()))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(egui::Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            match &hit.icon_url {
                                Some(url) if !url.is_empty() => self.remote_image(ui, url, 60.0, &hit.title),
                                _ => {
                                    avatar(ui, &hit.title, 60.0);
                                }
                            }
                            ui.vertical(|ui| {
                                ui.set_max_width((ui.available_width() - 150.0).max(120.0));
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new(&hit.title).strong().size(16.0));
                                    small_muted(ui, format!("by {}", hit.author));
                                });
                                ui.add(egui::Label::new(RichText::new(&hit.description).color(c_muted())).truncate());
                                let is_installed = kind != Kind::Modpack
                                    && installed.as_ref().is_some_and(|(_, _, ids)| ids.contains(&hit.project_id));
                                ui.horizontal_wrapped(|ui| {
                                    small_muted(ui, format!("{} downloads", human_count(hit.downloads)));
                                    for c in hit.categories.iter().take(4) {
                                        tag(ui, c, c_muted());
                                    }
                                    if is_installed {
                                        tag(ui, "Installed", c_accent());
                                    }
                                });
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let busy = self.discover.installing.contains(&hit.project_id)
                                    && self.tasks.iter().any(|t| t.label == format!("Installing {}", hit.title));
                                let is_installed = kind != Kind::Modpack
                                    && installed.as_ref().is_some_and(|(_, _, ids)| ids.contains(&hit.project_id));
                                if busy {
                                    ui.spinner();
                                } else if kind == Kind::Modpack {
                                    if primary(ui, "Install").clicked() {
                                        install = Some((hit.clone(), None));
                                    }
                                } else if let Some(t) = &target {
                                    // installing again puts the newest fitting version in place of the old one
                                    let clicked = if is_installed {
                                        secondary(ui, "Reinstall")
                                            .on_hover_text("Puts the newest version that fits in place of this one")
                                            .clicked()
                                    } else {
                                        primary(ui, "Install").clicked()
                                    };
                                    if clicked {
                                        install = Some((hit.clone(), Some(t.clone())));
                                    }
                                } else {
                                    ui.menu_button("Install", |ui| {
                                        if self.instances.is_empty() {
                                            ui.label("Create an instance first");
                                        }
                                        for inst in &self.instances {
                                            if ui.button(inst.name()).clicked() {
                                                install = Some((hit.clone(), Some(inst.folder.clone())));
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
            if (hits.len() as u64) < self.discover.total {
                ui.horizontal(|ui| {
                    if self.discover.loading {
                        ui.spinner();
                    } else if secondary(ui, "Load more").clicked() {
                        more = true;
                    }
                });
            } else if self.discover.loading {
                ui.spinner();
            }
        });
        self.discover.hits = hits;
        if self.discover.installed.is_none() {
            self.discover.installed = installed;
        }
        if let Some((hit, folder)) = install {
            self.install_hit(&hit, folder);
        }
        if more {
            self.discover_search(false);
        }
    }
}
