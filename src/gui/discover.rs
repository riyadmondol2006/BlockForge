//! Discover: search Modrinth and install into an instance.

use std::collections::HashSet;

use egui::{Align, CornerRadius, Layout, RichText, ScrollArea, Stroke, Ui};

use super::theme::*;
use super::widgets::*;
use super::{App, Msg};
use crate::loaders::Loader;
use crate::modrinth::{self, Hit, Kind, Sort};

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
        }
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

impl App {
    /// Start a search. `reset` begins again from the first result; otherwise the next page is added.
    pub(super) fn discover_search(&mut self, reset: bool) {
        let d = &mut self.discover;
        if reset {
            d.hits.clear();
        }
        d.offset_pending = d.hits.len() as u32;
        d.seq += 1;
        d.loading = true;
        let (seq, query, kind, sort, offset) = (d.seq, d.query.clone(), d.kind, d.sort, d.offset_pending);
        let (mc, loader) = match (&d.target, kind) {
            (Some(folder), Kind::Mod | Kind::ResourcePack | Kind::Shader) => {
                let inst = self.instances.iter().find(|i| &i.folder == folder);
                (
                    inst.map(|i| i.mc_version().to_string()),
                    inst.and_then(|i| i.loader()).and_then(|(k, _)| Loader::parse(k)).unwrap_or(Loader::Vanilla),
                )
            }
            _ => (None, Loader::Vanilla),
        };
        let loader = if kind == Kind::Mod { loader } else { Loader::Vanilla };
        let n = self.notifier();
        self.rt.spawn(async move {
            let r = modrinth::search(&query, kind, mc.as_deref(), loader, sort, offset, PAGE)
                .await
                .map_err(|e| format!("{e:#}"));
            n.send(Msg::Search { seq, result: r });
        });
    }

    fn install_hit(&mut self, hit: &Hit, folder: Option<String>) {
        let id = hit.project_id.clone();
        let title = hit.title.clone();
        let kind = self.discover.kind;
        self.discover.installing.insert(id.clone());
        if kind == Kind::Modpack {
            let dirs = self.dirs.clone();
            self.start_task(format!("Installing {title}"), move |p, _| async move {
                let inst = modrinth::install_modpack(&dirs, &id, &p).await?;
                Ok(format!("Created {}", inst.name()))
            });
            return;
        }
        let Some(inst) = folder.and_then(|f| self.instance(&f).cloned()) else { return };
        self.start_task(format!("Installing {title}"), move |p, _| async move {
            let added = modrinth::install_into(&inst, &id, kind, &p).await?;
            Ok(format!("Added {} to {}", added.join(", "), inst.name()))
        });
        self.mods_cache = None;
    }

    pub(super) fn discover_ui(&mut self, ui: &mut Ui) {
        // the CurseForge page, when that platform is chosen
        if self.cf_discover_ui(ui) {
            return;
        }
        let mut search = false;
        ui.horizontal(|ui| {
            heading(ui, "Discover");
            muted(ui, "from Modrinth");
            ui.add_space(8.0);
            self.cf_platform_chips(ui);
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for k in Kind::ALL {
                if chip(ui, k.label(), self.discover.kind == k).clicked() && self.discover.kind != k {
                    self.discover.kind = k;
                    search = true;
                }
            }
        });
        ui.horizontal(|ui| {
            let mut q = std::mem::take(&mut self.discover.query);
            let r = search_field(ui, &mut q, &format!("Search {}", self.discover.kind.label().to_lowercase()), 340.0);
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
        if self.discover.kind != Kind::Modpack {
            ui.horizontal(|ui| {
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

        if let Some(e) = &self.discover.error {
            card(ui, |ui| {
                ui.label(RichText::new("Couldn't reach Modrinth").color(c_danger()).strong());
                muted(ui, e.clone());
            });
        }
        if self.discover.hits.is_empty() && !self.discover.loading && self.discover.error.is_none() {
            card(ui, |ui| muted(ui, "No results. Try a different search."));
            return;
        }

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
                                ui.set_max_width((ui.available_width() - 150.0).max(240.0));
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(&hit.title).strong().size(16.0));
                                    small_muted(ui, format!("by {}", hit.author));
                                });
                                ui.add(egui::Label::new(RichText::new(&hit.description).color(c_muted())).truncate());
                                ui.horizontal(|ui| {
                                    small_muted(ui, format!("{} downloads", human_count(hit.downloads)));
                                    for c in hit.categories.iter().take(4) {
                                        tag(ui, c, c_muted());
                                    }
                                });
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let busy = self.discover.installing.contains(&hit.project_id)
                                    && self.tasks.iter().any(|t| t.label.ends_with(&hit.title));
                                if busy {
                                    ui.spinner();
                                } else if kind == Kind::Modpack {
                                    if primary(ui, "Install").clicked() {
                                        install = Some((hit.clone(), None));
                                    }
                                } else if let Some(t) = &target {
                                    if primary(ui, "Install").clicked() {
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
        if let Some((hit, folder)) = install {
            self.install_hit(&hit, folder);
        }
        if more {
            self.discover_search(false);
        }
    }
}
