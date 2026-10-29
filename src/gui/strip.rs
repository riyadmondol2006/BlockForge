//! The play strip: one cell per day, in the instance's own colour, filled on the days it was played. It is real data
//! (the sessions the launcher recorded and the dated logs Minecraft keeps), drawn as blocks.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use egui::{Color32, CornerRadius, Response, Sense, Stroke, Ui, vec2};

use super::App;
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
        let n = history_days(ui.available_width());
        let days = self.play_days(inst, n);
        let played = days.iter().filter(|d| **d > 0).count();
        super::instance_menu::block_title(ui, "Play history");
        ui.add_space(2.0);
        play_strip(ui, &days, Self::strip_hue(inst), HISTORY_CELL, HISTORY_GAP);
        ui.add_space(2.0);
        small_muted(ui, caption(played, n));
    }
}

/// The cells of the history strip on an instance's overview, and the space between them.
const HISTORY_CELL: f32 = 8.0;
const HISTORY_GAP: f32 = 2.0;

/// How many days the history strip shows in `width`: whole weeks, as many as fit, from four weeks to twelve.
fn history_days(width: f32) -> usize {
    let fit = ((width + HISTORY_GAP) / (HISTORY_CELL + HISTORY_GAP)).floor().max(0.0) as usize;
    (fit / 7 * 7).clamp(28, 84)
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
        let radius = CornerRadius::same(if cell >= 10.0 {
            3
        } else if cell >= 8.0 {
            2
        } else {
            1
        });
        // the cell under the pointer is lifted out with a ring, so it is clear which day the tip is about
        let pointed = resp.hover_pos().map(|pos| ((pos.x - rect.left()) / (cell + gap)).floor() as i64);
        for (i, d) in days.iter().enumerate() {
            let x = rect.left() + i as f32 * (cell + gap);
            let r = egui::Rect::from_min_size(egui::pos2(x, rect.top()), vec2(cell, cell));
            if *d > 0 {
                // a played day is a little brighter at the top, like the face of a block catching the light
                ui.painter().rect_filled(r, radius, hue);
                let top = egui::Rect::from_min_size(r.min, vec2(cell, (cell * 0.4).round()));
                ui.painter().rect_filled(
                    top,
                    CornerRadius { nw: radius.nw, ne: radius.ne, sw: 0, se: 0 },
                    Color32::from_white_alpha(if is_dark() { 34 } else { 46 }),
                );
            } else {
                ui.painter().rect_filled(r, radius, c_active());
            }
            if i + 1 == days.len() {
                ui.painter().rect_stroke(r.expand(1.0), radius, Stroke::new(1.0, c_text()), egui::StrokeKind::Outside);
            } else if pointed == Some(i as i64) {
                ui.painter().rect_stroke(r.expand(1.0), radius, Stroke::new(1.0, c_muted()), egui::StrokeKind::Outside);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The strip fills the width it has in whole weeks, never fewer than four nor more than twelve.
    #[test]
    fn the_history_strip_is_whole_weeks_that_fit() {
        assert_eq!(history_days(560.0), 56);
        assert_eq!(history_days(500.0), 49);
        assert_eq!(history_days(100.0), 28);
        assert_eq!(history_days(2000.0), 84);
        for w in [280.0, 430.0, 600.0, 870.0] {
            let n = history_days(w);
            assert_eq!(n % 7, 0);
            if n > 28 {
                assert!(n as f32 * (HISTORY_CELL + HISTORY_GAP) - HISTORY_GAP <= w, "{n} days in {w}");
            }
        }
    }
}
