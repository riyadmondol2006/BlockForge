//! Home, Library and the page for a single instance.

use egui::{Align, CornerRadius, Layout, RichText, ScrollArea, Sense, Stroke, Ui, UiBuilder, vec2};

use super::icons::Icon;
use super::instance_menu::{Act, SplitRow, menu_items, section_break};
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
    /// One instance as a card. Returns what the player asked for, if anything.
    fn instance_card(&self, ui: &mut Ui, inst: &Instance, width: f32) -> Option<Act> {
        let mut act = None;
        let (rect, resp) = ui.allocate_exact_size(vec2(width, 104.0), Sense::click());
        let hovered = resp.hovered();
        let folder = inst.folder.clone();
        let playing = self.is_playing(&folder);
        // the card never changes its fill under the pointer, only its edge; a running instance has a green edge
        let edge = if playing {
            c_accent()
        } else if hovered {
            c_control()
        } else {
            c_border()
        };
        ui.painter().rect(rect, CornerRadius::same(R_CARD), c_card(), Stroke::new(1.0, edge), egui::StrokeKind::Inside);
        // Tab reaches the card too (Enter opens it)
        focus_ring(ui, &resp, rect, R_CARD);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, inst.name()));
        let inner = rect.shrink(16.0);

        // the picture, the name and the details
        let mut top = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Min)));
        top.spacing_mut().item_spacing = vec2(12.0, 2.0);
        self.instance_icon(&mut top, inst, 40.0);
        top.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.set_max_width(inner.width() - 40.0 - 12.0 - 28.0);
            ui.add(egui::Label::new(RichText::new(inst.name()).semibold().size(FS_BODY)).truncate());
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
        let foot = egui::Rect::from_min_max(egui::pos2(inner.left(), inner.bottom() - 32.0), inner.right_bottom());
        let mut bottom = ui.new_child(UiBuilder::new().max_rect(foot).layout(Layout::right_to_left(Align::Center)));
        if play_chip(&mut bottom, playing, hovered).clicked() {
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

    /// The picture, name and details of the instance on Home, with the strip of the days it was played. With `beside`
    /// (the width of the buttons on its right) it leaves them their room, cutting a long name short.
    fn hero_title(&self, ui: &mut Ui, h: &Instance, beside: Option<f32>) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            self.instance_icon(ui, h, 80.0);
            ui.vertical(|ui| {
                if let Some(w) = beside {
                    ui.set_max_width(ui.available_width() - w - 16.0);
                }
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.add(egui::Label::new(RichText::new(h.name()).size(FS_TITLE).semibold()).truncate());
                let mut meta = format!("Minecraft {}", h.mc_version());
                if let Some((kind, ver)) = h.loader() {
                    meta.push_str(&format!("  \u{00B7}  {kind} {ver}"));
                }
                if h.mod_count > 0 {
                    meta.push_str(&format!("  \u{00B7}  {} mods", h.mod_count));
                }
                ui.add(egui::Label::new(RichText::new(meta).color(c_muted())).truncate());
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let days = self.play_days(h, 28);
                    let played = days.iter().filter(|d| **d > 0).count();
                    super::strip::play_strip(ui, &days, Self::strip_hue(h), 10.0, 3.0);
                    // the words "Last played" go when the line is too short for them
                    let room = ui.available_width() - 361.0 - 12.0;
                    let when = if h.last_played() > 0 {
                        let ago = time_ago(h.last_played()).to_lowercase();
                        if room >= 150.0 { format!("Last played {ago}") } else { ago }
                    } else {
                        super::strip::caption(played, 28)
                    };
                    ui.add(egui::Label::new(RichText::new(when).size(FS_META).color(c_faint())).truncate());
                });
            });
        });
    }

    /// The picture and name at the top of an instance's page, and under the name one line of what it is. The line
    /// is made shorter, a whole part at a time, until it fits: it never ends in the middle of a word.
    fn instance_title(&self, ui: &mut Ui, inst: &Instance) {
        ui.spacing_mut().item_spacing.x = 16.0;
        self.instance_icon(ui, inst, 48.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
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
                    egui::Frame::new()
                        .fill(c_card())
                        .stroke(Stroke::new(1.0, c_border()))
                        .corner_radius(CornerRadius::same(R_CARD))
                        .inner_margin(egui::Margin::same(20))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            // the buttons keep their room on the right; when the window is too narrow for both they
                            // go on a row of their own, and a long name is cut short (all of it shows on hover)
                            let buttons_id = egui::Id::unique("home-hero-buttons");
                            let buttons_w = remembered_width(ui, buttons_id, 330.0);
                            let narrow = ui.available_width() - 80.0 - buttons_w < 400.0;
                            let mut used = buttons_w;
                            if narrow {
                                self.hero_title(ui, h, None);
                            }
                            ui.horizontal(|ui| {
                                if !narrow {
                                    self.hero_title(ui, h, Some(buttons_w));
                                }
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if let Some(a) = self.play_as_menu(ui, &h.folder) {
                                        action = Some(a);
                                    }
                                    if self.is_playing(&h.folder) {
                                        if danger_big(ui, Icon::Stop, "Stop").clicked() {
                                            action = Some(Act::Stop(h.folder.clone()));
                                        }
                                    } else if play_button(ui, true).clicked() {
                                        action = Some(Act::Play(h.folder.clone()));
                                    }
                                    used = ui.min_rect().width();
                                });
                            });
                            remember_width(ui, buttons_id, used);
                        });
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
                ui.add_space(12.0);
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
                ui.add_space(18.0);
                let (cols, width) = grid_columns(ui.available_width(), 248.0, 16.0);
                let shown = self.instances.len().min(cols * 2);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Instances").size(FS_SECTION).semibold());
                    ui.label(RichText::new(self.instances.len().to_string()).size(FS_META).color(c_faint()));
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
                ui.add_space(4.0);
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
            ui.label(RichText::new(self.instances.len().to_string()).size(FS_BODY).color(c_faint()));
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
            ui.horizontal(|ui| self.library_filters(ui, false));
        }
        ui.add_space(8.0);
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
        ui.add_space(4.0);
        // the picture and the name on the left; what is going on, Play as, Play and the menu on the right, read from
        // left to right. "Play as" goes to the menu, which has it too, when the window is narrow or the name would
        // be left with less than 360 for its lines (after the picture and the gaps).
        let full_id = egui::Id::unique("instance-header-full");
        let full_w: f32 = ui.data(|d| d.get_temp(full_id)).unwrap_or(300.0);
        let room = ui.available_width();
        let compact = room < 560.0 || room - full_w - (48.0 + 16.0 + 16.0) < 360.0;
        let mut header = SplitRow::new(ui, egui::Id::unique("instance-header"), 48.0);
        header.left(ui, |ui| self.instance_title(ui, inst));
        header.right(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            match self.games.get(&folder).map(|g| g.phase) {
                Some(Phase::Preparing) => {
                    ui.add(egui::Spinner::new().size(14.0));
                    ui.label(RichText::new("Getting ready").color(c_muted()));
                }
                Some(Phase::Running) => {
                    let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, c_accent());
                    ui.label(RichText::new("Running").color(c_muted()));
                }
                _ => {}
            }
            if !compact {
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
        ui.add_space(12.0);
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
            let (rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.0, dot);
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

/// The flat panel the log is read in.
fn log_panel(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(c_field())
        .stroke(Stroke::new(1.0, c_border()))
        .corner_radius(CornerRadius::same(R_CARD))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        });
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
