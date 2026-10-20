//! Checks made when the launcher opens, with the Java edition's warnings and wording.

use egui::RichText;

use super::App;
use super::widgets::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Warning {
    JavaOptions,
    OneDrive,
    ProgramFiles,
    NotWritable,
}

impl Warning {
    fn title(self) -> &'static str {
        match self {
            Warning::JavaOptions => "Warning",
            Warning::OneDrive => "BlockForge installed within OneDrive",
            Warning::ProgramFiles => "BlockForge installed within Program Files",
            Warning::NotWritable => "BlockForge cannot write files",
        }
    }

    fn text(self, folder: &str) -> String {
        match self {
            Warning::JavaOptions => "We've detected that you have a _JAVA_OPTIONS environment variable which may cause issues installing and playing Minecraft.\n\nDo you want to fix this now so you don't have any issues in the future?".into(),
            Warning::OneDrive => "We have detected that you're running BlockForge from within OneDrive.\n\nThis can cause serious issues and you should move the folder outside of OneDrive.\n\nDo you want to close the launcher and do this now?".into(),
            Warning::ProgramFiles => "We have detected that you're running BlockForge from within Program Files.\n\nThis can cause serious issues and you should move the folder outside of Program Files.\n\nDo you want to close the launcher and do this now?".into(),
            Warning::NotWritable => format!("We have detected that BlockForge cannot write files in its current location ({folder}).\n\nWe cannot continue to run, you must move this folder somewhere else with write access.\n\nTry moving to a folder in your Desktop or another drive.\n\nYou can also try running BlockForge as administrator, but this is not recommended."),
        }
    }
}

/// The warnings that apply right now, in the order Java shows them.
pub fn collect(app: &App) -> Vec<Warning> {
    let mut out = Vec::new();
    if !app.settings.ignore_java_options_warning {
        if let Ok(o) = std::env::var("_JAVA_OPTIONS") {
            let o = o.to_lowercase();
            if o.contains("-xmx") || o.contains("-xms") || o.contains("-xss") {
                out.push(Warning::JavaOptions);
            }
        }
    }
    let base = app.dirs.base.to_string_lossy().to_string();
    if !app.settings.ignore_one_drive_warning && base.contains("OneDrive") {
        out.push(Warning::OneDrive);
    }
    if cfg!(target_os = "windows") && !app.settings.ignore_program_files_warning && base.contains("Program Files") {
        out.push(Warning::ProgramFiles);
    }
    let test = app.dirs.base.join(".test");
    if std::fs::write(&test, b"").is_err() {
        out.push(Warning::NotWritable);
    } else {
        // only a test: it isn't left lying in the data folder
        let _ = std::fs::remove_file(&test);
    }
    out
}

impl App {
    pub(super) fn startup_ui(&mut self, ctx: &egui::Context) {
        let Some(&w) = self.startup_warnings.first() else { return };
        let folder = self.dirs.base.to_string_lossy().to_string();
        let (mut yes, mut no, mut never) = (false, false, false);
        egui::Modal::new(egui::Id::unique("startup")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(RichText::new(w.title()).size(19.0).semibold());
            ui.add_space(6.0);
            ui.label(w.text(&folder));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if w == Warning::NotWritable {
                    if primary(ui, "Close BlockForge").clicked() {
                        yes = true;
                    }
                } else {
                    if primary(ui, "Yes").clicked() {
                        yes = true;
                    }
                    if secondary(ui, "No").clicked() {
                        no = true;
                    }
                    if secondary(ui, "Don't remind me again").clicked() {
                        never = true;
                    }
                }
            });
        });
        if yes {
            match w {
                Warning::JavaOptions => {
                    let _ = open::that("https://atl.pw/javaoptionsfromlauncher");
                }
                Warning::OneDrive | Warning::ProgramFiles => {
                    let _ = open::that(&self.dirs.base);
                }
                Warning::NotWritable => {}
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if never {
            match w {
                Warning::JavaOptions => self.settings.ignore_java_options_warning = true,
                Warning::OneDrive => self.settings.ignore_one_drive_warning = true,
                Warning::ProgramFiles => self.settings.ignore_program_files_warning = true,
                Warning::NotWritable => {}
            }
            self.save_settings();
        }
        if no || never {
            self.startup_warnings.remove(0);
        }
    }
}
