//! Icons drawn with lines and shapes, so they look the same on every system and need no font.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Home,
    Library,
    Search,
    Plus,
    User,
    Sliders,
    Play,
    Stop,
    Back,
    Folder,
    Info,
}

/// Draw `icon` centred on `c`, fitting a square of side `size`.
pub fn paint(p: &Painter, icon: Icon, c: Pos2, size: f32, color: Color32) {
    let s = size / 2.0;
    let w = (size / 11.0).max(1.4);
    let stroke = Stroke::new(w, color);
    let at = |x: f32, y: f32| pos2(c.x + x * s, c.y + y * s);
    let line = |pts: &[(f32, f32)]| {
        p.add(Shape::line(pts.iter().map(|&(x, y)| at(x, y)).collect(), stroke));
    };
    match icon {
        Icon::Home => {
            line(&[(-1.0, -0.05), (0.0, -0.9), (1.0, -0.05)]);
            line(&[(-0.68, -0.3), (-0.68, 0.85), (0.68, 0.85), (0.68, -0.3)]);
            line(&[(-0.2, 0.85), (-0.2, 0.3), (0.2, 0.3), (0.2, 0.85)]);
        }
        Icon::Library => {
            for (x, y) in [(-0.9, -0.9), (0.08, -0.9), (-0.9, 0.08), (0.08, 0.08)] {
                p.rect_stroke(
                    Rect::from_min_size(at(x, y), vec2(0.82 * s, 0.82 * s)),
                    2.0,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
        }
        Icon::Search => {
            p.circle_stroke(at(-0.15, -0.15), 0.62 * s, stroke);
            line(&[(0.3, 0.3), (0.92, 0.92)]);
        }
        Icon::Plus => {
            line(&[(-0.85, 0.0), (0.85, 0.0)]);
            line(&[(0.0, -0.85), (0.0, 0.85)]);
        }
        Icon::User => {
            p.circle_stroke(at(0.0, -0.4), 0.42 * s, stroke);
            let arc: Vec<(f32, f32)> = (0..=12)
                .map(|i| {
                    let t = std::f32::consts::PI + std::f32::consts::PI * i as f32 / 12.0;
                    (0.85 * t.cos(), 0.95 + 0.8 * t.sin())
                })
                .collect();
            line(&arc);
        }
        Icon::Sliders => {
            for (y, x) in [(-0.6, 0.35), (0.0, -0.35), (0.6, 0.45)] {
                line(&[(-0.9, y), (0.9, y)]);
                p.circle_filled(at(x, y), 0.26 * s, color);
            }
        }
        Icon::Play => {
            p.add(Shape::convex_polygon(vec![at(-0.5, -0.78), at(0.82, 0.0), at(-0.5, 0.78)], color, Stroke::NONE));
        }
        Icon::Stop => {
            p.rect_filled(Rect::from_center_size(c, vec2(1.35 * s, 1.35 * s)), 2.0, color);
        }
        Icon::Back => {
            line(&[(0.85, 0.0), (-0.85, 0.0)]);
            line(&[(-0.2, -0.65), (-0.85, 0.0), (-0.2, 0.65)]);
        }
        Icon::Info => {
            p.circle_stroke(c, 0.95 * s, stroke);
            line(&[(0.0, -0.05), (0.0, 0.5)]);
            p.circle_filled(at(0.0, -0.45), 0.09 * s + 0.4, color);
        }
        Icon::Folder => {
            line(&[
                (-0.95, -0.55),
                (-0.95, 0.7),
                (0.95, 0.7),
                (0.95, -0.35),
                (0.1, -0.35),
                (-0.15, -0.6),
                (-0.95, -0.6),
            ]);
        }
    }
}
