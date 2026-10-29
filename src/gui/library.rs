//! Home, Library and the page for a single instance.

use egui::{Align, Color32, CornerRadius, Layout, RichText, ScrollArea, Sense, Stroke, Ui, UiBuilder, vec2};

use super::icons::Icon;
use super::instance_menu::{Act, SplitRow, menu_items, section_break};
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

            match hero {
                Some(h) => {
                    let h = &self.instances[h];
                    // a panel as tall as what is on it was on the last frame: the large picture on the left, and
                    // beside it the name, the details, the strip and the buttons, Play first
                    let height_id = egui::Id::unique("home-hero-height");
                    let height = remembered_width(ui, height_id, 2.0 * HERO_PAD + HERO_ICON + 60.0);
                    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
                    let icon_center = egui::pos2(rect.left() + HERO_PAD + HERO_ICON / 2.0, rect.center().y);
                    hero_ground(ui, rect, icon_center, self.is_playing(&h.folder));
                    let mut icon = ui.new_child(
                        UiBuilder::new()
                            .max_rect(egui::Rect::from_center_size(icon_center, vec2(HERO_ICON, HERO_ICON))),
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
                                action = Some(Act::Stop(h.folder.clone()));
                            }
                        } else if play_button(ui, true).clicked() {
                            action = Some(Act::Play(h.folder.clone()));
                        }
                        if let Some(a) = self.play_as_menu(ui, &h.folder) {
                            action = Some(a);
                        }
                        if let Some(a) = self.performance_button(ui, h) {
                            action = Some(a);
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
                        ui.horizontal(|ui| {
                            if primary(ui, "Create your first instance").clicked() {
                                self.page = Page::Create;
                            }
                            if secondary(ui, "Browse modpacks").clicked() {
                                self.page = Page::Discover;
                                self.discover.kind = crate::modrinth::Kind::Modpack;
                                self.discover_search(true);
                            }
                            // instances from another launcher (MultiMC, Prism, CurseForge, a .mrpack) come in here
                            if secondary(ui, "Import")
                                .on_hover_text("From another launcher, or a modpack file")
                                .clicked()
                            {
                                self.open_import(None);
                            }
                        });
                        small_muted(ui, "You can also drop a modpack file (.mrpack or .zip) on this window.");
                    });
                }
            }

            if self.instances.len() == 1 {
                // the one instance is the card above; the ways to more stay in reach
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
                ui.add_space(24.0);
                let (cols, width) = grid_columns(ui.available_width(), 248.0, 16.0);
                let shown = self.instances.len().min(cols * 2);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Instances").size(FS_SECTION).semibold());
                    count_pill(ui, self.instances.len());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.instances.len() > shown && ghost(ui, "View all").clicked() {
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
                for row in (0..shown).collect::<Vec<_>>().chunks(cols) {
                    ui.horizontal(|ui| {
                        for &i in row {
                            if let Some(a) = self.instance_card(ui, &self.instances[i], width) {
                                action = Some(a);
                            }
                        }
                    });
                }
            }
            self.news_section(ui);
        });
        if let Some(a) = action {
            self.apply(a);
        }
    }

    pub(super) fn library_ui(&mut self, ui: &mut Ui) {
        // in a narrow window the search and the sorting go under the title instead of beside it
        let narrow = ui.available_width() < 780.0;
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
                if !narrow {
                    self.library_filters(ui, true);
                }
            });
        });
        if narrow {
            ui.add_space(4.0);
            ui.horizontal(|ui| self.library_filters(ui, false));
        }
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
        if let Some(a) = self.library_list(ui) {
            self.apply(a);
        }
    }

    /// The sorting and the search of the Library.
    fn library_filters(&mut self, ui: &mut Ui, right_to_left: bool) {
        let sorting = self.settings.default_instance_sorting.clone();
        let label = match sorting.as_str() {
            "BY_LAST_PLAYED" => "Last played first",
            "BY_NUMBER_OF_PLAYS" => "Most played first",
            _ => "By name",
        };
        // the search reads first, then the sorting: a right-to-left row draws its last widget on the left
        let search = |this: &mut Self, ui: &mut Ui| {
            let mut s = std::mem::take(&mut this.library_search);
            search_field(ui, &mut s, "Search instances", 240.0);
            this.library_search = s;
        };
        if !right_to_left {
            search(self, ui);
        }
        egui::ComboBox::from_id_salt("library-sort").selected_text(label).width(170.0).show_ui(ui, |ui| {
            for (id, text) in [
                ("BY_NAME", "By name"),
                ("BY_LAST_PLAYED", "Last played first"),
                ("BY_NUMBER_OF_PLAYS", "Most played first"),
            ] {
                if ui.selectable_label(sorting == id, text).clicked() && sorting != id {
                    self.settings.default_instance_sorting = id.to_string();
                    self.save_settings();
                }
            }
        });
        if right_to_left {
            search(self, ui);
        }
    }

    /// The instances matching the search, as cards. Returns what was asked of one of them.
    fn library_list(&self, ui: &mut Ui) -> Option<Act> {
        let mut action = None;
        let q = self.library_search.trim().to_lowercase();
        let mut list: Vec<&Instance> =
            self.instances.iter().filter(|i| q.is_empty() || i.name().to_lowercase().contains(&q)).collect();
        match self.settings.default_instance_sorting.as_str() {
            "BY_LAST_PLAYED" => list.sort_by_key(|i| std::cmp::Reverse(i.last_played())),
            "BY_NUMBER_OF_PLAYS" => list.sort_by_key(|i| std::cmp::Reverse(i.file.launcher.num_plays.unwrap_or(0))),
            _ => list.sort_by_cached_key(|i| i.name().to_lowercase()),
        }
        if list.is_empty() {
            card(ui, |ui| {
                if self.instances.is_empty() {
                    ui.label(RichText::new("Nothing here yet").size(FS_SECTION).semibold());
                    muted(ui, "Create an instance, or drop a Modrinth modpack (.mrpack) onto this window.");
                } else {
                    muted(ui, "No instance matches your search.");
                }
            });
            return None;
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
                            action = Some(a);
                        }
                    }
                });
            }
        });
        action
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
        ScrollArea::vertical().id_salt("instance-overview").auto_shrink(false).show(ui, |ui| {
            // one column, as wide as a form should be, so a switch stays near its words
            ui.set_max_width(ui.available_width().min(FORM_COLUMN));
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
            // what this instance is, in one strip (the version and the loader are under its name)
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
            ui.add_space(8.0);
            self.play_history(ui, inst);
            section_break(ui);

            // what the three switches say now, read only: they are changed in the settings of the instance
            let mut head = SplitRow::new(ui, egui::Id::unique("overview-performance"), 32.0);
            head.left(ui, |ui| {
                ui.label(RichText::new("Performance").size(FS_SECTION).semibold());
            });
            head.right(ui, |ui| {
                if ghost(ui, "Change").clicked() {
                    change = true;
                }
            });
            head.finish(ui);
            ui.add(egui::Label::new(RichText::new(self.speed_summary(inst)).size(FS_BODY).color(c_muted())).wrap());
            section_break(ui);

            section(ui, "Join a server on start");
            small_muted(ui, "Enter an address like play.example.com or play.example.com:25566");
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let mut s = std::mem::take(&mut self.quick_server);
                search_field(ui, &mut s, "Server address", 280.0);
                self.quick_server = s;
                let ready = !self.quick_server.trim().is_empty() && !self.is_playing(&inst.folder);
                // neutral: the one green button of the page is Play
                if ui.add_enabled_ui(ready, |ui| secondary_icon(ui, Icon::Play, "Play and join")).inner.clicked() {
                    let server = self.quick_server.trim().to_string();
                    self.play_checked(&inst.folder, super::PendingPlay::Join(server));
                }
            });
            section_break(ui);

            self.backups_card(ui, inst);
            section_break(ui);

            ui.horizontal(|ui| {
                ui.label(RichText::new("Folder").size(FS_SECTION).semibold());
                ui.add(egui::Label::new(RichText::new(inst.root.display().to_string()).color(c_muted())).truncate());
            });
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
            ui.label(RichText::new(status).font(super::fonts::medium(FS_BODY)));
        });
        bar.right(ui, |ui| {
            let mut f = std::mem::take(&mut self.log_filter);
            search_field(ui, &mut f, "Filter lines", 200.0);
            self.log_filter = f;
            copy = secondary(ui, "Copy all").clicked();
            clear = ghost(ui, "Clear").clicked();
        });
        bar.finish(ui);
        ui.add_space(4.0);
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
        // the filter are listed, by position
        let filter = self.log_filter.trim().to_lowercase();
        let shown: Vec<usize> = if filter.is_empty() {
            Vec::new()
        } else {
            (0..game.lines.len()).filter(|&i| contains_ignoring_case(&game.lines[i].text, &filter)).collect()
        };
        let count = if filter.is_empty() { game.lines.len() } else { shown.len() };
        log_panel(ui, |ui| {
            let height = ui.available_height().max(120.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
            if count == 0 && !filter.is_empty() {
                muted(ui, "No line matches the filter.");
            }
            ScrollArea::both()
                .id_salt("instance-log")
                .auto_shrink(false)
                .max_height(height)
                .stick_to_bottom(true)
                .show_rows(ui, row_h, count, |ui, range| {
                    for i in range {
                        let line = &game.lines[if filter.is_empty() { i } else { shown[i] }];
                        ui.add(egui::Label::new(log_text(line)).extend());
                    }
                });
        });
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
    use super::contains_ignoring_case;

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
