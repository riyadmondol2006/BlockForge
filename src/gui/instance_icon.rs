//! The picture of an instance: its own image when it has one (a pack's logo, or one the player chose), otherwise a
//! cube in the colour of its mod loader, so every instance is recognisable at a glance.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use egui::{Color32, CornerRadius, Pos2, Shape, Stroke, TextureHandle, Ui, vec2};

use super::App;
use super::theme::*;
use super::widgets::loader_color;
use crate::instances::Instance;

/// The pictures read from instance folders so far, so the files are not read again for every frame.
#[derive(Default)]
pub struct LocalPictures(RefCell<HashMap<PathBuf, Entry>>);

impl LocalPictures {
    /// Read this picture again the next time it is shown (it was just replaced).
    pub fn forget(&self, path: &std::path::Path) {
        self.0.borrow_mut().remove(path);
    }
}

struct Entry {
    looked: Instant,
    modified: Option<SystemTime>,
    /// `None` when the file isn't there or isn't a picture that can be read.
    texture: Option<TextureHandle>,
}

/// How long a result is trusted before the file is looked at again (a new image chosen, or a pack logo that has
/// just finished downloading, shows up within this).
const LOOK_AGAIN: std::time::Duration = std::time::Duration::from_secs(2);

/// A picture file read into a square texture image (shrunk to at most 256 points, centred on a transparent square
/// so a wide logo isn't stretched). The file may be a PNG, JPEG, WebP or GIF whatever its name says.
pub fn read_square(path: &std::path::Path) -> Option<egui::ColorImage> {
    let bytes = std::fs::read(path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    let img = if img.width().max(img.height()) > 256 { img.thumbnail(256, 256) } else { img }.to_rgba8();
    let side = img.width().max(img.height());
    if side == 0 {
        return None;
    }
    let mut square = image::RgbaImage::new(side, side);
    image::imageops::overlay(
        &mut square,
        &img,
        i64::from((side - img.width()) / 2),
        i64::from((side - img.height()) / 2),
    );
    Some(egui::ColorImage::from_rgba_unmultiplied([side as usize, side as usize], square.as_raw()))
}

impl App {
    fn local_picture(&self, path: PathBuf) -> Option<TextureHandle> {
        let mut all = self.local_pictures.0.borrow_mut();
        if let Some(e) = all.get(&path) {
            if e.looked.elapsed() < LOOK_AGAIN {
                return e.texture.clone();
            }
        }
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if let Some(e) = all.get_mut(&path) {
            if e.modified == modified {
                e.looked = Instant::now();
                return e.texture.clone();
            }
        }
        let texture = modified.and_then(|_| read_square(&path)).map(|img| {
            self.ctx.load_texture(format!("instance-{}", path.display()), img, egui::TextureOptions::LINEAR)
        });
        all.insert(path, Entry { looked: Instant::now(), modified, texture: texture.clone() });
        texture
    }

    /// The instance's picture: its own `instance.png` if it has one that can be read, otherwise the cube.
    pub(super) fn instance_icon(&self, ui: &mut Ui, inst: &Instance, size: f32) {
        match self.local_picture(inst.root.join("instance.png")) {
            Some(tex) => {
                ui.add(
                    egui::Image::from_texture(&tex)
                        .fit_to_exact_size(vec2(size, size))
                        .corner_radius(CornerRadius::same((size * 0.22) as u8)),
                );
            }
            None => {
                let (rect, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
                paint_default_icon(ui, rect, inst.loader().map(|(kind, _)| kind));
            }
        }
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

fn shade(c: Color32, factor: f32) -> Color32 {
    let f = |x: u8| (f32::from(x) * factor).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

/// A steady pseudo-random number between -1 and 1 for one cell of the cube, so the speckles don't flicker.
fn speckle(face: u32, i: u32, j: u32) -> f32 {
    let mut h = face.wrapping_mul(0x9E37_79B1) ^ i.wrapping_mul(0x85EB_CA6B) ^ j.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h % 2001) as f32 / 1000.0 - 1.0
}

/// The default picture: a block with a grassy top and earthy sides on a tile. The top and the tile take the colour
/// of the mod loader (green for plain Minecraft), so a Fabric, Forge, NeoForge or Quilt instance is easy to tell apart.
pub fn paint_default_icon(ui: &Ui, rect: egui::Rect, loader: Option<&str>) {
    let painter = ui.painter();
    let tint = match loader {
        None | Some("") => Color32::from_rgb(0x5F, 0xB0, 0x44),
        Some(kind) => loader_color(kind),
    };
    let dark = is_dark();
    let corner = CornerRadius::same((rect.width() * 0.22).round() as u8);
    let tile = mix(
        if dark { Color32::from_rgb(0x1B, 0x22, 0x1E) } else { Color32::from_rgb(0xEE, 0xF1, 0xEC) },
        tint,
        if dark { 0.22 } else { 0.18 },
    );
    painter.rect_filled(rect, corner, tile);
    painter.rect_stroke(rect, corner, Stroke::new(1.0, mix(tile, tint, 0.45)), egui::StrokeKind::Inside);

    let dirt = Color32::from_rgb(0x8B, 0x5E, 0x34);
    let r = rect.width() * 0.34;
    let c = rect.center() + vec2(0.0, rect.height() * 0.02);
    let w = r * 0.866;
    let (top, tr, mid, tl) = (c + vec2(0.0, -r), c + vec2(w, -r / 2.0), c, c + vec2(-w, -r / 2.0));
    let (bl, br, bottom) = (c + vec2(-w, r / 2.0), c + vec2(w, r / 2.0), c + vec2(0.0, r));
    const N: u32 = 4;
    // a face is cut into N x N cells of slightly different colour, like pixels of a texture
    let face = |painter: &egui::Painter,
                id: u32,
                origin: Pos2,
                u: egui::Vec2,
                v: egui::Vec2,
                base: Color32,
                fringe: Option<Color32>| {
        for i in 0..N {
            for j in 0..N {
                let (a, b) = (i as f32 / N as f32, j as f32 / N as f32);
                let (a2, b2) = ((i + 1) as f32 / N as f32, (j + 1) as f32 / N as f32);
                let p = |x: f32, y: f32| origin + u * x + v * y;
                let mut color = base;
                if let (Some(f), 0) = (fringe, j) {
                    color = f;
                } else if let (Some(f), 1) = (fringe, j) {
                    // the grass hangs down unevenly
                    if speckle(id, i, 9) > 0.1 {
                        color = f;
                    }
                }
                let color = shade(color, 1.0 + 0.07 * speckle(id, i, j));
                painter.add(Shape::convex_polygon(
                    vec![p(a, b), p(a2, b), p(a2, b2), p(a, b2)],
                    color,
                    Stroke::new(0.4, color),
                ));
            }
        }
    };
    let grass = tint;
    face(painter, 0, tl, top - tl, mid - tl, grass, None);
    face(painter, 1, tl, mid - tl, bl - tl, shade(dirt, 0.92), Some(shade(grass, 0.9)));
    face(painter, 2, mid, tr - mid, bottom - mid, shade(dirt, 0.72), Some(shade(grass, 0.72)));
    let edge = Stroke::new((rect.width() / 46.0).max(0.6), Color32::from_black_alpha(70));
    painter.add(Shape::closed_line(vec![top, tr, br, bottom, bl, tl], edge));
    painter.add(Shape::line_segment([mid, tl], edge));
    painter.add(Shape::line_segment([mid, tr], edge));
    painter.add(Shape::line_segment([mid, bottom], edge));
}
