//! The instance workspace, the page an instance opens on: its cover banner with Play and the menus, and the pieces of
//! its tabs that are new in 5.0 (the stat cards and the play history of the Overview, the cover picker of its
//! settings).

use egui::{Align, Color32, CornerRadius, Layout, Rect, Response, RichText, Sense, Stroke, Ui, UiBuilder, pos2, vec2};

use super::covers::CoverTheme;
use super::icons::{self, Icon};
use super::instance_menu::{Act, menu_items};
use super::motion;
use super::theme::*;
use super::widgets::*;
use super::{App, InstTab, Phase, workbench};
use crate::instances::Instance;

/// The banner's height on the Overview, and on the other tabs, where the lists under it want the room.
const BANNER_TALL: f32 = 196.0;
const BANNER_SHORT: f32 = 92.0;
/// The air inside the banner.
const BANNER_PAD: f32 = 22.0;
/// From this width on, the short banner has room for the performance switches beside Play.
const SHORT_ROOMY: f32 = 760.0;
/// The part of the Play button that opens its menu.
const MENU_PART: f32 = 38.0;

/// Text written over a cover. Its left side is always faded dark (see `fade_left`), so the text is light in both
/// looks.
fn on_cover(alpha: u8) -> Color32 {
    Color32::from_white_alpha(alpha)
}

/// "1.21.1 · Fabric 0.20.0 · 7 mods" in the monospace face, for the banner. `level` 0 is all of it; each next level
/// leaves out one more part from the end (the mods, then the loader's version).
fn banner_meta(inst: &Instance, level: usize) -> egui::text::LayoutJob {
    let part = |color: Color32| egui::TextFormat { font_id: super::fonts::mono(13.0), color, ..Default::default() };
    let sep = "  \u{00B7}  ";
    let mut job = egui::text::LayoutJob::default();
    job.append(inst.mc_version(), 0.0, part(on_cover(225)));
    job.append(sep, 0.0, part(on_cover(110)));
    match inst.loader() {
        Some((kind, ver)) if level == 0 || level == 1 => job.append(&format!("{kind} {ver}"), 0.0, part(on_cover(225))),
        Some((kind, _)) => job.append(kind, 0.0, part(on_cover(225))),
        None => job.append("Vanilla", 0.0, part(on_cover(225))),
    }
    if level == 0 {
        job.append(sep, 0.0, part(on_cover(110)));
        job.append(&mods_text(inst.mod_count), 0.0, part(on_cover(225)));
    }
    job
}

/// "1 mod", "7 mods", "No mods".
pub(super) fn mods_text(n: usize) -> String {
    match n {
        0 => "No mods".to_string(),
        1 => "1 mod".to_string(),
        n => format!("{n} mods"),
    }
}

/// "12 min" or "1 h 05 min": how long a game has been running.
fn running_for(t: std::time::Duration) -> String {
    let mins = t.as_secs() / 60;
    if mins < 60 { format!("{mins} min") } else { format!("{} h {:02} min", mins / 60, mins % 60) }
}

/// "23 hours", "45 min", "Under a minute": a play time, rounded the way it is said.
pub(super) fn play_time_text(secs: u64) -> String {
    let mins = secs / 60;
    match mins {
        0 => "Under a minute".to_string(),
        1..=59 => format!("{mins} min"),
        60..=119 => "1 hour".to_string(),
        _ if mins < 60 * 48 => format!("{} hours", mins / 60),
        _ => format!("{} days", mins / (60 * 24)),
    }
}

/// The split Play button of the banner: the main part (Play, or Stop while the game runs) and a part with a chevron
/// that opens the menu of who to play as. Returns the main part's response and the menu part's.
fn split_play(ui: &mut Ui, stop: bool, height: f32) -> (Response, Response) {
    let (text, icon) = if stop { ("Stop", Icon::Stop) } else { ("Play", Icon::Play) };
    let font = super::fonts::semibold(15.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), Color32::PLACEHOLDER);
    let main_w = (galley.size().x + 22.0 + 40.0).max(if stop { 96.0 } else { 112.0 });
    let (rect, _) = ui.allocate_exact_size(vec2(main_w + MENU_PART, height), Sense::hover());
    let main = Rect::from_min_max(rect.min, pos2(rect.left() + main_w, rect.bottom()));
    let more = Rect::from_min_max(pos2(main.right(), rect.top()), rect.max);
    let id = ui.make_persistent_id("instance-play");
    let main_resp = ui.interact(main, id, Sense::click());
    let more_resp = ui.interact(more, id.with("menu"), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&more_resp));
    let tm = motion::hover(ui, main_resp.id, main_resp.hovered());
    let tc = motion::hover(ui, more_resp.id, more_resp.hovered() || open);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let radius = CornerRadius::same(R_CONTROL);
        let fg = if stop { c_danger() } else { c_on_accent() };
        if stop {
            // laid on the card colour first: the cover must not show through the tint
            p.rect_filled(rect, radius, c_card());
            p.rect_filled(rect, radius, c_danger_bg());
            p.rect_stroke(rect, radius, Stroke::new(1.0, c_danger_line()), egui::StrokeKind::Inside);
        } else {
            let base = c_accent();
            let glow = egui::Shadow {
                offset: [0, 8],
                blur: (20.0 + 10.0 * tm.max(tc)) as u8,
                spread: 0,
                color: Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), (80.0 + 50.0 * tm) as u8),
            };
            p.add(glow.as_shape(rect, radius));
            motion::gradient_rect(p, rect, R_CONTROL, c_accent(), c_accent2());
            p.hline(
                rect.left() + R_CONTROL as f32..=rect.right() - R_CONTROL as f32,
                rect.top() + 0.5,
                Stroke::new(1.0, Color32::from_white_alpha(60)),
            );
        }
        // each part lightens on its own under the pointer
        let lift = |r: Rect, t: f32, corners: CornerRadius| {
            if t > 0.0 {
                p.rect_filled(r, corners, Color32::from_white_alpha((if stop { 10.0 } else { 34.0 } * t) as u8));
            }
        };
        let r = R_CONTROL;
        lift(main, tm, CornerRadius { nw: r, sw: r, ne: 0, se: 0 });
        lift(more, tc, CornerRadius { nw: 0, sw: 0, ne: r, se: r });
        p.vline(main.right(), rect.top() + 9.0..=rect.bottom() - 9.0, Stroke::new(1.0, fg.gamma_multiply(0.35)));
        let content_w = galley.size().x + 22.0;
        let left = main.center().x - content_w / 2.0;
        icons::paint(p, icon, pos2(left + 7.0, main.center().y), 15.0, fg);
        let galley = p.layout_no_wrap(text.to_string(), font, fg);
        p.galley(pos2(left + 22.0, main.center().y - galley.size().y / 2.0), galley, fg);
        // the chevron points down, and up while the menu is open
        let (c, d) = (more.center(), if open { -1.0 } else { 1.0 });
        let chevron = vec![c + vec2(-4.0, -2.0 * d), c + vec2(0.0, 2.0 * d), c + vec2(4.0, -2.0 * d)];
        p.add(egui::Shape::line(chevron, Stroke::new(1.6, fg)));
    }
    focus_ring(ui, &main_resp, main, R_CONTROL);
    focus_ring(ui, &more_resp, more, R_CONTROL);
    main_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    more_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Play as"));
    let main_resp = main_resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let more_resp = more_resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Play as another account, or start another game at the same time");
    (main_resp, more_resp)
}

/// A square button of glass over the cover, with an icon: the "…" of the banner.
fn cover_button(ui: &mut Ui, icon: Icon, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let t = motion::hover(ui, resp.id, resp.hovered() || open);
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(R_CONTROL);
        let fill = Color32::from_black_alpha((110.0 + 50.0 * t) as u8);
        ui.painter().rect_filled(rect, radius, fill);
        ui.painter().rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, Color32::from_white_alpha((36.0 + 40.0 * t) as u8)),
            egui::StrokeKind::Inside,
        );
        icons::paint(ui.painter(), icon, rect.center(), 17.0, on_cover(235));
    }
    focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "More"));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("More")
}

/// What is going on with the game, on a pill of dark glass: a light while it gets ready or runs.
fn status_pill(ui: &mut Ui, text: &str, live: bool) {
    let font = super::fonts::medium(FS_META);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, on_cover(235));
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 34.0, 28.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(14), Color32::from_black_alpha(120));
    let dot = pos2(rect.left() + 14.0, rect.center().y);
    if live {
        motion::glow(ui.painter(), dot, 9.0, c_accent().gamma_multiply(0.5));
    }
    ui.painter().circle_filled(dot, 4.0, if live { c_accent() } else { on_cover(140) });
    ui.painter().galley(pos2(rect.left() + 24.0, rect.center().y - galley.size().y / 2.0), galley, on_cover(235));
}

/// A darker band along the bottom of the banner, so the buttons sit on a calm ground whatever the picture shows.
fn shade_bottom(painter: &egui::Painter, rect: Rect, strength: u8) {
    let mut mesh = egui::Mesh::default();
    let top = rect.top() + rect.height() * 0.35;
    let clear = Color32::TRANSPARENT;
    let dark = Color32::from_black_alpha(strength);
    mesh.colored_vertex(pos2(rect.left(), top), clear);
    mesh.colored_vertex(pos2(rect.right(), top), clear);
    mesh.colored_vertex(pos2(rect.right(), rect.bottom()), dark);
    mesh.colored_vertex(pos2(rect.left(), rect.bottom()), dark);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

impl App {
    /// The cover of the instance across the page with its picture, name and what it is, and Play, Play as, the
    /// performance switches and the menu over it. The Overview has the tall banner; the other tabs a short one, and
    /// the height glides between the two when the tab changes.
    pub(super) fn instance_banner(&mut self, ui: &mut Ui, inst: &Instance, action: &mut Option<Act>) {
        let tall = self.tab == InstTab::Overview;
        let target = if tall { BANNER_TALL } else { BANNER_SHORT };
        let h = motion::glide(ui.ctx(), egui::Id::unique("instance-banner-height"), target, motion::MEDIUM);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
        let radius = R_CARD + 4;
        ui.painter().add(card_shadow().as_shape(rect, CornerRadius::same(radius)));
        self.paint_cover(ui, inst, rect, radius, true);
        // the shade is clipped to the banner's rounded corners by drawing it a little inside them
        shade_bottom(&ui.painter().with_clip_rect(rect.shrink2(vec2(0.0, 1.0))), rect, if tall { 120 } else { 90 });
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(radius),
            Stroke::new(1.0, Color32::from_white_alpha(22)),
            egui::StrokeKind::Inside,
        );
        let area = rect.shrink(if tall { BANNER_PAD } else { 18.0 });
        let mut inner = ui.new_child(UiBuilder::new().max_rect(area).layout(Layout::left_to_right(Align::Center)));
        inner.set_clip_rect(ui.clip_rect().intersect(rect));
        // the text is laid out for the banner it is going to (the height glides, the layout does not)
        if tall {
            self.banner_tall(&mut inner, inst, action);
        } else {
            self.banner_short(&mut inner, inst, action);
        }
    }

    /// The Overview's banner: the picture, then the name, what it is and the buttons in a column beside it.
    fn banner_tall(&mut self, ui: &mut Ui, inst: &Instance, action: &mut Option<Act>) {
        let room = ui.available_width();
        // the picture gives way first in a narrow window
        let picture = (ui.available_height()).min(124.0);
        if room >= 520.0 {
            self.instance_icon(ui, inst, picture);
            ui.add_space(22.0);
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let content = 34.0 + 18.0 + 16.0 + 44.0;
            ui.add_space(((ui.available_height() - content) / 2.0).max(0.0));
            self.banner_title(ui, inst, FS_TITLE + 2.0);
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let roomy = ui.available_width() >= 430.0;
                self.banner_play(ui, inst, 44.0, action);
                self.banner_extras(ui, inst, roomy, action);
            });
        });
    }

    /// The short banner of the other tabs: the picture, the name and what it is, and the buttons at the right end.
    fn banner_short(&mut self, ui: &mut Ui, inst: &Instance, action: &mut Option<Act>) {
        let room = ui.available_width();
        let id = egui::Id::unique("instance-banner-buttons");
        let buttons_w = remembered_width(ui, id, 220.0);
        // Performance stays in a wide banner; decided by the banner's width alone, not by the buttons' (which it
        // changes), so the row doesn't flip between the two
        let roomy = room >= SHORT_ROOMY;
        if room >= 480.0 {
            let side = ui.available_height().min(56.0);
            self.instance_icon(ui, inst, side);
            ui.add_space(16.0);
        }
        let text_w = (ui.available_width() - buttons_w - 16.0).max(80.0);
        ui.allocate_ui_with_layout(vec2(text_w, ui.available_height()), Layout::top_down(Align::Min), |ui| {
            ui.set_width(text_w);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(((ui.available_height() - 44.0) / 2.0).max(0.0));
            self.banner_title(ui, inst, FS_FIGURE + 2.0);
        });
        // the buttons at the right end, read from left to right: what the game is doing, the performance switches,
        // Play and the menu
        let area = ui.max_rect();
        let rect = Rect::from_min_max(pos2(area.right() - buttons_w, area.top()), area.max);
        let mut right = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)));
        right.spacing_mut().item_spacing.x = 8.0;
        self.banner_extras(&mut right, inst, roomy, action);
        self.banner_play(&mut right, inst, 40.0, action);
        let used = right.min_rect().width();
        remember_width(ui, id, used);
    }

    /// The name of the instance and the line of what it is, shortened a whole part at a time to fit.
    fn banner_title(&self, ui: &mut Ui, inst: &Instance, size: f32) {
        let name = RichText::new(inst.name()).font(super::fonts::semibold(size)).color(on_cover(250));
        ui.add(egui::Label::new(name).truncate()).on_hover_text(inst.name());
        let room = ui.available_width();
        let job = (0..3)
            .map(|level| banner_meta(inst, level))
            .find(|job| ui.painter().layout_job(job.clone()).size().x <= room)
            .unwrap_or_else(|| banner_meta(inst, 2));
        ui.add(egui::Label::new(job).truncate());
    }

    /// Play (Stop while the game runs) with its Play as part, and the menu of the instance.
    fn banner_play(&mut self, ui: &mut Ui, inst: &Instance, height: f32, action: &mut Option<Act>) {
        let folder = inst.folder.clone();
        let playing = self.is_playing(&folder);
        let (main, more) = split_play(ui, playing, height);
        if main.clicked() {
            *action = Some(if playing { Act::Stop(folder.clone()) } else { Act::Play(folder.clone()) });
        }
        let picked = egui::Popup::menu(&more).gap(6.0).show(|ui| {
            menu_items(ui);
            self.account_menu(ui, &folder)
        });
        if let Some(a) = picked.and_then(|r| r.inner) {
            *action = Some(a);
        }
        let kebab = cover_button(ui, Icon::Kebab, height);
        let picked = egui::Popup::menu(&kebab).gap(6.0).show(|ui| {
            menu_items(ui);
            self.instance_menu(ui, inst)
        });
        if let Some(a) = picked.and_then(|r| r.inner) {
            *action = Some(a);
        }
    }

    /// What the game is doing, and when there is room the performance switches (they are in the menu too, under
    /// Performance).
    fn banner_extras(&mut self, ui: &mut Ui, inst: &Instance, roomy: bool, action: &mut Option<Act>) {
        let folder = inst.folder.clone();
        let sessions = self.sessions_of(&folder);
        let game = sessions.iter().filter_map(|k| self.games.get(k)).find(|g| g.active());
        match game.map(|g| (g.phase, g.started)) {
            Some((Phase::Preparing, _)) => status_pill(ui, "Getting ready", true),
            Some((Phase::Running, started)) if roomy => {
                status_pill(ui, &format!("Running \u{00B7} {}", running_for(started.elapsed())), true);
                ui.ctx().request_repaint_after(std::time::Duration::from_secs(20));
            }
            Some((Phase::Running, _)) => status_pill(ui, "Running", true),
            _ => {}
        }
        if roomy {
            if let Some(a) = self.performance_button(ui, inst) {
                *action = Some(a);
            }
        }
    }

    // ---------------------------------------------------------------------------------------------------------
    // The Overview

    /// The four facts at the top of the Overview, each on a small card: when it was last played, how long it has
    /// been played, its mods and its Java.
    pub(super) fn overview_stats(&self, ui: &mut Ui, inst: &Instance) {
        let (last, last_sub) = if inst.last_played() > 0 {
            let at = chrono::DateTime::from_timestamp_millis(inst.last_played())
                .map(|t| t.with_timezone(&chrono::Local).format("%-d %b %Y").to_string())
                .unwrap_or_default();
            (time_ago(inst.last_played()), at)
        } else {
            ("Never".to_string(), "Not played yet".to_string())
        };
        let plays = inst.file.launcher.num_plays.unwrap_or(0).max(0);
        let sessions = if plays == 1 { "1 session".to_string() } else { format!("{plays} sessions") };
        let secs = inst.play_seconds();
        let time = if secs > 0 { play_time_text(secs) } else { "\u{2014}".to_string() };
        let enabled = inst.enabled_mod_count();
        let mods_sub = match (inst.mod_count, inst.loader()) {
            (0, None) => "Vanilla".to_string(),
            (0, Some((kind, _))) => kind.to_string(),
            (n, _) if enabled < n => format!("{} off", n - enabled),
            _ => "All on".to_string(),
        };
        let (java, java_sub) = match inst.file.version.java_version.as_ref() {
            Some(j) => (j.major_version.to_string(), "Provided by Minecraft".to_string()),
            None => ("Auto".to_string(), "Picked at launch".to_string()),
        };
        stat_cards(
            ui,
            &[
                ("Last played", last, last_sub),
                ("Play time", time, sessions),
                ("Mods", inst.mod_count.to_string(), mods_sub),
                ("Java", java, java_sub),
            ],
        );
    }

    /// The days the instance was played over the last weeks, as a strip of bars under a title.
    pub(super) fn overview_history(&mut self, ui: &mut Ui, inst: &Instance) {
        card(ui, |ui| {
            let n = history_span(ui.available_width());
            let days = self.play_days(inst, n);
            let played = days.iter().filter(|d| **d > 0).count();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Play history").size(FS_SECTION).semibold());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} weeks", n / 7)).font(super::fonts::mono(FS_META)).color(c_faint()),
                    );
                });
            });
            ui.add_space(10.0);
            history_bars(ui, &days, Self::strip_hue(inst));
            ui.add_space(8.0);
            small_muted(ui, super::strip::caption(played, n));
        });
    }

    // ---------------------------------------------------------------------------------------------------------
    // The cover picker of the settings

    /// One small preview per cover theme: the instance's own landscape drawn in each. Picking one saves it in the
    /// instance's file at once; Auto (the default) lets the instance's seed choose.
    pub(super) fn cover_picker(&mut self, ui: &mut Ui, inst: &Instance) {
        let current = workbench::cover_theme(inst);
        let (cols, w) = grid_columns(ui.available_width(), 104.0, 10.0);
        let h = (w * 0.56).round();
        let mut picked = None;
        for row in CoverTheme::ALL.chunks(cols.max(1)) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                for theme in row {
                    if self.cover_choice(ui, inst, *theme, *theme == current, vec2(w, h)).clicked() {
                        picked = Some(*theme);
                    }
                }
            });
            ui.add_space(6.0);
        }
        if let Some(theme) = picked.filter(|t| *t != current) {
            self.set_instance_cover(&inst.folder, theme);
        }
    }

    /// One preview of the cover picker: the cover in `theme`, its name under it, a ring in the brand colour when it is
    /// the one in use.
    fn cover_choice(
        &mut self,
        ui: &mut Ui,
        inst: &Instance,
        theme: CoverTheme,
        on: bool,
        size: egui::Vec2,
    ) -> Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(size.x, size.y + 22.0), Sense::click());
        let picture = Rect::from_min_size(rect.min, size);
        if ui.is_rect_visible(rect) {
            let spec = super::covers::CoverSpec { theme, ..workbench::cover_spec(inst) };
            let (tw, th) = workbench::cover_size(size);
            let tex = self.covers.texture(&self.ctx, &spec, tw, th);
            let t = motion::hover(ui, resp.id, resp.hovered());
            let s = motion::toward(ui.ctx(), resp.id.with("on"), on, motion::FAST);
            let lifted = picture.translate(vec2(0.0, -2.0 * t));
            egui::Image::from_texture((tex.id(), vec2(tw as f32, th as f32)))
                .uv(workbench::cover_uv((tw, th), size))
                .corner_radius(R_CONTROL)
                .paint_at(ui, lifted);
            let line = motion::mix(motion::mix(c_border(), c_control(), t), c_accent(), s);
            ui.painter().rect_stroke(
                lifted,
                CornerRadius::same(R_CONTROL),
                Stroke::new(1.0 + 1.0 * s, line),
                egui::StrokeKind::Inside,
            );
            if on {
                // a tick in the corner of the one in use
                let c = pos2(lifted.right() - 12.0, lifted.top() + 12.0);
                ui.painter().circle_filled(c, 8.0, c_accent());
                let tick = vec![c + vec2(-3.5, 0.2), c + vec2(-1.0, 2.7), c + vec2(3.6, -2.4)];
                ui.painter().add(egui::Shape::line(tick, Stroke::new(1.6, c_on_accent())));
            }
            let label = theme.label();
            let fg = motion::mix(c_muted(), c_text(), t.max(s));
            let galley = ui.painter().layout_no_wrap(label.to_string(), super::fonts::medium(FS_META), fg);
            ui.painter().galley(pos2(rect.left() + 2.0, picture.bottom() + 6.0), galley, fg);
        }
        focus_ring(ui, &resp, picture, R_CONTROL);
        let label = theme.label();
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, label));
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    }

    /// Keep the cover theme of an instance in its file: read as it is on disk now (a mod job may have written to it),
    /// changed, saved.
    pub(super) fn set_instance_cover(&mut self, folder: &str, theme: CoverTheme) {
        let Some(idx) = self.instances.iter().position(|i| i.folder == folder) else { return };
        let mut fresh =
            crate::instances::load_one(&self.instances[idx].root).unwrap_or_else(|| self.instances[idx].clone());
        workbench::set_cover_theme(&mut fresh, theme);
        match fresh.save() {
            Ok(()) => {
                // the extras are the only part changed, so what else is in memory stays as it is
                self.instances[idx].file.launcher.extras = fresh.file.launcher.extras;
            }
            Err(e) => self.toast_err(format!("Couldn't save the cover: {e:#}")),
        }
    }
}

/// How many stat cards share a row in `width`: all four, or two and two in a narrow window.
fn stat_columns(width: f32, n: usize) -> usize {
    if width >= 560.0 { n.max(1) } else { n.clamp(1, 2) }
}

/// Facts on small cards in a row: a grey label, the value large, and a quiet line under it.
fn stat_cards(ui: &mut Ui, cells: &[(&str, String, String)]) {
    const GAP: f32 = 12.0;
    const H: f32 = 86.0;
    let w = ui.available_width();
    let cols = stat_columns(w, cells.len());
    let cw = ((w - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
    for (r, row) in cells.chunks(cols).enumerate() {
        if r > 0 {
            ui.add_space(GAP);
        }
        let (line, _) = ui.allocate_exact_size(vec2(w, H), Sense::hover());
        for (i, (label, value, sub)) in row.iter().enumerate() {
            let rect = Rect::from_min_size(pos2(line.left() + i as f32 * (cw + GAP), line.top()), vec2(cw, H));
            let p = ui.painter();
            let radius = CornerRadius::same(R_CARD);
            p.add(card_shadow().as_shape(rect, radius));
            p.rect(rect, radius, c_card(), Stroke::new(1.0, c_border()), egui::StrokeKind::Inside);
            glass_edge(p, rect, R_CARD);
            // a child of its own: a scope would move the page's cursor back up to where the text ends
            let mut cell = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(16.0, 13.0))));
            cell.spacing_mut().item_spacing.y = 1.0;
            cell.add(egui::Label::new(RichText::new(*label).size(FS_META).color(c_muted())).truncate());
            cell.add(egui::Label::new(RichText::new(value).font(super::fonts::medium(FS_FIGURE))).truncate());
            cell.add(egui::Label::new(RichText::new(sub).font(super::fonts::mono(11.0)).color(c_faint())).truncate());
        }
    }
}

/// The narrowest a bar of the play history gets, and the gap between bars.
const BAR_MIN: f32 = 6.0;
const BAR_GAP: f32 = 3.0;
/// The height of the bars' strip.
const BARS_H: f32 = 44.0;

/// How many days the play history shows in `width`: whole weeks, eight of them when the bars stay wide enough, down
/// to two in a very narrow window.
fn history_span(width: f32) -> usize {
    let fit = ((width + BAR_GAP) / (BAR_MIN + BAR_GAP)).floor().max(0.0) as usize;
    (fit / 7 * 7).clamp(14, 56)
}

/// How tall the bar of a day with `sessions` is in a strip `full` high: a short stub for a day not played, and taller
/// the more it was played, up to four sessions.
fn bar_height(sessions: u8, full: f32) -> f32 {
    if sessions == 0 { 4.0 } else { full * (0.4 + 0.15 * f32::from(sessions.min(4))) }
}

/// The play history as bars across the width: one per day, oldest first and today last (with a ring), filled in the
/// instance's colour on the days it was played. Hovering a bar says which day it is.
fn history_bars(ui: &mut Ui, days: &[u8], hue: Color32) -> Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, BARS_H), Sense::hover());
    let n = days.len().max(1) as f32;
    let bar = ((w - BAR_GAP * (n - 1.0)) / n).max(1.0);
    let index_at = |x: f32| (((x - rect.left()) / (bar + BAR_GAP)).floor().max(0.0) as usize).min(days.len() - 1);
    let pointed = resp.hover_pos().filter(|_| !days.is_empty()).map(|p| index_at(p.x));
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let radius = CornerRadius::same(if bar >= 8.0 { 3 } else { 2 });
        for (i, d) in days.iter().enumerate() {
            let x = rect.left() + i as f32 * (bar + BAR_GAP);
            let h = bar_height(*d, BARS_H);
            let r = Rect::from_min_max(pos2(x, rect.bottom() - h), pos2(x + bar, rect.bottom()));
            let lit = pointed == Some(i);
            if *d > 0 {
                p.rect_filled(r, radius, if lit { motion::mix(hue, c_text(), 0.25) } else { hue });
                // the top catches the light, like the face of a block
                let top = Rect::from_min_size(r.min, vec2(bar, (r.height() * 0.25).min(6.0)));
                p.rect_filled(
                    top,
                    CornerRadius { nw: radius.nw, ne: radius.ne, sw: 0, se: 0 },
                    Color32::from_white_alpha(if is_dark() { 34 } else { 46 }),
                );
            } else {
                p.rect_filled(r, radius, if lit { c_control() } else { c_active() });
            }
            if i + 1 == days.len() {
                p.rect_stroke(r.expand(1.5), radius, Stroke::new(1.0, c_muted()), egui::StrokeKind::Outside);
            }
        }
    }
    let label = super::strip::caption(days.iter().filter(|d| **d > 0).count(), days.len());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &label));
    if let Some(i) = pointed {
        let ago = days.len() - 1 - i;
        let day = chrono::Local::now().date_naive() - chrono::Duration::days(ago as i64);
        let sessions = match days[i] {
            0 => "not played".to_string(),
            1 => "1 session".to_string(),
            n => format!("{n} sessions"),
        };
        return resp.on_hover_text(format!("{}: {sessions}", day.format("%a %-d %b")));
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_history_is_whole_weeks_up_to_eight() {
        assert_eq!(history_span(900.0), 56);
        assert_eq!(history_span(500.0), 49);
        assert_eq!(history_span(300.0), 28);
        assert_eq!(history_span(40.0), 14);
        for w in [200.0, 333.0, 480.0, 640.0] {
            let n = history_span(w);
            assert_eq!(n % 7, 0);
            if n > 14 {
                assert!(n as f32 * (BAR_MIN + BAR_GAP) - BAR_GAP <= w, "{n} days in {w}");
            }
        }
    }

    #[test]
    fn a_day_played_more_has_a_taller_bar() {
        let full = BARS_H;
        assert!(bar_height(0, full) < bar_height(1, full));
        assert!(bar_height(1, full) < bar_height(2, full));
        assert!(bar_height(4, full) <= full);
        assert_eq!(bar_height(4, full), bar_height(200, full));
    }

    #[test]
    fn the_stat_cards_go_two_by_two_in_a_narrow_window() {
        assert_eq!(stat_columns(900.0, 4), 4);
        assert_eq!(stat_columns(420.0, 4), 2);
        assert_eq!(stat_columns(420.0, 1), 1);
    }

    #[test]
    fn play_time_is_said_the_way_people_say_it() {
        assert_eq!(play_time_text(20), "Under a minute");
        assert_eq!(play_time_text(45 * 60), "45 min");
        assert_eq!(play_time_text(90 * 60), "1 hour");
        assert_eq!(play_time_text(23 * 3600 + 100), "23 hours");
        assert_eq!(play_time_text(80 * 3600), "3 days");
        assert_eq!(mods_text(0), "No mods");
        assert_eq!(mods_text(1), "1 mod");
        assert_eq!(mods_text(7), "7 mods");
        assert_eq!(running_for(std::time::Duration::from_secs(65 * 60)), "1 h 05 min");
    }
}
