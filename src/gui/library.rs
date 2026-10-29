//! Home, Library and the page for a single instance.

use egui::{Align, Color32, CornerRadius, Layout, RichText, ScrollArea, Sense, Stroke, Ui, UiBuilder, vec2};

use super::icons::Icon;
use super::instance_menu::{Act, BLOCK_TITLE_H, SplitRow, block_title, menu_items, section_break};
use super::motion;
use super::theme::*;
use super::widgets::*;
use super::{App, InstTab, Page, Phase};
use crate::instances::Instance;
use crate::launch::Level;

/// "12 min" or "1 h 05 min": how long a game has been running.
fn elapsed_text(t: std::time::Duration) -> String {
    let mins = t.as_secs() / 60;
    if mins < 60 { format!("{mins} min") } else { format!("{} h {:02} min", mins / 60, mins % 60) }
}

impl App {
    /// One instance as a tile that lifts under the pointer. Returns what the player asked for, if anything.
    fn instance_card(&self, ui: &mut Ui, inst: &Instance, width: f32) -> Option<Act> {
        let mut act = None;
        let (slot, resp) = ui.allocate_exact_size(vec2(width, TILE_H), Sense::click());
        // the tile counts as pointed at over its buttons too, so it doesn't sink while the Play chip is reached for
        let hot = ui.rect_contains_pointer(slot);
        let folder = inst.folder.clone();
        let playing = self.is_playing(&folder);
        let t = motion::hover(ui, resp.id, hot);
        // it rises a little, its shadow grows and its line brightens; a running instance has a green edge
        let rect = slot.translate(vec2(0.0, -3.0 * t));
        if ui.is_rect_visible(slot) {
            let p = ui.painter();
            p.add(lifted_shadow(t).as_shape(rect, CornerRadius::same(R_CARD)));
            p.rect_filled(rect, CornerRadius::same(R_CARD), motion::mix(c_card(), c_raised(), t));
            // a soft light in the instance's own colour behind its picture (green while it runs)
            let hue = if playing { c_accent() } else { Self::strip_hue(inst) };
            let a = if is_dark() { 26.0 + 22.0 * t } else { 30.0 + 22.0 * t };
            let icon_center = rect.left_top() + vec2(16.0 + TILE_ICON / 2.0, 16.0 + TILE_ICON / 2.0);
            motion::glow(&p.with_clip_rect(rect.shrink(1.0)), icon_center, 74.0, with_alpha(hue, a as u8));
            let edge = if playing { c_accent() } else { motion::mix(c_border(), c_control().gamma_multiply(0.7), t) };
            p.rect_stroke(rect, CornerRadius::same(R_CARD), Stroke::new(1.0, edge), egui::StrokeKind::Inside);
            glass_edge(p, rect, R_CARD);
        }
        // Tab reaches the card too (Enter opens it)
        focus_ring(ui, &resp, rect, R_CARD);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, inst.name()));
        let inner = rect.shrink(16.0);

        // the picture, the name and the details
        let mut top = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Min)));
        top.spacing_mut().item_spacing = vec2(12.0, 2.0);
        self.instance_icon(&mut top, inst, TILE_ICON);
        top.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.set_max_width(inner.width() - TILE_ICON - 12.0 - 28.0);
            ui.add_space(3.0);
            ui.add(egui::Label::new(RichText::new(inst.name()).font(super::fonts::semibold(15.0))).truncate());
            self.loader_line(ui, inst);
        });
        let mut menu = ui.new_child(
            UiBuilder::new()
                .max_rect(egui::Rect::from_min_size(
                    egui::pos2(inner.right() - 28.0, inner.top() - 4.0),
                    vec2(28.0, 28.0),
                ))
                .layout(Layout::top_down(Align::Min)),
        );
        if let Some(Some(a)) = kebab_menu(&mut menu, |ui| self.instance_menu(ui, inst)) {
            act = Some(a);
        }

        // how much it has been played, and the quiet Play button
        let foot = egui::Rect::from_min_max(egui::pos2(inner.left(), inner.bottom() - 34.0), inner.right_bottom());
        let mut bottom = ui.new_child(UiBuilder::new().max_rect(foot).layout(Layout::right_to_left(Align::Center)));
        if play_chip(&mut bottom, playing, hot).clicked() {
            act = Some(if playing { Act::Stop(folder.clone()) } else { Act::Play(folder.clone()) });
        }
        let started = self.games.values().find(|g| g.folder == folder && g.active()).map(|g| g.started.elapsed());
        let (note, color) = match started {
            Some(t) => (format!("Playing  \u{00B7}  {}", elapsed_text(t)), c_accent()),
            None => {
                let when =
                    if inst.last_played() > 0 { time_ago(inst.last_played()) } else { "Not played yet".to_string() };
                let note = if inst.mod_count > 0 { format!("{} mods  \u{00B7}  {when}", inst.mod_count) } else { when };
                (note, c_faint())
            }
        };
        bottom.with_layout(Layout::left_to_right(Align::Center), |ui| {
            if started.is_some() {
                live_dot(ui);
            }
            ui.add(egui::Label::new(RichText::new(note).size(FS_META).color(color)).truncate());
        });
        if act.is_none() && resp.clicked() {
            act = Some(Act::Open(inst.folder.clone()));
        }
        act
    }

    /// "1.21.1 · Fabric 0.20.0": the version in grey and the loader in its own colour.
    fn loader_line(&self, ui: &mut Ui, inst: &Instance) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            ui.label(RichText::new(inst.mc_version().to_string()).size(FS_META).color(c_muted()));
            ui.label(RichText::new("\u{00B7}").size(FS_META).color(c_faint()));
            match inst.loader() {
                Some((kind, ver)) => {
                    // when the whole name would be cut, show just the loader: a line never ends on a dot and ellipsis
                    let full = format!("{kind} {ver}");
                    let w = ui
                        .painter()
                        .layout_no_wrap(full.clone(), egui::FontId::proportional(FS_META), c_text())
                        .size()
                        .x;
                    let text = if w <= ui.available_width() { full } else { kind.to_string() };
                    ui.add(egui::Label::new(RichText::new(text).size(FS_META).color(loader_color(kind))).truncate());
                }
                None => {
                    ui.label(RichText::new("Vanilla").size(FS_META).color(c_muted()));
                }
            }
        });
    }

    /// The name and details of the instance on Home, with the strip of the days it was played.
    fn hero_title(&self, ui: &mut Ui, h: &Instance) {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.add(egui::Label::new(RichText::new(h.name()).size(FS_TITLE).semibold()).truncate());
        // the version, the loader in its own colour and the mods, shortened a whole part at a time to fit
        let room = ui.available_width();
        let job = (0..META_LEVELS)
            .map(|level| instance_meta(h, level))
            .find(|job| ui.painter().layout_job(job.clone()).size().x <= room)
            .unwrap_or_else(|| instance_meta(h, META_LEVELS - 1));
        ui.add(egui::Label::new(job).truncate());
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            // four weeks of days, or fewer (down to two) in a narrow window: the strip gives up its oldest days
            // before the time is cut short
            const DAY: f32 = 13.0;
            const WHEN: f32 = 150.0;
            let n = ((ui.available_width() - 12.0 - WHEN) / DAY).floor().clamp(14.0, 28.0) as usize;
            let days = self.play_days(h, n);
            let played = days.iter().filter(|d| **d > 0).count();
            super::strip::play_strip(ui, &days, Self::strip_hue(h), 10.0, 3.0);
            // the words "Last played" go when the line is too short for them
            let room = ui.available_width();
            let when = if h.last_played() > 0 {
                let ago = time_ago(h.last_played()).to_lowercase();
                if room >= 150.0 { format!("Last played {ago}") } else { ago }
            } else {
                super::strip::caption(played, n)
            };
            ui.add(egui::Label::new(RichText::new(when).size(FS_META).color(c_faint())).truncate());
        });
    }

    /// The picture and name at the top of an instance's page, and under the name one line of what it is. The line
    /// is made shorter, a whole part at a time, until it fits: it never ends in the middle of a word.
    fn instance_title(&self, ui: &mut Ui, inst: &Instance) {
        ui.spacing_mut().item_spacing.x = 18.0;
        self.instance_icon(ui, inst, PAGE_ICON);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.add(egui::Label::new(RichText::new(inst.name()).size(FS_TITLE).semibold()).truncate());
            let room = ui.available_width();
            let job = (0..META_LEVELS)
                .map(|level| instance_meta(inst, level))
                .find(|job| ui.painter().layout_job(job.clone()).size().x <= room)
                .unwrap_or_else(|| instance_meta(inst, META_LEVELS - 1));
            ui.add(egui::Label::new(job).truncate());
        });
    }

    pub(super) fn home_ui(&mut self, ui: &mut Ui) {
        // the instances are looked up by position, not copied: an instance carries its whole version file
        let hero =
            self.instances.iter().position(|i| i.last_played() > 0).or((!self.instances.is_empty()).then_some(0));
        let mut action = None;
        ScrollArea::vertical().id_salt("home").auto_shrink(false).show(ui, |ui| {
            self.low_end_hint(ui);
            if self.accounts.is_empty() {
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Choose who plays").semibold());
                            small_muted(
                                ui,
                                "Sign in with Microsoft, or play offline with any name. You only do this once.",
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if primary(ui, "Choose").clicked() {
                                self.who = Some(Default::default());
                            }
                        });
                    });
                });
                ui.add_space(8.0);
            }

            // in a wide window the news sits beside the instance and the ones played lately, instead of under them
            let news = self.settings.home_news;
            let room = ui.available_width();
            if news && hero.is_some() && room >= HOME_SPLIT {
                let news_w = (room * 0.36).clamp(360.0, 520.0);
                two_columns(
                    ui,
                    room - news_w - HOME_GAP,
                    HOME_GAP,
                    self,
                    |ui, this| this.home_main(ui, hero, &mut action),
                    |ui, this| this.news_section(ui),
                );
            } else {
                self.home_main(ui, hero, &mut action);
                if news {
                    ui.add_space(28.0);
                    self.news_section(ui);
                }
            }
            if !news {
                // the news was hidden on this page: the way to bring it back is where it was
                ui.add_space(20.0);
                if ghost(ui, "Show news").on_hover_text("Show the launcher news on Home again").clicked() {
                    self.settings.home_news = true;
                    self.save_settings();
                }
            }
            ui.add_space(12.0);
        });
        if let Some(a) = action {
            self.apply(a);
        }
    }

    /// The main part of Home: the instance played last on its panel, and under it the others, most recently played
    /// first (or the way to a first instance when there is none).
    fn home_main(&mut self, ui: &mut Ui, hero: Option<usize>, action: &mut Option<Act>) {
        match hero {
            Some(h) => {
                let h = &self.instances[h];
                // a panel as tall as what is on it was on the last frame: the large picture on the left, and beside
                // it the name, the details, the strip and the buttons, Play first
                let height_id = egui::Id::unique("home-hero-height");
                let height = remembered_width(ui, height_id, 2.0 * HERO_PAD + HERO_ICON + 60.0);
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
                let icon_center = egui::pos2(rect.left() + HERO_PAD + HERO_ICON / 2.0, rect.center().y);
                hero_ground(ui, rect, icon_center, self.is_playing(&h.folder));
                let mut icon = ui.new_child(
                    UiBuilder::new().max_rect(egui::Rect::from_center_size(icon_center, vec2(HERO_ICON, HERO_ICON))),
                );
                self.instance_icon(&mut icon, h, HERO_ICON);
                let inner = rect.shrink(HERO_PAD);
                let column = egui::Rect::from_min_max(
                    egui::pos2(inner.left() + HERO_ICON + 28.0, inner.top()),
                    egui::pos2(inner.right(), inner.top() + 1000.0),
                );
                let mut info = ui.new_child(UiBuilder::new().max_rect(column).layout(Layout::top_down(Align::Min)));
                self.hero_title(&mut info, h);
                info.add_space(14.0);
                info.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    if self.is_playing(&h.folder) {
                        if danger_big(ui, Icon::Stop, "Stop").clicked() {
                            *action = Some(Act::Stop(h.folder.clone()));
                        }
                    } else if play_button(ui, true).clicked() {
                        *action = Some(Act::Play(h.folder.clone()));
                    }
                    if let Some(a) = self.play_as_menu(ui, &h.folder) {
                        *action = Some(a);
                    }
                    if let Some(a) = self.performance_button(ui, h) {
                        *action = Some(a);
                    }
                });
                let content = info.min_rect().height().max(HERO_ICON);
                remember_width(ui, height_id, (content + 2.0 * HERO_PAD).ceil());
            }
            None => {
                card(ui, |ui| {
                    ui.label(RichText::new("No instances yet").size(FS_SECTION).semibold());
                    muted(ui, "An instance is one copy of Minecraft with its own worlds and mods.");
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        if primary(ui, "Create your first instance").clicked() {
                            self.page = Page::Create;
                        }
                        if secondary(ui, "Browse modpacks").clicked() {
                            self.page = Page::Discover;
                            self.discover.kind = crate::modrinth::Kind::Modpack;
                            self.discover_search(true);
                        }
                        // instances from another launcher (MultiMC, Prism, CurseForge, a .mrpack) come in here
                        if secondary(ui, "Import").on_hover_text("From another launcher, or a modpack file").clicked() {
                            self.open_import(None);
                        }
                    });
                    small_muted(ui, "You can also drop a modpack file (.mrpack or .zip) on this window.");
                });
            }
        }

        if self.instances.len() == 1 {
            // the one instance is the panel above; the ways to more stay in reach
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if secondary_icon(ui, Icon::Plus, "New instance").clicked() {
                    self.page = Page::Create;
                }
                if secondary(ui, "Find mods").clicked() {
                    self.page = Page::Discover;
                }
            });
        }
        if self.instances.len() > 1 {
            // the others, most recently played first: the one on the panel above isn't shown twice
            let mut rest: Vec<usize> = (0..self.instances.len()).filter(|&i| Some(i) != hero).collect();
            rest.sort_by(|&a, &b| {
                let (a, b) = (&self.instances[a], &self.instances[b]);
                b.last_played()
                    .cmp(&a.last_played())
                    .then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
            });
            ui.add_space(24.0);
            let (cols, width) = grid_columns(ui.available_width(), 248.0, 16.0);
            let shown = rest.len().min(cols * 2);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Continue playing").size(FS_SECTION).semibold());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // the whole Library, with how many instances there are
                    let all = format!("View all {}", self.instances.len());
                    if ghost(ui, &all).on_hover_text("Open the Library").clicked() {
                        self.page = Page::Library;
                    }
                    if ghost(ui, "Find mods").clicked() {
                        self.page = Page::Discover;
                    }
                    if ghost(ui, "New instance").clicked() {
                        self.page = Page::Create;
                    }
                });
            });
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing = vec2(16.0, 16.0);
            for row in rest[..shown].chunks(cols) {
                ui.horizontal(|ui| {
                    for &i in row {
                        if let Some(a) = self.instance_card(ui, &self.instances[i], width) {
                            *action = Some(a);
                        }
                    }
                });
            }
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        }
    }

    pub(super) fn library_ui(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            heading(ui, "Library");
            count_pill(ui, self.instances.len());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if primary_icon(ui, Icon::Plus, "New instance").clicked() {
                    self.page = Page::Create;
                }
                if secondary(ui, "Import").clicked() {
                    self.open_import(None);
                }
            });
        });
        ui.add_space(8.0);
        let mut show = library_show(ui.ctx());
        // a filter for a loader no instance has any more (the last one was deleted) is dropped
        if let Show::Loader(kind) = &show {
            if !self.instances.iter().any(|i| loader_kind(i) == kind) {
                show = Show::All;
            }
        }
        if show == Show::Running && !self.instances.iter().any(|i| self.is_playing(&i.folder)) {
            show = Show::All;
        }
        self.library_toolbar(ui, &mut show);
        ui.add_space(14.0);
        // a folder whose instance.json is damaged isn't in the list: said here, so its worlds aren't taken for lost
        if let Some(first) = self.unreadable.first().cloned() {
            let names: Vec<String> =
                self.unreadable.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).collect();
            let text = if names.len() == 1 {
                format!(
                    "The instance in the folder \"{}\" can't be read: its settings file (instance.json) is damaged. Its worlds and mods are still in the folder.",
                    names[0]
                )
            } else {
                let mut shown: Vec<String> = names.iter().take(3).cloned().collect();
                if names.len() > 3 {
                    shown.push(format!("and {} more", names.len() - 3));
                }
                format!(
                    "{} instance folders can't be read ({}): their settings files (instance.json) are damaged. Their worlds and mods are still in the folders.",
                    names.len(),
                    shown.join(", ")
                )
            };
            if notice(ui, NoticeKind::Warn, &text, Some("Open folder")).is_some_and(|r| r.clicked()) {
                self.open_folder(&first);
            }
            ui.add_space(8.0);
        }
        let out = self.library_list(ui, &show);
        if out.clear {
            self.library_search.clear();
            show = Show::All;
        }
        if let Some(sort) = out.sort {
            self.set_library_sort(sort);
        }
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(SHOW_KEY), show));
        if let Some(a) = out.act {
            self.apply(a);
        }
    }

    /// Remember how the Library is sorted. The sortings the Java edition knows go in the setting it shares; the others
    /// in one of BlockForge's own, so the Java edition never reads one it can't use.
    fn set_library_sort(&mut self, sort: Sort) {
        if Sort::of(&self.settings) == sort {
            return;
        }
        sort.store(&mut self.settings);
        self.save_settings();
    }

    /// The search, the loader filters, the sorting and the view of the Library. On one line when they fit, otherwise
    /// the filters go on a line of their own under the others.
    fn library_toolbar(&mut self, ui: &mut Ui, show: &mut Show) {
        let chips = self.library_chips();
        let chips_id = egui::Id::unique("library-chips-width");
        let chips_w = if chips.is_empty() { 0.0 } else { remembered_width(ui, chips_id, 0.0) };
        let tools_id = egui::Id::unique("library-tools");
        let tools_w = remembered_width(ui, tools_id, 300.0);
        let one_row = ui.available_width() >= LIBRARY_SEARCH_W + 16.0 + chips_w + 24.0 + tools_w;
        let mut measured = None;
        let mut row = SplitRow::new(ui, tools_id, 34.0);
        row.left(ui, |ui| {
            let mut s = std::mem::take(&mut self.library_search);
            search_field(ui, &mut s, "Search instances", LIBRARY_SEARCH_W.min(ui.available_width()));
            self.library_search = s;
            if one_row && !chips.is_empty() {
                ui.add_space(8.0);
                measured = Some(filter_chips(ui, &chips, show));
            }
        });
        row.right(ui, |ui| {
            let sort = Sort::of(&self.settings);
            let mut picked = None;
            egui::ComboBox::from_id_salt("library-sort").selected_text(sort.label()).width(176.0).show_ui(ui, |ui| {
                for s in Sort::ALL {
                    if ui.selectable_label(sort == s, s.label()).clicked() {
                        picked = Some(s);
                    }
                }
            });
            if let Some(s) = picked {
                self.set_library_sort(s);
            }
            let list = self.settings.library_view == "list";
            if let Some(i) = segmented(ui, &["Grid", "List"], usize::from(list)) {
                let view = if i == 1 { "list" } else { "grid" };
                if self.settings.library_view != view {
                    self.settings.library_view = view.to_string();
                    self.save_settings();
                }
            }
        });
        row.finish(ui);
        if !one_row && !chips.is_empty() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| measured = Some(filter_chips(ui, &chips, show)));
        }
        if let Some(w) = measured {
            remember_width(ui, chips_id, w);
        }
    }

    /// The filter chips the Library offers: All, one per loader its instances use (when there is more than one), and
    /// Running while a game is. Each with how many of the instances matching the search it would show.
    fn library_chips(&self) -> Vec<(Show, String)> {
        let q = self.library_search.trim().to_lowercase();
        let found: Vec<&Instance> = self.instances.iter().filter(|i| matches_search(i, &q)).collect();
        let mut kinds: Vec<&str> = self.instances.iter().map(loader_kind).collect();
        kinds.sort_by_key(|k| loader_rank(k));
        kinds.dedup();
        let running = found.iter().filter(|i| self.is_playing(&i.folder)).count();
        let any_running = self.instances.iter().any(|i| self.is_playing(&i.folder));
        if kinds.len() < 2 && !any_running {
            return Vec::new();
        }
        let mut chips = vec![(Show::All, format!("All  {}", found.len()))];
        if kinds.len() >= 2 {
            for kind in kinds {
                let n = found.iter().filter(|i| loader_kind(i) == kind).count();
                chips.push((Show::Loader(kind.to_string()), format!("{kind}  {n}")));
            }
        }
        if any_running {
            chips.push((Show::Running, format!("Running  {running}")));
        }
        chips
    }

    /// The instances that match the search and the filter, sorted as chosen: as tiles or as rows. Returns what was
    /// asked of them.
    fn library_list(&self, ui: &mut Ui, show: &Show) -> LibraryOut {
        let mut out = LibraryOut::default();
        let q = self.library_search.trim().to_lowercase();
        let mut list: Vec<&Instance> = self
            .instances
            .iter()
            .filter(|i| matches_search(i, &q) && show.admits(i, self.is_playing(&i.folder)))
            .collect();
        let sort = Sort::of(&self.settings);
        let added = if sort == Sort::Added { added_times(ui.ctx(), &self.instances) } else { Default::default() };
        sort_instances(&mut list, sort, |i| added.get(&i.folder).copied().unwrap_or(0));
        if list.is_empty() {
            card(ui, |ui| {
                if self.instances.is_empty() {
                    ui.label(RichText::new("Nothing here yet").size(FS_SECTION).semibold());
                    muted(ui, "Create an instance, or drop a Modrinth modpack (.mrpack) onto this window.");
                } else {
                    ui.label(RichText::new("No instances match").size(FS_SECTION).semibold());
                    muted(ui, "No instance matches your search and filters.");
                    ui.add_space(6.0);
                    if secondary(ui, "Clear filters").clicked() {
                        out.clear = true;
                    }
                }
            });
            return out;
        }
        if self.settings.library_view == "list" {
            self.library_rows(ui, &list, sort, &mut out);
            return out;
        }
        ScrollArea::vertical().id_salt("library").auto_shrink(false).show(ui, |ui| {
            // room above the first row for a tile that rises
            ui.add_space(4.0);
            let (per_row, width) = grid_columns(ui.available_width(), 248.0, 16.0);
            ui.spacing_mut().item_spacing = vec2(16.0, 16.0);
            for row in list.chunks(per_row) {
                ui.horizontal(|ui| {
                    for inst in row {
                        if let Some(a) = self.instance_card(ui, inst, width) {
                            out.act = Some(a);
                        }
                    }
                });
            }
        });
        out
    }

    /// The Library as a list: one panel with a head that names the columns (and sorts by one when it is clicked) and
    /// a row per instance under it. The head stays put while the rows scroll.
    fn library_rows(&self, ui: &mut Ui, list: &[&Instance], sort: Sort, out: &mut LibraryOut) {
        let panel = card_frame().inner_margin(egui::Margin::same(6)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let width = ui.available_width() - ui.spacing().scroll.allocated_width();
            let cols = list_columns(width);
            let (head, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
            if let Some(s) = list_head(ui, head, &cols, sort) {
                out.sort = Some(s);
            }
            rule(ui);
            ui.add_space(4.0);
            ScrollArea::vertical()
                .id_salt("library-rows")
                .auto_shrink([false, true])
                .max_height(ui.available_height())
                .show(ui, |ui| {
                    for (n, inst) in list.iter().enumerate() {
                        if n > 0 {
                            ui.add_space(2.0);
                        }
                        if let Some(a) = self.instance_row(ui, inst, &cols, width) {
                            out.act = Some(a);
                        }
                    }
                    ui.add_space(2.0);
                });
        });
        glass_edge(ui.painter(), panel.response.rect, R_CARD);
    }

    /// One instance as a row of the list: the picture and the name, the columns there is room for, and Play and the
    /// menu at the end. The row is pointed at as a whole and opens the instance when clicked.
    fn instance_row(&self, ui: &mut Ui, inst: &Instance, cols: &ListColumns, width: f32) -> Option<Act> {
        let mut act = None;
        let (row, resp) = ui.allocate_exact_size(vec2(width, LIST_ROW_H), Sense::click());
        let hot = ui.rect_contains_pointer(row);
        let folder = inst.folder.clone();
        let playing = self.is_playing(&folder);
        let t = motion::hover(ui, resp.id, hot);
        if ui.is_rect_visible(row) {
            if t > 0.0 {
                ui.painter().rect_filled(row, CornerRadius::same(R_CONTROL), c_hover().gamma_multiply(t));
            }
            if playing {
                // a short green mark at the start of a row whose game is running
                let mark =
                    egui::Rect::from_min_size(egui::pos2(row.left() + 1.0, row.center().y - 10.0), vec2(3.0, 20.0));
                ui.painter().rect_filled(mark, CornerRadius::same(2), c_accent());
            }
        }
        focus_ring(ui, &resp, row, R_CONTROL);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, inst.name()));
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);

        let spans = cols.spans(row.left(), row.right());
        let mid = row.center().y;
        let mut icon = ui.new_child(UiBuilder::new().max_rect(egui::Rect::from_center_size(
            egui::pos2(spans.icon + LIST_ICON / 2.0, mid),
            vec2(LIST_ICON, LIST_ICON),
        )));
        self.instance_icon(&mut icon, inst, LIST_ICON);
        list_cell(ui, row, spans.name, |ui| {
            ui.add(egui::Label::new(RichText::new(inst.name()).font(super::fonts::semibold(FS_BODY))).truncate());
        });
        if let Some(x) = spans.version {
            list_cell(ui, row, x, |ui| {
                ui.add(egui::Label::new(RichText::new(inst.mc_version()).color(c_muted())).truncate());
            });
        }
        if let Some(x) = spans.loader {
            list_cell(ui, row, x, |ui| {
                let (text, color) = match inst.loader() {
                    Some((kind, ver)) => (format!("{kind} {ver}"), loader_color(kind)),
                    None => ("Vanilla".to_string(), c_muted()),
                };
                ui.add(egui::Label::new(RichText::new(text).color(color)).truncate());
            });
        }
        if let Some(x) = spans.mods {
            list_cell(ui, row, x, |ui| {
                let n = if inst.mod_count > 0 { inst.mod_count.to_string() } else { "\u{2014}".to_string() };
                ui.label(RichText::new(n).color(c_muted()));
            });
        }
        if let Some(x) = spans.played {
            list_cell(ui, row, x, |ui| {
                let started =
                    self.games.values().find(|g| g.folder == folder && g.active()).map(|g| g.started.elapsed());
                match started {
                    Some(t) => {
                        live_dot(ui);
                        let text = format!("Playing  \u{00B7}  {}", elapsed_text(t));
                        ui.add(egui::Label::new(RichText::new(text).size(FS_META).color(c_accent())).truncate());
                    }
                    None => {
                        let when = if inst.last_played() > 0 {
                            time_ago(inst.last_played())
                        } else {
                            "Not played yet".to_string()
                        };
                        ui.add(egui::Label::new(RichText::new(when).size(FS_META).color(c_faint())).truncate());
                    }
                }
            });
        }
        let mut end = ui.new_child(
            UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(
                    egui::pos2(spans.actions.0, row.top()),
                    egui::pos2(spans.actions.1, row.bottom()),
                ))
                .layout(Layout::left_to_right(Align::Center)),
        );
        end.spacing_mut().item_spacing.x = LIST_ACTION_GAP;
        if play_chip(&mut end, playing, hot).clicked() {
            act = Some(if playing { Act::Stop(folder.clone()) } else { Act::Play(folder.clone()) });
        }
        if let Some(Some(a)) = kebab_menu(&mut end, |ui| self.instance_menu(ui, inst)) {
            act = Some(a);
        }
        if act.is_none() && resp.clicked() {
            act = Some(Act::Open(folder));
        }
        act
    }

    pub(super) fn instance_ui(&mut self, ui: &mut Ui) {
        let Some(folder) = self.selected.clone() else {
            self.page = Page::Library;
            return;
        };
        let Some(inst) = self.instance(&folder).cloned() else {
            self.page = Page::Library;
            return;
        };
        let mut action = None;
        // one right edge, and the same one on every tab: the back link, the title row, the tab bar and what is under
        // them all end at the same x, so Play stays where it is when the tab changes. In a wider window the page
        // keeps to this column instead of stretching; the scroll bar has its own strip at the window's edge.
        let room = ui.available_width();
        let column = (room - ui.spacing().scroll.allocated_width()).min(INSTANCE_COLUMN);
        ui.scope(|ui| {
            ui.set_max_width(column);
            self.instance_header(ui, &inst, &mut action);
        });
        match self.tab {
            InstTab::Overview => self.tab_overview(ui, &inst),
            InstTab::Mods => {
                ui.scope(|ui| {
                    ui.set_max_width(column);
                    self.mods_tab(ui, &inst);
                })
                .inner
            }
            InstTab::Logs => {
                ui.scope(|ui| {
                    ui.set_max_width(column);
                    self.tab_logs(ui, &inst);
                })
                .inner
            }
            InstTab::Settings => self.tab_settings(ui, &inst),
        }
        if let Some(a) = action {
            self.apply(a);
        }
    }

    /// The back link, the picture and name with Play and the menu, and the tabs of an instance's page.
    fn instance_header(&mut self, ui: &mut Ui, inst: &Instance, action: &mut Option<Act>) {
        let folder = inst.folder.clone();
        if back_link(ui, "Library").clicked() {
            self.page = Page::Library;
        }
        ui.add_space(6.0);
        // the picture and the name on the left; what is going on, Play as, Play and the menu on the right, read from
        // left to right, all on a panel of their own. "Play as" goes to the menu, which has it too, when the window is
        // narrow or the name would be left with less than 300 for its lines (after the picture and the gaps).
        let full_id = egui::Id::unique("instance-header-full");
        let full_w: f32 = ui.data(|d| d.get_temp(full_id)).unwrap_or(300.0);
        let room = ui.available_width() - 2.0 * PAGE_PAD;
        let compact = room < 560.0 || room - full_w - (PAGE_ICON + 18.0 + 16.0) < 300.0;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), PAGE_ICON + 2.0 * PAGE_PAD), Sense::hover());
        let icon_center = rect.left_top() + vec2(PAGE_PAD + PAGE_ICON / 2.0, PAGE_PAD + PAGE_ICON / 2.0);
        hero_ground(ui, rect, icon_center, self.is_playing(&folder));
        let mut panel =
            ui.new_child(UiBuilder::new().max_rect(rect.shrink(PAGE_PAD)).layout(Layout::top_down(Align::Min)));
        {
            let ui = &mut panel;
            let mut header = SplitRow::new(ui, egui::Id::unique("instance-header"), PAGE_ICON);
            header.left(ui, |ui| self.instance_title(ui, inst));
            header.right(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                match self.games.get(&folder).map(|g| g.phase) {
                    Some(Phase::Preparing) => {
                        spinner_sized(ui, 14.0);
                        ui.label(RichText::new("Getting ready").color(c_muted()));
                    }
                    Some(Phase::Running) => {
                        live_dot(ui);
                        ui.label(RichText::new("Running").color(c_muted()));
                    }
                    _ => {}
                }
                if !compact {
                    // a narrow header has these in its menu (Play as, and Performance…)
                    if let Some(a) = self.performance_button(ui, inst) {
                        *action = Some(a);
                    }
                    if let Some(a) = self.play_as_menu(ui, &folder) {
                        *action = Some(a);
                    }
                }
                if self.is_playing(&folder) {
                    if danger_big(ui, Icon::Stop, "Stop").clicked() {
                        *action = Some(Act::Stop(folder.clone()));
                    }
                } else if play_button(ui, true).clicked() {
                    *action = Some(Act::Play(folder.clone()));
                }
                if let Some(Some(a)) = kebab_menu(ui, |ui| {
                    menu_items(ui);
                    self.instance_menu(ui, inst)
                }) {
                    *action = Some(a);
                }
            });
            if !compact {
                let used = header.used();
                ui.data_mut(|d| d.insert_temp(full_id, used));
            }
            header.finish(ui);
        }
        ui.add_space(10.0);
        // a dot on Settings while changes there weren't saved
        let unsaved = self.unsaved_draft(&folder);
        tab_bar(ui, |ui| {
            for (t, label) in [
                (InstTab::Overview, "Overview"),
                (InstTab::Mods, "Mods"),
                (InstTab::Logs, "Logs"),
                (InstTab::Settings, "Settings"),
            ] {
                let mark = (t == InstTab::Settings).then_some(unsaved);
                let r = tab_marked(ui, label, self.tab == t, mark);
                let r = if mark == Some(true) { r.on_hover_text("Changes here aren't saved yet") } else { r };
                if r.clicked() {
                    self.tab = t;
                    if t == InstTab::Mods {
                        self.mods_cache = None;
                    }
                }
            }
        });
        // the page's own gap under the tabs is 16: this one and the spacing the scope gets after it
        ui.add_space(8.0);
    }

    fn tab_overview(&mut self, ui: &mut Ui, inst: &Instance) {
        let mut change = false;
        // as wide as the header above it, so the page has one right edge: the strip of the scroll bar is left out
        // whether the bar shows or not (it only shows when the page is longer than the window)
        let column = (ui.available_width() - ui.spacing().scroll.allocated_width()).min(FORM_COLUMN);
        ScrollArea::vertical().id_salt("instance-overview").auto_shrink(false).show(ui, |ui| {
            ui.set_max_width(column);
            // a newer version of the pack this instance came from
            if let Some(v) = self.ftb.updates.get(&inst.folder).cloned() {
                let text = format!("A newer version of this modpack is available: {}", v.label());
                if notice(ui, NoticeKind::Info, &text, Some(&format!("Update to {}", v.label())))
                    .is_some_and(|r| r.clicked())
                {
                    self.ftb_update_instance(&inst.folder);
                }
                ui.add_space(12.0);
            }
            // the description, when it says more than the version and loader under the name already do (a pack's own
            // words, or what the player wrote with "Change description")
            let desc = inst.file.launcher.description.trim();
            if !desc.is_empty() && !is_automatic_description(desc, inst) {
                let format = egui::TextFormat {
                    font_id: egui::FontId::proportional(FS_BODY),
                    color: c_muted(),
                    ..Default::default()
                };
                let mut job = egui::text::LayoutJob::single_section(desc.to_string(), format);
                job.wrap = egui::text::TextWrapping {
                    max_width: ui.available_width(),
                    max_rows: 3,
                    break_anywhere: false,
                    overflow_character: Some('\u{2026}'),
                };
                ui.add(egui::Label::new(job)).on_hover_text(desc);
                ui.add_space(10.0);
            }
            // what this instance is, in one strip across the page (the version and the loader are under its name)
            let java = inst
                .file
                .version
                .java_version
                .as_ref()
                .map(|j| j.major_version.to_string())
                .unwrap_or_else(|| "Automatic".into());
            let played = if inst.last_played() > 0 { time_ago(inst.last_played()) } else { "\u{2014}".to_string() };
            facts(
                ui,
                &[
                    ("Last played", played),
                    ("Times played", inst.file.launcher.num_plays.unwrap_or(0).to_string()),
                    ("Mods", inst.mod_count.to_string()),
                    ("Java", java),
                ],
            );
            section_break(ui);
            // under it, what happens when it is played on the left and its files on the right; one column when the
            // page is narrow
            let room = ui.available_width();
            if room >= OVERVIEW_SPLIT {
                let left_w = ((room - HOME_GAP) * 0.56).floor();
                two_columns(
                    ui,
                    left_w,
                    HOME_GAP,
                    self,
                    |ui, this| this.overview_play(ui, inst, &mut change),
                    |ui, this| this.overview_files(ui, inst),
                );
            } else {
                self.overview_play(ui, inst, &mut change);
                section_break(ui);
                self.overview_files(ui, inst);
            }
            ui.add_space(12.0);
        });
        if change {
            // the Performance group is on the first tab of the settings, whichever one was open last
            if let Some(d) = self.inst_draft.as_mut().filter(|d| d.folder == inst.folder) {
                d.tab = 0;
            }
            self.tab = InstTab::Settings;
        }
    }

    /// The overview's blocks about playing: the history, the performance switches, and joining a server on start.
    fn overview_play(&mut self, ui: &mut Ui, inst: &Instance, change: &mut bool) {
        self.play_history(ui, inst);
        section_break(ui);

        // what the three switches say now, read only: they are changed in the settings of the instance
        let mut head = SplitRow::new(ui, egui::Id::unique("overview-performance"), BLOCK_TITLE_H);
        head.left(ui, |ui| {
            ui.label(RichText::new("Performance").size(FS_SECTION).semibold());
        });
        head.right(ui, |ui| {
            if ghost(ui, "Change").clicked() {
                *change = true;
            }
        });
        head.finish(ui);
        ui.add(egui::Label::new(RichText::new(self.speed_summary(inst)).size(FS_BODY).color(c_muted())).wrap());
        section_break(ui);

        block_title(ui, "Join a server on start");
        small_muted(ui, "Enter an address like play.example.com or play.example.com:25566");
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            let mut s = std::mem::take(&mut self.quick_server);
            search_field(ui, &mut s, "Server address", ui.available_width().clamp(160.0, 280.0));
            self.quick_server = s;
            let ready = !self.quick_server.trim().is_empty() && !self.is_playing(&inst.folder);
            // neutral: the one green button of the page is Play
            if ui.add_enabled_ui(ready, |ui| secondary_icon(ui, Icon::Play, "Play and join")).inner.clicked() {
                let server = self.quick_server.trim().to_string();
                self.play_checked(&inst.folder, super::PendingPlay::Join(server));
            }
        });
    }

    /// The overview's blocks about files: the backups and the instance's folder.
    fn overview_files(&mut self, ui: &mut Ui, inst: &Instance) {
        self.backups_card(ui, inst);
        section_break(ui);

        block_title(ui, "Folder");
        // the whole path when it is pointed at; cut at the edge otherwise
        let path = inst.root.display().to_string();
        ui.add(egui::Label::new(RichText::new(&path).size(FS_META).color(c_muted())).truncate()).on_hover_text(&path);
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            if secondary_icon(ui, Icon::Folder, "Open folder").clicked() {
                self.open_path(&inst.root);
            }
            if secondary(ui, "Open saves").clicked() {
                // a new instance has no saves folder until its first world
                let saves = inst.root.join("saves");
                let _ = std::fs::create_dir_all(&saves);
                self.open_path(&saves);
            }
            #[cfg(feature = "converter")]
            self.add_bedrock_world_button(ui, inst);
        });
    }

    fn tab_logs(&mut self, ui: &mut Ui, inst: &Instance) {
        let sessions = self.sessions_of(&inst.folder);
        if sessions.is_empty() {
            // nothing has run yet: say so inside the panel where the output will be
            log_panel(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space((ui.available_height() / 2.0 - 40.0).clamp(12.0, 140.0));
                    ui.label(RichText::new("No output yet").semibold());
                    muted(ui, "The game's output shows up here while it runs. Press Play to start it.");
                });
            });
            return;
        }
        // with several games of this instance going at once, each has its own log
        let folder = match self.log_session.clone().filter(|k| sessions.contains(k)) {
            Some(k) => k,
            None => sessions
                .iter()
                .rev()
                .find(|k| self.games.get(*k).is_some_and(|g| g.active()))
                .unwrap_or(&sessions[0])
                .clone(),
        };
        if sessions.len() > 1 {
            ui.horizontal(|ui| {
                for k in &sessions {
                    let label = self.games.get(k).map(|g| g.player.clone()).unwrap_or_default();
                    if chip(ui, &label, *k == folder).clicked() {
                        self.log_session = Some(k.clone());
                    }
                }
            });
        }
        let Some(phase) = self.games.get(&folder).map(|g| g.phase) else { return };
        let (mut copy, mut clear) = (false, false);
        // the status on the left; the filter and the buttons on the right, read from left to right
        let mut bar = SplitRow::new(ui, egui::Id::unique("instance-log-bar"), 32.0);
        bar.left(ui, |ui| {
            let (status, dot) = match phase {
                Phase::Preparing => ("Getting ready".to_string(), c_accent()),
                Phase::Running => ("Running".to_string(), c_accent()),
                Phase::Exited(Some(0)) => ("Closed normally".to_string(), c_faint()),
                Phase::Exited(Some(c)) => (format!("Closed with code {c}"), c_danger()),
                Phase::Exited(None) => ("Closed".to_string(), c_faint()),
                Phase::Failed => ("Couldn't start".to_string(), c_danger()),
            };
            if matches!(phase, Phase::Preparing | Phase::Running) {
                live_dot(ui);
            } else {
                let (rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.0, dot);
            }
            ui.add(egui::Label::new(RichText::new(status).font(super::fonts::medium(FS_BODY))).truncate());
        });
        bar.right(ui, |ui| {
            let mut f = std::mem::take(&mut self.log_filter);
            search_field(ui, &mut f, "Filter lines", 200.0);
            self.log_filter = f;
            copy = secondary(ui, "Copy all").clicked();
            clear = ghost(ui, "Clear").clicked();
        });
        bar.finish(ui);
        if clear {
            if let Some(g) = self.games.get_mut(&folder) {
                g.lines.clear();
            }
        }
        let Some(game) = self.games.get(&folder) else { return };
        if copy {
            let text: Vec<&str> = game.lines.iter().map(|l| l.text.as_str()).collect();
            ui.ctx().copy_text(text.join("\n"));
        }
        // the log is read where it is (thousands of lines, redrawn with every new one); only the lines that match
        // the filter and the level are listed, by position
        let filter = self.log_filter.trim().to_lowercase();
        let level_id = egui::Id::unique("instance-log-level");
        let mut level: LogLevels = ui.ctx().data(|d| d.get_temp(level_id)).unwrap_or_default();
        let shown = log_lines_shown(&game.lines, &filter, level);
        let count = shown.as_ref().map_or(game.lines.len(), Vec::len);
        let errors = game.lines.iter().filter(|l| l.level == Level::Error).count();
        let warnings = game.lines.iter().filter(|l| l.level == Level::Warn).count();
        // under it, the levels to show on the left; how many lines that is and the wrapping on the right
        let mut wrap = self.settings.log_wrap;
        let mut tools = SplitRow::new(ui, egui::Id::unique("instance-log-tools"), 34.0);
        tools.left(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (which, label) in [
                (LogLevels::All, "All".to_string()),
                (LogLevels::Errors, format!("Errors  {errors}")),
                (LogLevels::Warnings, format!("Warnings  {warnings}")),
            ] {
                let on = level == which;
                if chip(ui, &label, on).clicked() {
                    level = if on { LogLevels::All } else { which };
                }
            }
        });
        tools.right(ui, |ui| {
            let total = game.lines.len();
            let lines = if count == total {
                if total == 1 { "1 line".to_string() } else { format!("{total} lines") }
            } else {
                format!("{count} of {total} lines")
            };
            ui.label(RichText::new(lines).size(FS_META).color(c_faint()));
            if chip(ui, "Wrap lines", wrap).on_hover_text("Break long lines instead of scrolling sideways").clicked() {
                wrap = !wrap;
            }
        });
        tools.finish(ui);
        ui.ctx().data_mut(|d| d.insert_temp(level_id, level));
        ui.add_space(4.0);
        log_panel(ui, |ui| {
            let height = ui.available_height().max(120.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
            if count == 0 && !game.lines.is_empty() {
                muted(ui, "No line matches the filter.");
            }
            let line = |i: usize| &game.lines[shown.as_ref().map_or(i, |s| s[i])];
            if wrap {
                // wrapped lines differ in height, so all of them are laid out (the text of each is kept by egui
                // between frames, and the log is kept short enough for that)
                ScrollArea::vertical()
                    .id_salt("instance-log-wrapped")
                    .auto_shrink(false)
                    .max_height(height)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for i in 0..count {
                            ui.add(egui::Label::new(log_text(line(i))).wrap());
                        }
                    });
            } else {
                ScrollArea::both()
                    .id_salt("instance-log")
                    .auto_shrink(false)
                    .max_height(height)
                    .stick_to_bottom(true)
                    .show_rows(ui, row_h, count, |ui, range| {
                        for i in range {
                            ui.add(egui::Label::new(log_text(line(i))).extend());
                        }
                    });
            }
        });
        if wrap != self.settings.log_wrap {
            self.settings.log_wrap = wrap;
            self.save_settings();
        }
        if copy {
            self.toast("Log copied");
        }
    }

    fn tab_settings(&mut self, ui: &mut Ui, inst: &Instance) {
        self.instance_settings_ui(ui, inst);
    }
}

/// The height of an instance's tile, and the size of its picture.
const TILE_H: f32 = 116.0;
const TILE_ICON: f32 = 46.0;
/// The picture of the instance on Home, and the air around what is on its panel.
const HERO_ICON: f32 = 112.0;
const HERO_PAD: f32 = 24.0;
/// The picture at the top of an instance's page, and the air around the panel it is on.
const PAGE_ICON: f32 = 64.0;
const PAGE_PAD: f32 = 20.0;

/// What the Library shows: every instance, those of one loader (`Vanilla` for plain Minecraft), or those running.
#[derive(Clone, Debug, Default, PartialEq)]
enum Show {
    #[default]
    All,
    Loader(String),
    Running,
}

impl Show {
    fn admits(&self, inst: &Instance, running: bool) -> bool {
        match self {
            Show::All => true,
            Show::Loader(kind) => loader_kind(inst) == kind,
            Show::Running => running,
        }
    }
}

/// Where the Library's filter is kept while the launcher runs (it starts at All again next time).
const SHOW_KEY: &str = "library-show";

fn library_show(ctx: &egui::Context) -> Show {
    ctx.data(|d| d.get_temp::<Show>(egui::Id::unique(SHOW_KEY))).unwrap_or_default()
}

/// The loader of an instance as the filters name it: `Fabric`, `Forge`, … or `Vanilla`.
fn loader_kind(inst: &Instance) -> &str {
    inst.loader().map(|(kind, _)| kind).unwrap_or("Vanilla")
}

/// The order loaders are listed and grouped in: the known ones by name, then any other, plain Minecraft last.
fn loader_rank(kind: &str) -> (u8, String) {
    let n = match kind {
        "Fabric" => 0,
        "Forge" => 1,
        "NeoForge" => 2,
        "Quilt" => 3,
        "Vanilla" => 9,
        _ => 5,
    };
    (n, kind.to_lowercase())
}

/// Whether an instance matches the search (already in lower case): its name, its Minecraft version or its loader.
fn matches_search(inst: &Instance, q: &str) -> bool {
    q.is_empty()
        || inst.name().to_lowercase().contains(q)
        || inst.mc_version().to_lowercase().contains(q)
        || loader_kind(inst).to_lowercase().contains(q)
}

/// The ways the Library can be sorted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sort {
    Name,
    LastPlayed,
    MostPlayed,
    Added,
    Version,
    Loader,
}

impl Sort {
    const ALL: [Sort; 6] = [Sort::Name, Sort::LastPlayed, Sort::MostPlayed, Sort::Added, Sort::Version, Sort::Loader];

    /// The id in the settings file (the first three are the Java edition's own).
    fn id(self) -> &'static str {
        match self {
            Sort::Name => "BY_NAME",
            Sort::LastPlayed => "BY_LAST_PLAYED",
            Sort::MostPlayed => "BY_NUMBER_OF_PLAYS",
            Sort::Added => "BY_ADDED",
            Sort::Version => "BY_MINECRAFT_VERSION",
            Sort::Loader => "BY_LOADER",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Sort::Name => "By name",
            Sort::LastPlayed => "Last played first",
            Sort::MostPlayed => "Most played first",
            Sort::Added => "Recently added first",
            Sort::Version => "Newest Minecraft first",
            Sort::Loader => "By loader",
        }
    }

    /// Whether the Java edition knows this sorting (and shares the setting it is kept in).
    fn shared(self) -> bool {
        matches!(self, Sort::Name | Sort::LastPlayed | Sort::MostPlayed)
    }

    fn from_id(id: &str) -> Option<Sort> {
        Sort::ALL.into_iter().find(|s| s.id() == id)
    }

    /// The sorting in use: one of BlockForge's own when one is set, otherwise the one shared with the Java edition.
    fn of(settings: &crate::config::Settings) -> Sort {
        Sort::from_id(&settings.library_sort)
            .filter(|s| !s.shared())
            .or_else(|| Sort::from_id(&settings.default_instance_sorting))
            .unwrap_or(Sort::Name)
    }

    fn store(self, settings: &mut crate::config::Settings) {
        if self.shared() {
            settings.default_instance_sorting = self.id().to_string();
            settings.library_sort.clear();
        } else {
            settings.library_sort = self.id().to_string();
        }
    }
}

/// Sort the Library's instances. `added` is when an instance came into the Library (0 when that isn't known). Ties go
/// by name, so the order never jumps between two frames.
fn sort_instances(list: &mut [&Instance], sort: Sort, added: impl Fn(&Instance) -> i64) {
    use std::cmp::Reverse;
    match sort {
        Sort::Name => list.sort_by_cached_key(|i| i.name().to_lowercase()),
        Sort::LastPlayed => list.sort_by_cached_key(|i| (Reverse(i.last_played()), i.name().to_lowercase())),
        Sort::MostPlayed => list.sort_by_cached_key(|i| {
            (Reverse(i.file.launcher.num_plays.unwrap_or(0)), Reverse(i.last_played()), i.name().to_lowercase())
        }),
        Sort::Added => list.sort_by_cached_key(|i| (Reverse(added(i)), i.name().to_lowercase())),
        Sort::Version => list.sort_by(|a, b| {
            crate::model::compare_versions(b.mc_version(), a.mc_version())
                .then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
        }),
        Sort::Loader => list.sort_by_cached_key(|i| (loader_rank(loader_kind(i)), i.name().to_lowercase())),
    }
}

/// When each instance's folder was made (milliseconds since 1970), as far as the file system keeps it: read once per
/// folder while the launcher runs, not on every frame.
fn added_times(ctx: &egui::Context, instances: &[Instance]) -> std::collections::HashMap<String, i64> {
    let id = egui::Id::unique("library-added-times");
    let mut known: std::collections::HashMap<String, i64> = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    let mut changed = false;
    for inst in instances {
        if !known.contains_key(&inst.folder) {
            let made = std::fs::metadata(&inst.root)
                .and_then(|m| m.created())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            known.insert(inst.folder.clone(), made);
            changed = true;
        }
    }
    if changed {
        ctx.data_mut(|d| d.insert_temp(id, known.clone()));
    }
    known
}

/// What the player did on the Library's list.
#[derive(Default)]
struct LibraryOut {
    act: Option<Act>,
    /// A column head was clicked: sort by it.
    sort: Option<Sort>,
    /// "Clear filters" on the empty list.
    clear: bool,
}

/// The filter chips in a row; the chosen one is lit, and choosing it again goes back to All. Returns how wide the
/// chips are side by side (to know next time whether they fit beside the search).
fn filter_chips(ui: &mut Ui, chips: &[(Show, String)], show: &mut Show) -> f32 {
    let gap = ui.spacing().item_spacing.x;
    let mut width = 0.0;
    for (n, (which, label)) in chips.iter().enumerate() {
        let on = show == which;
        let r = chip(ui, label, on);
        width += r.rect.width() + if n > 0 { gap } else { 0.0 };
        if r.clicked() {
            *show = if on { Show::All } else { which.clone() };
        }
    }
    width
}

/// The width of the Library's search field.
const LIBRARY_SEARCH_W: f32 = 240.0;
/// A row of the Library's list, its picture, and the room for Play and the menu at its end.
const LIST_ROW_H: f32 = 52.0;
const LIST_ICON: f32 = 32.0;
const LIST_ACTION_GAP: f32 = 6.0;
const LIST_ACTIONS_W: f32 = 34.0 + LIST_ACTION_GAP + 28.0;
const LIST_PAD: f32 = 10.0;
const LIST_GAP: f32 = 16.0;
/// The widths of the list's columns, and the least the name keeps before a column makes way for it.
const COL_VERSION: f32 = 96.0;
const COL_LOADER: f32 = 140.0;
const COL_MODS: f32 = 56.0;
const COL_PLAYED: f32 = 150.0;
const NAME_MIN: f32 = 150.0;

/// Which of the list's columns are shown: in a narrow window the number of mods goes first, then when it was last
/// played, then the loader, then the version; the name always stays.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ListColumns {
    version: bool,
    loader: bool,
    mods: bool,
    played: bool,
}

/// Where each part of a list row starts and ends, left to right.
struct Spans {
    icon: f32,
    name: (f32, f32),
    version: Option<(f32, f32)>,
    loader: Option<(f32, f32)>,
    mods: Option<(f32, f32)>,
    played: Option<(f32, f32)>,
    actions: (f32, f32),
}

fn list_columns(width: f32) -> ListColumns {
    let mut cols = ListColumns { version: true, loader: true, mods: true, played: true };
    loop {
        if cols.name_width(width) >= NAME_MIN || !cols.version {
            return cols;
        }
        if cols.mods {
            cols.mods = false;
        } else if cols.played {
            cols.played = false;
        } else if cols.loader {
            cols.loader = false;
        } else {
            cols.version = false;
        }
    }
}

impl ListColumns {
    fn name_width(&self, width: f32) -> f32 {
        let s = self.spans(0.0, width);
        s.name.1 - s.name.0
    }

    fn spans(&self, left: f32, right: f32) -> Spans {
        let actions = (right - LIST_PAD - LIST_ACTIONS_W, right - LIST_PAD);
        // the columns are laid from the end of the row backwards, each with a gap before what follows it
        let mut x = actions.0 - LIST_GAP;
        let mut take = |on: bool, w: f32| {
            on.then(|| {
                let span = (x - w, x);
                x -= w + LIST_GAP;
                span
            })
        };
        let played = take(self.played, COL_PLAYED);
        let mods = take(self.mods, COL_MODS);
        let loader = take(self.loader, COL_LOADER);
        let version = take(self.version, COL_VERSION);
        let icon = left + LIST_PAD;
        let name = (icon + LIST_ICON + 12.0, x);
        Spans { icon, name, version, loader, mods, played, actions }
    }
}

/// A part of a list row: a child laid out left to right, centred in the row's height, cut at its edges.
fn list_cell(ui: &mut Ui, row: egui::Rect, span: (f32, f32), add: impl FnOnce(&mut Ui)) {
    let rect = egui::Rect::from_min_max(egui::pos2(span.0, row.top()), egui::pos2(span.1.max(span.0), row.bottom()));
    let mut cell = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)));
    cell.spacing_mut().item_spacing.x = 6.0;
    cell.set_clip_rect(rect.intersect(ui.clip_rect()));
    add(&mut cell);
}

/// The head of the Library's list: the names of the columns, in small grey type over them. A column that can sort the
/// list sorts it when clicked; the one it is sorted by is in ink with a small chevron.
fn list_head(ui: &mut Ui, head: egui::Rect, cols: &ListColumns, current: Sort) -> Option<Sort> {
    let spans = cols.spans(head.left(), head.right());
    let mut picked = None;
    let parts = [
        (Some(spans.name), "Name", Some(Sort::Name)),
        (spans.version, "Minecraft", Some(Sort::Version)),
        (spans.loader, "Loader", Some(Sort::Loader)),
        (spans.mods, "Mods", None),
        (spans.played, "Last played", Some(Sort::LastPlayed)),
    ];
    for (span, text, sort) in parts {
        let Some(span) = span else { continue };
        let font = super::fonts::medium(FS_META);
        let galley = ui.painter().layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER);
        let on = sort == Some(current);
        let w = (galley.size().x + if on { 14.0 } else { 0.0 }).min(span.1 - span.0);
        let rect = egui::Rect::from_min_size(egui::pos2(span.0, head.center().y - 12.0), vec2(w, 24.0));
        let Some(sort) = sort else {
            ui.painter().galley(egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, c_faint());
            continue;
        };
        let resp = ui.interact(rect, ui.scope_id().with(("library-head", text)), Sense::click());
        let t = motion::hover(ui, resp.id, resp.hovered());
        let rest = if on { c_text() } else { c_faint() };
        let color = motion::mix(rest, c_text(), t);
        ui.painter().galley(egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley.clone(), color);
        if on {
            let c = egui::pos2(rect.left() + galley.size().x + 8.0, rect.center().y);
            let stroke = Stroke::new(1.5, color);
            ui.painter().line_segment([c + vec2(-3.0, -1.5), c + vec2(0.0, 1.5)], stroke);
            ui.painter().line_segment([c + vec2(0.0, 1.5), c + vec2(3.0, -1.5)], stroke);
        }
        focus_ring(ui, &resp, rect.expand2(vec2(4.0, 0.0)), R_TAG);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, format!("Sort by {text}")));
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(sort.label()).clicked() {
            picked = Some(sort);
        }
    }
    picked
}

/// From this width on, Home puts the news beside the instances instead of under them.
const HOME_SPLIT: f32 = 1100.0;
/// The gap between the columns of Home and of an instance's overview.
const HOME_GAP: f32 = 28.0;
/// From this width on, an instance's overview is in two columns.
const OVERVIEW_SPLIT: f32 = 860.0;

/// Two columns side by side, both starting at the same height: the left one `left_w` wide, the right one what is left
/// after `gap`. The page goes on under the taller of the two. What both columns change (`state`) is handed to each.
fn two_columns<S, L, R>(ui: &mut Ui, left_w: f32, gap: f32, state: &mut S, left: L, right: R)
where
    L: FnOnce(&mut Ui, &mut S),
    R: FnOnce(&mut Ui, &mut S),
{
    let right_w = (ui.available_width() - left_w - gap).max(0.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.allocate_ui_with_layout(vec2(left_w, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.set_width(left_w);
            left(ui, state);
        });
        ui.add_space(gap);
        ui.allocate_ui_with_layout(vec2(right_w, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.set_width(right_w);
            right(ui, state);
        });
    });
}

/// `c` with its alpha set to `a`.
fn with_alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

/// The shadow of a tile on its way from resting (0) to lifted (1).
fn lifted_shadow(t: f32) -> egui::Shadow {
    let (a, b) = (card_shadow(), card_shadow_raised());
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    egui::Shadow {
        offset: [0, lerp(a.offset[1] as f32, b.offset[1] as f32).round() as i8],
        blur: lerp(a.blur as f32, b.blur as f32).round() as u8,
        spread: 0,
        color: motion::mix(a.color, b.color, t),
    }
}

/// The panel at the top of Home and of an instance's page: a card of glass with a soft light of the brand colours
/// behind the picture at `glow_at` and a fainter one at the far end, under the buttons. While the game runs the light
/// is a little stronger.
fn hero_ground(ui: &Ui, rect: egui::Rect, glow_at: egui::Pos2, running: bool) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = ui.painter();
    let radius = CornerRadius::same(R_CARD + 4);
    p.add(card_shadow().as_shape(rect, radius));
    p.rect_filled(rect, radius, c_card());
    // the light stays inside the panel
    let lit = p.with_clip_rect(rect.shrink(2.0));
    let boost = if running { 1.35 } else { 1.0 };
    let (a1, a2) = if is_dark() { (58.0, 30.0) } else { (52.0, 30.0) };
    let reach = rect.height().max(120.0);
    motion::glow(&lit, glow_at, reach * 1.25, with_alpha(c_accent(), (a1 * boost) as u8));
    let far = egui::pos2(rect.right() - rect.width() * 0.12, rect.top() + rect.height() * 0.2);
    motion::glow(&lit, far, reach * 1.5, with_alpha(c_accent2(), (a2 * boost) as u8));
    p.rect_stroke(rect, radius, Stroke::new(1.0, c_border()), egui::StrokeKind::Inside);
    glass_edge(p, rect, R_CARD + 4);
}

/// The number of things next to a title, in a small quiet pill.
fn count_pill(ui: &mut Ui, n: usize) {
    let text = n.to_string();
    let galley = ui.painter().layout_no_wrap(text.clone(), super::fonts::medium(FS_META), c_muted());
    let (rect, resp) = ui.allocate_exact_size(vec2((galley.size().x + 14.0).max(24.0), 22.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(11), c_field(), Stroke::new(1.0, c_border()), egui::StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, c_muted());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &text));
}

/// The green dot of something that is running, with a soft light around it.
fn live_dot(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
    motion::glow(ui.painter(), rect.center(), 9.0, with_alpha(c_accent(), if is_dark() { 110 } else { 80 }));
    ui.painter().circle_filled(rect.center(), 4.0, c_accent());
}

/// The width of an instance's page on every tab (the header and what is under it end at the same edge, so Play stays
/// where it is when the tab changes).
const INSTANCE_COLUMN: f32 = 1040.0;

/// The widest a form on an instance's page gets.
pub(super) const FORM_COLUMN: f32 = INSTANCE_COLUMN;

/// How many shortenings [`instance_meta`] has.
const META_LEVELS: usize = 4;

/// "Minecraft 1.21.1 · Fabric 0.20.0 · 7 mods": what an instance is, with the loader in its own colour. `level` 0 is
/// all of it; each next level leaves out one more part, from the end (the number of mods, which the facts strip and
/// the Mods tab show too; then the loader's version; then the loader).
fn instance_meta(inst: &Instance, level: usize) -> egui::text::LayoutJob {
    let part = |color: egui::Color32| egui::TextFormat {
        font_id: egui::FontId::proportional(FS_BODY),
        color,
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    job.append(&format!("Minecraft {}", inst.mc_version()), 0.0, part(c_muted()));
    if level >= 3 {
        return job;
    }
    job.append("  \u{00B7}  ", 0.0, part(c_faint()));
    match inst.loader() {
        Some((kind, _)) if level >= 2 => job.append(kind, 0.0, part(loader_color(kind))),
        Some((kind, ver)) => job.append(&format!("{kind} {ver}"), 0.0, part(loader_color(kind))),
        None => job.append("Vanilla", 0.0, part(c_muted())),
    }
    if inst.mod_count > 0 && level == 0 {
        job.append("  \u{00B7}  ", 0.0, part(c_faint()));
        let mods = if inst.mod_count == 1 { "1 mod".to_string() } else { format!("{} mods", inst.mod_count) };
        job.append(&mods, 0.0, part(c_muted()));
    }
    job
}

/// The panel the log is read in: a well a step below the page, so the lines sit in it rather than on a card.
fn log_panel(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let well = if is_dark() { motion::mix(c_bg(), c_card(), 0.55) } else { c_field() };
    let out = egui::Frame::new()
        .fill(well)
        .stroke(Stroke::new(1.0, c_border()))
        .corner_radius(CornerRadius::same(R_CARD))
        .inner_margin(egui::Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        });
    // a faint shade along the top inside edge: the well is pressed into the page
    let r = out.response.rect;
    let shade = Color32::from_black_alpha(if is_dark() { 60 } else { 14 });
    ui.painter().hline(r.left() + R_CARD as f32..=r.right() - R_CARD as f32, r.top() + 1.5, Stroke::new(1.0, shade));
}

/// Which lines of a log are shown by their level.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum LogLevels {
    #[default]
    All,
    Errors,
    Warnings,
}

/// The positions of the lines that contain `filter` (in lower case) and have the level asked for, or `None` when every
/// line is shown.
fn log_lines_shown(lines: &[crate::launch::LogLine], filter: &str, level: LogLevels) -> Option<Vec<usize>> {
    if filter.is_empty() && level == LogLevels::All {
        return None;
    }
    let wanted = |l: &crate::launch::LogLine| match level {
        LogLevels::All => true,
        LogLevels::Errors => l.level == Level::Error,
        LogLevels::Warnings => l.level == Level::Warn,
    };
    Some((0..lines.len()).filter(|&i| wanted(&lines[i]) && contains_ignoring_case(&lines[i].text, filter)).collect())
}

/// One log line in its colour.
pub(super) fn log_text(line: &crate::launch::LogLine) -> RichText {
    let color = match line.level {
        Level::Error => c_danger(),
        Level::Warn => c_warn(),
        Level::Debug => c_muted(),
        Level::Info => c_text(),
    };
    RichText::new(line.text.as_str()).monospace().color(color)
}

/// Whether `text` contains `lower` (already in lower case), ignoring case, without making a lower-case copy of
/// every line on every frame.
pub(super) fn contains_ignoring_case(text: &str, lower: &str) -> bool {
    if lower.is_empty() {
        return true;
    }
    if text.is_ascii() && lower.is_ascii() {
        let (t, l) = (text.as_bytes(), lower.as_bytes());
        return t.len() >= l.len() && t.windows(l.len()).any(|w| w.eq_ignore_ascii_case(l));
    }
    text.to_lowercase().contains(lower)
}

/// The description an instance gets when it is made ("Minecraft 1.21.1 with Fabric"): it repeats what the line under
/// the name says, so the Overview leaves it out.
fn is_automatic_description(desc: &str, inst: &Instance) -> bool {
    let mc = inst.mc_version();
    desc == format!("Minecraft {mc}")
        || inst.loader().is_some_and(|(kind, _)| desc == format!("Minecraft {mc} with {kind}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use crate::instances::{InstanceFile, LoaderVersion};

    fn inst(name: &str, mc: &str, loader: Option<&str>, last_played: i64, plays: i64) -> Instance {
        let mut file = InstanceFile::default();
        file.launcher.name = name.into();
        file.launcher.last_played = Some(last_played);
        file.launcher.num_plays = Some(plays);
        file.launcher.loader_version =
            loader.map(|kind| LoaderVersion { kind: kind.into(), version: "1.0".into(), ..Default::default() });
        file.version.id = mc.into();
        Instance { folder: name.replace(' ', ""), mod_count: 0, root: "/nonexistent".into(), file }
    }

    fn names(list: &[&Instance]) -> Vec<String> {
        list.iter().map(|i| i.name().to_string()).collect()
    }

    fn library() -> Vec<Instance> {
        vec![
            inst("beta", "1.20.1", Some("Forge"), 300, 2),
            inst("Alpha", "1.21.1", Some("Fabric"), 100, 9),
            inst("gamma", "1.8.9", None, 0, 0),
            inst("Delta", "26.3", Some("Fabric"), 200, 2),
        ]
    }

    #[test]
    fn the_library_sorts_every_way_with_ties_by_name() {
        let all = library();
        let added = |i: &Instance| match i.name() {
            "gamma" => 30,
            "Delta" => 20,
            _ => 0,
        };
        let sorted = |sort: Sort| {
            let mut list: Vec<&Instance> = all.iter().collect();
            sort_instances(&mut list, sort, added);
            names(&list)
        };
        assert_eq!(sorted(Sort::Name), ["Alpha", "beta", "Delta", "gamma"], "names ignore case");
        assert_eq!(sorted(Sort::LastPlayed), ["beta", "Delta", "Alpha", "gamma"]);
        // the same number of plays: the one played last first
        assert_eq!(sorted(Sort::MostPlayed), ["Alpha", "beta", "Delta", "gamma"]);
        // an unknown time (0) goes last, those by name
        assert_eq!(sorted(Sort::Added), ["gamma", "Delta", "Alpha", "beta"]);
        // versions by number, not by text: 26.3 is newer than 1.21.1, and 1.8.9 the oldest
        assert_eq!(sorted(Sort::Version), ["Delta", "Alpha", "beta", "gamma"]);
        // the loaders grouped, plain Minecraft last
        assert_eq!(sorted(Sort::Loader), ["Alpha", "Delta", "beta", "gamma"]);
    }

    #[test]
    fn the_search_and_the_filters_narrow_the_list() {
        let all = library();
        let shown = |q: &str, show: Show| {
            let list: Vec<&Instance> =
                all.iter().filter(|i| matches_search(i, q) && show.admits(i, i.name() == "beta")).collect();
            names(&list)
        };
        assert_eq!(shown("", Show::All).len(), 4);
        assert_eq!(shown("alp", Show::All), ["Alpha"], "the name, in any case");
        assert_eq!(shown("1.20", Show::All), ["beta"], "the Minecraft version");
        assert_eq!(shown("fabric", Show::All), ["Alpha", "Delta"], "the loader");
        assert_eq!(shown("", Show::Loader("Fabric".into())), ["Alpha", "Delta"]);
        assert_eq!(shown("", Show::Loader("Vanilla".into())), ["gamma"]);
        assert_eq!(shown("", Show::Running), ["beta"]);
        assert!(shown("delta", Show::Loader("Forge".into())).is_empty(), "both have to match");
    }

    #[test]
    fn a_sorting_the_java_edition_knows_stays_in_its_setting() {
        let mut s = Settings::default();
        assert_eq!(Sort::of(&s), Sort::Name);
        Sort::Version.store(&mut s);
        assert_eq!((s.library_sort.as_str(), s.default_instance_sorting.as_str()), ("BY_MINECRAFT_VERSION", "BY_NAME"));
        assert_eq!(Sort::of(&s), Sort::Version);
        Sort::LastPlayed.store(&mut s);
        assert_eq!((s.library_sort.as_str(), s.default_instance_sorting.as_str()), ("", "BY_LAST_PLAYED"));
        assert_eq!(Sort::of(&s), Sort::LastPlayed);
        // a value the launcher doesn't know (written by hand, or by a newer version) falls back
        s.library_sort = "BY_SOMETHING".into();
        s.default_instance_sorting = "BY_SOMETHING_ELSE".into();
        assert_eq!(Sort::of(&s), Sort::Name);
        // a shared sorting written into BlockForge's own setting doesn't hide the shared one
        s.library_sort = "BY_NAME".into();
        s.default_instance_sorting = "BY_NUMBER_OF_PLAYS".into();
        assert_eq!(Sort::of(&s), Sort::MostPlayed);
    }

    #[test]
    fn the_list_gives_up_columns_from_the_least_needed_and_keeps_the_name() {
        let all = ListColumns { version: true, loader: true, mods: true, played: true };
        assert_eq!(list_columns(1300.0), all);
        let narrow = list_columns(600.0);
        assert!(narrow.version && narrow.loader && !narrow.mods && !narrow.played, "{narrow:?}");
        let tiny = list_columns(250.0);
        assert!(!tiny.version && !tiny.loader && !tiny.mods && !tiny.played);
        for w in [250.0, 420.0, 600.0, 800.0, 1000.0, 1600.0] {
            let cols = list_columns(w);
            let s = cols.spans(0.0, w);
            assert!(cols.name_width(w) >= NAME_MIN || !cols.version, "{w}: {cols:?}");
            // nothing overlaps: every part ends before the next one starts
            let mut edges = vec![s.name];
            edges.extend([s.version, s.loader, s.mods, s.played].into_iter().flatten());
            edges.push(s.actions);
            for pair in edges.windows(2) {
                assert!(pair[0].1 <= pair[1].0, "{w}: {pair:?}");
            }
            assert!(s.actions.1 <= w);
        }
    }

    #[test]
    fn the_log_shows_the_lines_of_the_level_and_filter_asked_for() {
        use crate::launch::LogLine;
        let l = |level, text: &str| LogLine { level, text: text.into() };
        let lines = vec![
            l(Level::Info, "[main/INFO] Loading"),
            l(Level::Warn, "[main/WARN] Slow"),
            l(Level::Error, "[main/ERROR] Failed to load sound"),
            l(Level::Warn, "[main/WARN] Failed to find"),
        ];
        assert_eq!(log_lines_shown(&lines, "", LogLevels::All), None, "every line, without a list");
        assert_eq!(log_lines_shown(&lines, "", LogLevels::Errors), Some(vec![2]));
        assert_eq!(log_lines_shown(&lines, "", LogLevels::Warnings), Some(vec![1, 3]));
        assert_eq!(log_lines_shown(&lines, "failed", LogLevels::All), Some(vec![2, 3]));
        assert_eq!(log_lines_shown(&lines, "failed", LogLevels::Warnings), Some(vec![3]));
        assert_eq!(log_lines_shown(&lines, "nothing", LogLevels::All), Some(vec![]));
    }

    #[test]
    fn the_log_filter_ignores_case() {
        assert!(contains_ignoring_case("[Render thread/ERROR] Failed", "error"));
        assert!(contains_ignoring_case("anything", ""));
        assert!(!contains_ignoring_case("short", "much longer text"));
        assert!(!contains_ignoring_case("[main/INFO] Loading", "warn"));
        assert!(contains_ignoring_case("Caf\u{c9} OK", "caf\u{e9}"));
        assert!(contains_ignoring_case("x", "x"));
    }
}
