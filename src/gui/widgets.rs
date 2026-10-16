//! Small reusable pieces: cards, buttons, chips, avatars, toggles.

use egui::{Align, Color32, CornerRadius, FontId, Layout, Response, RichText, Sense, Stroke, Ui, Vec2, vec2};

use super::icons::{self, Icon};
use super::theme::*;

pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(c_card())
        .stroke(Stroke::new(1.0, c_border()))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

pub fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(26.0).strong());
}

pub fn section(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).size(17.0).strong());
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
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(14.5), fg);
    let icon_w = if icon.is_some() { 24.0 } else { 0.0 };
    let size = vec2((galley.size().x + icon_w + 32.0).max(min.x), min.y.max(36.0));
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let (bg, fg) = if !enabled {
            (c_card(), c_muted())
        } else if resp.hovered() || resp.is_pointer_button_down_on() {
            (hover, fg)
        } else {
            (fill, fg)
        };
        ui.painter().rect(
            rect,
            CornerRadius::same(10),
            bg,
            if enabled { stroke } else { Stroke::new(1.0, c_border()) },
            egui::StrokeKind::Inside,
        );
        let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(14.5), fg);
        let content_w = galley.size().x + icon_w;
        let left = rect.center().x - content_w / 2.0;
        if let Some(icon) = icon {
            icons::paint(ui.painter(), icon, egui::pos2(left + 8.0, rect.center().y), 16.0, fg);
        }
        ui.painter().galley(egui::pos2(left + icon_w, rect.center().y - galley.size().y / 2.0), galley, fg);
        focus_ring(ui, &resp, rect, 10);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, text));
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
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

pub fn primary(ui: &mut Ui, text: &str) -> Response {
    styled_button(ui, text, None, c_accent(), c_accent_hover(), c_on_accent(), Stroke::NONE, Vec2::ZERO, true)
}

pub fn primary_icon(ui: &mut Ui, icon: Icon, text: &str) -> Response {
    styled_button(ui, text, Some(icon), c_accent(), c_accent_hover(), c_on_accent(), Stroke::NONE, Vec2::ZERO, true)
}

pub fn primary_enabled(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    styled_button(ui, text, None, c_accent(), c_accent_hover(), c_on_accent(), Stroke::NONE, Vec2::ZERO, enabled)
}

pub fn primary_enabled_icon(ui: &mut Ui, icon: Icon, text: &str, enabled: bool) -> Response {
    styled_button(ui, text, Some(icon), c_accent(), c_accent_hover(), c_on_accent(), Stroke::NONE, Vec2::ZERO, enabled)
}

pub fn primary_big(ui: &mut Ui, icon: Icon, text: &str) -> Response {
    styled_button(
        ui,
        text,
        Some(icon),
        c_accent(),
        c_accent_hover(),
        c_on_accent(),
        Stroke::NONE,
        vec2(132.0, 44.0),
        true,
    )
}

pub fn secondary(ui: &mut Ui, text: &str) -> Response {
    styled_button(
        ui,
        text,
        None,
        c_card_hover(),
        c_field_hover(),
        c_text(),
        Stroke::new(1.0, c_border()),
        Vec2::ZERO,
        true,
    )
}

pub fn secondary_icon(ui: &mut Ui, icon: Icon, text: &str) -> Response {
    styled_button(
        ui,
        text,
        Some(icon),
        c_card_hover(),
        c_field_hover(),
        c_text(),
        Stroke::new(1.0, c_border()),
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

pub fn danger_icon(ui: &mut Ui, icon: Icon, text: &str) -> Response {
    styled_button(
        ui,
        text,
        Some(icon),
        c_danger_bg(),
        c_danger_hover(),
        c_danger(),
        Stroke::new(1.0, c_danger_line()),
        Vec2::ZERO,
        true,
    )
}

pub fn danger_big(ui: &mut Ui, icon: Icon, text: &str) -> Response {
    styled_button(
        ui,
        text,
        Some(icon),
        c_danger_bg(),
        c_danger_hover(),
        c_danger(),
        Stroke::new(1.0, c_danger_line()),
        vec2(132.0, 44.0),
        true,
    )
}

/// A pill that can be switched on, used for filters and choices.
pub fn chip(ui: &mut Ui, text: &str, selected: bool) -> Response {
    let fg = if selected { c_on_accent() } else { c_text() };
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(13.5), fg);
    let size = vec2(galley.size().x + 26.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let bg = if selected {
        c_accent()
    } else if resp.hovered() {
        c_field_hover()
    } else {
        c_card_hover()
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(15),
        bg,
        if selected { Stroke::NONE } else { Stroke::new(1.0, c_border()) },
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    focus_ring(ui, &resp, rect, 15);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, text));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small non-interactive label pill (loader name, account kind).
pub fn tag(ui: &mut Ui, text: &str, color: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(12.0), color);
    let size = vec2(galley.size().x + 16.0, 22.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(11),
        color.gamma_multiply(0.16),
        Stroke::new(1.0, color.gamma_multiply(0.35)),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
}

pub fn loader_color(name: &str) -> Color32 {
    // lighter tones read well on the dark look, darker ones on the light look
    let dark = super::theme::is_dark();
    let (d, l) = match name {
        "Fabric" => ((0xD9, 0xB5, 0x8A), (0x8A, 0x5A, 0x20)),
        "Quilt" => ((0xB9, 0x8B, 0xE0), (0x7B, 0x4F, 0xB5)),
        "Forge" => ((0x7F, 0xA8, 0xE8), (0x2F, 0x6F, 0xB8)),
        "NeoForge" => ((0xF0, 0x9A, 0x4F), (0xB8, 0x5F, 0x14)),
        _ => return c_accent(),
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
    const COLORS: [(u8, u8, u8); 6] = [
        (0x2E, 0x7D, 0x4F),
        (0x2F, 0x6F, 0x8F),
        (0x8A, 0x5A, 0x2B),
        (0x6B, 0x4F, 0x9A),
        (0x8F, 0x3F, 0x5A),
        (0x4F, 0x7A, 0x2F),
    ];
    let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let (r, g, b) = COLORS[(h % COLORS.len() as u32) as usize];
    let base = Color32::from_rgb(r, g, b);
    let radius = (rect.width() * 0.24).round() as u8;
    ui.painter().rect_filled(rect, CornerRadius::same(radius), base);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(radius),
        Stroke::new(1.0, Color32::from_white_alpha(28)),
        egui::StrokeKind::Inside,
    );
    let letter: String =
        name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into());
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        letter,
        FontId::proportional(rect.width() * 0.46),
        Color32::WHITE,
    );
}

/// An on/off switch.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let size = vec2(42.0, 24.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let bg = if *on { c_accent() } else { c_field_hover() };
    ui.painter().rect_filled(rect, CornerRadius::same(12), bg);
    let cx = rect.left() + 12.0 + t * (size.x - 24.0);
    ui.painter().circle_filled(egui::pos2(cx, rect.center().y), 9.0, if *on { c_on_accent() } else { c_muted() });
    focus_ring(ui, &resp, rect, 12);
    let state = *on;
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, true, state, ""));
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
            ui.label(RichText::new(title).strong());
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

pub fn search_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    ui.add_sized(
        [width, 36.0],
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .margin(egui::Margin::symmetric(12, 8))
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

#[cfg(test)]
mod tests {
    use super::*;

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
