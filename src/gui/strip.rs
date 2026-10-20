//! The play strip: one cell per day, in the instance's own colour, filled on the days it was played. It is real data
//! (the sessions the launcher recorded and the dated logs Minecraft keeps), drawn as blocks.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use egui::{Color32, CornerRadius, Response, Sense, Stroke, Ui, vec2};

use super::App;
use super::fonts::TextExt;
use super::theme::*;
use super::widgets::{loader_color, small_muted};
use crate::instances::Instance;

/// The strips already worked out, so the instance folders are not read again for every frame.
#[derive(Default)]
pub struct PlayCache(RefCell<HashMap<(String, usize), (Instant, Vec<u8>)>>);

impl PlayCache {
    /// Work them out again the next time (a game was just started or has just ended).
    pub fn forget(&self) {
        self.0.borrow_mut().clear();
    }
}

/// How long a strip is kept before the instance's folder is looked at again.
const KEEP: Duration = Duration::from_secs(30);

impl App {
    /// The sessions on each of the last `days` days of an instance, oldest first.
    pub(super) fn play_days(&self, inst: &Instance, days: usize) -> Vec<u8> {
        let key = (inst.folder.clone(), days);
        if let Some((at, v)) = self.play_cache.0.borrow().get(&key) {
            if at.elapsed() < KEEP {
                return v.clone();
            }
        }
        let v = inst.play_days(chrono::Local::now().date_naive(), days);
        self.play_cache.0.borrow_mut().insert(key, (Instant::now(), v.clone()));
        v
    }

    /// The colour of an instance's strip: its loader's hue, or the muted text colour for plain Minecraft.
    pub(super) fn strip_hue(inst: &Instance) -> Color32 {
        inst.loader().map(|(kind, _)| loader_color(kind)).unwrap_or_else(c_muted)
    }

    /// The "Play history" block of an instance's overview.
    pub(super) fn play_history(&mut self, ui: &mut Ui, inst: &Instance) {
        let n = if ui.available_width() >= 560.0 { 56 } else { 28 };
        let days = self.play_days(inst, n);
        let played = days.iter().filter(|d| **d > 0).count();
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Play history").size(FS_SECTION).semibold());
        ui.add_space(2.0);
        play_strip(ui, &days, Self::strip_hue(inst), 8.0, 2.0);
        ui.add_space(2.0);
        small_muted(ui, caption(played, n));
    }
}

/// "Played on 9 of the last 56 days", or "Not played yet" when it has not been.
pub fn caption(played: usize, days: usize) -> String {
    if played == 0 { "Not played yet".to_string() } else { format!("Played on {played} of the last {days} days") }
}

/// The strip itself: `days` is oldest first and ends with today; a day with sessions is filled in `hue`, an empty one
/// is the quiet cell colour, and today has a thin outline. Hovering a cell says which day it is.
pub fn play_strip(ui: &mut Ui, days: &[u8], hue: Color32, cell: f32, gap: f32) -> Response {
    let n = days.len().max(1) as f32;
    let size = vec2(n * cell + (n - 1.0) * gap, cell);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(if cell >= 8.0 { 2 } else { 1 });
        for (i, d) in days.iter().enumerate() {
            let x = rect.left() + i as f32 * (cell + gap);
            let r = egui::Rect::from_min_size(egui::pos2(x, rect.top()), vec2(cell, cell));
            ui.painter().rect_filled(r, radius, if *d > 0 { hue } else { c_active() });
            if i + 1 == days.len() {
                ui.painter().rect_stroke(r.expand(1.0), radius, Stroke::new(1.0, c_text()), egui::StrokeKind::Outside);
            }
        }
    }
    let played = days.iter().filter(|d| **d > 0).count();
    let label = caption(played, days.len());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &label));
    // one hover area for the whole strip: the cell under the pointer says its day
    if let Some(pos) = resp.hover_pos() {
        let i = (((pos.x - rect.left()) / (cell + gap)).floor().max(0.0) as usize).min(days.len().saturating_sub(1));
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
