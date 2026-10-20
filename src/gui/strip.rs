//! The play strip: one cell per day, in the instance's own colour, filled on the days it was played.

use egui::Ui;

use super::App;
use crate::instances::Instance;

impl App {
    /// The "Play history" block of an instance's overview.
    pub(super) fn play_history(&mut self, _ui: &mut Ui, _inst: &Instance) {}
}
