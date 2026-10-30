//! The window's own title bar, on Linux. GNOME on Wayland leaves the frame of a window to the app, and the window
//! library's stand-in is a plain grey strip that belongs to no look at all. So BlockForge draws the top of its window
//! itself, in its own colours: the strip along the top moves the window (a double click maximizes it), the three
//! buttons sit in the top right corner, and the edges resize it. "Use the system's title bar" (Settings > Appearance)
//! gives the window its frame back, for a desktop whose own frame is preferred. Windows and macOS keep theirs.

use egui::{
    Color32, Context, CornerRadius, CursorIcon, Id, Pos2, Rect, ResizeDirection, Sense, Stroke, Ui, ViewportCommand,
    pos2, vec2,
};

use super::motion;
use super::theme::*;

/// How tall the strip along the top that moves the window is; the buttons are as tall.
pub const BAR_H: f32 = 36.0;
/// Where a page starts with the strip above it: below the buttons, with a little air.
pub const PAGE_TOP: i8 = 44;

/// How close to an edge the pointer must be to resize the window from it, and from a corner.
const EDGE: f32 = 5.0;
const CORNER: f32 = 14.0;

/// Where the settings keep "Use the system's title bar", among the launcher's extras.
const KEY: &str = "systemTitleBar";

/// Whether the window draws its own title bar: on Linux, unless the system's was asked for.
pub fn own(s: &crate::config::Settings) -> bool {
    cfg!(target_os = "linux") && !s.extras.get(KEY).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Ask for the system's title bar, or go back to the window's own (the default, which is not written).
pub fn set_system(s: &mut crate::config::Settings, system: bool) {
    if system {
        s.extras.insert(KEY.into(), true.into());
    } else {
        s.extras.remove(KEY);
    }
}

fn maximized(ctx: &Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false))
}

/// The strip along the top of the window that moves it. Call before the panels: whatever they put in the strip (the
/// logo, a tab, a button) is on top of it and keeps its own clicks.
pub fn drag_strip(ui: &mut Ui, window: Rect) {
    let strip = Rect::from_min_size(window.min, vec2(window.width(), BAR_H));
    let resp = ui.interact(strip, Id::unique("title-bar-drag"), Sense::click_and_drag());
    if resp.double_clicked() {
        let max = maximized(ui.ctx());
        ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!max));
    } else if resp.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Button {
    Minimize,
    Maximize,
    Close,
}

/// Minimize, maximize (restore when maximized) and close, in the top right corner over everything else, and the
/// outline of the window and its edges that resize it. Call after the page, so they are on top.
pub fn window_controls(ctx: &Context) {
    let window = ctx.content_rect();
    let max = maximized(ctx);
    let size = vec2(40.0, 28.0);
    let gap = 4.0;
    let top = (BAR_H - size.y) / 2.0;
    let right = window.right() - 8.0;
    let buttons = [Button::Minimize, Button::Maximize, Button::Close];
    let width = buttons.len() as f32 * size.x + (buttons.len() - 1) as f32 * gap;
    egui::Area::new(Id::unique("title-bar-buttons"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(right - width, window.top() + top))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for b in buttons {
                    if button(ui, b, max, size).clicked() {
                        match b {
                            Button::Minimize => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                            Button::Maximize => ctx.send_viewport_cmd(ViewportCommand::Maximized(!max)),
                            Button::Close => ctx.send_viewport_cmd(ViewportCommand::Close),
                        }
                    }
                }
            });
        });
    if max {
        return;
    }
    // without a frame the desktop draws no shadow either: a hairline says where the window ends
    ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("window-outline"))).rect_stroke(
        window,
        CornerRadius::ZERO,
        Stroke::new(1.0, c_border()),
        egui::StrokeKind::Inside,
    );
    resize_edges(ctx, window);
}

/// One of the three buttons: a quiet rounded square that lights up under the pointer, Close in red.
fn button(ui: &mut Ui, which: Button, maximized: bool, size: egui::Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hot = motion::hover(ui, resp.id, resp.hovered());
    let (fill, ink) = match which {
        Button::Close => {
            (motion::mix(Color32::TRANSPARENT, c_danger_bg(), hot), motion::mix(c_muted(), c_danger(), hot))
        }
        _ => (motion::mix(Color32::TRANSPARENT, c_hover(), hot), motion::mix(c_muted(), c_text(), hot)),
    };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(R_CONTROL), fill);
    let c = rect.center();
    let stroke = Stroke::new(1.4, ink);
    let s = 5.0;
    match which {
        Button::Minimize => {
            p.line_segment([pos2(c.x - s, c.y + 0.5), pos2(c.x + s, c.y + 0.5)], stroke);
        }
        Button::Maximize if maximized => {
            // restore: two squares, the one behind showing at its top right
            let front = Rect::from_center_size(c + vec2(-1.5, 1.5), vec2(7.5, 7.5));
            p.rect_stroke(front, CornerRadius::same(1), stroke, egui::StrokeKind::Middle);
            p.line_segment(
                [pos2(front.left() + 2.5, front.top() - 2.5), pos2(front.right() + 2.5, front.top() - 2.5)],
                stroke,
            );
            p.line_segment(
                [pos2(front.right() + 2.5, front.top() - 2.5), pos2(front.right() + 2.5, front.bottom() - 2.5)],
                stroke,
            );
        }
        Button::Maximize => {
            p.rect_stroke(
                Rect::from_center_size(c, vec2(9.5, 9.5)),
                CornerRadius::same(1),
                stroke,
                egui::StrokeKind::Middle,
            );
        }
        Button::Close => {
            p.line_segment([pos2(c.x - s, c.y - s), pos2(c.x + s, c.y + s)], stroke);
            p.line_segment([pos2(c.x - s, c.y + s), pos2(c.x + s, c.y - s)], stroke);
        }
    }
    super::widgets::focus_ring(ui, &resp, rect, R_CONTROL);
    let label = match which {
        Button::Minimize => "Minimize",
        Button::Maximize if maximized => "Restore",
        Button::Maximize => "Maximize",
        Button::Close => "Close",
    };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    resp
}

/// The edge or corner of `window` the pointer at `p` would resize from, if it is near one.
fn edge_at(window: Rect, p: Pos2) -> Option<ResizeDirection> {
    if !window.expand(1.0).contains(p) {
        return None;
    }
    let near = |d: f32, reach: f32| d <= reach;
    let (l, r) = (p.x - window.left(), window.right() - p.x);
    let (t, b) = (p.y - window.top(), window.bottom() - p.y);
    let corner = |a: f32, b: f32| near(a, CORNER) && near(b, CORNER) && (near(a, EDGE) || near(b, EDGE));
    Some(if corner(t, l) {
        ResizeDirection::NorthWest
    } else if corner(t, r) {
        ResizeDirection::NorthEast
    } else if corner(b, l) {
        ResizeDirection::SouthWest
    } else if corner(b, r) {
        ResizeDirection::SouthEast
    } else if near(t, EDGE) {
        ResizeDirection::North
    } else if near(b, EDGE) {
        ResizeDirection::South
    } else if near(l, EDGE) {
        ResizeDirection::West
    } else if near(r, EDGE) {
        ResizeDirection::East
    } else {
        return None;
    })
}

/// The pointer near an edge shows the resize arrows, and pressing there hands the resize to the desktop.
fn resize_edges(ctx: &Context, window: Rect) {
    let Some(p) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let Some(dir) = edge_at(window, p) else { return };
    let icon = match dir {
        ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
        ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
    };
    ctx.set_cursor_icon(icon);
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_edges_and_corners_resize_and_the_middle_does_not() {
        let w = Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 700.0));
        assert_eq!(edge_at(w, pos2(600.0, 2.0)), Some(ResizeDirection::North));
        assert_eq!(edge_at(w, pos2(600.0, 698.0)), Some(ResizeDirection::South));
        assert_eq!(edge_at(w, pos2(1.0, 300.0)), Some(ResizeDirection::West));
        assert_eq!(edge_at(w, pos2(1198.0, 300.0)), Some(ResizeDirection::East));
        // a corner reaches a little further along each edge
        assert_eq!(edge_at(w, pos2(10.0, 2.0)), Some(ResizeDirection::NorthWest));
        assert_eq!(edge_at(w, pos2(1197.0, 690.0)), Some(ResizeDirection::SouthEast));
        assert_eq!(edge_at(w, pos2(600.0, 300.0)), None);
        assert_eq!(edge_at(w, pos2(600.0, 20.0)), None, "the strip below the edge moves the window");
        assert_eq!(edge_at(w, pos2(-20.0, 300.0)), None);
    }

    #[test]
    fn the_own_title_bar_is_the_default_on_linux_and_the_system_one_can_be_asked_for() {
        let mut s = crate::config::Settings::default();
        assert_eq!(own(&s), cfg!(target_os = "linux"));
        set_system(&mut s, true);
        assert!(!own(&s));
        set_system(&mut s, false);
        assert_eq!(own(&s), cfg!(target_os = "linux"));
        assert!(!s.extras.contains_key(KEY), "the default isn't written");
    }
}
