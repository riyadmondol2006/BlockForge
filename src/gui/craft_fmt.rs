//! Small helpers of the crafting page: number formats and little widgets.

use super::fonts::TextExt;
use egui::{CornerRadius, FontId, RichText, Sense, Stroke, Ui, vec2};

use super::theme::*;
use super::widgets::{NoticeKind, card, facts, small_muted};
use crate::craft::MAX_QUANTITY;

pub use crate::craft::saved::thousands;

/// `2500` of a 64-stack -> `39 stacks + 4`; empty when the amount is less than a stack or items don't stack.
pub fn stack_note(n: u64, stack: u32) -> String {
    let st = u64::from(stack);
    if st <= 1 || n < st {
        return String::new();
    }
    let (full, rest) = (n / st, n % st);
    let head = if full == 1 { "1 stack".to_string() } else { format!("{} stacks", thousands(full)) };
    if rest == 0 { head } else { format!("{head} + {rest}") }
}

/// Read a quantity typed by the player: a whole number from 1 to a billion, commas and spaces allowed.
pub fn parse_qty(text: &str) -> Result<u64, String> {
    let digits: String = text.chars().filter(|c| !matches!(c, ',' | ' ' | '_' | '\u{a0}')).collect();
    if digits.is_empty() {
        return Err("Type how many you want".into());
    }
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err("Use a whole number, like 64".into());
    }
    match digits.parse::<u64>() {
        Ok(0) => Err("Choose at least 1".into()),
        Ok(n) if n > MAX_QUANTITY => Err(format!("The most is {}", thousands(MAX_QUANTITY))),
        Ok(n) => Ok(n),
        Err(_) => Err(format!("The most is {}", thousands(MAX_QUANTITY))),
    }
}

/// How long `ticks` game ticks (twenty a second) take, as "2 min 5 s".
pub fn ticks_text(ticks: u64) -> String {
    let secs = ticks / 20;
    match secs {
        0 => "under a second".into(),
        1..=59 => format!("{secs} s"),
        60..=3599 => match secs % 60 {
            0 => format!("{} min", secs / 60),
            s => format!("{} min {s} s", secs / 60),
        },
        _ => match (secs % 3600) / 60 {
            0 => format!("{} h", secs / 3600),
            m => format!("{} h {m} min", secs / 3600),
        },
    }
}

/// A short sentence about when something was last changed.
pub fn changed_text(ms: i64) -> String {
    let secs = chrono::Utc::now().timestamp_millis().saturating_sub(ms) / 1000;
    let n = |count: i64, one: &str| if count == 1 { format!("1 {one} ago") } else { format!("{count} {one}s ago") };
    match secs {
        s if s < 60 => "just now".into(),
        s if s < 3600 => n(s / 60, "minute"),
        s if s < 86400 => n(s / 3600, "hour"),
        s => n(s / 86400, "day"),
    }
}

/// Cut `text` to at most `max` characters, with an ellipsis.
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max.saturating_sub(1)).collect();
    s.push('\u{2026}');
    s
}

/// The shared facts strip, as one row when there is room for it and as rows of two when there is not, so four figures
/// never wrap three and one.
pub fn facts_rows(ui: &mut Ui, cells: &[(&str, String)]) {
    let per = if ui.available_width() >= 600.0 { cells.len().max(1) } else { 2 };
    for row in cells.chunks(per) {
        facts(ui, row);
    }
}

/// A card whose rows sit closer together than the page's blocks do.
pub fn panel<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    card(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        add(ui)
    })
}

/// The box of a tick box: outlined while off, filled with ink and a check mark when on.
fn paint_check(ui: &Ui, boxed: egui::Rect, on: bool, enabled: bool, hovered: bool) {
    let radius = CornerRadius::same(R_TAG);
    if on {
        let (fill, mark) = match (enabled, hovered) {
            (false, _) => (c_active(), c_muted()),
            (true, true) => (c_ink_hover(), c_on_ink()),
            (true, false) => (c_ink(), c_on_ink()),
        };
        ui.painter().rect_filled(boxed, radius, fill);
        let c = boxed.center();
        let stroke = Stroke::new(1.8, mark);
        ui.painter().line_segment([c + vec2(-4.0, 0.5), c + vec2(-1.0, 3.5)], stroke);
        ui.painter().line_segment([c + vec2(-1.0, 3.5), c + vec2(4.5, -3.0)], stroke);
    } else {
        let fill = if hovered && enabled { c_hover() } else { c_field() };
        let edge = if enabled { c_control() } else { c_border() };
        ui.painter().rect(boxed, radius, fill, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
    }
}

/// A tick box on its own. The hit area is larger than the box; `label` is what a screen reader says.
pub fn check_box(ui: &mut Ui, on: &mut bool, enabled: bool, label: &str) -> egui::Response {
    let (rect, mut resp) =
        ui.allocate_exact_size(vec2(24.0, 24.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let boxed = egui::Rect::from_center_size(rect.center(), vec2(18.0, 18.0));
    paint_check(ui, boxed, *on, enabled, resp.hovered());
    super::widgets::focus_ring(ui, &resp, boxed, R_TAG);
    let state = *on;
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, state, label));
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// A tick box with its text beside it; pressing the text ticks it too.
#[cfg_attr(not(feature = "converter"), allow(dead_code))]
pub fn check_labeled(ui: &mut Ui, on: &mut bool, label: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(FS_BODY), c_text());
    let (rect, mut resp) = ui.allocate_exact_size(vec2(24.0 + 4.0 + galley.size().x, 24.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let boxed = egui::Rect::from_center_size(rect.left_center() + vec2(12.0, 0.0), vec2(18.0, 18.0));
    paint_check(ui, boxed, *on, true, resp.hovered());
    let at = egui::pos2(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(at, galley, c_text());
    super::widgets::focus_ring(ui, &resp, boxed, R_TAG);
    let state = *on;
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, state, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small cross button; true when clicked.
pub fn x_button(ui: &mut Ui, hint: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), c_hover());
    }
    let color = if resp.hovered() { c_text() } else { c_muted() };
    let c = rect.center();
    let stroke = Stroke::new(1.6, color);
    ui.painter().line_segment([c + vec2(-4.5, -4.5), c + vec2(4.5, 4.5)], stroke);
    ui.painter().line_segment([c + vec2(-4.5, 4.5), c + vec2(4.5, -4.5)], stroke);
    super::widgets::focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, hint));
    resp.on_hover_text(hint).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A short arrow pointing right, drawn with lines.
pub fn arrow(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
    let c = rect.center();
    let stroke = Stroke::new(1.6, c_muted());
    ui.painter().line_segment([c + vec2(-7.0, 0.0), c + vec2(7.0, 0.0)], stroke);
    ui.painter().line_segment([c + vec2(2.0, -5.0), c + vec2(7.0, 0.0)], stroke);
    ui.painter().line_segment([c + vec2(2.0, 5.0), c + vec2(7.0, 0.0)], stroke);
}

/// A card title.
pub fn card_title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(FS_SECTION).semibold());
}

/// Quiet text for a state that is not worth a tag ("gather", "Latest").
pub fn small_faint(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).color(c_faint()).size(FS_META));
}

/// How the plan stands, as plain small text with a mark in front: a check when all is well, an exclamation mark when
/// it needs a look.
pub fn status_text(ui: &mut Ui, good: bool, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        let c = rect.center();
        let stroke = Stroke::new(1.5, c_muted());
        if good {
            ui.painter().line_segment([c + vec2(-4.0, 0.5), c + vec2(-1.5, 3.0)], stroke);
            ui.painter().line_segment([c + vec2(-1.5, 3.0), c + vec2(4.0, -3.0)], stroke);
        } else {
            ui.painter().circle_stroke(c, 6.0, stroke);
            ui.painter().line_segment([c + vec2(0.0, -3.0), c + vec2(0.0, 0.5)], stroke);
            ui.painter().circle_filled(c + vec2(0.0, 3.0), 0.9, c_muted());
        }
        small_muted(ui, text);
    });
}

/// A calm box for notes and warnings that need a title and several lines (the shared `notice` has one sentence).
pub fn note_box<R>(ui: &mut Ui, kind: NoticeKind, add: impl FnOnce(&mut Ui) -> R) -> R {
    let (bg, line) = match kind {
        NoticeKind::Info => (c_field(), c_border()),
        NoticeKind::Warn => (c_warn_bg(), c_warn().gamma_multiply(0.4)),
        NoticeKind::Error => (c_danger_bg(), c_danger_line()),
    };
    egui::Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0, line))
        .corner_radius(CornerRadius::same(R_CONTROL))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_well() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(stack_note(10, 64), "");
        assert_eq!(stack_note(64, 64), "1 stack");
        assert_eq!(stack_note(2500, 64), "39 stacks + 4");
        assert_eq!(stack_note(100, 1), "");
        assert_eq!(ticks_text(20 * 125), "2 min 5 s");
    }

    #[test]
    fn quantities_are_checked() {
        assert_eq!(parse_qty("1,728"), Ok(1728));
        assert_eq!(parse_qty(" 64 "), Ok(64));
        assert!(parse_qty("").is_err());
        assert!(parse_qty("0").is_err());
        assert!(parse_qty("-3").is_err());
        assert!(parse_qty("2.5").is_err());
        assert!(parse_qty("1000000001").is_err());
        assert_eq!(parse_qty("1000000000"), Ok(1_000_000_000));
        assert!(parse_qty("99999999999999999999999").is_err());
    }
}
