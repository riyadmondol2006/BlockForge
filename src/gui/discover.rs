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

/// The widest a search field or a reading column gets on these pages.
pub(super) const SEARCH_W: f32 = 640.0;

/// The frame of a message that takes the place of a list: one hairline, no fill, content left-aligned.
pub(super) fn state_frame(ui: &mut Ui, stroke: egui::Color32, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .stroke(Stroke::new(1.0, stroke))
        .corner_radius(CornerRadius::same(R_CARD))
        .inner_margin(egui::Margin::same(20))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// A list that couldn't be fetched: what failed and what to do in plain words, the technical details small
/// underneath, and a button to try again. Returns whether "Try again" was pressed.
pub(super) fn fetch_error_card(ui: &mut Ui, title: &str, detail: &str) -> bool {
    let mut retry = false;
    state_frame(ui, c_danger_line(), |ui| {
        ui.label(RichText::new(title).color(c_danger()).semibold());
        muted(ui, "Check your internet connection, or try again in a few minutes.");
        ui.add(egui::Label::new(RichText::new(detail).color(c_faint()).size(12.0)).wrap());
        ui.add_space(4.0);
        retry = secondary(ui, "Try again").clicked();
    });
    ui.add_space(8.0);
    retry
}

/// An empty list, said calmly: what is missing, what to do about it, and the one button for it when there is one.
/// Returns whether that button was pressed.
pub(super) fn empty_state(ui: &mut Ui, title: &str, hint: &str, action: Option<&str>) -> bool {
    let mut pressed = false;
    state_frame(ui, c_border(), |ui| {
        ui.label(RichText::new(title).size(15.0).semibold());
        if !hint.is_empty() {
            muted(ui, hint);
        }
        if let Some(label) = action {
            ui.add_space(4.0);
            pressed = secondary(ui, label).clicked();
        }
    });
    pressed
}

/// Link-looking text in ink (not green): underlined, with the hand cursor.
pub(super) fn text_link(ui: &mut Ui, text: &str, size: f32) -> egui::Response {
    ui.add(egui::Label::new(RichText::new(text).size(size).color(c_text()).underline()).sense(egui::Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A row of tabs with a hairline under it, [`tab`] calls inside. (Like `tab_bar`, but the line sits under the tabs
/// themselves, wherever the rest of the page ends.)
pub(super) fn tab_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let (left, width) = (ui.cursor().min.x, ui.available_width());
    let out = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 22.0;
        add(ui)
    });
    ui.painter().hline(left..=left + width, out.response.rect.bottom(), Stroke::new(1.0, c_border()));
    out.inner
}

/// The grey label in front of a filter control.
pub(super) fn filter_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(13.0).color(c_muted()));
}

/// A search field `width` wide with a magnifier in it.
pub(super) fn search_input(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    let r = ui.add_sized(
        [width, 32.0],
        egui::TextEdit::singleline(text)
            .hint_text(RichText::new(hint).color(c_faint()))
            .margin(egui::Margin { left: 34, right: 10, top: 7, bottom: 7 })
            .background_color(c_field()),
    );
    super::icons::paint(
        ui.painter(),
        super::icons::Icon::Search,
        egui::pos2(r.rect.left() + 18.0, r.rect.center().y),
        15.0,
        c_faint(),
    );
    r
}

/// The search field of a page: wide, with a magnifier in it, and the Search button after it. Returns true when a
/// search was asked for (Enter or the button).
pub(super) fn big_search(ui: &mut Ui, text: &mut String, hint: &str) -> bool {
    let mut go = false;
    ui.horizontal(|ui| {
        let w = (ui.available_width().min(SEARCH_W) - 84.0).max(160.0);
        let r = search_input(ui, text, hint, w);
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            go = true;
        }
        if secondary(ui, "Search").clicked() {
            go = true;
        }
    });
    go
}

/// The colour of a tag on a result: loaders have theirs, everything else is quiet.
pub(super) fn tag_color(name: &str) -> egui::Color32 {
    match name.to_ascii_lowercase().as_str() {
        "fabric" => loader_color("Fabric"),
        "quilt" => loader_color("Quilt"),
        "forge" => loader_color("Forge"),
        "neoforge" => loader_color("NeoForge"),
        _ => c_muted(),
    }
}

fn is_loader(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "fabric" | "quilt" | "forge" | "neoforge")
}

/// The categories of a result with the loaders first, at most `max` of them.
pub(super) fn tags_for<'a>(names: impl Iterator<Item = &'a str>, max: usize) -> Vec<&'a str> {
    let mut v: Vec<&str> = names.collect();
    v.sort_by_key(|n| !is_loader(n));
    v.truncate(max);
    v
}

/// One result in a list: no box at rest, a tone step under it when pointed at, a hairline between rows.
pub(super) fn row_frame<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    row_frame_at(ui, true, add)
}

fn row_frame_at<R>(ui: &mut Ui, hover: bool, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.spacing_mut().item_spacing.y = 0.0;
    let bg = ui.painter().add(egui::Shape::Noop);
    let out = egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    });
    let rect = out.response.rect;
    if hover && ui.rect_contains_pointer(rect) {
        ui.painter().set(bg, egui::Shape::rect_filled(rect, CornerRadius::same(R_CARD), c_card()));
    } else {
        ui.painter().hline(rect.x_range().shrink(12.0), rect.bottom(), Stroke::new(1.0, c_border()));
    }
    out.inner
}

/// How wide the text of a row may be: what is left of the row after the icon and the actions (as wide as they were
/// on the last frame, found by [`row_actions`]).
pub(super) fn row_text_width(ui: &Ui, key: &str) -> f32 {
    let w = remembered_width(ui, egui::Id::unique(("row-actions", key)), 100.0);
    (ui.available_width() - w - 16.0).max(120.0)
}

/// The right-hand end of a row: buttons and figures, laid out from the right edge.
pub(super) fn row_actions(ui: &mut Ui, key: &str, add: impl FnOnce(&mut Ui)) {
    let id = egui::Id::unique(("row-actions", key));
    let used = ui
        .with_layout(Layout::right_to_left(Align::Center), |ui| {
            add(ui);
            ui.min_rect().width()
        })
        .inner;
    remember_width(ui, id, used);
}

/// A figure with its unit under it, right-aligned in a fixed column so a list of them lines up.
pub(super) fn stat_column(ui: &mut Ui, figure: &str, unit: &str) {
    ui.allocate_ui_with_layout(egui::vec2(72.0, 34.0), Layout::top_down(Align::Max), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(RichText::new(figure).size(14.0).semibold());
        ui.label(RichText::new(unit).size(12.0).color(c_faint()));
    });
}

/// The tags and, when there is no room for the column, the figure of a result.
pub(super) fn meta_line(ui: &mut Ui, figure: Option<String>, tags: &[(&str, egui::Color32)], installed: bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if let Some(f) = figure {
            ui.label(RichText::new(f).size(12.0).color(c_muted()));
        }
        for (t, c) in tags {
            tag(ui, t, *c);
        }
        if installed {
            tag(ui, "Installed", c_accent());
        }
    });
}

/// Grey placeholders in the shape of rows, while the first results are on their way.
pub(super) fn skeleton_rows(ui: &mut Ui, count: usize) {
    for _ in 0..count {
        row_frame_at(ui, false, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let (rect, _) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(11), c_field());
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(260.0f32.min(ui.available_width()), 48.0), egui::Sense::hover());
                let bar = |y: f32, w: f32, h: f32| {
                    egui::Rect::from_min_size(rect.min + egui::vec2(0.0, y), egui::vec2(w.min(rect.width()), h))
                };
                ui.painter().rect_filled(bar(4.0, 150.0, 12.0), CornerRadius::same(4), c_field());
                ui.painter().rect_filled(bar(22.0, 260.0, 10.0), CornerRadius::same(4), c_field());
                ui.painter().rect_filled(bar(38.0, 90.0, 8.0), CornerRadius::same(4), c_field());
            });
        });
    }
}

/// The count under the filters: "1.2K results", right where the list starts.
pub(super) fn count_line(ui: &mut Ui, text: String) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).size(12.0).color(c_muted()));
    ui.add_space(4.0);
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
            if text_link(ui, "ATLauncher and Technic", 12.0).clicked() {
                self.page = Page::Packs;
            }
            small_muted(ui, "and");
            if text_link(ui, "Feed The Beast", 12.0).clicked() {
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
        ui.horizontal(|ui| {
            heading(ui, "Discover");
            ui.add_space(8.0);
            self.cf_platform_chips(ui);
        });
        ui.add_space(8.0);
        let mut q = std::mem::take(&mut self.discover.query);
        if big_search(ui, &mut q, &format!("Search {}", self.discover.kind.label().to_lowercase())) {
            search = true;
        }
        self.discover.query = q;
        ui.add_space(4.0);
        tab_row(ui, |ui| {
            for k in Kind::ALL {
                if tab(ui, k.label(), self.discover.kind == k).clicked() && self.discover.kind != k {
                    self.discover.kind = k;
                    search = true;
                }
            }
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if self.discover.kind != Kind::Modpack {
                filter_label(ui, "Install into");
                let current = self
                    .discover
                    .target
                    .as_ref()
                    .and_then(|f| self.instance(f))
                    .map(|i| i.name().to_string())
                    .unwrap_or_else(|| "Choose an instance".into());
                let names: Vec<(String, String)> =
                    self.instances.iter().map(|i| (i.folder.clone(), i.name().to_string())).collect();
                egui::ComboBox::from_id_salt("target").selected_text(current).width(200.0).show_ui(ui, |ui| {
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
                ui.add_space(8.0);
            }
            filter_label(ui, "Sort by");
            egui::ComboBox::from_id_salt("sort")
                .selected_text(match self.discover.sort {
                    Sort::Relevance => "Most relevant",
                    Sort::Downloads => "Most downloaded",
                    Sort::Updated => "Recently updated",
                })
                .width(150.0)
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
            ui.add_space(4.0);
            self.other_modpack_sources(ui);
        } else if let Some(inst) = self.discover.target.as_ref().and_then(|f| self.instance(f)) {
            ui.add_space(4.0);
            small_muted(
                ui,
                format!(
                    "Showing only what works with Minecraft {}{}",
                    inst.mc_version(),
                    inst.loader().map(|(k, _)| format!(" and {k}")).unwrap_or_default()
                ),
            );
        }
        if search {
            self.discover_search(true);
        }
        ui.add_space(8.0);

        if let Some(e) = self.discover.error.clone() {
            if fetch_error_card(ui, "Couldn't reach Modrinth", &e) && !self.discover.loading {
                // the first page again, or the page that failed to load
                self.discover_search(self.discover.hits.is_empty());
            }
        }
        if self.discover.hits.is_empty() && self.discover.loading {
            skeleton_rows(ui, 6);
            return;
        }
        if self.discover.hits.is_empty() && self.discover.error.is_none() {
            let query = self.discover.query.trim().to_string();
            let (title, hint) = if query.is_empty() {
                ("Nothing to show".to_string(), "Try another kind, or pick a different instance.")
            } else {
                (format!("No results for \u{201C}{query}\u{201D}"), "Try fewer words, or another kind.")
            };
            if empty_state(ui, &title, hint, (!query.is_empty()).then_some("Clear search")) {
                self.discover.query.clear();
                self.discover_search(true);
            }
            return;
        }

        self.discover_refresh_installed();
        let installed = self.discover.installed.take();
        let hits = std::mem::take(&mut self.discover.hits);
        let kind = self.discover.kind;
        let target = self.discover.target.clone();
        let mut install: Option<(Hit, Option<String>)> = None;
        let mut more = false;
        if self.discover.total > 0 {
            count_line(ui, format!("{} results", human_count(self.discover.total)));
        }
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for hit in &hits {
                let key = format!("mr:{}", hit.project_id);
                let is_installed = kind != Kind::Modpack
                    && installed.as_ref().is_some_and(|(_, _, ids)| ids.contains(&hit.project_id));
                row_frame(ui, |ui| {
                    let wide = ui.available_width() >= 560.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        match &hit.icon_url {
                            Some(url) if !url.is_empty() => self.remote_image(ui, url, 48.0, &hit.title),
                            _ => {
                                avatar(ui, &hit.title, 48.0);
                            }
                        }
                        let text_w = row_text_width(ui, &key);
                        ui.vertical(|ui| {
                            ui.set_max_width(text_w);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                ui.label(RichText::new(&hit.title).size(15.0).semibold());
                                ui.label(RichText::new(format!("by {}", hit.author)).size(12.5).color(c_muted()));
                            });
                            ui.add(
                                egui::Label::new(RichText::new(&hit.description).color(c_muted()).size(13.0))
                                    .truncate(),
                            );
                            let tags: Vec<(&str, egui::Color32)> =
                                tags_for(hit.categories.iter().map(String::as_str), 4)
                                    .into_iter()
                                    .map(|t| (t, tag_color(t)))
                                    .collect();
                            let figure = (!wide).then(|| format!("{} downloads", human_count(hit.downloads)));
                            meta_line(ui, figure, &tags, is_installed);
                        });
                        row_actions(ui, &key, |ui| {
                            let busy = self.discover.installing.contains(&hit.project_id)
                                && self.tasks.iter().any(|t| t.label == format!("Installing {}", hit.title));
                            if busy {
                                ui.spinner();
                            } else if kind == Kind::Modpack {
                                if secondary(ui, "Install").clicked() {
                                    install = Some((hit.clone(), None));
                                }
                            } else if let Some(t) = &target {
                                // installing again puts the newest fitting version in place of the old one
                                let clicked = if is_installed {
                                    secondary(ui, "Reinstall")
                                        .on_hover_text("Puts the newest version that fits in place of this one")
                                        .clicked()
                                } else {
                                    secondary(ui, "Install").clicked()
                                };
                                if clicked {
                                    install = Some((hit.clone(), Some(t.clone())));
                                }
                            } else {
                                let r = secondary(ui, "Install");
                                egui::Popup::menu(&r).show(|ui| {
                                    ui.set_min_width(180.0);
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
                            if wide {
                                ui.add_space(8.0);
                                stat_column(ui, &human_count(hit.downloads), "downloads");
                            }
                        });
                    });
                });
            }
            ui.add_space(8.0);
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
            ui.add_space(8.0);
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
