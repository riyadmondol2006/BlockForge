//! The instance workspace, the page an instance opens on: its cover banner with Play and the menus, and the pieces of
//! its tabs that are new in 5.0 (the stat cards and the play history of the Overview, the cover picker of its
//! settings).

use egui::{Align, Color32, CornerRadius, Layout, Rect, Response, RichText, Sense, Stroke, Ui, UiBuilder, pos2, vec2};

use super::covers::CoverTheme;
use super::icons::{self, Icon};
use super::instance_menu::{Act, SplitRow, menu_items};
use super::motion;
use super::theme::*;
use super::widgets::*;
use super::{App, InstTab, Phase, workbench};
use crate::instances::Instance;

/// The banner's height on the Overview, and on the other tabs, where the lists under it want the room.
const BANNER_TALL: f32 = 196.0;
const BANNER_SHORT: f32 = 64.0;
/// The air inside the banner, left of the name.
const BANNER_PAD: f32 = 22.0;
/// Play, Performance and the menu sit in the banner's top right corner, this far in from its right edge and its top,
/// on every tab: the banner grows and shrinks under them, and Play never moves when the tab changes.
const ACTIONS_RIGHT: f32 = 16.0;
const ACTIONS_TOP: f32 = 10.0;
/// The height of those buttons.
const ACTION_H: f32 = 44.0;
/// From this width of the banner on, Performance has a button of its own beside Play (it is in the menu as well).
const PERFORMANCE_ROOM: f32 = 600.0;
/// How long the banner takes to grow or shrink when the tab changes.
const BANNER_GLIDE: f32 = 0.18;
/// The part of the Play button that opens its menu.
const MENU_PART: f32 = 38.0;

/// Text written over a cover. Its left side is always faded dark (see `fade_left`), so the text is light in both
/// looks.
fn on_cover(alpha: u8) -> Color32 {
    Color32::from_white_alpha(alpha)
}

/// "1.21.1 · Fabric 0.20.0 · 7 mods" in the monospace face, for the banner. `level` 0 is all of it; each next level
/// leaves out one more part from the end (the mods, then the loader's version). A Horizon instance has "Horizon" and
/// its build (`build`) for the loader, and no mods.
fn banner_meta(inst: &Instance, level: usize, build: Option<&str>) -> egui::text::LayoutJob {
    let part = |color: Color32| egui::TextFormat { font_id: super::fonts::mono(13.0), color, ..Default::default() };
    let sep = "  \u{00B7}  ";
    let mut job = egui::text::LayoutJob::default();
    job.append(inst.mc_version(), 0.0, part(on_cover(225)));
    job.append(sep, 0.0, part(on_cover(110)));
    match super::edition::loader_parts(inst, build) {
        Some((kind, ver)) if level == 0 || level == 1 => {
            job.append(&super::edition::joined(kind, ver), 0.0, part(on_cover(225)))
        }
        Some((kind, _)) => job.append(kind, 0.0, part(on_cover(225))),
        None => job.append("Vanilla", 0.0, part(on_cover(225))),
    }
    if level == 0 && !crate::horizon::is_horizon(inst) {
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
    // Stop as wide as Play, so the buttons beside it keep their places when the game starts
    let main_w = (galley.size().x + 22.0 + 40.0).max(112.0);
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

fn banner_snap_id() -> egui::Id {
    egui::Id::unique("instance-banner-snap")
}

/// The banner takes its new height at once instead of gliding to it: the tab was changed from the keyboard.
pub(super) fn snap_banner(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(banner_snap_id(), true));
}

/// A darker band along the bottom of the banner, so the name sits on a calm ground whatever the picture shows. It
/// follows the banner's rounded bottom corners (`radius`), so no square of shade shows past them.
fn shade_bottom(painter: &egui::Painter, rect: Rect, radius: f32, strength: u8) {
    painter.add(bottom_shade_mesh(rect, radius, strength));
}

/// The band of [`shade_bottom`]: the lower part of `rect` from a third of the way down, its outline the rounded
/// rectangle's (corners of `radius`), clear at its top and `strength` dark at the bottom. That part of a rounded
/// rectangle is convex, so it is a fan of triangles from its middle; the shade changes evenly from top to bottom, so
/// each corner of a triangle can carry its own and the triangles fill in between exactly.
fn bottom_shade_mesh(rect: Rect, radius: f32, strength: u8) -> egui::Mesh {
    let top = rect.top() + rect.height() * 0.35;
    let r = radius.min(rect.bottom() - top).min(rect.width() / 2.0).max(0.0);
    let shade = |y: f32| {
        let t = ((y - top) / (rect.bottom() - top).max(1.0)).clamp(0.0, 1.0);
        Color32::from_black_alpha((f32::from(strength) * t).round() as u8)
    };
    let mut outline = vec![pos2(rect.left(), top), pos2(rect.right(), top)];
    const STEPS: usize = 8;
    // down the right edge and round its corner, along the bottom, round the left corner and up
    for (cx, from) in [(rect.right() - r, 0.0), (rect.left() + r, std::f32::consts::FRAC_PI_2)] {
        for i in 0..=STEPS {
            let a = from + std::f32::consts::FRAC_PI_2 * i as f32 / STEPS as f32;
            outline.push(pos2(cx + r * a.cos(), rect.bottom() - r + r * a.sin()));
        }
    }
    let mut mesh = egui::Mesh::default();
    let middle = pos2(rect.center().x, (top + rect.bottom()) / 2.0);
    mesh.colored_vertex(middle, shade(middle.y));
    for p in &outline {
        mesh.colored_vertex(*p, shade(p.y));
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    mesh
}

impl App {
    /// The cover of the instance across the page with its picture, name and what it is, and Play, Play as, the
    /// performance switches and the menu over it. The Overview has the tall banner; the other tabs a short one, and
    /// the height glides between the two when the tab changes (at once for a tab picked from the keyboard). The
    /// buttons keep to the banner's top right corner whatever its height, so Play never moves.
    pub(super) fn instance_banner(&mut self, ui: &mut Ui, inst: &Instance, action: &mut Option<Act>) {
        let tall = self.tab == InstTab::Overview;
        let target = if tall { BANNER_TALL } else { BANNER_SHORT };
        let snap = ui.ctx().data_mut(|d| d.remove_temp::<bool>(banner_snap_id())).unwrap_or(false);
        let pace = if snap { 0.0 } else { BANNER_GLIDE };
        let h = motion::glide(ui.ctx(), egui::Id::unique("instance-banner-height"), target, pace);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
        let radius = R_CARD + 4;
        ui.painter().add(card_shadow().as_shape(rect, CornerRadius::same(radius)));
        self.paint_cover(ui, inst, rect, radius, true);
        shade_bottom(ui.painter(), rect, f32::from(radius), if tall { 120 } else { 90 });
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(radius),
            Stroke::new(1.0, Color32::from_white_alpha(22)),
            egui::StrokeKind::Inside,
        );
        let clip = ui.clip_rect().intersect(rect);
        // the buttons first, from the right edge leftwards
        let actions = Rect::from_min_max(
            pos2(rect.left() + BANNER_PAD, rect.top() + ACTIONS_TOP),
            pos2(rect.right() - ACTIONS_RIGHT, rect.top() + ACTIONS_TOP + ACTION_H),
        );
        let mut right = ui.new_child(UiBuilder::new().max_rect(actions).layout(Layout::right_to_left(Align::Center)));
        right.set_clip_rect(clip);
        right.spacing_mut().item_spacing.x = 8.0;
        self.banner_actions(&mut right, inst, tall, action);
        // then the picture and the name: along the bottom of the tall banner, under the buttons; in the middle of the
        // short one, left of them
        let area = if tall {
            Rect::from_min_max(
                pos2(rect.left() + BANNER_PAD, actions.bottom() + 8.0),
                pos2(rect.right() - BANNER_PAD, rect.bottom() - BANNER_PAD + 2.0),
            )
        } else {
            let end = (right.min_rect().left() - 16.0).max(rect.left() + BANNER_PAD + 60.0);
            Rect::from_min_max(pos2(rect.left() + 18.0, rect.top() + 6.0), pos2(end, rect.bottom() - 6.0))
        };
        self.banner_identity(ui, inst, area, tall, clip);
    }

    /// The instance's own picture (when it has one) and its name with what it is, at the bottom of `area` in the tall
    /// banner and in its middle in the short one.
    fn banner_identity(&mut self, ui: &mut Ui, inst: &Instance, area: Rect, tall: bool, clip: Rect) {
        let id = egui::Id::unique(if tall { "instance-banner-name-tall" } else { "instance-banner-name-short" });
        let h = remembered_width(ui, id, if tall { 64.0 } else { 44.0 });
        let top = if tall { area.bottom() - h } else { area.center().y - h / 2.0 };
        let block = Rect::from_min_max(pos2(area.left(), top), pos2(area.right(), top + h));
        let mut row = ui.new_child(UiBuilder::new().max_rect(block).layout(Layout::left_to_right(Align::Center)));
        row.set_clip_rect(clip);
        // the cover is the instance's picture; a picture of its own (instance.png) sits on it, the stand-in block would
        // only cover it up. The picture gives way first in a narrow window
        let own = self.local_picture(inst.root.join("instance.png")).is_some();
        if own && area.width() >= if tall { 420.0 } else { 320.0 } {
            self.instance_icon(&mut row, inst, if tall { 72.0 } else { 44.0 });
            row.add_space(if tall { 18.0 } else { 14.0 });
        }
        let w = row.available_width();
        row.vertical(|ui| {
            ui.set_max_width(w);
            ui.spacing_mut().item_spacing.y = if tall { 4.0 } else { 2.0 };
            self.banner_title(ui, inst, if tall { FS_TITLE + 2.0 } else { FS_FIGURE + 2.0 });
        });
        remember_width(ui, id, row.min_rect().height().ceil());
    }

    /// The name of the instance and the line of what it is, shortened a whole part at a time to fit.
    fn banner_title(&self, ui: &mut Ui, inst: &Instance, size: f32) {
        let name = RichText::new(inst.name()).font(super::fonts::semibold(size)).color(on_cover(250));
        ui.add(egui::Label::new(name).truncate()).on_hover_text(inst.name());
        let room = ui.available_width();
        let build = self.horizon_tag();
        let job = (0..3)
            .map(|level| banner_meta(inst, level, build))
            .find(|job| ui.painter().layout_job(job.clone()).size().x <= room)
            .unwrap_or_else(|| banner_meta(inst, 2, build));
        ui.add(egui::Label::new(job).truncate());
    }

    /// The buttons of the banner, laid out from its right edge leftwards so each keeps its place: the menu,
    /// Performance (when the banner has room for it), Play (Stop while the game runs) with its Play as part, and left
    /// of Play what the game is doing, where its changing words can't push Play along.
    fn banner_actions(&mut self, ui: &mut Ui, inst: &Instance, tall: bool, action: &mut Option<Act>) {
        let folder = inst.folder.clone();
        let width = ui.max_rect().width();
        let kebab = cover_button(ui, Icon::Kebab, ACTION_H);
        let picked = egui::Popup::menu(&kebab).gap(6.0).show(|ui| {
            menu_items(ui);
            self.instance_menu(ui, inst)
        });
        if let Some(a) = picked.and_then(|r| r.inner) {
            *action = Some(a);
        }
        // decided by the banner's width alone, the same on every tab (it is under Performance in the menu as well).
        // Its helpers are Java's: a Horizon instance has none
        if width >= PERFORMANCE_ROOM - ACTIONS_RIGHT - BANNER_PAD && !crate::horizon::is_horizon(inst) {
            if let Some(a) = self.performance_button(ui, inst, ACTION_H) {
                *action = Some(a);
            }
        }
        let playing = self.is_playing(&folder);
        let (main, more) = split_play(ui, playing, ACTION_H);
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
        // the name of the short banner keeps at least this much room beside the status
        let left = ui.available_width() - if tall { 0.0 } else { 240.0 };
        let sessions = self.sessions_of(&folder);
        let game = sessions.iter().filter_map(|k| self.games.get(k)).find(|g| g.active());
        match game.map(|g| (g.phase, g.started)) {
            Some((Phase::Preparing, _)) if left >= 150.0 => status_pill(ui, "Getting ready", true),
            Some((Phase::Running, started)) if left >= 170.0 => {
                status_pill(ui, &format!("Running \u{00B7} {}", running_for(started.elapsed())), true);
                ui.ctx().request_repaint_after(std::time::Duration::from_secs(20));
            }
            Some((Phase::Running, _)) if left >= 110.0 => status_pill(ui, "Running", true),
            _ => {}
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
        if crate::horizon::is_horizon(inst) {
            // no mods of BlockForge's to count, and no Java: what it plays instead
            let build = self.horizon_tag().map(str::to_string).unwrap_or_else(|| "Not downloaded yet".into());
            stat_cards(
                ui,
                &[
                    ("Last played", last, last_sub),
                    ("Play time", time, sessions),
                    ("Edition", crate::horizon::NAME.to_string(), build),
                    ("Runs as", "Native Rust".to_string(), "No Java while playing".to_string()),
                ],
            );
            return;
        }
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

    /// The days the instance was played over the last weeks, as a strip of bars under a title that says what a bar
    /// measures, with the first and the last day under the strip.
    pub(super) fn overview_history(&mut self, ui: &mut Ui, inst: &Instance) {
        card(ui, |ui| {
            let n = history_span(ui.available_width());
            let days = self.play_days(inst, n);
            let played = days.iter().filter(|d| **d > 0).count();
            let mut head = SplitRow::new(ui, egui::Id::unique("overview-history-head"), 22.0);
            head.left(ui, |ui| {
                ui.label(RichText::new("Play history").size(FS_SECTION).semibold());
            });
            head.right(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(HISTORY_UNIT).size(FS_META).color(c_muted())).truncate());
            });
            head.finish_whole(ui);
            ui.add_space(8.0);
            history_bars(ui, &days, Self::strip_hue(inst));
            ui.add_space(6.0);
            let today = chrono::Local::now().date_naive();
            history_dates(ui, &history_ends(today, n), &super::strip::caption(played, n));
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

/// What the bars of the play history measure, said under its title.
const HISTORY_UNIT: &str = "Sessions per day \u{00B7} height capped at 4";

/// The first day of a play history `n` days long ending `today`, and the last, as they are written under the strip.
fn history_ends(today: chrono::NaiveDate, n: usize) -> (String, String) {
    let first = today - chrono::Duration::days(n.saturating_sub(1) as i64);
    (first.format("%-d %b").to_string(), "Today".to_string())
}

/// What one day of the play history was: "Wed 21 Oct: 5 sessions". The real count, also for a day whose bar is
/// capped.
fn day_detail(day: chrono::NaiveDate, sessions: u8) -> String {
    let n = match sessions {
        0 => "not played".to_string(),
        1 => "1 session".to_string(),
        n => format!("{n} sessions"),
    };
    format!("{}: {n}", day.format("%a %-d %b"))
}

/// The line under the play history: its first day at the left, "Today" at the right, and how many days it was
/// played in the middle (under the strip when there is no room between them).
fn history_dates(ui: &mut Ui, ends: &(String, String), caption: &str) {
    let w = ui.available_width();
    let (row, _) = ui.allocate_exact_size(vec2(w, 16.0), Sense::hover());
    let p = ui.painter();
    let date = |text: &str| p.layout_no_wrap(text.to_string(), super::fonts::mono(FS_META), c_faint());
    let (first, last) = (date(&ends.0), date(&ends.1));
    let mid = p.layout_no_wrap(caption.to_string(), egui::FontId::proportional(FS_META), c_muted());
    let y = |g: &egui::Galley| row.center().y - g.size().y / 2.0;
    let fits = first.size().x + last.size().x + mid.size().x + 48.0 <= w;
    p.galley(pos2(row.left(), y(&first)), first.clone(), c_faint());
    p.galley(pos2(row.right() - last.size().x, y(&last)), last, c_faint());
    if fits {
        p.galley(pos2(row.center().x - mid.size().x / 2.0, y(&mid)), mid, c_muted());
    } else {
        ui.add_space(4.0);
        small_muted(ui, caption);
    }
}

/// The play history as bars across the width: one per day, oldest first and today last (with a ring), filled in the
/// instance's colour on the days it was played. Pointing at a bar says which day it is and how often it was played;
/// so does the strip from the keyboard: it takes the focus, and the arrow keys (Home and End too) go from day to day.
fn history_bars(ui: &mut Ui, days: &[u8], hue: Color32) -> Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, BARS_H), Sense::hover() | Sense::FOCUSABLE);
    let n = days.len().max(1) as f32;
    let bar = ((w - BAR_GAP * (n - 1.0)) / n).max(1.0);
    let last = days.len().saturating_sub(1);
    let index_at = |x: f32| (((x - rect.left()) / (bar + BAR_GAP)).floor().max(0.0) as usize).min(last);
    // the day chosen from the keyboard, kept while the strip has the focus; it starts at today
    let key = resp.id.with("day");
    let focused = resp.has_focus() && !days.is_empty();
    let chosen = if focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(resp.id, egui::EventFilter { horizontal_arrows: true, ..Default::default() })
        });
        let at: usize = ui.data(|d| d.get_temp(key)).unwrap_or(last).min(last);
        let at = ui.input(|i| {
            if i.key_pressed(egui::Key::ArrowLeft) {
                at.saturating_sub(1)
            } else if i.key_pressed(egui::Key::ArrowRight) {
                (at + 1).min(last)
            } else if i.key_pressed(egui::Key::Home) {
                0
            } else if i.key_pressed(egui::Key::End) {
                last
            } else {
                at
            }
        });
        ui.data_mut(|d| d.insert_temp(key, at));
        Some(at)
    } else {
        ui.data_mut(|d| d.remove_temp::<usize>(key));
        None
    };
    let pointed = resp.hover_pos().filter(|_| !days.is_empty()).map(|p| index_at(p.x)).or(chosen);
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
            if i == last {
                p.rect_stroke(r.expand(1.5), radius, Stroke::new(1.0, c_muted()), egui::StrokeKind::Outside);
            }
            if chosen == Some(i) {
                // the day the keyboard is on, under the strip's focus ring
                let mark = Rect::from_min_max(pos2(x, rect.top()), pos2(x + bar, rect.bottom()));
                p.rect_stroke(mark.expand(2.0), radius, Stroke::new(1.5, c_accent()), egui::StrokeKind::Outside);
            }
        }
    }
    focus_ring(ui, &resp, rect.expand(2.0), R_CONTROL);
    let today = chrono::Local::now().date_naive();
    let detail = |i: usize| day_detail(today - chrono::Duration::days((last - i) as i64), days[i]);
    let caption = super::strip::caption(days.iter().filter(|d| **d > 0).count(), days.len());
    let label = match chosen {
        Some(i) => format!("Play history, {caption}. {}", detail(i)),
        None => format!("Play history, {caption}. {HISTORY_UNIT}. Arrow keys go from day to day"),
    };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &label));
    match (resp.hover_pos().and(pointed), chosen) {
        (Some(i), _) => resp.on_hover_text(detail(i)),
        (None, Some(i)) => {
            resp.show_tooltip_text(detail(i));
            resp
        }
        (None, None) => resp,
    }
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
    fn the_banner_shade_keeps_inside_the_rounded_corners() {
        for h in [64.0, 120.0, 196.0] {
            let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(600.0, h));
            let r = 16.0;
            let mesh = bottom_shade_mesh(rect, r, 120);
            for v in &mesh.vertices {
                let p = v.pos;
                assert!(rect.expand(0.01).contains(p), "{p:?} outside {rect:?}");
                // in a corner's square, a point must be within the corner's circle
                for cx in [rect.left() + r, rect.right() - r] {
                    let in_square = (p.x - cx).abs() <= r
                        && p.y > rect.bottom() - r
                        && (p.x < rect.left() + r || p.x > rect.right() - r);
                    if in_square {
                        assert!(p.distance(pos2(cx, rect.bottom() - r)) <= r + 0.01, "{p:?} past the corner (h {h})");
                    }
                }
            }
            // clear at the top, darkest along the bottom
            let top = mesh.vertices.iter().map(|v| v.pos.y).fold(f32::MAX, f32::min);
            assert!(mesh.vertices.iter().filter(|v| v.pos.y == top).all(|v| v.color.a() == 0));
            assert!(mesh.vertices.iter().filter(|v| v.pos.y == rect.bottom()).all(|v| v.color.a() == 120));
        }
    }

    #[test]
    fn a_day_of_the_history_says_how_often_it_was_played_not_how_tall_its_bar_is() {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 21).unwrap();
        assert_eq!(day_detail(day, 0), "Wed 21 Oct: not played");
        assert_eq!(day_detail(day, 1), "Wed 21 Oct: 1 session");
        assert_eq!(day_detail(day, 5), "Wed 21 Oct: 5 sessions");
        // five sessions draw as tall as four, the cap the title speaks of
        assert_eq!(bar_height(5, BARS_H), bar_height(4, BARS_H));
        assert!(HISTORY_UNIT.contains("capped at 4"));
    }

    #[test]
    fn the_history_names_its_first_day_and_today() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        assert_eq!(history_ends(today, 56), ("5 Sep".to_string(), "Today".to_string()));
        assert_eq!(history_ends(today, 14).0, "17 Oct");
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
