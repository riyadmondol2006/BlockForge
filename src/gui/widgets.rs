//! Small reusable pieces: cards, buttons, chips, avatars, toggles.

use egui::{Align, Color32, CornerRadius, FontId, Layout, Response, RichText, Sense, Stroke, Ui, Vec2, vec2};

pub use super::fonts::TextExt;
use super::icons::{self, Icon};
use super::motion;
use super::theme::*;

/// A panel of glass: the card colour, a hairline, a soft shadow and a faint light along its top edge.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let out = card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    });
    glass_edge(ui.painter(), out.response.rect, R_CARD);
    out.inner
}

/// The frame of a [`card`], for code that needs the frame itself.
pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(c_card())
        .stroke(Stroke::new(1.0, c_border()))
        .corner_radius(CornerRadius::same(R_CARD))
        .inner_margin(egui::Margin::same(18))
        .shadow(card_shadow())
}

/// A card that rises a little under the pointer (its shadow grows and its line brightens), for things that open
/// when clicked. Returns what `add` returned and whether the card is under the pointer.
pub fn hover_card<R>(ui: &mut Ui, id: egui::Id, add: impl FnOnce(&mut Ui) -> R) -> (R, bool) {
    let was_hot = ui.data(|d| d.get_temp::<bool>(id.with("hot"))).unwrap_or(false);
    let t = motion::hover(ui, id, was_hot);
    let a = card_shadow();
    let b = card_shadow_raised();
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    let shadow = egui::Shadow {
        offset: [0, lerp(a.offset[1] as u8, b.offset[1] as u8) as i8],
        blur: lerp(a.blur, b.blur),
        spread: 0,
        color: motion::mix(a.color, b.color, t),
    };
    let out = egui::Frame::new()
        .fill(motion::mix(c_card(), c_raised(), t))
        .stroke(Stroke::new(1.0, motion::mix(c_border(), c_control().gamma_multiply(0.7), t)))
        .corner_radius(CornerRadius::same(R_CARD))
        .inner_margin(egui::Margin::same(16))
        .shadow(shadow)
        .show(ui, |ui| add(ui));
    let rect = out.response.rect;
    glass_edge(ui.painter(), rect, R_CARD);
    let hot = ui.rect_contains_pointer(rect);
    ui.data_mut(|d| d.insert_temp(id.with("hot"), hot));
    (out.inner, hot)
}

/// The faint light along the top edge of a glass surface.
pub fn glass_edge(painter: &egui::Painter, rect: egui::Rect, radius: u8) {
    let r = radius as f32;
    if rect.width() <= 2.0 * r {
        return;
    }
    painter.hline(rect.left() + r..=rect.right() - r, rect.top() + 0.5, Stroke::new(1.0, c_glass_edge()));
}

/// The title of a page: a noun, large, no greeting.
pub fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(FS_TITLE).semibold());
}

// for the pages to take up: none uses it yet
#[allow(dead_code)]
/// The room between a page's title and its actions, and under the whole head.
pub const HEADER_GAP: f32 = 16.0;

// for the pages to take up: none uses it yet
#[allow(dead_code)]
/// Whether a page head's actions have to go under its title: they don't fit beside it in `avail` points.
pub fn header_wraps(avail: f32, title_w: f32, actions_w: f32) -> bool {
    actions_w > 0.0 && title_w + HEADER_GAP + actions_w > avail
}

// for the pages to take up: none uses it yet
#[allow(dead_code)]
/// The head of a page, the same on every page: the title (with a count beside it and a short line under it, both
/// optional) on the left and the page's actions on the right. When the window is too narrow for both on one line,
/// the actions go on a line of their own under the title instead of running over it.
///
/// ```ignore
/// PageHeader::new("Library").count(instances.len()).show(ui, |ui| {
///     // right to left: the main action first
///     primary_icon(ui, Icon::Plus, "New instance");
///     secondary(ui, "Import");
/// });
/// ```
pub struct PageHeader<'a> {
    title: &'a str,
    count: Option<String>,
    subtitle: Option<&'a str>,
}

// for the pages to take up: none uses it yet
#[allow(dead_code)]
impl<'a> PageHeader<'a> {
    pub fn new(title: &'a str) -> Self {
        PageHeader { title, count: None, subtitle: None }
    }

    /// A number in a small pill after the title (how many instances, servers, results).
    pub fn count(mut self, n: impl ToString) -> Self {
        self.count = Some(n.to_string());
        self
    }

    /// A short grey line under the title.
    pub fn subtitle(mut self, text: &'a str) -> Self {
        self.subtitle = Some(text).filter(|t| !t.is_empty());
        self
    }

    /// Lay out the head. `actions` is laid out right to left (the first one added sits at the right edge) when it
    /// fits beside the title, and left to right on a line of its own under the title when it doesn't.
    pub fn show(self, ui: &mut Ui, actions: impl FnOnce(&mut Ui)) {
        let id = ui.scope_id().with(("page-header", self.title));
        let actions_w = remembered_width(ui, id, 0.0);
        let title_w = {
            let font = FontId::proportional(FS_TITLE);
            let w = ui.painter().layout_no_wrap(self.title.to_string(), font, c_text()).size().x;
            w + self.count.as_ref().map_or(0.0, |c| count_badge_width(ui, c) + 10.0)
        };
        let avail = ui.available_width();
        let wraps = header_wraps(avail, title_w, actions_w);
        let title = |ui: &mut Ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    heading(ui, self.title);
                    if let Some(c) = &self.count {
                        count_badge(ui, c);
                    }
                });
                if let Some(s) = self.subtitle {
                    ui.add(egui::Label::new(RichText::new(s).color(c_muted())).wrap());
                }
            });
        };
        if wraps {
            title(ui);
            ui.add_space(8.0);
            let used = ui.horizontal_wrapped(|ui| {
                actions(ui);
                ui.min_rect().width()
            });
            // only ever wider while wrapped, so the head doesn't flip back and forth at one window width
            remember_width(ui, id, used.inner.max(actions_w));
        } else {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_max_width((avail - actions_w - HEADER_GAP).max(title_w.min(avail)));
                    title(ui);
                });
                let used = ui
                    .with_layout(Layout::right_to_left(Align::Center), |ui| {
                        actions(ui);
                        ui.min_rect().width()
                    })
                    .inner;
                remember_width(ui, id, used);
            });
        }
        ui.add_space(HEADER_GAP);
    }
}

// for the pages to take up: none uses it yet
#[allow(dead_code)]
fn count_badge_width(ui: &Ui, text: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(FS_META), c_muted());
    (galley.size().x + 14.0).max(24.0)
}

// for the pages to take up: none uses it yet
#[allow(dead_code)]
/// A number in a small pill, beside a title (how many instances, servers, results).
pub fn count_badge(ui: &mut Ui, text: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(FS_META), c_muted());
    let (rect, resp) = ui.allocate_exact_size(vec2(count_badge_width(ui, text), 22.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(11), c_field(), Stroke::new(1.0, c_border()), egui::StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, c_muted());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
    resp
}

/// Where a column of at most `max` points starts inside `avail` points, and how wide it is: the whole width when
/// there is no more room than that, centred when there is.
pub fn centred_column(avail: f32, max: f32) -> (f32, f32) {
    if avail <= max { (0.0, avail.max(0.0)) } else { (((avail - max) / 2.0).floor(), max) }
}

/// Lay out `add` in a column of at most `max` points, centred in the room there is: for text and forms, which are
/// hard to read when they run across a wide screen. Lists and grids should use the whole width instead.
pub fn readable_column<R>(ui: &mut Ui, max: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let rect = ui.available_rect_before_wrap();
    let (offset, width) = centred_column(rect.width(), max);
    if offset <= 0.0 {
        return add(ui);
    }
    let column = egui::Rect::from_min_size(rect.min + vec2(offset, 0.0), vec2(width, rect.height()));
    // the wheel over the empty sides scrolls the page too: the movement is handed to the page's scroll area (the
    // first one laid out in the column) as if the pointer were over it
    let in_margin = ui.ctx().pointer_hover_pos().is_some_and(|p| rect.contains(p) && !column.contains(p))
        && ui.ctx().layer_id_at(ui.ctx().pointer_hover_pos().unwrap_or_default()) == Some(ui.layer_id());
    if in_margin {
        let delta = ui.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta));
        if delta != Vec2::ZERO {
            ui.scroll_with_delta(delta);
        }
    }
    ui.scope_builder(egui::UiBuilder::new().max_rect(column).layout(*ui.layout()), add).inner
}

/// The title of a block on a page.
pub fn section(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).size(FS_SECTION).semibold());
}

pub fn muted(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).color(c_muted()));
}

pub fn small_muted(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).color(c_muted()).size(12.0));
}

fn styled_button(
    ui: &mut Ui,
    text: &str,
    icon: Option<Icon>,
    fill: Color32,
    hover: Color32,
    fg: Color32,
    stroke: Stroke,
    min: Vec2,
    enabled: bool,
) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(14.0), fg);
    let icon_w = if icon.is_some() { 22.0 } else { 0.0 };
    let size = vec2((galley.size().x + icon_w + 28.0).max(min.x), min.y.max(32.0));
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    shot_note(text, rect);
    if ui.is_rect_visible(rect) {
        let t = motion::hover(ui, resp.id, enabled && resp.hovered());
        let press = motion::toward(ui.ctx(), resp.id.with("press"), resp.is_pointer_button_down_on(), 0.08);
        let shape = rect.shrink(press * 1.0);
        let (bg, fg) = if !enabled { (c_field(), c_faint()) } else { (motion::mix(fill, hover, t), fg) };
        ui.painter().rect(
            shape,
            CornerRadius::same(R_CONTROL),
            bg,
            if enabled { stroke } else { Stroke::new(1.0, c_border()) },
            egui::StrokeKind::Inside,
        );
        let galley = ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(14.0), fg);
        let content_w = galley.size().x + icon_w;
        let left = rect.center().x - content_w / 2.0;
        if let Some(icon) = icon {
            icons::paint(ui.painter(), icon, egui::pos2(left + 7.0, rect.center().y), 15.0, fg);
        }
        ui.painter().galley(egui::pos2(left + icon_w, rect.center().y - galley.size().y / 2.0), galley, fg);
        focus_ring(ui, &resp, rect, R_CONTROL);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, text));
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// A button in the brand gradient: the main action of a screen, and Play. `glow` lights the ground under it.
fn brand_button(ui: &mut Ui, text: &str, icon: Option<Icon>, min: Vec2, enabled: bool, glow: bool) -> Response {
    if !enabled {
        return styled_button(ui, text, icon, c_field(), c_field(), c_faint(), Stroke::NONE, min, false);
    }
    let fg = c_on_accent();
    let galley = ui.painter().layout_no_wrap(text.to_string(), super::fonts::semibold(14.0), fg);
    let icon_w = if icon.is_some() { 22.0 } else { 0.0 };
    let size = vec2((galley.size().x + icon_w + 32.0).max(min.x), min.y.max(34.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    shot_note(text, rect);
    if ui.is_rect_visible(rect) {
        let t = motion::hover(ui, resp.id, resp.hovered());
        let press = motion::toward(ui.ctx(), resp.id.with("press"), resp.is_pointer_button_down_on(), 0.08);
        let shape = rect.shrink(press * 1.5);
        if glow {
            let base = c_accent();
            let alpha = if is_dark() { 70.0 + 50.0 * t } else { 45.0 + 35.0 * t };
            let shadow = egui::Shadow {
                offset: [0, 8],
                blur: (18.0 + 10.0 * t) as u8,
                spread: 0,
                color: Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha as u8),
            };
            ui.painter().add(shadow.as_shape(shape, CornerRadius::same(R_CONTROL)));
        }
        let (a, b) =
            (motion::mix(c_accent(), c_accent_hover(), t * 0.6), motion::mix(c_accent2(), c_accent_hover(), t * 0.3));
        motion::gradient_rect(ui.painter(), shape, R_CONTROL, a, b);
        // a sheen along the top
        ui.painter().hline(
            shape.left() + R_CONTROL as f32..=shape.right() - R_CONTROL as f32,
            shape.top() + 0.5,
            Stroke::new(1.0, Color32::from_white_alpha(60)),
        );
        let content_w = galley.size().x + icon_w;
        let left = shape.center().x - content_w / 2.0;
        if let Some(icon) = icon {
            icons::paint(ui.painter(), icon, egui::pos2(left + 7.0, shape.center().y), 15.0, fg);
        }
        ui.painter().galley(egui::pos2(left + icon_w, shape.center().y - galley.size().y / 2.0), galley, fg);
        focus_ring(ui, &resp, rect, R_CONTROL);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The outline a hand-drawn control gets when the keyboard (Tab) has moved to it, so Tab and Enter can be used
/// without guessing where the focus is.
pub fn focus_ring(ui: &Ui, resp: &Response, rect: egui::Rect, radius: u8) {
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(radius.saturating_add(2)),
            Stroke::new(2.0, c_accent()),
            egui::StrokeKind::Outside,
        );
    }
}

// The main action of a screen is the brand gradient.
pub fn primary(ui: &mut Ui, text: &str) -> Response {
    brand_button(ui, text, None, Vec2::ZERO, true, false)
}

pub fn primary_icon(ui: &mut Ui, icon: Icon, text: &str) -> Response {
    brand_button(ui, text, Some(icon), Vec2::ZERO, true, false)
}

pub fn primary_enabled(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    brand_button(ui, text, None, Vec2::ZERO, enabled, false)
}

#[cfg_attr(not(feature = "converter"), allow(dead_code))]
pub fn primary_enabled_icon(ui: &mut Ui, icon: Icon, text: &str, enabled: bool) -> Response {
    brand_button(ui, text, Some(icon), Vec2::ZERO, enabled, false)
}

/// The button that starts Minecraft: the brand gradient with a glow under it. `big` is the 44px version of Home and
/// an instance's page.
pub fn play_button(ui: &mut Ui, big: bool) -> Response {
    let min = if big { vec2(136.0, 44.0) } else { Vec2::ZERO };
    brand_button(ui, "Play", Some(Icon::Play), min, true, true)
}

pub fn secondary(ui: &mut Ui, text: &str) -> Response {
    styled_button(ui, text, None, c_field(), c_field_hover(), c_text(), Stroke::new(1.0, c_control()), Vec2::ZERO, true)
}

pub fn secondary_icon(ui: &mut Ui, icon: Icon, text: &str) -> Response {
    styled_button(
        ui,
        text,
        Some(icon),
        c_field(),
        c_field_hover(),
        c_text(),
        Stroke::new(1.0, c_control()),
        Vec2::ZERO,
        true,
    )
}

pub fn danger(ui: &mut Ui, text: &str) -> Response {
    styled_button(
        ui,
        text,
        None,
        c_danger_bg(),
        c_danger_hover(),
        c_danger(),
        Stroke::new(1.0, c_danger_line()),
        Vec2::ZERO,
        true,
    )
}

pub fn danger_enabled(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    styled_button(
        ui,
        text,
        None,
        c_danger_bg(),
        c_danger_hover(),
        c_danger(),
        Stroke::new(1.0, c_danger_line()),
        Vec2::ZERO,
        enabled,
    )
}

/// A filter or a choice that can be switched on: a pill, tinted with the brand colour when selected.
pub fn chip(ui: &mut Ui, text: &str, selected: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(13.0), Color32::PLACEHOLDER);
    let size = vec2(galley.size().x + 26.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let t = motion::hover(ui, resp.id, resp.hovered());
    let s = motion::toward(ui.ctx(), resp.id.with("on"), selected, motion::FAST);
    let bg = motion::mix(motion::mix(c_field(), c_field_hover(), t), c_accent_soft(), s);
    let line = motion::mix(c_border(), c_accent().gamma_multiply(0.6), s);
    let fg = motion::mix(c_text(), if is_dark() { c_accent() } else { c_accent_hover() }, s);
    ui.painter().rect(rect, CornerRadius::same(15), bg, Stroke::new(1.0, line), egui::StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    focus_ring(ui, &resp, rect, 15);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, text));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A row of tabs: the labels sit on one line with a hairline under them, and the open one is underlined in ink. Fill
/// it with [`tab`] calls.
pub fn tab_bar<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let width = ui.available_width();
    let left = ui.cursor().min.x;
    let out = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 24.0;
        let key = ui.scope_id().with("tab-selected");
        ui.data_mut(|d| d.remove::<egui::Rect>(key));
        (add(ui), key)
    });
    let (inner, key) = out.inner;
    let y = out.response.rect.bottom();
    ui.painter().hline(left..=left + width, y, Stroke::new(1.0, c_border()));
    // the line under the open tab slides to the tab that was chosen
    if let Some(sel) = ui.data(|d| d.get_temp::<egui::Rect>(key)) {
        let x = motion::glide(ui.ctx(), key.with("x"), sel.left(), motion::MEDIUM);
        let w = motion::glide(ui.ctx(), key.with("w"), sel.width(), motion::MEDIUM);
        let bar = egui::Rect::from_min_size(egui::pos2(x, y - 2.5), vec2(w.max(4.0), 3.0));
        motion::brand_rect(ui.painter(), bar, 2);
    }
    inner
}

/// One tab of a [`tab_bar`].
pub fn tab(ui: &mut Ui, text: &str, selected: bool) -> Response {
    tab_marked(ui, text, selected, None)
}

/// A [`tab`] with room for a small dot after its name, shown when `mark` is `Some(true)` (something waits there,
/// such as changes not saved yet). The room is kept when there is no dot, so the tabs don't move when it comes.
pub fn tab_marked(ui: &mut Ui, text: &str, selected: bool, mark: Option<bool>) -> Response {
    const DOT_ROOM: f32 = 10.0;
    let galley = ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(14.0), egui::Color32::PLACEHOLDER);
    let extra = if mark.is_some() { DOT_ROOM } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + extra, 38.0), Sense::click());
    let t = motion::hover(ui, resp.id, selected || resp.hovered());
    let color = motion::mix(c_muted(), c_text(), t);
    let text_right = rect.left() + galley.size().x;
    ui.painter().galley(egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0 - 1.0), galley, color);
    if mark == Some(true) {
        ui.painter().circle_filled(egui::pos2(text_right + DOT_ROOM / 2.0 + 1.0, rect.center().y - 5.0), 3.0, c_warn());
    }
    if selected {
        // the bar itself is drawn by the tab bar, which slides it from tab to tab. The text is measured before the
        // data is locked: laying text out needs that lock too
        let key = ui.scope_id().with("tab-selected");
        let underline =
            egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.left() + galley_w(text, ui), rect.bottom()));
        ui.data_mut(|d| d.insert_temp(key, underline));
    }
    focus_ring(ui, &resp, rect.expand2(vec2(4.0, -2.0)), 6);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, text));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn galley_w(text: &str, ui: &Ui) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(14.0), Color32::PLACEHOLDER).size().x
}

/// The three dots of a card or a row: a small square that opens a menu. Returns what the menu's closure returned when
/// the menu is open this frame.
pub fn kebab_menu<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let t = motion::hover(ui, resp.id, resp.hovered() || open);
    if t > 0.0 {
        ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), c_hover().gamma_multiply(t));
    }
    let color = motion::mix(c_muted(), c_text(), t);
    icons::paint(ui.painter(), Icon::Kebab, rect.center(), 16.0, color);
    focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "More"));
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("More");
    egui::Popup::menu(&resp).show(add).map(|r| r.inner)
}

/// How many cards of at least `min` width fit in `avail` with `gap` between them, and the width each then gets so
/// the row ends exactly at the edge instead of leaving a ragged strip.
pub fn grid_columns(avail: f32, min: f32, gap: f32) -> (usize, f32) {
    let cols = (((avail + gap) / (min + gap)).floor() as usize).max(1);
    (cols, ((avail - gap * (cols as f32 - 1.0)) / cols as f32).floor())
}

/// A small non-interactive label (loader name, account kind): a pale tint of its colour, no outline.
pub fn tag(ui: &mut Ui, text: &str, color: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(12.0), color);
    let size = vec2(galley.size().x + 12.0, 20.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(R_TAG), color.gamma_multiply(0.14));
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
}

/// The quiet Play button of a card: neutral at rest, green while the card is under the pointer, so a screen of
/// instances is not a screen of green buttons.
pub fn play_chip(ui: &mut Ui, playing: bool, card_hovered: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
    let hot = resp.hovered() || resp.is_pointer_button_down_on();
    let t = motion::hover(ui, resp.id, !playing && (hot || card_hovered));
    let fg = if playing {
        let (bg, stroke) = if hot { (c_danger_hover(), Stroke::NONE) } else { (c_danger_bg(), Stroke::NONE) };
        ui.painter().rect(rect, CornerRadius::same(17), bg, stroke, egui::StrokeKind::Inside);
        c_danger()
    } else {
        // neutral at rest, the brand gradient fading in as the card or the button is pointed at
        ui.painter().rect(
            rect,
            CornerRadius::same(17),
            c_field(),
            Stroke::new(1.0, c_control().gamma_multiply(1.0 - t)),
            egui::StrokeKind::Inside,
        );
        if t > 0.0 {
            let (a, b) = (c_accent().gamma_multiply(t), c_accent2().gamma_multiply(t));
            motion::gradient_rect(ui.painter(), rect, 17, a, b);
        }
        motion::mix(c_text(), c_on_accent(), t)
    };
    icons::paint(ui.painter(), if playing { Icon::Stop } else { Icon::Play }, rect.center(), 13.0, fg);
    focus_ring(ui, &resp, rect, 17);
    let label = if playing { "Stop" } else { "Play" };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(label)
}

pub fn loader_color(name: &str) -> Color32 {
    // lighter tones read well on the dark look, darker ones on the light look
    let dark = super::theme::is_dark();
    let (d, l) = match name {
        "Fabric" => ((0xD9, 0xB5, 0x8A), (0x7A, 0x50, 0x19)),
        "Quilt" => ((0xB9, 0x8B, 0xE0), (0x6E, 0x45, 0xA6)),
        "Forge" => ((0x7F, 0xA8, 0xE8), (0x2A, 0x5F, 0x9E)),
        "NeoForge" => ((0xF0, 0x9A, 0x4F), (0x9A, 0x4E, 0x0E)),
        _ => return c_muted(),
    };
    let (r, g, b) = if dark { d } else { l };
    Color32::from_rgb(r, g, b)
}

/// A rounded square with the first letter, in a colour taken from the name.
pub fn avatar(ui: &mut Ui, name: &str, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    paint_avatar(ui, rect, name);
    resp
}

pub fn paint_avatar(ui: &Ui, rect: egui::Rect, name: &str) {
    // each name gets a stable hue; the square is a pale tint of it with the letter in the hue itself, so a list of
    // instances is told apart without being a wall of saturated blocks
    const HUES: [(u8, u8, u8); 8] = [
        (0x4A, 0xC9, 0x7D),
        (0x4F, 0xA8, 0xE0),
        (0xE0, 0x9A, 0x4F),
        (0xA8, 0x86, 0xE8),
        (0xE0, 0x6F, 0x8F),
        (0x8F, 0xC8, 0x4A),
        (0x3F, 0xC4, 0xC0),
        (0xD9, 0xB5, 0x4A),
    ];
    let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let (r, g, b) = HUES[(h % HUES.len() as u32) as usize];
    let hue = Color32::from_rgb(r, g, b);
    let (bg, ink) = if is_dark() {
        (hue.gamma_multiply(0.20), hue)
    } else {
        // darker letter on a pale tint keeps 4.5:1 on white
        (hue.gamma_multiply(0.22), Color32::from_rgb(r / 2, g / 2, b / 2))
    };
    let radius = (rect.width() * 0.26).round() as u8;
    // the tint is laid over the card colour so it does not change with what is behind it
    ui.painter().rect_filled(rect, CornerRadius::same(radius), c_card_hover());
    ui.painter().rect_filled(rect, CornerRadius::same(radius), bg);
    let letter: String =
        name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into());
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        letter,
        FontId::proportional(rect.width() * 0.44),
        ink,
    );
}

/// An on/off switch: ink when on, a quiet outlined track when off.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    toggle_named(ui, on, "")
}

/// [`toggle`] that tells a screen reader what it switches: `name` is the title of its row ("Item frames", the mod's
/// name).
pub fn toggle_named(ui: &mut Ui, on: &mut bool, name: &str) -> Response {
    let size = vec2(36.0, 20.0);
    // the track is 36 x 20; the part that takes a click is 40 x 28
    let (hit, mut resp) = ui.allocate_exact_size(vec2(40.0, 28.0), Sense::click());
    let rect = egui::Rect::from_center_size(hit.center(), size);
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = motion::toward(ui.ctx(), resp.id.with("on"), *on, motion::MEDIUM);
    let radius = CornerRadius::same(10);
    ui.painter().rect_filled(rect, radius, c_active());
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, c_control().gamma_multiply(1.0 - t)),
        egui::StrokeKind::Inside,
    );
    if t > 0.0 {
        motion::gradient_rect(ui.painter(), rect, 10, c_accent().gamma_multiply(t), c_accent2().gamma_multiply(t));
    }
    let cx = rect.left() + 10.0 + t * (size.x - 20.0);
    let knob = motion::mix(c_muted(), Color32::WHITE, t);
    ui.painter().circle_filled(egui::pos2(cx, rect.center().y + 1.0), 7.5, Color32::from_black_alpha(40));
    ui.painter().circle_filled(egui::pos2(cx, rect.center().y), 7.0, knob);
    focus_ring(ui, &resp, rect, 10);
    let state = *on;
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, true, state, name));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A setting row: title and explanation on the left, the control on the right. The text makes room for the control
/// (as wide as it was on the last frame), so a wide drop-down never runs over the text or out of the card.
pub fn setting_row(ui: &mut Ui, title: &str, hint: &str, control: impl FnOnce(&mut Ui)) {
    let id = ui.scope_id().with(("setting-row", title));
    let control_w = remembered_width(ui, id, 190.0);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width((ui.available_width() - control_w - 12.0).max(140.0));
            ui.label(RichText::new(title).semibold());
            if !hint.is_empty() {
                small_muted(ui, hint);
            }
        });
        let used = ui
            .with_layout(Layout::right_to_left(Align::Center), |ui| {
                control(ui);
                ui.min_rect().width()
            })
            .inner;
        remember_width(ui, id, used);
    });
}

/// How wide a group of controls was on the last frame (see [`remember_width`]), or `guess` the first time.
pub fn remembered_width(ui: &Ui, id: egui::Id, guess: f32) -> f32 {
    ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(guess)
}

/// Keep how wide a group of controls turned out, for what is laid out beside it on the next frame. When it changed,
/// the frame is laid out once more before it is shown, so nothing overlaps even for a moment.
pub fn remember_width(ui: &Ui, id: egui::Id, width: f32) {
    let before = ui.data(|d| d.get_temp::<f32>(id));
    if before.is_none_or(|b| (b - width).abs() > 0.5) {
        ui.data_mut(|d| d.insert_temp(id, width));
        ui.ctx().request_discard("the width of a row changed");
    }
}

/// Give a dialog's text field the keyboard focus on the first frame it is shown, so typing can start at once (Tab still
/// moves on from it afterwards).
pub fn focus_when_shown(ui: &Ui, field: &Response) {
    let pass = ui.ctx().cumulative_pass_nr();
    let key = field.id.with("shown-in-pass");
    let before = ui.data_mut(|d| d.get_temp::<u64>(key));
    ui.data_mut(|d| d.insert_temp(key, pass));
    if before.is_none_or(|b| b + 1 < pass) {
        field.request_focus();
    }
}

/// A hairline across the width of a card or list, for dividing rows.
pub fn rule(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, c_rule());
}

/// A row of choices joined into one control, the chosen one raised by one tone step. Returns the one that was pressed.
pub fn segmented(ui: &mut Ui, labels: &[&str], selected: usize) -> Option<usize> {
    let widths: Vec<f32> = labels
        .iter()
        .map(|l| ui.painter().layout_no_wrap(l.to_string(), super::fonts::medium(13.0), c_text()).size().x + 24.0)
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 6.0;
    let (outer, outer_resp) = ui.allocate_exact_size(vec2(total, 34.0), Sense::hover());
    ui.painter().rect(
        outer,
        CornerRadius::same(R_CONTROL),
        c_field(),
        Stroke::new(1.0, c_border()),
        egui::StrokeKind::Inside,
    );
    // the chosen segment's pill slides to the one that is chosen next
    let mut sel_x = outer.left() + 3.0;
    for w in widths.iter().take(selected) {
        sel_x += w;
    }
    if let Some(w) = widths.get(selected) {
        let id = outer_resp.id.with("pill");
        let x = motion::glide(ui.ctx(), id.with("x"), sel_x, motion::MEDIUM);
        let pw = motion::glide(ui.ctx(), id.with("w"), *w, motion::MEDIUM);
        let pill = egui::Rect::from_min_size(egui::pos2(x, outer.top() + 3.0), vec2(pw, 28.0));
        ui.painter().add(
            egui::Shadow {
                offset: [0, 2],
                blur: 6,
                spread: 0,
                color: Color32::from_black_alpha(if is_dark() { 80 } else { 22 }),
            }
            .as_shape(pill, CornerRadius::same(R_CONTROL - 3)),
        );
        ui.painter().rect_filled(
            pill,
            CornerRadius::same(R_CONTROL - 3),
            if is_dark() { c_active() } else { c_card() },
        );
    }
    let mut pressed = None;
    let mut x = outer.left() + 3.0;
    for (i, w) in widths.into_iter().enumerate() {
        let rect = egui::Rect::from_min_size(egui::pos2(x, outer.top() + 3.0), vec2(w, 28.0));
        x += w;
        let resp = ui.interact(rect, ui.scope_id().with(("segment", i, labels[i])), Sense::click());
        let on = i == selected;
        let t = motion::hover(ui, resp.id, on || resp.hovered());
        let fg = motion::mix(c_muted(), c_text(), t);
        let galley = ui.painter().layout_no_wrap(labels[i].to_string(), super::fonts::medium(13.0), fg);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
        focus_ring(ui, &resp, rect, R_CONTROL - 3);
        let label = labels[i];
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, label));
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            pressed = Some(i);
        }
    }
    pressed
}

/// Numbers as type: a grey label over a value, the cells divided by hairlines. In a narrow window the cells wrap onto
/// more rows.
pub fn facts(ui: &mut Ui, cells: &[(&str, String)]) {
    let w = ui.available_width();
    let cols = ((w / 128.0).floor() as usize).clamp(1, cells.len().max(1));
    let cw = (w / cols as f32).floor();
    for row in cells.chunks(cols) {
        let (rect, _) = ui.allocate_exact_size(vec2(w, 52.0), Sense::hover());
        for (i, (label, value)) in row.iter().enumerate() {
            let left = rect.left() + i as f32 * cw;
            if i > 0 {
                ui.painter().vline(left, rect.top() + 4.0..=rect.bottom() - 4.0, Stroke::new(1.0, c_rule()));
            }
            let cell = egui::Rect::from_min_max(
                egui::pos2(left + if i == 0 { 0.0 } else { 16.0 }, rect.top()),
                egui::pos2(left + cw - 8.0, rect.bottom()),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(cell), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                small_muted(ui, label.to_string());
                ui.add(
                    egui::Label::new(
                        RichText::new(value.as_str()).size(FS_FIGURE).family(egui::FontFamily::Name("medium".into())),
                    )
                    .truncate(),
                );
            });
        }
    }
}

/// A neutral button that can be switched off (greyed and outlined, and it does nothing).
pub fn secondary_enabled(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    if enabled { secondary(ui, text) } else { danger_enabled(ui, text, false) }
}

/// A low-weight action: text with a fill only under the pointer.
pub fn ghost(ui: &mut Ui, text: &str) -> Response {
    // laid out without a colour of its own, so the one given when it is painted (muted, or full under the pointer)
    // is the one it gets
    let galley = ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(14.0), egui::Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 24.0, 34.0), Sense::click());
    let t = motion::hover(ui, resp.id, resp.hovered());
    if t > 0.0 {
        let fill = if resp.is_pointer_button_down_on() { c_active() } else { c_hover() };
        ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), fill.gamma_multiply(t));
    }
    let fg = motion::mix(c_muted(), c_text(), t);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Grey bars in the shape of rows, while the first content is on its way.
pub fn skeleton_rows(ui: &mut Ui, rows: usize, height: f32) {
    // the bars breathe gently while the list is on its way
    let phase = if motion::reduced(ui.ctx()) { 0.5 } else { (ui.input(|i| i.time) * 2.4).sin() as f32 * 0.5 + 0.5 };
    if !motion::reduced(ui.ctx()) {
        ui.ctx().request_repaint_after(ANIMATION_FRAME);
    }
    let shade = motion::mix(c_field(), c_active(), phase * 0.7);
    for _ in 0..rows {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        let p = ui.painter();
        let icon = height.min(48.0) - 12.0;
        let r = CornerRadius::same(R_TAG);
        p.rect_filled(
            egui::Rect::from_min_size(rect.min + vec2(8.0, 6.0), vec2(icon, icon)),
            CornerRadius::same(R_CONTROL),
            shade,
        );
        let x = rect.left() + icon + 20.0;
        p.rect_filled(
            egui::Rect::from_min_size(egui::pos2(x, rect.top() + 10.0), vec2(rect.width() * 0.25, 10.0)),
            r,
            shade,
        );
        p.rect_filled(
            egui::Rect::from_min_size(egui::pos2(x, rect.top() + 28.0), vec2(rect.width() * 0.45, 8.0)),
            r,
            shade,
        );
    }
}

/// A thin progress bar; `None` is "working, no number yet".
pub fn progress_bar(ui: &mut Ui, fraction: Option<f32>, height: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let r = CornerRadius::same((height / 2.0) as u8);
    ui.painter().rect_filled(rect, r, c_active());
    match fraction {
        Some(f) => {
            // the bar eases to each new figure instead of stepping
            let f = motion::glide(ui.ctx(), resp.id.with("fill"), f.clamp(0.0, 1.0), motion::MEDIUM);
            let w = rect.width() * f;
            if w >= 1.0 {
                let bar = egui::Rect::from_min_size(rect.min, vec2(w.max(height), height));
                motion::gradient_rect(
                    ui.painter(),
                    bar,
                    (height / 2.0) as u8,
                    c_accent(),
                    motion::mix(c_accent(), c_accent2(), f),
                );
            }
        }
        None => {
            let t = ui.input(|i| i.time) as f32;
            let seg = rect.width() * 0.3;
            let x = rect.left() + (t * 0.8).fract() * (rect.width() + seg) - seg;
            let bar = egui::Rect::from_min_max(
                egui::pos2(x.max(rect.left()), rect.top()),
                egui::pos2((x + seg).min(rect.right()), rect.bottom()),
            );
            if bar.width() > 0.0 {
                motion::brand_rect(ui.painter(), bar, (height / 2.0) as u8);
            }
            ui.ctx().request_repaint_after(ANIMATION_FRAME);
        }
    }
    resp
}

/// How often something that moves by itself (a spinner, a bar with no number yet) is drawn again: smooth to the eye,
/// but not every frame the screen can show. A long install or a search shows one for a while, and drawing the whole
/// window 60 to 144 times a second for it kept a processor core busy (or several, without a screen to wait for).
pub const ANIMATION_FRAME: std::time::Duration = std::time::Duration::from_millis(33);

/// The "working" spinner, as egui draws it, but drawn again only every [`ANIMATION_FRAME`].
pub fn spinner(ui: &mut Ui) -> Response {
    let size = ui.style().spacing.interact_size.y;
    spinner_sized(ui, size)
}

/// [`spinner`] at `size` points.
pub fn spinner_sized(ui: &mut Ui, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::new(egui::Role::ProgressIndicator));
    if ui.is_rect_visible(rect) {
        ui.ctx().request_repaint_after(ANIMATION_FRAME);
        let radius = (rect.height().min(rect.width()) * 0.5) - 2.0;
        let n_points = (radius.round() as u32).clamp(8, 128);
        let time = ui.input(|i| i.time);
        let start = time * std::f64::consts::TAU;
        let end = start + 240f64.to_radians() * time.sin();
        let points: Vec<egui::Pos2> = (0..n_points)
            .map(|i| {
                let angle = egui::emath::lerp(start..=end, i as f64 / n_points as f64);
                let (sin, cos) = angle.sin_cos();
                rect.center() + radius * vec2(cos as f32, sin as f32)
            })
            .collect();
        ui.painter().add(egui::Shape::line(points, Stroke::new(3.0, c_accent())));
    }
    resp
}

/// The kinds of [`notice`].
#[derive(Clone, Copy, PartialEq)]
pub enum NoticeKind {
    Info,
    Warn,
    Error,
}

/// A calm inline message: a tinted row with one sentence, and an optional action. Returns the action's response.
pub fn notice(ui: &mut Ui, kind: NoticeKind, text: &str, action: Option<&str>) -> Option<Response> {
    let (bg, fg, line) = match kind {
        NoticeKind::Info => (c_field(), c_text(), c_border()),
        NoticeKind::Warn => (c_warn_bg(), c_warn(), c_warn().gamma_multiply(0.4)),
        NoticeKind::Error => (c_danger_bg(), c_danger(), c_danger_line()),
    };
    egui::Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0, line))
        .corner_radius(CornerRadius::same(R_CONTROL))
        .inner_margin(egui::Margin::symmetric(14, 11))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(text).color(fg)).wrap());
                action.map(|a| ui.with_layout(Layout::right_to_left(Align::Center), |ui| secondary(ui, a)).inner)
            })
            .inner
        })
        .inner
}

/// "‹ Library": the small way back above a page's title.
pub fn back_link(ui: &mut Ui, text: &str) -> Response {
    let font = super::fonts::medium(13.0);
    let w = ui.painter().layout_no_wrap(text.to_string(), font.clone(), c_muted()).size().x;
    let (rect, resp) = ui.allocate_exact_size(vec2(w + 12.0 + 14.0, 28.0), Sense::click());
    let t = motion::hover(ui, resp.id, resp.hovered());
    let fg = motion::mix(c_muted(), c_text(), t);
    if ui.is_rect_visible(rect) {
        icons::paint(ui.painter(), Icon::Back, egui::pos2(rect.left() + 8.0, rect.center().y), 13.0, fg);
        let galley = ui.painter().layout_no_wrap(text.to_string(), font, fg);
        ui.painter().galley(egui::pos2(rect.left() + 18.0, rect.center().y - galley.size().y / 2.0), galley, fg);
    }
    focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn search_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    ui.add_sized(
        [width, 34.0],
        egui::TextEdit::singleline(text)
            .hint_text(RichText::new(hint).color(c_faint()))
            .margin(egui::Margin::symmetric(10, 7))
            .background_color(c_field()),
    )
}

pub fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{bytes} B")
    }
}

pub fn human_count(n: u64) -> String {
    let n = n as f64;
    if n >= 1e9 {
        format!("{:.1}B", n / 1e9)
    } else if n >= 1e6 {
        format!("{:.1}M", n / 1e6)
    } else if n >= 1e3 {
        format!("{:.1}K", n / 1e3)
    } else {
        format!("{n:.0}")
    }
}

pub fn time_ago(ms: i64) -> String {
    if ms <= 0 {
        return "Never played".into();
    }
    ago(chrono::Utc::now().timestamp_millis().saturating_sub(ms) / 1000)
}

/// "5 minutes ago" for a number of seconds (a time in the future, from a clock that was wrong, is "just now").
fn ago(secs: i64) -> String {
    let n = |count: i64, one: &str| {
        if count == 1 { format!("1 {one} ago") } else { format!("{count} {one}s ago") }
    };
    match secs {
        s if s < 60 => "Just now".into(),
        s if s < 3600 => n(s / 60, "minute"),
        s if s < 86400 => n(s / 3600, "hour"),
        s if s < 86400 * 30 => n(s / 86400, "day"),
        s if s < 86400 * 365 => n(s / (86400 * 30), "month"),
        s => n(s / (86400 * 365), "year"),
    }
}

// The screenshot tour (`BLOCKFORGE_SHOT`) opens some windows the way a player does: it asks where the first button
// with a given label is drawn, then clicks there with a pointer of its own. Nothing is looked for otherwise, and then
// a button only reads one flag.

static SHOT_SEEKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SHOT_SEEK: std::sync::Mutex<Option<(String, Option<egui::Rect>)>> = std::sync::Mutex::new(None);

/// Start looking for the first button labelled `label` (or stop looking, with `None`).
pub fn shot_seek(label: Option<&str>) {
    let mut seek = SHOT_SEEK.lock().unwrap_or_else(|e| e.into_inner());
    *seek = label.map(|l| (l.to_string(), None));
    SHOT_SEEKING.store(label.is_some(), std::sync::atomic::Ordering::Relaxed);
}

/// Where the button looked for with [`shot_seek`] was drawn on the last frame, if it was. Asked once a frame, before
/// the frame is laid out: the answer is cleared, so a button that moves (a list filling in above it) is found again
/// where it is now.
pub fn shot_found() -> Option<egui::Rect> {
    SHOT_SEEK.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|s| s.1.take())
}

fn shot_note(text: &str, rect: egui::Rect) {
    if !SHOT_SEEKING.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let mut seek = SHOT_SEEK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((label, found @ None)) = seek.as_mut()
        && label == text
    {
        *found = Some(rect);
    }
}

/// [`play_button`] at a size of its own: the large Play over the cover of Home's banner.
pub fn play_button_sized(ui: &mut Ui, min: Vec2) -> Response {
    brand_button(ui, "Play", Some(Icon::Play), min, true, true)
}

/// A danger button at least `min` big: the Stop that takes the place of [`play_button_sized`] while the game runs.
pub fn danger_sized(ui: &mut Ui, icon: Icon, text: &str, min: Vec2) -> Response {
    styled_button(
        ui,
        text,
        Some(icon),
        c_danger_bg(),
        c_danger_hover(),
        c_danger(),
        Stroke::new(1.0, c_danger_line()),
        min,
        true,
    )
}

/// [`primary_enabled`] at least `min` big: the Install of a result, which lines up with the ones above and below it,
/// or the button across a panel.
pub fn primary_sized(ui: &mut Ui, text: &str, min: Vec2, enabled: bool) -> Response {
    brand_button(ui, text, None, min, enabled, false)
}

/// How wide one of [`pill_tabs`]'s pills is.
fn pill_tab_width(ui: &Ui, text: &str, dot: bool) -> f32 {
    let w = ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(13.0), Color32::PLACEHOLDER).size().x;
    w + 28.0 + if dot { 14.0 } else { 0.0 }
}

/// How wide a row of [`pill_tabs`] is, gaps included, so a head can tell whether it fits beside its title.
// Explore's head no longer measures its sources (they wrap under the kinds); kept for the heads that may
#[allow(dead_code)]
pub fn pill_tabs_width(ui: &Ui, items: &[(&str, Option<Color32>)]) -> f32 {
    let gaps = items.len().saturating_sub(1) as f32 * PILL_TAB_GAP;
    items.iter().map(|(t, dot)| pill_tab_width(ui, t, dot.is_some())).sum::<f32>() + gaps
}

/// The room between two of [`pill_tabs`]'s pills.
pub const PILL_TAB_GAP: f32 = 6.0;

/// A row of pills that switch between the pages of a place (or the sources of Explore): each a rounded outline with,
/// if it has one, a small dot of its own colour; the open one in the brand's soft tint with a brand outline and ink.
/// Laid out in the row the caller gives (wrap it in `horizontal_wrapped` to let it wrap). Returns the one pressed.
pub fn pill_tabs(ui: &mut Ui, items: &[(&str, Option<Color32>)], selected: usize) -> Option<usize> {
    let mut pressed = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = PILL_TAB_GAP;
        for (i, (text, dot)) in items.iter().enumerate() {
            let (rect, resp) =
                ui.allocate_exact_size(vec2(pill_tab_width(ui, text, dot.is_some()), 32.0), Sense::click());
            let on = i == selected;
            if ui.is_rect_visible(rect) {
                let t = motion::hover(ui, resp.id, resp.hovered());
                let s = motion::toward(ui.ctx(), resp.id.with("on"), on, motion::FAST);
                let ink = if is_dark() { c_accent() } else { c_accent_hover() };
                let bg = motion::mix(motion::mix(c_field(), c_field_hover(), t), c_accent_soft(), s);
                let line = motion::mix(
                    motion::mix(c_border(), c_control().gamma_multiply(0.8), t),
                    ink.gamma_multiply(0.7),
                    s,
                );
                let p = ui.painter();
                p.rect(rect, CornerRadius::same(R_CONTROL), bg, Stroke::new(1.0, line), egui::StrokeKind::Inside);
                let fg = motion::mix(motion::mix(c_muted(), c_text(), t), ink, s);
                let galley = p.layout_no_wrap(text.to_string(), super::fonts::medium(13.0), fg);
                let mut x = rect.left() + 14.0;
                if let Some(c) = dot {
                    p.circle_filled(egui::pos2(x + 3.5, rect.center().y), 3.5, *c);
                    x += 14.0;
                }
                p.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
            }
            focus_ring(ui, &resp, rect, R_CONTROL);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, on, *text));
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                pressed = Some(i);
            }
        }
    });
    pressed
}

/// How something stands, for a [`status_line`]: the same marks and colours for every check, log and clean-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// It worked, or all is well: a check in the brand colour.
    Good,
    /// It can't go on until something is done: an exclamation mark in amber.
    Warn,
    /// It failed: an exclamation mark in red.
    Error,
    /// It is running: a turning arc.
    Busy,
}

impl Status {
    /// The colour of the mark and the words.
    pub fn color(self) -> Color32 {
        match self {
            Status::Good | Status::Busy => {
                if is_dark() {
                    c_accent()
                } else {
                    c_accent_hover()
                }
            }
            Status::Warn => c_warn(),
            Status::Error => c_danger(),
        }
    }
}

/// One line that says how something stands and what to do about it: a small round mark (a check, an exclamation
/// mark or a turning arc) and the words, in the colour of the state. The words wrap.
pub fn status_line(ui: &mut Ui, status: Status, text: &str) -> Response {
    let color = status.color();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 7.0;
        let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        paint_status_mark(ui, rect, status);
        ui.add(egui::Label::new(RichText::new(text).color(color).size(FS_META)).wrap())
    })
    .inner
}

/// The mark of a [`status_line`] in `rect`.
pub fn paint_status_mark(ui: &Ui, rect: egui::Rect, status: Status) {
    let color = status.color();
    let p = ui.painter();
    let c = rect.center();
    let r = rect.width().min(rect.height()) / 2.0;
    match status {
        Status::Busy => {
            if !motion::reduced(ui.ctx()) {
                ui.ctx().request_repaint_after(ANIMATION_FRAME);
            }
            let start = if motion::reduced(ui.ctx()) { 0.0 } else { ui.input(|i| i.time) as f32 * 4.0 };
            let points: Vec<egui::Pos2> = (0..=14)
                .map(|k| {
                    let a = start + k as f32 / 14.0 * 4.4;
                    c + vec2(a.cos(), a.sin()) * (r - 2.0)
                })
                .collect();
            p.add(egui::Shape::line(points, Stroke::new(1.8, color)));
        }
        Status::Good => {
            p.circle_filled(c, r, color.gamma_multiply(0.18));
            let s = Stroke::new(1.6, color);
            p.line_segment([c + vec2(-3.5, 0.3), c + vec2(-1.0, 2.8)], s);
            p.line_segment([c + vec2(-1.0, 2.8), c + vec2(3.8, -2.6)], s);
        }
        Status::Warn | Status::Error => {
            p.circle_filled(c, r, color.gamma_multiply(0.18));
            p.line_segment([c + vec2(0.0, -4.0), c + vec2(0.0, 1.2)], Stroke::new(1.8, color));
            p.circle_filled(c + vec2(0.0, 3.8), 1.1, color);
        }
    }
}

/// `text` cut in the middle to fit `max_w`, so its end stays whole: for a path, the folder it names. `widths` is the
/// width of each of its characters and `dots_w` that of the ellipsis. The part after the last separator is kept; what
/// room is left goes to the start and to what comes just before that part, a character each in turn. A last part
/// that does not fit on its own keeps its end.
pub fn middle_ellipsis(text: &str, widths: &[f32], dots_w: f32, max_w: f32) -> String {
    let chars: Vec<char> = text.chars().collect();
    debug_assert_eq!(chars.len(), widths.len());
    if widths.iter().sum::<f32>() <= max_w {
        return text.to_string();
    }
    let sep = |c: &char| matches!(c, '/' | '\\');
    let mut end = chars.len();
    while end > 0 && sep(&chars[end - 1]) {
        end -= 1;
    }
    let tail_start = chars[..end].iter().rposition(sep).unwrap_or(0);
    let budget = max_w - dots_w;
    let width = |r: std::ops::Range<usize>| widths[r].iter().sum::<f32>();
    let (mut head, mut tail) = (0, tail_start);
    let mut used = width(tail..chars.len());
    if used > budget {
        // even the last part is too long: as much of its end as fits
        while tail < chars.len() && used > budget {
            used -= widths[tail];
            tail += 1;
        }
    } else {
        loop {
            let mut grew = false;
            if head < tail && used + widths[head] <= budget {
                used += widths[head];
                head += 1;
                grew = true;
            }
            if tail > head && used + widths[tail - 1] <= budget {
                used += widths[tail - 1];
                tail -= 1;
                grew = true;
            }
            if !grew {
                break;
            }
        }
    }
    let mut out: String = chars[..head].iter().collect();
    out.push('\u{2026}');
    out.extend(&chars[tail..]);
    out
}

/// A path that can't be edited, in the monospace face, cut in the middle when it is too long so the folder at its end
/// stays readable. The whole path shows under the pointer, and Copy after it copies it.
pub fn path_line(ui: &mut Ui, path: &str) {
    let font = super::fonts::mono(FS_META);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let copy_w = ui.painter().layout_no_wrap("Copy".into(), super::fonts::medium(14.0), c_text()).size().x + 24.0;
        let max_w = (ui.available_width() - copy_w - 4.0).max(40.0);
        let (widths, dots_w) = ui.fonts_mut(|f| {
            (path.chars().map(|c| f.glyph_width(&font, c)).collect::<Vec<_>>(), f.glyph_width(&font, '\u{2026}'))
        });
        let shown = middle_ellipsis(path, &widths, dots_w, max_w);
        ui.add(egui::Label::new(RichText::new(shown).font(font.clone()).color(c_text())).selectable(false))
            .on_hover_text(RichText::new(path).font(font.clone()));
        if ghost(ui, "Copy").on_hover_text("Copy the whole path").clicked() {
            ui.ctx().copy_text(path.to_string());
        }
    });
}

#[cfg(test)]
mod path_tests {
    use super::middle_ellipsis;

    /// Every character one unit wide, the ellipsis too.
    fn cut(text: &str, max: f32) -> String {
        middle_ellipsis(text, &vec![1.0; text.chars().count()], 1.0, max)
    }

    #[test]
    fn a_path_that_fits_is_left_alone() {
        assert_eq!(cut("/home/a/World", 13.0), "/home/a/World");
    }

    #[test]
    fn a_long_path_keeps_its_last_folder_and_fills_the_room() {
        let p = "/home/player/.local/share/saves/My World";
        for max in [12.0, 20.0, 30.0] {
            let out = cut(p, max);
            assert!(out.ends_with("/My World"), "{out}");
            assert!(out.contains('\u{2026}'), "{out}");
            assert_eq!(out.chars().count() as f32, max, "{out}");
        }
        assert_eq!(cut(p, 20.0), "/home\u{2026}saves/My World");
        // a closing separator is not the last folder
        assert!(cut("/home/player/.local/share/World/", 12.0).ends_with("/World/"));
        assert!(cut("C:\\Users\\me\\AppData\\Roaming\\saves\\World", 12.0).ends_with("\\World"));
    }

    #[test]
    fn a_last_folder_too_long_for_the_room_keeps_its_end() {
        assert_eq!(cut("/a/abcdefghij", 6.0), "\u{2026}fghij");
        assert_eq!(cut("abcdefghij", 4.0), "\u{2026}hij");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_head_puts_its_actions_under_the_title_only_when_they_dont_fit() {
        // 120 of title, 16 of gap, 300 of buttons
        assert!(!header_wraps(800.0, 120.0, 300.0));
        assert!(!header_wraps(436.0, 120.0, 300.0));
        assert!(header_wraps(435.0, 120.0, 300.0));
        // no actions: nothing to move
        assert!(!header_wraps(50.0, 120.0, 0.0));
    }

    #[test]
    fn a_readable_column_is_centred_once_there_is_more_room_than_it_needs() {
        assert_eq!(centred_column(800.0, 1000.0), (0.0, 800.0));
        assert_eq!(centred_column(1000.0, 1000.0), (0.0, 1000.0));
        assert_eq!(centred_column(1401.0, 1000.0), (200.0, 1000.0));
        assert_eq!(centred_column(-5.0, 1000.0), (0.0, 0.0));
    }

    #[test]
    fn times_ago_read_naturally() {
        assert_eq!(ago(-5000), "Just now");
        assert_eq!(ago(59), "Just now");
        assert_eq!(ago(60), "1 minute ago");
        assert_eq!(ago(150), "2 minutes ago");
        assert_eq!(ago(3600), "1 hour ago");
        assert_eq!(ago(86400), "1 day ago");
        assert_eq!(ago(86400 * 3), "3 days ago");
        assert_eq!(ago(86400 * 45), "1 month ago");
        assert_eq!(ago(86400 * 400), "1 year ago");
        assert_eq!(time_ago(0), "Never played");
        assert_eq!(time_ago(i64::MIN + 1), "Never played");
        assert!(time_ago(1).ends_with(" years ago"));
    }
}
