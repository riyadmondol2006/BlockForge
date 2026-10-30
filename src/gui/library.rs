//! Home, Library and the page for a single instance.

use egui::{Align, Color32, CornerRadius, Layout, RichText, ScrollArea, Sense, Stroke, Ui, UiBuilder, vec2};

use super::icons::{self, Icon};
use super::instance_menu::{Act, BLOCK_TITLE_H, SplitRow, block_title, menu_items};
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
    /// One instance as a tile of Home's row: its cover on top, under it the name, what it is and how many mods, with
    /// the menu and a quick Play. It lifts a little under the pointer; the one picked (or played last) has a line in
    /// the brand colour. Returns what the player asked for, if anything.
    fn instance_card(
        &self,
        ui: &mut Ui,
        inst: &Instance,
        cover: &egui::TextureHandle,
        width: f32,
        picked: bool,
    ) -> Option<Act> {
        let mut act = None;
        let cover_h = tile_cover_h(width);
        let (slot, resp) = ui.allocate_exact_size(vec2(width, cover_h + TILE_INFO_H), Sense::click());
        // the tile counts as pointed at over its buttons too, so it doesn't sink while Play is reached for
        let hot = ui.rect_contains_pointer(slot);
        let folder = inst.folder.clone();
        let playing = self.is_playing(&folder);
        let t = motion::hover(ui, resp.id, hot);
        let rect = slot.translate(vec2(0.0, -3.0 * t));
        let pic = egui::Rect::from_min_size(rect.min, vec2(rect.width(), cover_h));
        let round = CornerRadius::same(R_CARD);
        if ui.is_rect_visible(slot) {
            let p = ui.painter();
            p.add(lifted_shadow(t).as_shape(rect, round));
            p.rect_filled(rect, round, motion::mix(c_card(), c_raised(), t));
            paint_cover_tex(ui, cover, pic, CornerRadius { nw: R_CARD, ne: R_CARD, sw: 0, se: 0 });
            // the picked tile (and one whose game runs) has a line in the brand colour; the others brighten theirs
            let (w, edge) = if picked || playing {
                (1.5, c_accent())
            } else {
                (1.0, motion::mix(c_border(), c_control().gamma_multiply(0.7), t))
            };
            p.rect_stroke(rect, round, Stroke::new(w, edge), egui::StrokeKind::Inside);
        }
        let badge =
            egui::Rect::from_min_size(pic.left_bottom() + vec2(10.0, -10.0 - TILE_BADGE), vec2(TILE_BADGE, TILE_BADGE));
        self.picture_badge(ui, inst, badge);
        // Tab reaches the tile too (Enter opens it)
        focus_ring(ui, &resp, rect, R_CARD);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, inst.name()));
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);

        // the name with the menu at its end, then what it is and how many mods, and Play in the corner
        let inner = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 14.0, pic.bottom() + 10.0),
            rect.right_bottom() - vec2(8.0, 10.0),
        );
        let mut text = ui.new_child(
            UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(inner.min, egui::pos2(inner.right() - 30.0, inner.bottom())))
                .layout(Layout::top_down(Align::Min)),
        );
        text.spacing_mut().item_spacing.y = 4.0;
        text.add(egui::Label::new(RichText::new(inst.name()).font(super::fonts::semibold(FS_BODY))).truncate());
        text.set_max_width(inner.width() - 30.0 - 34.0);
        self.loader_line(&mut text, inst, false);
        let started = self.games.values().find(|g| g.folder == folder && g.active()).map(|g| g.started.elapsed());
        text.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            match started {
                Some(t) => {
                    live_dot(ui);
                    let note = format!("Playing \u{00B7} {}", elapsed_text(t));
                    ui.add(egui::Label::new(RichText::new(note).size(FS_META).color(c_accent())).truncate());
                }
                None => {
                    ui.add(
                        egui::Label::new(RichText::new(contents_text(inst)).size(FS_META).color(c_faint())).truncate(),
                    );
                }
            }
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
        let mut corner = ui.new_child(
            UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(inner.right_bottom() - vec2(34.0, 34.0), inner.right_bottom()))
                .layout(Layout::top_down(Align::Min)),
        );
        if play_chip(&mut corner, playing, hot).clicked() {
            act = Some(if playing { Act::Stop(folder.clone()) } else { Act::Play(folder.clone()) });
        }
        if act.is_none() && resp.clicked() {
            act = Some(Act::Open(folder));
        }
        act
    }

    /// "1.21.1 · Fabric 0.20.0" in the monospace face, the loader in its own colour. With `version` false, or when the
    /// whole line would be cut, the loader's version is left out: a line never ends on a dot and an ellipsis.
    fn loader_line(&self, ui: &mut Ui, inst: &Instance, version: bool) {
        let room = ui.available_width();
        let build = self.horizon_tag();
        let mut job = loader_job(inst, FS_META, version, build);
        if version && ui.painter().layout_job(job.clone()).size().x > room {
            job = loader_job(inst, FS_META, false, build);
        }
        ui.add(egui::Label::new(job).truncate());
    }

    /// What sits over the cover of Home's banner: a small label, the name, what the instance is, the big Play (Stop
    /// while it runs), the menu of the other ways to play, when it was played, and the strip of the days it was.
    /// `compact` in a low banner (a short window): the name a little smaller and the gaps closer.
    fn hero_title(&self, ui: &mut Ui, h: &Instance, compact: bool, action: &mut Option<Act>) {
        let folder = h.folder.clone();
        let playing = self.is_playing(&folder);
        let wide = ui.available_width() >= 520.0;
        let gap = |roomy: f32| if compact { roomy * 0.6 } else { roomy };
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let label = if playing {
            "Now playing"
        } else if h.last_played() > 0 {
            "Continue playing"
        } else {
            "Ready to play"
        };
        ui.label(RichText::new(label).font(super::fonts::semibold(13.0)).color(COVER_ACCENT));
        ui.add_space(4.0);
        let name_size = if wide && !compact { 34.0 } else { 28.0 };
        ui.scope(|ui| {
            ui.set_max_width(ui.available_width().min(640.0));
            ui.add(
                egui::Label::new(RichText::new(h.name()).font(super::fonts::semibold(name_size)).color(ON_COVER))
                    .truncate(),
            );
        });
        ui.add_space(6.0);
        // the version, the loader and its version and the mods, a whole part at a time left out until it fits
        let room = ui.available_width();
        let build = self.horizon_tag();
        let job = (0..BANNER_META_LEVELS)
            .map(|level| banner_meta(h, level, build))
            .find(|job| ui.painter().layout_job(job.clone()).size().x <= room)
            .unwrap_or_else(|| banner_meta(h, BANNER_META_LEVELS - 1, build));
        ui.add(egui::Label::new(job).truncate());
        ui.add_space(gap(20.0));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            // Stop takes Play's place at Play's size, so nothing beside it moves when the game starts
            let size = vec2(if wide { 160.0 } else { 136.0 }, 46.0);
            if playing {
                if danger_sized(ui, Icon::Stop, "Stop", size).clicked() {
                    *action = Some(Act::Stop(folder.clone()));
                }
            } else if play_button_sized(ui, size).clicked() {
                *action = Some(Act::Play(folder.clone()));
            }
            // the other ways to play: as somebody else, and the switches that make the game run better
            let more = cover_button(ui, 46.0).on_hover_text("Play as another account, and performance");
            let menu = egui::Popup::menu(&more).gap(6.0).show(|ui| {
                menu_items(ui);
                ui.set_min_width(220.0);
                let mut act = None;
                let running =
                    self.sessions_of(&folder).iter().filter(|k| self.games.get(*k).is_some_and(|g| g.active())).count();
                let play_as = if running > 0 {
                    format!("Play as\u{2026} ({running} running)")
                } else {
                    "Play as\u{2026}".to_string()
                };
                ui.menu_button(play_as, |ui| {
                    if let Some(a) = self.account_menu(ui, &folder) {
                        act = Some(a);
                    }
                });
                // the helpers are Java's: Horizon starts without them
                if !crate::horizon::is_horizon(h) {
                    if let Some(a) = self.helpers_menu(ui, h) {
                        act = Some(a);
                    }
                    ui.separator();
                    ui.add_enabled(
                        false,
                        egui::Label::new(RichText::new(self.speed_summary(h)).size(FS_META).color(c_faint())),
                    );
                }
                act
            });
            if let Some(Some(a)) = menu.map(|r| r.inner) {
                *action = Some(a);
            }
            // a thin line, then when it was played (or for how long it has been running)
            ui.add_space(6.0);
            let (line, _) = ui.allocate_exact_size(vec2(1.0, 34.0), Sense::hover());
            ui.painter().rect_filled(line, 0.0, Color32::from_white_alpha(56));
            ui.add_space(6.0);
            let started = self.games.values().find(|g| g.folder == folder && g.active()).map(|g| g.started.elapsed());
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let (top, value) = match started {
                    Some(t) => ("Playing for".to_string(), elapsed_text(t)),
                    None if h.last_played() > 0 => ("Last played".to_string(), time_ago(h.last_played())),
                    None => ("Last played".to_string(), "Not yet".to_string()),
                };
                ui.label(RichText::new(top).size(FS_META).color(Color32::from_white_alpha(170)));
                ui.label(RichText::new(value).font(super::fonts::semibold(FS_BODY)).color(ON_COVER));
            });
        });
        ui.add_space(gap(18.0));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            // four weeks of days, or two in a narrow window: the strip gives up its oldest days before its words go
            const DAY: f32 = 13.0;
            const WORDS: f32 = 210.0;
            let n = if ui.available_width() >= 28.0 * DAY + 12.0 + WORDS { 28 } else { 14 };
            let days = self.play_days(h, n);
            let played = days.iter().filter(|d| **d > 0).count();
            super::strip::play_strip_inked(ui, &days, COVER_ACCENT, 10.0, 3.0, super::strip::Ink::on_cover());
            let caption = super::strip::caption(played, n);
            ui.add(
                egui::Label::new(RichText::new(caption).size(FS_META).color(Color32::from_white_alpha(170))).truncate(),
            );
        });
    }

    pub(super) fn home_ui(&mut self, ui: &mut Ui) {
        // the instances are looked up by position, not copied: an instance carries its whole version file. The banner
        // is the one played last (the first one when none has been played yet)
        let hero = (0..self.instances.len()).max_by_key(|&i| (self.instances[i].last_played(), std::cmp::Reverse(i)));
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
                ui.add_space(12.0);
            }

            match hero {
                Some(h) => self.home_banner(ui, h, &mut action),
                None => self.home_empty(ui),
            }
            ui.add_space(HOME_SECTION_GAP);
            // in a wide window the news sits beside the instances instead of under them
            let news = self.settings.home_news;
            let room = ui.available_width();
            if news && hero.is_some() && room >= HOME_SPLIT {
                let news_w = (room * 0.34).clamp(340.0, 480.0);
                two_columns(
                    ui,
                    room - news_w - HOME_GAP,
                    HOME_GAP,
                    self,
                    |ui, this| this.home_main(ui, hero, &mut action),
                    |ui, this| this.news_section(ui),
                );
            } else {
                if hero.is_some() {
                    self.home_main(ui, hero, &mut action);
                }
                if news {
                    if hero.is_some() {
                        ui.add_space(HOME_SECTION_GAP);
                    }
                    self.news_section(ui);
                }
            }
            if !news {
                // the news was hidden on this page: the way to bring it back is where it was
                ui.add_space(14.0);
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

    /// The banner at the top of Home: the cover of the instance played last fills the panel, and over its darker left
    /// side are what it is, Play and when it was played (see [`Self::hero_title`]). It is as tall as the window's width
    /// allows, between a low and a high limit (lower in a short window, so the instances and the news show under it),
    /// and what is on it is centred in it.
    fn home_banner(&mut self, ui: &mut Ui, h: usize, action: &mut Option<Act>) {
        let w = ui.available_width();
        let height = hero_height(w, ui.ctx().content_rect().height());
        let (rect, _) = ui.allocate_exact_size(vec2(w, height), Sense::hover());
        let cover = self.cover_at(h, rect.size(), true);
        let inst = &self.instances[h];
        let round = CornerRadius::same(R_CARD + 4);
        if ui.is_rect_visible(rect) {
            let p = ui.painter();
            p.add(card_shadow().as_shape(rect, round));
            paint_cover_tex(ui, &cover, rect, round);
            // a little more shade on the left, under the words, so they read on a bright sky in either look
            shade_left(p, rect, R_CARD + 4, 120, rect.width() * 0.7);
            p.rect_stroke(rect, round, Stroke::new(1.0, Color32::from_white_alpha(18)), egui::StrokeKind::Inside);
        }
        // the instance's own picture, when it has one, in the far corner
        let badge = egui::Rect::from_min_size(
            rect.right_top() + vec2(-20.0 - BANNER_BADGE, 20.0),
            vec2(BANNER_BADGE, BANNER_BADGE),
        );
        self.picture_badge(ui, inst, badge);
        let pad = if w < 720.0 { 24.0 } else { 36.0 };
        let compact = height < 230.0;
        let content_id = egui::Id::unique("home-banner-content");
        let content_h = remembered_width(ui, content_id, 200.0);
        let top = rect.top() + ((height - content_h) / 2.0).max(if compact { 16.0 } else { pad * 0.6 });
        let column = egui::Rect::from_min_max(egui::pos2(rect.left() + pad, top), rect.right_bottom() - vec2(pad, 0.0));
        let mut info = ui.new_child(UiBuilder::new().max_rect(column).layout(Layout::top_down(Align::Min)));
        info.set_clip_rect(rect.intersect(ui.clip_rect()));
        self.hero_title(&mut info, inst, compact, action);
        remember_width(ui, content_id, info.min_rect().height().ceil());
    }

    /// Home without any instance: the way to a first one, a modpack, or one from another launcher.
    fn home_empty(&mut self, ui: &mut Ui) {
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

    /// "Your instances" under the banner: a row of tiles, most recently played first, as many as fit on one line,
    /// with the ways to more (a new instance, mods, an import, the whole Library) at the end of the title.
    fn home_main(&mut self, ui: &mut Ui, hero: Option<usize>, action: &mut Option<Act>) {
        let mut order: Vec<usize> = (0..self.instances.len()).collect();
        order.sort_by(|&a, &b| {
            let (a, b) = (&self.instances[a], &self.instances[b]);
            b.last_played().cmp(&a.last_played()).then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
        });
        let mut head = SplitRow::new(ui, egui::Id::unique("home-instances-head"), 34.0);
        head.left(ui, |ui| {
            ui.add(egui::Label::new(RichText::new("Your instances").size(FS_SECTION).semibold()).truncate());
        });
        head.right(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if ghost(ui, "New instance").clicked() {
                self.page = Page::Create;
            }
            if ghost(ui, "Find mods").clicked() {
                self.page = Page::Discover;
            }
            // instances from another launcher (MultiMC, Prism, CurseForge, a .mrpack) come in here
            if ghost(ui, "Import").on_hover_text("From another launcher, or a modpack file").clicked() {
                self.open_import(None);
            }
            let all = format!("View library ({}) \u{2192}", self.instances.len());
            if ghost(ui, &all).on_hover_text("Every instance, in the Library").clicked() {
                self.go(Page::Library);
            }
        });
        head.finish(ui);
        ui.add_space(10.0);
        let (cols, width) = grid_columns(ui.available_width(), HOME_TILE_MIN, HOME_TILE_GAP);
        let shown: Vec<usize> = order.into_iter().take(cols).collect();
        let size = vec2(width, tile_cover_h(width));
        let covers: Vec<egui::TextureHandle> = shown.iter().map(|&i| self.cover_at(i, size, false)).collect();
        // the outlined tile is the instance last opened, else the one on the banner
        let picked = self.selected.as_deref().and_then(|f| self.instances.iter().position(|i| i.folder == f)).or(hero);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = HOME_TILE_GAP;
            for (k, &i) in shown.iter().enumerate() {
                if let Some(a) = self.instance_card(ui, &self.instances[i], &covers[k], width, picked == Some(i)) {
                    *action = Some(a);
                }
            }
        });
    }

    /// The cover of the instance at position `i` for a place of `size` points, drawn once and then kept. By position,
    /// so the instance isn't borrowed while the store of covers changes.
    fn cover_at(&mut self, i: usize, size: egui::Vec2, fade_left: bool) -> egui::TextureHandle {
        let spec = super::workbench::cover_spec(&self.instances[i]).with_fade_left(fade_left);
        let (w, h) = super::workbench::cover_size(size);
        self.covers.texture(&self.ctx, &spec, w, h)
    }

    /// The instance's own picture (its `instance.png`), when it has one, as a small rounded badge in `rect` over its
    /// cover: the cover stays the backdrop, and a picture that was chosen isn't lost.
    fn picture_badge(&self, ui: &Ui, inst: &Instance, rect: egui::Rect) {
        if !ui.is_rect_visible(rect) {
            return;
        }
        let Some(tex) = self.local_picture(inst.root.join("instance.png")) else { return };
        let r = (rect.width() * 0.24).round() as u8;
        let p = ui.painter();
        let shadow = egui::Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(120) };
        p.add(shadow.as_shape(rect, CornerRadius::same(r)));
        p.rect_filled(rect.expand(2.0), CornerRadius::same(r + 2), Color32::from_rgb(0x15, 0x1D, 0x29));
        egui::Image::from_texture(&tex).corner_radius(r).paint_at(ui, rect);
        p.rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(r + 2),
            Stroke::new(1.0, Color32::from_white_alpha(70)),
            egui::StrokeKind::Inside,
        );
    }

    pub(super) fn library_ui(&mut self, ui: &mut Ui) {
        // the title with how many there are, and on the right the search, the view and the ways to more instances
        PageHeader::new("Library").count(self.instances.len()).show(ui, |ui| {
            if primary_icon(ui, Icon::Plus, "New instance").clicked() {
                self.page = Page::Create;
            }
            // instances from another launcher (MultiMC, Prism, CurseForge, a .mrpack) come in here
            if secondary(ui, "Import").on_hover_text("From another launcher, or a modpack file").clicked() {
                self.open_import(None);
            }
            let list = self.settings.library_view == "list";
            if let Some(list) = view_switch(ui, list) {
                let view = if list { "list" } else { "grid" };
                if self.settings.library_view != view {
                    self.settings.library_view = view.to_string();
                    self.save_settings();
                }
            }
            let mut s = std::mem::take(&mut self.library_search);
            search_field(ui, &mut s, "Search instances", LIBRARY_SEARCH_W.min(ui.available_width()));
            self.library_search = s;
        });
        let mut show = library_show(ui.ctx());
        // a filter for a loader no instance has any more (the last one was deleted), for favourites when there are
        // none left, or for running games when none runs, is dropped
        let gone = match &show {
            Show::All => false,
            Show::Loader(kind) => !self.instances.iter().any(|i| loader_kind(i) == kind),
            Show::Favorites => !self.instances.iter().any(is_favorite),
            Show::Running => !self.instances.iter().any(|i| self.is_playing(&i.folder)),
        };
        if gone {
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
        if let Some((folder, on)) = out.favorite {
            self.set_favorite(&folder, on);
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

    /// Mark an instance as a favourite or not, in its own file.
    fn set_favorite(&mut self, folder: &str, on: bool) {
        let Some(inst) = self.instances.iter_mut().find(|i| i.folder == folder) else { return };
        set_favorite(inst, on);
        if let Err(e) = inst.save() {
            self.toast_err(format!("Couldn't save the favourite: {e:#}"));
        }
    }

    /// The filter chips of the Library and, at the end of their line, how it is sorted. The chips wrap onto more lines
    /// in a narrow window; the sorting stays at the right of the first.
    fn library_toolbar(&mut self, ui: &mut Ui, show: &mut Show) {
        let chips = self.library_chips();
        let sort = Sort::of(&self.settings);
        let sort_id = egui::Id::unique("library-sort-width");
        let sort_w = remembered_width(ui, sort_id, 190.0);
        let room = ui.available_width();
        let mut picked = None;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let left_w = (room - sort_w - 16.0).max(120.0);
            ui.allocate_ui_with_layout(
                vec2(left_w, 30.0),
                Layout::left_to_right(Align::Center).with_main_wrap(true),
                |ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    ui.set_max_width(left_w);
                    filter_chips(ui, &chips, show);
                },
            );
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                let label = format!("Sort: {}", sort.short());
                let (resp, _) = super::instance_menu::secondary_menu(ui, &label, |ui| {
                    ui.set_min_width(220.0);
                    for s in Sort::ALL {
                        if ui.selectable_label(sort == s, s.label()).clicked() {
                            picked = Some(s);
                        }
                    }
                });
                remember_width(ui, sort_id, resp.rect.width());
            });
        });
        if let Some(s) = picked {
            self.set_library_sort(s);
        }
    }

    /// The filter chips the Library offers: All, Favourites when there are any, one per loader its instances use (when
    /// there is more than one), and Running while a game is. Each with how many of the instances matching the search
    /// it would show.
    fn library_chips(&self) -> Vec<(Show, String)> {
        let q = self.library_search.trim().to_lowercase();
        let found: Vec<&Instance> = self.instances.iter().filter(|i| matches_search(i, &q)).collect();
        let mut kinds: Vec<&str> = self.instances.iter().map(loader_kind).collect();
        kinds.sort_by_key(|k| loader_rank(k));
        kinds.dedup();
        let mut chips = vec![(Show::All, format!("All ({})", found.len()))];
        if self.instances.iter().any(is_favorite) {
            let n = found.iter().filter(|i| is_favorite(i)).count();
            chips.push((Show::Favorites, format!("Favourites ({n})")));
        }
        if kinds.len() >= 2 {
            for kind in kinds {
                let n = found.iter().filter(|i| loader_kind(i) == kind).count();
                chips.push((Show::Loader(kind.to_string()), format!("{kind} ({n})")));
            }
        }
        if self.instances.iter().any(|i| self.is_playing(&i.folder)) {
            let n = found.iter().filter(|i| self.is_playing(&i.folder)).count();
            chips.push((Show::Running, format!("Running ({n})")));
        }
        chips
    }

    /// The instances that match the search and the filter, sorted as chosen: as cards or as rows. Returns what was
    /// asked of them.
    fn library_list(&mut self, ui: &mut Ui, show: &Show) -> LibraryOut {
        let mut out = LibraryOut::default();
        let q = self.library_search.trim().to_lowercase();
        let sort = Sort::of(&self.settings);
        // the positions of the instances shown, in order (by position, so the covers can be fetched for them)
        let order: Vec<usize> = {
            let mut list: Vec<&Instance> = self
                .instances
                .iter()
                .filter(|i| matches_search(i, &q) && show.admits(i, self.is_playing(&i.folder)))
                .collect();
            let added = if sort == Sort::Added { added_times(ui.ctx(), &self.instances) } else { Default::default() };
            sort_instances(&mut list, sort, |i| added.get(&i.folder).copied().unwrap_or(0));
            list.iter().filter_map(|i| self.instances.iter().position(|x| std::ptr::eq(x, *i))).collect()
        };
        if order.is_empty() {
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
            let size = vec2(LIST_ICON, LIST_ICON);
            let covers: Vec<egui::TextureHandle> = order.iter().map(|&i| self.cover_at(i, size, false)).collect();
            self.library_rows(ui, &order, &covers, sort, &mut out);
            return out;
        }
        let size = vec2(LIB_CARD_H * 0.8, LIB_CARD_H - 2.0 * LIB_CARD_PAD);
        let covers: Vec<egui::TextureHandle> = order.iter().map(|&i| self.cover_at(i, size, false)).collect();
        ScrollArea::vertical().id_salt("library").auto_shrink(false).show(ui, |ui| {
            // room above the first row for a card that rises
            ui.add_space(4.0);
            let (per_row, width) = grid_columns(ui.available_width(), LIB_CARD_MIN, LIB_CARD_GAP);
            ui.spacing_mut().item_spacing = vec2(LIB_CARD_GAP, LIB_CARD_GAP);
            for (r, row) in order.chunks(per_row).enumerate() {
                ui.horizontal(|ui| {
                    for (k, &i) in row.iter().enumerate() {
                        let cover = &covers[r * per_row + k];
                        if let Some(a) = self.library_card(ui, &self.instances[i], cover, width, &mut out) {
                            out.act = Some(a);
                        }
                    }
                });
            }
            ui.add_space(8.0);
        });
        out
    }

    /// One instance as a card of the Library's grid: its cover as a picture on the left, and beside it the name with
    /// the favourite star, what it is, its mods and when it was played, and Play with the menu. The card lifts a
    /// little under the pointer and opens the instance when clicked.
    fn library_card(
        &self,
        ui: &mut Ui,
        inst: &Instance,
        cover: &egui::TextureHandle,
        width: f32,
        out: &mut LibraryOut,
    ) -> Option<Act> {
        let mut act = None;
        let (slot, resp) = ui.allocate_exact_size(vec2(width, LIB_CARD_H), Sense::click());
        let hot = ui.rect_contains_pointer(slot);
        let folder = inst.folder.clone();
        let playing = self.is_playing(&folder);
        let t = motion::hover(ui, resp.id, hot);
        let rect = slot.translate(vec2(0.0, -2.0 * t));
        let side = rect.height() - 2.0 * LIB_CARD_PAD;
        let pic = egui::Rect::from_min_size(
            rect.min + vec2(LIB_CARD_PAD, LIB_CARD_PAD),
            vec2((side * 0.86).min(rect.width() * 0.34).round(), side),
        );
        if ui.is_rect_visible(slot) {
            let p = ui.painter();
            let round = CornerRadius::same(R_CARD);
            p.add(lifted_shadow(t).as_shape(rect, round));
            p.rect_filled(rect, round, motion::mix(c_card(), c_raised(), t));
            let edge = if playing { c_accent() } else { motion::mix(c_border(), c_control().gamma_multiply(0.7), t) };
            p.rect_stroke(rect, round, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
            glass_edge(p, rect, R_CARD);
            paint_cover_tex(ui, cover, pic, CornerRadius::same(R_CONTROL + 2));
        }
        let badge =
            egui::Rect::from_min_size(pic.left_bottom() + vec2(8.0, -8.0 - LIB_BADGE), vec2(LIB_BADGE, LIB_BADGE));
        self.picture_badge(ui, inst, badge);
        focus_ring(ui, &resp, rect, R_CARD);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, inst.name()));
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);

        let inner = egui::Rect::from_min_max(
            egui::pos2(pic.right() + 16.0, rect.top() + LIB_CARD_PAD + 2.0),
            rect.right_bottom() - vec2(LIB_CARD_PAD, LIB_CARD_PAD),
        );
        let mut text = ui.new_child(
            UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(inner.min, egui::pos2(inner.right() - 32.0, inner.bottom())))
                .layout(Layout::top_down(Align::Min)),
        );
        text.spacing_mut().item_spacing.y = 6.0;
        text.add(egui::Label::new(RichText::new(inst.name()).font(super::fonts::semibold(16.0))).truncate());
        text.set_max_width(inner.width());
        self.loader_line(&mut text, inst, true);
        let started = self.games.values().find(|g| g.folder == folder && g.active()).map(|g| g.started.elapsed());
        text.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            match started {
                Some(t) => {
                    live_dot(ui);
                    let note = format!("Playing \u{00B7} {}", elapsed_text(t));
                    ui.add(egui::Label::new(RichText::new(note).size(FS_META).color(c_accent())).truncate());
                }
                None => {
                    let when =
                        if inst.last_played() > 0 { time_ago(inst.last_played()) } else { "Not played yet".into() };
                    let note = format!("{} \u{00B7} {}", contents_text(inst), when.to_lowercase());
                    ui.add(egui::Label::new(RichText::new(note).size(FS_META).color(c_faint())).truncate());
                }
            }
        });
        // the star at the top right: a favourite's is amber, the others' a faint outline until pointed at
        let mut star = ui.new_child(
            UiBuilder::new()
                .max_rect(egui::Rect::from_min_size(
                    egui::pos2(inner.right() - 28.0, inner.top() - 4.0),
                    vec2(28.0, 28.0),
                ))
                .layout(Layout::top_down(Align::Min)),
        );
        let fav = is_favorite(inst);
        if star_toggle(&mut star, fav, hot).clicked() {
            out.favorite = Some((folder.clone(), !fav));
        }
        // Play and the menu along the bottom
        let mut foot = ui.new_child(
            UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(
                    egui::pos2(inner.left(), inner.bottom() - 32.0),
                    inner.right_bottom(),
                ))
                .layout(Layout::left_to_right(Align::Center)),
        );
        foot.spacing_mut().item_spacing.x = 6.0;
        if play_pill(&mut foot, playing, hot).clicked() {
            act = Some(if playing { Act::Stop(folder.clone()) } else { Act::Play(folder.clone()) });
        }
        if let Some(Some(a)) = kebab_menu(&mut foot, |ui| self.instance_menu(ui, inst)) {
            act = Some(a);
        }
        if act.is_none() && resp.clicked() {
            act = Some(Act::Open(folder));
        }
        act
    }

    /// The Library as a list: one panel with a head that names the columns (and sorts by one when it is clicked) and
    /// a row per instance under it. The head stays put while the rows scroll.
    fn library_rows(
        &self,
        ui: &mut Ui,
        order: &[usize],
        covers: &[egui::TextureHandle],
        sort: Sort,
        out: &mut LibraryOut,
    ) {
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
                    for (n, &i) in order.iter().enumerate() {
                        if n > 0 {
                            ui.add_space(2.0);
                        }
                        if let Some(a) = self.instance_row(ui, &self.instances[i], &covers[n], &cols, width, out) {
                            out.act = Some(a);
                        }
                    }
                    ui.add_space(2.0);
                });
        });
        glass_edge(ui.painter(), panel.response.rect, R_CARD);
    }

    /// One instance as a row of the list: a small square of its cover and the name (with the star of a favourite),
    /// the columns there is room for, and Play and the menu at the end. The row is pointed at as a whole and opens
    /// the instance when clicked.
    fn instance_row(
        &self,
        ui: &mut Ui,
        inst: &Instance,
        cover: &egui::TextureHandle,
        cols: &ListColumns,
        width: f32,
        out: &mut LibraryOut,
    ) -> Option<Act> {
        let mut act = None;
        let (row, resp) = ui.allocate_exact_size(vec2(width, LIST_ROW_H), Sense::click());
        let hot = ui.rect_contains_pointer(row);
        let folder = inst.folder.clone();
        let playing = self.is_playing(&folder);
        let t = motion::hover(ui, resp.id, hot);
        let spans = cols.spans(row.left(), row.right());
        let mid = row.center().y;
        let thumb =
            egui::Rect::from_center_size(egui::pos2(spans.icon + LIST_ICON / 2.0, mid), vec2(LIST_ICON, LIST_ICON));
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
            paint_cover_tex(ui, cover, thumb, CornerRadius::same(R_TAG + 1));
        }
        let badge = egui::Rect::from_min_size(
            thumb.right_bottom() - vec2(LIST_BADGE - 5.0, LIST_BADGE - 5.0),
            vec2(LIST_BADGE, LIST_BADGE),
        );
        self.picture_badge(ui, inst, badge);
        focus_ring(ui, &resp, row, R_CONTROL);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, inst.name()));
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);

        let fav = is_favorite(inst);
        list_cell(ui, row, spans.name, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let name_w = ui.available_width() - 28.0;
            ui.scope(|ui| {
                ui.set_max_width(name_w);
                ui.add(egui::Label::new(RichText::new(inst.name()).font(super::fonts::semibold(FS_BODY))).truncate());
            });
            if star_toggle(ui, fav, hot).clicked() {
                out.favorite = Some((folder.clone(), !fav));
            }
        });
        if let Some(x) = spans.version {
            list_cell(ui, row, x, |ui| {
                let text = RichText::new(inst.mc_version()).font(super::fonts::mono(13.0)).color(c_muted());
                ui.add(egui::Label::new(text).truncate());
            });
        }
        if let Some(x) = spans.loader {
            list_cell(ui, row, x, |ui| {
                let (text, color) = match super::edition::loader_parts(inst, self.horizon_tag()) {
                    Some((kind, ver)) => (super::edition::joined(kind, ver), loader_color(kind)),
                    None => ("Vanilla".to_string(), c_muted()),
                };
                ui.add(egui::Label::new(RichText::new(text).font(super::fonts::mono(13.0)).color(color)).truncate());
            });
        }
        if let Some(x) = spans.mods {
            list_cell(ui, row, x, |ui| {
                let n = if inst.mod_count > 0 { inst.mod_count.to_string() } else { "\u{2014}".to_string() };
                ui.label(RichText::new(n).font(super::fonts::mono(13.0)).color(c_muted()));
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
        // one right edge, and the same one on every tab: the breadcrumb, the banner, the tab bar and what is under
        // them all end at the same x, so Play stays where it is when the tab changes. In a wider window the page
        // keeps to this column instead of stretching; the scroll bar has its own strip at the window's edge.
        let room = ui.available_width();
        let column = (room - ui.spacing().scroll.allocated_width()).min(INSTANCE_COLUMN);
        // a Horizon instance has no Mods tab (whichever tab was open for the instance before)
        if !super::edition::has_tab(&inst, self.tab) {
            self.tab = InstTab::Overview;
        }
        // the Mods tab lists the mods, Resource packs the packs and the shaders (only the packs for Horizon)
        self.mods_follow_tab();
        if crate::horizon::is_horizon(&inst) {
            self.mods_keep_to_packs();
        }
        ui.scope(|ui| {
            ui.set_max_width(column);
            self.instance_header(ui, &inst, &mut action);
        });
        match self.tab {
            InstTab::Overview => self.tab_overview(ui, &inst),
            InstTab::Mods | InstTab::ResourcePacks => {
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

    /// The breadcrumb, the cover banner with Play and the menus, and the tabs of an instance's page, with the search
    /// and Add of the Mods and Resource packs tabs at the right end of the tabs. In a short window the breadcrumb is
    /// left out (the banner names the instance, and Library in the sidebar goes back), so the lists start higher.
    fn instance_header(&mut self, ui: &mut Ui, inst: &Instance, action: &mut Option<Act>) {
        let lists = matches!(self.tab, InstTab::Mods | InstTab::ResourcePacks);
        if ui.ctx().content_rect().height() >= CRUMB_WINDOW_H {
            ui.horizontal(|ui| {
                ui.set_height(32.0);
                ui.spacing_mut().item_spacing.x = 2.0;
                if back_link(ui, "Library").clicked() {
                    self.page = Page::Library;
                }
                ui.label(RichText::new("\u{203A}").size(FS_BODY).color(c_faint()));
                ui.add_space(6.0);
                ui.add(egui::Label::new(RichText::new(inst.name()).font(super::fonts::medium(13.0))).truncate());
            });
            ui.add_space(6.0 - ui.spacing().item_spacing.y);
        }
        self.instance_banner(ui, inst, action);
        ui.add_space(6.0 - ui.spacing().item_spacing.y);
        // a dot on Settings while changes there weren't saved
        let unsaved = self.unsaved_draft(&inst.folder);
        tab_bar(ui, |ui| {
            for (t, label) in [
                (InstTab::Overview, "Overview"),
                (InstTab::Mods, "Mods"),
                (InstTab::ResourcePacks, "Resource packs"),
                (InstTab::Logs, "Logs"),
                (InstTab::Settings, "Settings"),
            ] {
                if !super::edition::has_tab(inst, t) {
                    continue;
                }
                let mark = (t == InstTab::Settings).then_some(unsaved);
                let r = tab_marked(ui, label, self.tab == t, mark);
                let r = if mark == Some(true) { r.on_hover_text("Changes here aren't saved yet") } else { r };
                if r.clicked() {
                    if self.tab != t && !r.clicked_by(egui::PointerButton::Primary) {
                        // picked from the keyboard: the banner changes at once, not under a moving focus
                        super::workspace::snap_banner(ui.ctx());
                    }
                    self.tab = t;
                    if t == InstTab::Mods {
                        self.mods_cache = None;
                    }
                }
            }
            if lists {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    self.mods_header_tools(ui, inst);
                });
            }
        });
        // under the tabs the Overview has room to breathe; the other tabs start their toolbar close under them
        let gap = if self.tab == InstTab::Overview { 16.0 } else { 10.0 };
        ui.add_space(gap - 2.0 * ui.spacing().item_spacing.y);
    }

    fn tab_overview(&mut self, ui: &mut Ui, inst: &Instance) {
        let mut change = false;
        // as wide as the header above it, so the page has one right edge: the strip of the scroll bar is left out
        // whether the bar shows or not (it only shows when the page is longer than the window)
        let column = (ui.available_width() - ui.spacing().scroll.allocated_width()).min(FORM_COLUMN);
        ScrollArea::vertical().id_salt("instance-overview").auto_shrink(false).show(ui, |ui| {
            ui.set_max_width(column);
            ui.spacing_mut().item_spacing.y = 0.0;
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
            // what this instance is, on four small cards. In a wide window what it is about (with its folder and
            // backups at the top) sits beside the days it was played; in a narrower one the folder and the backups
            // get a row of their own under the cards, so they are there without scrolling, then the days, then the rest
            self.overview_stats(ui, inst);
            ui.add_space(OVERVIEW_GAP);
            if ui.ctx().content_rect().width() >= OVERVIEW_WIDE {
                let half = ((ui.available_width() - OVERVIEW_GAP) / 2.0).floor();
                two_columns(
                    ui,
                    half,
                    OVERVIEW_GAP,
                    self,
                    |ui, this| this.overview_files(ui, inst),
                    |ui, this| this.overview_history(ui, inst),
                );
            } else {
                self.overview_quick_actions(ui, inst);
                ui.add_space(OVERVIEW_GAP);
                self.overview_history(ui, inst);
                ui.add_space(OVERVIEW_GAP);
                self.overview_files(ui, inst);
            }
            ui.add_space(OVERVIEW_GAP);
            if crate::horizon::is_horizon(inst) {
                // what it is and who makes it, then joining a server; the Java helpers have nothing to say here
                self.horizon_about_card(ui);
                ui.add_space(OVERVIEW_GAP);
                card(ui, |ui| self.overview_join(ui, inst));
            } else {
                self.overview_play(ui, inst, &mut change);
            }
            ui.add_space(16.0);
        });
        if change {
            // the Performance group is on the first tab of the settings, whichever one was open last
            if let Some(d) = self.inst_draft.as_mut().filter(|d| d.folder == inst.folder) {
                d.tab = 0;
            }
            self.tab = InstTab::Settings;
        }
    }

    /// The overview's cards about playing: the performance switches, and joining a server on start. Side by side, or
    /// one under the other when the page is narrow.
    fn overview_play(&mut self, ui: &mut Ui, inst: &Instance, change: &mut bool) {
        let performance = |ui: &mut Ui, this: &mut Self, change: &mut bool| {
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
            head.finish_whole(ui);
            ui.add_space(2.0);
            ui.add(egui::Label::new(RichText::new(this.speed_summary(inst)).size(FS_BODY).color(c_muted())).wrap());
        };
        let join = |ui: &mut Ui, this: &mut Self| this.overview_join(ui, inst);
        // one card: the two halves side by side with a line between them, or one over the other when it is narrow
        card(ui, |ui| {
            let room = ui.available_width();
            if room >= OVERVIEW_SPLIT * 0.7 {
                let half = ((room - 2.0 * OVERVIEW_GAP) / 2.0).floor();
                let mut state = (&mut *self, change);
                two_columns(
                    ui,
                    half,
                    2.0 * OVERVIEW_GAP,
                    &mut state,
                    |ui, (this, change)| performance(ui, this, change),
                    |ui, (this, _)| join(ui, this),
                );
                let r = ui.min_rect();
                let x = r.left() + half + OVERVIEW_GAP;
                ui.painter().vline(x, r.top() + 4.0..=r.bottom() - 4.0, Stroke::new(1.0, c_rule()));
            } else {
                performance(ui, self, change);
                ui.add_space(12.0);
                rule(ui);
                ui.add_space(12.0);
                join(ui, self);
            }
        });
    }

    /// Joining a server on start: an address, and the button that plays and joins it.
    fn overview_join(&mut self, ui: &mut Ui, inst: &Instance) {
        block_title(ui, "Join a server on start");
        small_muted(ui, "An address like play.example.com, or play.example.com:25566");
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            let mut s = std::mem::take(&mut self.quick_server);
            search_field(ui, &mut s, "Server address", (ui.available_width() - 150.0).clamp(150.0, 280.0));
            self.quick_server = s;
            let ready = !self.quick_server.trim().is_empty() && !self.is_playing(&inst.folder);
            // neutral: the one green button of the page is Play
            if ui.add_enabled_ui(ready, |ui| secondary_icon(ui, Icon::Play, "Play and join")).inner.clicked() {
                let server = self.quick_server.trim().to_string();
                self.play_checked(&inst.folder, super::PendingPlay::Join(server));
            }
        });
    }

    /// The row of a narrow Overview right under the stat cards: the instance's folder and its backups, the things
    /// most often opened from here, with what the backups are at the end.
    fn overview_quick_actions(&mut self, ui: &mut Ui, inst: &Instance) {
        let backups = self.backups_summary(inst);
        let mut manage = false;
        ui.horizontal(|ui| {
            ui.set_height(34.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            if secondary_icon(ui, Icon::Folder, "Open folder").on_hover_text(inst.root.display().to_string()).clicked()
            {
                self.open_path(&inst.root);
            }
            if secondary_icon(ui, Icon::Package, "Manage backups").clicked() {
                manage = true;
            }
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(&backups).size(FS_META).color(c_muted())).truncate());
        });
        if manage {
            self.manage_backups(&inst.folder);
        }
    }

    /// "About this instance": the game directory and its backups first (so their buttons are near the top of the
    /// card), then its description.
    fn overview_files(&mut self, ui: &mut Ui, inst: &Instance) {
        let mut edit = false;
        let mut manage = false;
        let backups = self.backups_summary(inst);
        card(ui, |ui| {
            let mut head = SplitRow::new(ui, egui::Id::unique("overview-about"), BLOCK_TITLE_H);
            head.left(ui, |ui| {
                ui.label(RichText::new("About this instance").size(FS_SECTION).semibold());
            });
            head.right(ui, |ui| {
                if ghost(ui, "Edit description").clicked() {
                    edit = true;
                }
            });
            head.finish_whole(ui);
            // the game directory: the whole path when it is pointed at, cut at the edge otherwise
            let path = inst.root.display().to_string();
            about_row(ui, Icon::Folder, "Game directory", &path, true, egui::Id::unique("about-dir"), |ui| {
                if secondary(ui, "Open").clicked() {
                    self.open_path(&inst.root);
                }
                if ghost(ui, "Saves").on_hover_text("Open the folder of its worlds").clicked() {
                    // a new instance has no saves folder until its first world
                    let saves = inst.root.join("saves");
                    let _ = std::fs::create_dir_all(&saves);
                    self.open_path(&saves);
                }
                #[cfg(feature = "converter")]
                self.add_bedrock_world_button(ui, inst);
            });
            rule(ui);
            about_row(ui, Icon::Package, "Backups", &backups, false, egui::Id::unique("about-backups"), |ui| {
                if secondary(ui, "Manage").clicked() {
                    manage = true;
                }
            });
            rule(ui);
            ui.add_space(12.0);
            // the description, when it says more than the version and loader in the banner already do (a pack's own
            // words, or what the player wrote)
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
                    max_rows: 4,
                    break_anywhere: false,
                    overflow_character: Some('\u{2026}'),
                };
                ui.add(egui::Label::new(job)).on_hover_text(desc);
            } else {
                ui.label(RichText::new("No description yet.").size(FS_BODY).color(c_faint()));
            }
        });
        if edit {
            self.desc_edit = Some((inst.folder.clone(), inst.file.launcher.description.clone()));
        }
        if manage {
            self.manage_backups(&inst.folder);
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
                ui.spacing_mut().item_spacing.x = 6.0;
                for k in &sessions {
                    let label = self.games.get(k).map(|g| g.player.clone()).unwrap_or_default();
                    if chip(ui, &label, *k == folder).clicked() {
                        self.log_session = Some(k.clone());
                    }
                }
            });
            ui.add_space(6.0);
        }
        let Some(phase) = self.games.get(&folder).map(|g| g.phase) else { return };
        let (mut copy, mut clear) = (false, false);
        let filter = self.log_filter.trim().to_lowercase();
        let level_id = egui::Id::unique("instance-log-level");
        let mut level: LogLevels = ui.ctx().data(|d| d.get_temp(level_id)).unwrap_or_default();
        let mut wrap = self.settings.log_wrap;
        // the levels on the left; the filter and what can be done with the log on the right
        let (errors, warnings) = self.games.get(&folder).map_or((0, 0), |g| {
            let n = |l: Level| g.lines.iter().filter(|x| x.level == l).count();
            (n(Level::Error), n(Level::Warn))
        });
        let mut st = (&mut *self, &mut level, &mut wrap, &mut copy, &mut clear);
        super::instance_menu::split_or_stack(
            ui,
            egui::Id::unique("instance-log-bar"),
            34.0,
            &mut st,
            |ui, (_, level, _, _, _)| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (which, label) in [
                    (LogLevels::All, "All".to_string()),
                    (LogLevels::Errors, format!("Errors ({errors})")),
                    (LogLevels::Warnings, format!("Warnings ({warnings})")),
                ] {
                    let on = **level == which;
                    if chip(ui, &label, on).clicked() {
                        **level = if on { LogLevels::All } else { which };
                    }
                }
            },
            |ui, (this, _, wrap, copy, clear)| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let mut f = std::mem::take(&mut this.log_filter);
                search_field(ui, &mut f, "Filter lines", 190.0);
                this.log_filter = f;
                if chip(ui, "Wrap", **wrap).on_hover_text("Break long lines instead of scrolling sideways").clicked() {
                    **wrap = !**wrap;
                }
                **copy = secondary(ui, "Copy all").clicked();
                **clear = ghost(ui, "Clear").clicked();
            },
        );
        ui.ctx().data_mut(|d| d.insert_temp(level_id, level));
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
        let shown = log_lines_shown(&game.lines, &filter, level);
        let count = shown.as_ref().map_or(game.lines.len(), Vec::len);
        ui.add_space(10.0);
        log_panel(ui, |ui| {
            // what the game is doing and how many lines there are, along the top of the panel
            let (status, dot) = match phase {
                Phase::Preparing => ("Getting ready".to_string(), c_accent()),
                Phase::Running => (format!("Running \u{00B7} {}", elapsed_text(game.started.elapsed())), c_accent()),
                Phase::Exited(Some(0)) => ("Closed normally".to_string(), c_faint()),
                Phase::Exited(Some(c)) => (format!("Closed with code {c}"), c_danger()),
                Phase::Exited(None) => ("Closed".to_string(), c_faint()),
                Phase::Failed => ("Couldn't start".to_string(), c_danger()),
            };
            let mut top = SplitRow::new(ui, egui::Id::unique("instance-log-top"), 22.0);
            top.left(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if matches!(phase, Phase::Preparing | Phase::Running) {
                    live_dot(ui);
                } else {
                    let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(rect.center(), 4.0, dot);
                }
                ui.add(egui::Label::new(RichText::new(status).font(super::fonts::medium(FS_META))).truncate());
                ui.label(RichText::new(&game.player).size(FS_META).color(c_faint()));
            });
            top.right(ui, |ui| {
                let total = game.lines.len();
                let lines = if count == total {
                    if total == 1 { "1 line".to_string() } else { format!("{total} lines") }
                } else {
                    format!("{count} of {total} lines")
                };
                ui.label(RichText::new(lines).font(super::fonts::mono(FS_META - 1.0)).color(c_faint()));
            });
            top.finish_whole(ui);
            ui.add_space(6.0);
            rule(ui);
            ui.add_space(8.0);
            let height = ui.available_height().max(120.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + 1.0;
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
                            log_row(ui, line(i), true);
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
                            log_row(ui, line(i), false);
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

/// The space between the cards of an instance's overview.
const OVERVIEW_GAP: f32 = 14.0;
/// From this height of the window on, an instance's page has its breadcrumb over the banner.
const CRUMB_WINDOW_H: f32 = 660.0;
/// From this width of the window on, "About this instance" sits beside the play history.
const OVERVIEW_WIDE: f32 = 1100.0;

/// A row of "About this instance": an icon on a tile, a title over a detail (in the monospace face when `mono`, cut
/// at the edge with the whole of it on hover), and the buttons at the right end, or under the title when the card is
/// too narrow for both on one line.
fn about_row(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    detail: &str,
    mono: bool,
    id: egui::Id,
    buttons: impl FnOnce(&mut Ui),
) {
    let text = |ui: &mut Ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (tile, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
        ui.painter().rect_filled(tile, CornerRadius::same(R_CONTROL), c_field());
        ui.painter().rect_stroke(
            tile,
            CornerRadius::same(R_CONTROL),
            Stroke::new(1.0, c_border()),
            egui::StrokeKind::Inside,
        );
        super::icons::paint(ui.painter(), icon, tile.center(), 15.0, c_muted());
        // a block of a known height, so the row's centring puts it level with the tile
        let w = ui.available_width();
        ui.allocate_ui_with_layout(vec2(w, 36.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(w);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add(egui::Label::new(RichText::new(title).font(super::fonts::medium(FS_BODY))).truncate());
            let text = if mono {
                RichText::new(detail).font(super::fonts::mono(FS_META)).color(c_muted())
            } else {
                RichText::new(detail).size(FS_META).color(c_muted())
            };
            ui.add(egui::Label::new(text).truncate()).on_hover_text(detail);
        });
    };
    // the buttons' width is remembered from the last frame, as a SplitRow does
    if ui.available_width() >= remembered_width(ui, id, 240.0) + 16.0 + ABOUT_TEXT_MIN {
        let mut row = SplitRow::new(ui, id, 62.0);
        row.left(ui, text);
        row.right(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            buttons(ui);
        });
        row.finish_whole(ui);
    } else {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.set_height(36.0);
            text(ui);
        });
        ui.add_space(6.0);
        let used = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.add_space(46.0);
                let left = ui.cursor().left();
                buttons(ui);
                ui.min_rect().right() - left
            })
            .inner;
        remember_width(ui, id, used);
        ui.add_space(12.0);
    }
}

/// The least room the title and the detail of a row of "About this instance" keep beside its buttons.
const ABOUT_TEXT_MIN: f32 = 200.0;

/// One line of a game's log: monospace in the colour of its level, and a warning or an error on a faint band of that
/// colour with a mark at its start, so they stand out in a long log.
pub(super) fn log_row(ui: &mut Ui, line: &crate::launch::LogLine, wrap: bool) {
    let band = ui.painter().add(egui::Shape::Noop);
    let label = egui::Label::new(log_text(line));
    let r = ui.add(if wrap { label.wrap() } else { label.extend() });
    let tint = match line.level {
        Level::Error => Some(c_danger()),
        Level::Warn => Some(c_warn()),
        _ => None,
    };
    if let Some(c) = tint {
        let right = r.rect.right().max(ui.clip_rect().right());
        let rect = egui::Rect::from_x_y_ranges(r.rect.left() - 8.0..=right, r.rect.y_range()).expand2(vec2(0.0, 0.5));
        ui.painter().set(band, egui::Shape::rect_filled(rect, CornerRadius::same(2), c.gamma_multiply(0.08)));
        let mark = egui::Rect::from_x_y_ranges(rect.left()..=rect.left() + 2.0, rect.y_range());
        ui.painter().rect_filled(mark, CornerRadius::same(1), c.gamma_multiply(0.7));
    }
}

/// What the Library shows: every instance, those of one loader (`Vanilla` for plain Minecraft), the favourites, or
/// those running.
#[derive(Clone, Debug, Default, PartialEq)]
enum Show {
    #[default]
    All,
    Loader(String),
    Favorites,
    Running,
}

impl Show {
    fn admits(&self, inst: &Instance, running: bool) -> bool {
        match self {
            Show::All => true,
            Show::Loader(kind) => loader_kind(inst) == kind,
            Show::Favorites => is_favorite(inst),
            Show::Running => running,
        }
    }
}

/// Where the Library's filter is kept while the launcher runs (it starts at All again next time).
const SHOW_KEY: &str = "library-show";

fn library_show(ctx: &egui::Context) -> Show {
    ctx.data(|d| d.get_temp::<Show>(egui::Id::unique(SHOW_KEY))).unwrap_or_default()
}

/// The loader of an instance as the filters name it: `Fabric`, `Forge`, …, `Horizon` or `Vanilla`.
fn loader_kind(inst: &Instance) -> &str {
    super::edition::loader_kind(inst).unwrap_or("Vanilla")
}

/// The order loaders are listed and grouped in: the known ones by name, then any other, plain Minecraft last.
fn loader_rank(kind: &str) -> (u8, String) {
    let n = match kind {
        "Fabric" => 0,
        "Forge" => 1,
        "NeoForge" => 2,
        "Quilt" => 3,
        "Horizon" => 4,
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

    /// The sorting in a word or two, after "Sort:" on its button.
    fn short(self) -> &'static str {
        match self {
            Sort::Name => "Name",
            Sort::LastPlayed => "Last played",
            Sort::MostPlayed => "Most played",
            Sort::Added => "Recently added",
            Sort::Version => "Newest Minecraft",
            Sort::Loader => "Loader",
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
    /// A star was clicked: make this instance (its folder) a favourite, or not.
    favorite: Option<(String, bool)>,
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

/// Whether an instance is one of the player's favourites (`favorite` in its file's fields).
fn is_favorite(inst: &Instance) -> bool {
    inst.file.launcher.extras.get("favorite").and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Mark an instance as a favourite in its file's fields, or not (which removes the field). The caller saves the file.
fn set_favorite(inst: &mut Instance, on: bool) {
    if on {
        inst.file.launcher.extras.insert("favorite".into(), serde_json::Value::Bool(true));
    } else {
        inst.file.launcher.extras.remove("favorite");
    }
}

/// What a card says an instance holds: its mods, or for a Horizon instance (which has none of BlockForge's to count)
/// what it is.
fn contents_text(inst: &Instance) -> String {
    if crate::horizon::is_horizon(inst) { "Rust rewrite".to_string() } else { mods_text(inst.mod_count) }
}

/// "No mods", "1 mod", "12 mods".
fn mods_text(n: usize) -> String {
    match n {
        0 => "No mods".to_string(),
        1 => "1 mod".to_string(),
        n => format!("{n} mods"),
    }
}

/// "1.21.1 · Fabric 0.20.0" in the monospace face at `size`: the version in grey, the loader in its own colour (with
/// its version when `version` is true), "Vanilla" for plain Minecraft. A Horizon instance has "Horizon" there, with
/// the installed build (`build`) as its version.
fn loader_job(inst: &Instance, size: f32, version: bool, build: Option<&str>) -> egui::text::LayoutJob {
    let part = |color: Color32| egui::TextFormat { font_id: super::fonts::mono(size), color, ..Default::default() };
    let mut job = egui::text::LayoutJob::default();
    job.append(inst.mc_version(), 0.0, part(c_muted()));
    job.append(" \u{00B7} ", 0.0, part(c_faint()));
    match super::edition::loader_parts(inst, build) {
        Some((kind, ver)) if version => job.append(&super::edition::joined(kind, ver), 0.0, part(loader_color(kind))),
        Some((kind, _)) => job.append(kind, 0.0, part(loader_color(kind))),
        None => job.append("Vanilla", 0.0, part(c_muted())),
    }
    job
}

/// How many shortenings [`banner_meta`] has.
const BANNER_META_LEVELS: usize = 3;

/// "1.21.1 · Fabric 0.20.0 · 7 mods" in light monospace type, for the banner's cover. `level` 0 is all of it; each next
/// level leaves out one more part from the end (the mods, then the loader's version). A Horizon instance has
/// "Horizon" and its build (`build`) for the loader, and no mods.
fn banner_meta(inst: &Instance, level: usize, build: Option<&str>) -> egui::text::LayoutJob {
    let part = |a: u8| egui::TextFormat {
        font_id: super::fonts::mono(14.0),
        color: Color32::from_white_alpha(a),
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    job.append(inst.mc_version(), 0.0, part(225));
    job.append(" \u{00B7} ", 0.0, part(120));
    match super::edition::loader_parts(inst, build) {
        Some((kind, ver)) if level < 2 => job.append(&super::edition::joined(kind, ver), 0.0, part(225)),
        Some((kind, _)) => job.append(kind, 0.0, part(225)),
        None => job.append("Vanilla", 0.0, part(225)),
    }
    if level == 0 && !crate::horizon::is_horizon(inst) {
        job.append(" \u{00B7} ", 0.0, part(120));
        job.append(&mods_text(inst.mod_count), 0.0, part(225));
    }
    job
}

/// A shade of black over the left of `rect` (whose corners are rounded by `radius`): `alpha` at the left edge, fading
/// evenly to nothing `reach` points in.
fn shade_left(p: &egui::Painter, rect: egui::Rect, radius: u8, alpha: u8, reach: f32) {
    let r = radius as f32;
    let at =
        |x: f32| Color32::from_black_alpha((alpha as f32 * (1.0 - (x - rect.left()) / reach).clamp(0.0, 1.0)) as u8);
    // the band along the rounded left edge is one shade (it is narrow), the rest strips that fade
    let mut edge = Vec::new();
    for i in 0..=8 {
        let a = std::f32::consts::FRAC_PI_2 * i as f32 / 8.0;
        edge.push(egui::pos2(rect.left() + r - r * a.sin(), rect.bottom() - r + r * a.cos()));
    }
    for i in 0..=8 {
        let a = std::f32::consts::FRAC_PI_2 * i as f32 / 8.0;
        edge.push(egui::pos2(rect.left() + r - r * a.cos(), rect.top() + r - r * a.sin()));
    }
    p.add(egui::Shape::convex_polygon(edge, at(rect.left() + r / 2.0), Stroke::NONE));
    let mut mesh = egui::Mesh::default();
    let steps = 24;
    for i in 0..=steps {
        let x = rect.left() + r + (reach - r) * i as f32 / steps as f32;
        mesh.colored_vertex(egui::pos2(x, rect.top()), at(x));
        mesh.colored_vertex(egui::pos2(x, rect.bottom()), at(x));
        if i > 0 {
            let k = 2 * i as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k, k + 1);
        }
    }
    p.add(egui::Shape::mesh(mesh));
}

/// Paint a cover's texture filling `rect`, cropped to its shape rather than stretched.
fn paint_cover_tex(ui: &Ui, tex: &egui::TextureHandle, rect: egui::Rect, radius: CornerRadius) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let [w, h] = tex.size();
    egui::Image::from_texture((tex.id(), vec2(w as f32, h as f32)))
        .uv(super::workbench::cover_uv((w, h), rect.size()))
        .corner_radius(radius)
        .paint_at(ui, rect);
}

/// The height of the cover on a Home tile: low, so the tiles' names and the news under them show in a short window.
fn tile_cover_h(_width: f32) -> f32 {
    96.0
}

/// The height of Home's banner `width` wide in a window `window_h` high: a third of its width, but never more than
/// 208 in a window 600 high, 248 in one 700 high, and [`BANNER_MAX_H`] in a tall one.
fn hero_height(width: f32, window_h: f32) -> f32 {
    let cap = (208.0 + 0.4 * (window_h - 600.0)).clamp(208.0, BANNER_MAX_H);
    (width * 0.31).clamp(BANNER_MIN_H, cap).round()
}

/// The grid or list switch of the Library: two small squares with a picture each, the chosen one raised. Returns
/// `Some(true)` when the list was chosen, `Some(false)` for the grid.
fn view_switch(ui: &mut Ui, list: bool) -> Option<bool> {
    let (outer, _) = ui.allocate_exact_size(vec2(2.0 * 32.0 + 6.0, 34.0), Sense::hover());
    ui.painter().rect(
        outer,
        CornerRadius::same(R_CONTROL),
        c_field(),
        Stroke::new(1.0, c_border()),
        egui::StrokeKind::Inside,
    );
    let mut picked = None;
    for (i, (label, is_list)) in [("Grid", false), ("List", true)].into_iter().enumerate() {
        let rect = egui::Rect::from_min_size(outer.min + vec2(3.0 + 32.0 * i as f32, 3.0), vec2(32.0, 28.0));
        let resp = ui.interact(rect, ui.scope_id().with(("library-view", i)), Sense::click());
        let on = list == is_list;
        if on {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(R_CONTROL - 3),
                if is_dark() { c_active() } else { c_card() },
            );
        }
        let t = motion::hover(ui, resp.id, on || resp.hovered());
        let color = motion::mix(c_faint(), c_text(), t);
        let c = rect.center();
        if is_list {
            for dy in [-4.5, 0.0, 4.5] {
                ui.painter().circle_filled(c + vec2(-5.5, dy), 1.3, color);
                ui.painter().hline(c.x - 2.5..=c.x + 6.5, c.y + dy, Stroke::new(1.6, color));
            }
        } else {
            for (dx, dy) in [(-3.5, -3.5), (3.5, -3.5), (-3.5, 3.5), (3.5, 3.5)] {
                let cell = egui::Rect::from_center_size(c + vec2(dx, dy), vec2(5.5, 5.5));
                ui.painter().rect_filled(cell, CornerRadius::same(1), color);
            }
        }
        focus_ring(ui, &resp, rect, R_CONTROL - 3);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, label));
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(label).clicked() && !on {
            picked = Some(is_list);
        }
    }
    picked
}

/// A five-pointed star centred on `c`, `r` from its middle to a point: filled, or as an outline.
fn paint_star(p: &egui::Painter, c: egui::Pos2, r: f32, color: Color32, filled: bool) {
    let points: Vec<egui::Pos2> = (0..10)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            let reach = if i % 2 == 0 { r } else { r * 0.46 };
            c + vec2(a.cos(), a.sin()) * reach
        })
        .collect();
    if filled {
        // a star is not convex: it is filled as a fan of triangles from its middle
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(c, color);
        for p in &points {
            mesh.colored_vertex(*p, color);
        }
        for i in 0..10u32 {
            mesh.add_triangle(0, 1 + i, 1 + (i + 1) % 10);
        }
        p.add(egui::Shape::mesh(mesh));
    }
    p.add(egui::Shape::closed_line(points, Stroke::new(if filled { 1.0 } else { 1.4 }, color)));
}

/// The star that makes an instance a favourite: amber when it is one; otherwise a faint outline while its card or row
/// is pointed at (`shown`), and nothing at rest, so a page of instances isn't a page of stars.
fn star_toggle(ui: &mut Ui, on: bool, shown: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    let t = motion::hover(ui, resp.id, resp.hovered());
    let seen = motion::toward(ui.ctx(), resp.id.with("seen"), on || shown || resp.has_focus(), motion::FAST);
    if ui.is_rect_visible(rect) {
        if t > 0.0 {
            ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), c_hover().gamma_multiply(t));
        }
        let color = if on { c_secondary() } else { motion::mix(c_faint(), c_secondary(), t).gamma_multiply(seen) };
        if seen > 0.0 {
            paint_star(ui.painter(), rect.center() + vec2(0.0, 0.5), 7.5, color, on);
        }
    }
    focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, "Favourite"));
    let tip = if on { "Remove from favourites" } else { "Add to favourites" };
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

/// Play (Stop while it runs) with its name, on a card of the Library: neutral at rest, the brand gradient fading in as
/// the card is pointed at, like [`play_chip`].
fn play_pill(ui: &mut Ui, playing: bool, card_hot: bool) -> egui::Response {
    let text = if playing { "Stop" } else { "Play" };
    let galley = ui.painter().layout_no_wrap(text.to_string(), super::fonts::semibold(13.0), Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 46.0, 32.0), Sense::click());
    let hot = resp.hovered() || resp.is_pointer_button_down_on();
    let t = motion::hover(ui, resp.id, !playing && (hot || card_hot));
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let round = CornerRadius::same(R_CONTROL);
        let fg = if playing {
            let bg = if hot { c_danger_hover() } else { c_danger_bg() };
            p.rect(rect, round, bg, Stroke::new(1.0, c_danger_line()), egui::StrokeKind::Inside);
            c_danger()
        } else {
            p.rect(
                rect,
                round,
                c_field(),
                Stroke::new(1.0, c_control().gamma_multiply(1.0 - t)),
                egui::StrokeKind::Inside,
            );
            if t > 0.0 {
                motion::gradient_rect(p, rect, R_CONTROL, c_accent().gamma_multiply(t), c_accent2().gamma_multiply(t));
            }
            motion::mix(c_text(), c_on_accent(), t)
        };
        let left = rect.center().x - (galley.size().x + 20.0) / 2.0;
        let icon = if playing { Icon::Stop } else { Icon::Play };
        icons::paint(p, icon, egui::pos2(left + 6.0, rect.center().y), 12.0, fg);
        p.galley(egui::pos2(left + 20.0, rect.center().y - galley.size().y / 2.0), galley, fg);
    }
    focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A square button with three dots over a cover: dark glass with a light rim, so it reads on any picture.
fn cover_button(ui: &mut Ui, side: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let t = motion::hover(ui, resp.id, resp.hovered() || open);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let round = CornerRadius::same(R_CONTROL + 2);
        let fill = Color32::from_black_alpha((105.0 + 45.0 * t) as u8);
        let rim = Color32::from_white_alpha((46.0 + 50.0 * t) as u8);
        p.rect(rect, round, fill, Stroke::new(1.0, rim), egui::StrokeKind::Inside);
        for dx in [-6.0, 0.0, 6.0] {
            p.circle_filled(rect.center() + vec2(dx, 0.0), 2.1, ON_COVER);
        }
    }
    focus_ring(ui, &resp, rect, R_CONTROL + 2);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "More ways to play"));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Words over a cover: light in both looks, since a cover is dark enough under its shade in either.
const ON_COVER: Color32 = Color32::from_rgb(0xF4, 0xF7, 0xFA);
/// The brand colour over a cover: the bright one of the dark look, in both looks.
const COVER_ACCENT: Color32 = Color32::from_rgb(0x2D, 0xD4, 0xAA);
/// How tall Home's banner gets, from a narrow window to a wide one.
const BANNER_MIN_H: f32 = 200.0;
const BANNER_MAX_H: f32 = 330.0;
/// The instance's own picture over the corner of the banner, a Home tile, a Library card and a list row.
const BANNER_BADGE: f32 = 52.0;
const TILE_BADGE: f32 = 30.0;
const LIB_BADGE: f32 = 28.0;
const LIST_BADGE: f32 = 16.0;
/// The tiles of Home: the least width of one, the gap between them, and the height under the cover.
const HOME_TILE_MIN: f32 = 196.0;
const HOME_TILE_GAP: f32 = 14.0;
const TILE_INFO_H: f32 = 84.0;
/// The cards of the Library's grid: their height, the air inside them, their least width and the gap between them.
const LIB_CARD_H: f32 = 148.0;
const LIB_CARD_PAD: f32 = 12.0;
const LIB_CARD_MIN: f32 = 330.0;
const LIB_CARD_GAP: f32 = 16.0;

/// From this width on, Home puts the news beside the instances instead of under them.
const HOME_SPLIT: f32 = 1100.0;
/// The gap between the columns of Home and of an instance's overview.
const HOME_GAP: f32 = 28.0;
/// The space between the banner, the instances and the news of Home.
const HOME_SECTION_GAP: f32 = 24.0;
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
        || (crate::horizon::is_horizon(inst)
            && desc == format!("{}: {} ({mc})", crate::horizon::NAME, crate::horizon::TAGLINE))
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
    fn a_favourite_is_kept_in_the_instance_file_and_filtered_on() {
        let root = std::env::temp_dir().join(format!("bf-library-favourite-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut a = inst("Alpha", "1.21.1", Some("Fabric"), 100, 9);
        a.root = root.clone();
        assert!(!is_favorite(&a));
        set_favorite(&mut a, true);
        assert!(is_favorite(&a));
        a.save().unwrap();
        // it is in the launcher's part of the file, next to the other fields BlockForge keeps there
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("instance.json")).unwrap()).unwrap();
        assert_eq!(written["launcher"]["favorite"], serde_json::Value::Bool(true));
        let mut back = crate::instances::load_one(&root).expect("the instance reads back");
        assert!(is_favorite(&back));
        assert!(Show::Favorites.admits(&back, false));
        assert!(!Show::Favorites.admits(&inst("beta", "1.20.1", None, 0, 0), false));
        // no longer a favourite: the field goes, rather than staying as false
        set_favorite(&mut back, false);
        back.save().unwrap();
        let written = std::fs::read_to_string(root.join("instance.json")).unwrap();
        assert!(!written.contains("\"favorite\""), "{written}");
        assert!(!is_favorite(&crate::instances::load_one(&root).unwrap()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn homes_banner_leaves_room_under_it_in_a_short_window() {
        // the page of a 1200 x 700 window is about 910 wide, of a 900 x 600 one about 770
        assert_eq!(hero_height(910.0, 700.0), 248.0);
        assert_eq!(hero_height(770.0, 600.0), 208.0);
        assert_eq!(hero_height(770.0, 560.0), 208.0);
        // a tall, wide window gets the whole banner, a narrow one the lowest
        assert_eq!(hero_height(1400.0, 1000.0), BANNER_MAX_H);
        assert_eq!(hero_height(500.0, 900.0), BANNER_MIN_H);
        for (w, h) in [(600.0, 600.0), (900.0, 650.0), (1200.0, 700.0)] {
            assert!(hero_height(w, h) <= 248.0, "{w} x {h}");
        }
    }

    #[test]
    fn every_sorting_has_a_short_name_for_its_button() {
        for s in Sort::ALL {
            assert!(!s.short().is_empty() && s.short().len() < s.label().len().max(8), "{s:?}");
        }
        assert_eq!(mods_text(0), "No mods");
        assert_eq!(mods_text(1), "1 mod");
        assert_eq!(mods_text(12), "12 mods");
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
