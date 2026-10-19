//! Small helpers of the crafting page: number formats and little widgets.

use egui::{Color32, CornerRadius, FontId, RichText, Sense, Stroke, Ui, vec2};

use super::theme::*;
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

/// A number with a caption under it, in a soft tile.
pub fn stat_tile(ui: &mut Ui, width: f32, value: &str, label: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 62.0), Sense::hover());
    let p = ui.painter();
    p.rect(rect, CornerRadius::same(10), c_field(), Stroke::NONE, egui::StrokeKind::Inside);
    p.text(rect.left_top() + vec2(12.0, 11.0), egui::Align2::LEFT_TOP, value, FontId::proportional(20.0), c_text());
    p.text(rect.left_top() + vec2(12.0, 38.0), egui::Align2::LEFT_TOP, label, FontId::proportional(12.0), c_muted());
}

/// A small round cross button; true when clicked.
pub fn x_button(ui: &mut Ui, hint: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(8), c_field_hover());
    }
    let color = if resp.hovered() { c_text() } else { c_muted() };
    let c = rect.center();
    let stroke = Stroke::new(1.6, color);
    ui.painter().line_segment([c + vec2(-4.5, -4.5), c + vec2(4.5, 4.5)], stroke);
    ui.painter().line_segment([c + vec2(-4.5, 4.5), c + vec2(4.5, -4.5)], stroke);
    super::widgets::focus_ring(ui, &resp, rect, 8);
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
    ui.label(RichText::new(text).size(16.0).strong());
}

/// A calm box with a coloured edge, for notes and warnings.
pub fn note_box<R>(ui: &mut Ui, color: Color32, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.4)))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::same(14))
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
