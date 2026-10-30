//! Motion: the eased transitions that make the window feel smooth. Hovers fade, selections slide, pages and
//! dialogs fade in. Everything here honours "reduce motion" (the style's `animation_time` is 0 then): values jump
//! straight to where they are going. A transition asks for the next frame only while it runs, so a still window
//! costs nothing.

use std::time::Duration;

use egui::{Color32, Context, Id, Painter, Pos2, Rect, Ui};

use super::theme::*;

/// A hover or a press: quick.
pub const FAST: f32 = 0.14;
/// A selection that slides or a pane that fades: the usual pace.
pub const MEDIUM: f32 = 0.22;
/// A page coming in.
pub const SLOW: f32 = 0.30;

/// Whether motion is switched off (Settings: reduce motion).
pub fn reduced(ctx: &Context) -> bool {
    ctx.global_style().animation_time <= 0.0
}

/// Fast at first, gentle at the end.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// 0 to 1 as `on` turns true, back to 0 as it turns false, eased, over `seconds`.
pub fn toward(ctx: &Context, id: Id, on: bool, seconds: f32) -> f32 {
    if reduced(ctx) {
        return if on { 1.0 } else { 0.0 };
    }
    ctx.animate_bool_with_time_and_easing(id, on, seconds, ease_out)
}

/// [`toward`] at the pace of a hover.
pub fn hover(ui: &Ui, id: Id, on: bool) -> f32 {
    toward(ui.ctx(), id.with("hover-t"), on, FAST)
}

#[derive(Clone, Copy)]
struct Glide {
    from: f32,
    to: f32,
    start: f64,
    seconds: f32,
}

/// A number that glides to `target` whenever the target changes (a pill sliding to the chosen item, an underline
/// moving to the open tab). The first value is taken as it is.
pub fn glide(ctx: &Context, id: Id, target: f32, seconds: f32) -> f32 {
    let now = ctx.input(|i| i.time);
    let key = id.with("glide");
    let state: Option<Glide> = ctx.data(|d| d.get_temp(key));
    let mut g = match state {
        None => Glide { from: target, to: target, start: now, seconds },
        Some(g) => g,
    };
    if reduced(ctx) {
        g = Glide { from: target, to: target, start: now, seconds };
    } else if (g.to - target).abs() > 0.01 {
        // a new target: start from where the value is right now, so a change in the middle never jumps
        let current = value(&g, now);
        g = Glide { from: current, to: target, start: now, seconds };
    }
    ctx.data_mut(|d| d.insert_temp(key, g));
    let v = value(&g, now);
    if (v - g.to).abs() > 0.01 {
        ctx.request_repaint();
    }
    v
}

fn value(g: &Glide, now: f64) -> f32 {
    let t = if g.seconds <= 0.0 { 1.0 } else { ((now - g.start) as f32 / g.seconds).clamp(0.0, 1.0) };
    g.from + (g.to - g.from) * ease_out(t)
}

/// How far a page has come in since `since` (seconds of the input clock): 0 when it just opened, 1 when it is in.
/// Asks for the next frame while it is still coming.
pub fn arrival(ctx: &Context, since: f64) -> f32 {
    if reduced(ctx) {
        return 1.0;
    }
    let t = ((ctx.input(|i| i.time) - since) as f32 / SLOW).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint_after(Duration::from_millis(8));
    }
    ease_out(t)
}

/// A colour on the way from `a` to `b`.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t.clamp(0.0, 1.0))
}

/// A rounded rectangle filled with a gradient from `left` to `right` (the brand gradient of the main buttons and
/// the progress bars). The edge is drawn smooth by laying the gradient over a plain shape in the first colour.
pub fn gradient_rect(painter: &Painter, rect: Rect, radius: u8, left: Color32, right: Color32) {
    painter.rect_filled(rect, egui::CornerRadius::same(radius), left);
    let inner = rect.shrink(0.5);
    if inner.width() <= 1.0 || inner.height() <= 1.0 {
        return;
    }
    let mut points: Vec<Pos2> = Vec::new();
    let r = (radius as f32 - 0.5).clamp(0.0, inner.height().min(inner.width()) / 2.0);
    egui::epaint::tessellator::path::rounded_rectangle(
        &mut points,
        egui::epaint::RoundedRect::new(inner, egui::epaint::CornerRadiusF32::same(r)),
    );
    if points.len() < 3 {
        return;
    }
    let mut mesh = egui::Mesh::default();
    let color_at = |p: Pos2| mix(left, right, (p.x - inner.left()) / inner.width());
    mesh.colored_vertex(inner.center(), color_at(inner.center()));
    for p in &points {
        mesh.colored_vertex(*p, color_at(*p));
    }
    let n = points.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// The brand gradient across `rect`.
pub fn brand_rect(painter: &Painter, rect: Rect, radius: u8) {
    gradient_rect(painter, rect, radius, c_accent(), c_accent2());
}

/// A soft round glow: `color` in the middle fading to nothing at `radius`.
pub fn glow(painter: &Painter, center: Pos2, radius: f32, color: Color32) {
    let mut mesh = egui::Mesh::default();
    let steps = 48u32;
    mesh.colored_vertex(center, color);
    // two rings: most of the light near the middle, a long soft tail
    // the colour is already premultiplied: scaling all four parts keeps its hue (unpacking it again would grey it)
    let mid = color.gamma_multiply(1.0 / 3.0);
    for ring in 0..2u32 {
        let (rr, c) = if ring == 0 { (radius * 0.45, mid) } else { (radius, Color32::TRANSPARENT) };
        for i in 0..steps {
            let a = i as f32 / steps as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(center + egui::vec2(a.cos(), a.sin()) * rr, c);
        }
    }
    for i in 0..steps {
        let j = (i + 1) % steps;
        mesh.add_triangle(0, 1 + i, 1 + j);
        let (a, b, c, d) = (1 + i, 1 + j, 1 + steps + i, 1 + steps + j);
        mesh.add_triangle(a, c, b);
        mesh.add_triangle(b, c, d);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// The ground of the window: the background colour, a faint pattern of blocks that fades out lower down (when
/// "background artwork" is on), and one wide, faint glow of the primary colour in the upper left.
pub fn backdrop(painter: &Painter, rect: Rect) {
    painter.rect_filled(rect, 0.0, c_bg());
    if pattern_on() {
        let tex = pattern_texture(painter.ctx());
        let tile = PATTERN_TILE;
        let uv = Rect::from_min_max(
            egui::pos2(rect.left() / tile, rect.top() / tile),
            egui::pos2(rect.right() / tile, rect.bottom() / tile),
        );
        painter.image(tex.id(), rect, uv, Color32::WHITE);
        // the pattern is there at the top and gone by two thirds of the way down
        let bg = c_bg();
        let clear = Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), 0);
        let mut mesh = egui::Mesh::default();
        let fade = rect.top() + rect.height() * 0.66;
        mesh.colored_vertex(rect.left_top(), clear);
        mesh.colored_vertex(rect.right_top(), clear);
        mesh.colored_vertex(egui::pos2(rect.right(), fade), bg);
        mesh.colored_vertex(egui::pos2(rect.left(), fade), bg);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(egui::Shape::mesh(mesh));
        painter.rect_filled(Rect::from_min_max(egui::pos2(rect.left(), fade), rect.right_bottom()), 0.0, bg);
    }
    let a = if is_dark() { 22 } else { 26 };
    let c = c_accent();
    let w = rect.width().max(rect.height());
    glow(
        painter,
        rect.left_top() + egui::vec2(rect.width() * 0.3, -w * 0.08),
        w * 0.5,
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a),
    );
}

/// Points one repeat of the block pattern covers (it is 32 by 32 texels, so a block is 4 points across).
const PATTERN_TILE: f32 = 128.0;

static PATTERN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Settings: "Show background artwork".
pub fn set_pattern(on: bool) {
    PATTERN.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn pattern_on() -> bool {
    PATTERN.load(std::sync::atomic::Ordering::Relaxed)
}

/// The block pattern, made once for each look: most cells empty, some a breath lighter (darker in the light look),
/// a few in little clusters like the blocks of a hillside seen from far away.
fn pattern_texture(ctx: &Context) -> egui::TextureHandle {
    let dark = is_dark();
    let key = Id::unique(if dark { "bg-pattern-dark" } else { "bg-pattern-light" });
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        return t;
    }
    let image = pattern_image(dark);
    let t = ctx.load_texture("bg-pattern", image, egui::TextureOptions::NEAREST_REPEAT);
    ctx.data_mut(|d| d.insert_temp(key, t.clone()));
    t
}

fn pattern_image(dark: bool) -> egui::ColorImage {
    const N: usize = 32;
    let cell = |x: usize, y: usize| -> u32 {
        // a small fixed hash, so the pattern is the same on every start
        let mut h = (x as u32).wrapping_mul(0x9E37_79B9) ^ (y as u32).wrapping_mul(0x85EB_CA6B);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2C1B_3C6D);
        h ^ (h >> 12)
    };
    let mut pixels = vec![Color32::TRANSPARENT; N * N];
    for y in 0..N {
        for x in 0..N {
            let h = cell(x / 2, y / 2) % 100;
            let fine = cell(x, y) % 100;
            let level = if h < 9 && fine < 85 {
                3
            } else if fine < 7 {
                2
            } else if fine < 12 {
                1
            } else {
                0
            };
            let a = [0u8, 4, 6, 9][level];
            pixels[y * N + x] =
                if dark { Color32::from_white_alpha(a) } else { Color32::from_black_alpha(a.saturating_sub(1)) };
        }
    }
    egui::ColorImage::new([N, N], pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_pattern_is_faint_and_the_same_every_time() {
        let a = pattern_image(true);
        assert_eq!(a.pixels, pattern_image(true).pixels);
        let lit = a.pixels.iter().filter(|p| p.a() > 0).count();
        assert!(lit > 40 && lit < 32 * 32 / 3, "some blocks, mostly ground: {lit}");
        assert!(a.pixels.iter().all(|p| p.a() <= 9), "never more than a breath");
    }

    #[test]
    fn easing_starts_fast_and_lands_gently() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!(ease_out(0.5) > 0.8);
        assert_eq!(ease_out(2.0), 1.0);
    }

    #[test]
    fn a_glide_moves_toward_its_new_target_without_jumping() {
        // a target that changes halfway starts from where the value was, and lands on the new target
        let g = Glide { from: 10.0, to: 20.0, start: 0.0, seconds: 0.2 };
        assert_eq!(value(&g, 0.0), 10.0);
        let mid = value(&g, 0.1);
        assert!(mid > 10.0 && mid < 20.0, "on its way: {mid}");
        assert_eq!(value(&g, 0.5), 20.0);
        let instant = Glide { seconds: 0.0, ..g };
        assert_eq!(value(&instant, 0.0), 20.0);
    }
}
