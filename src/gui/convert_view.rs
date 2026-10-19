//! What the Convert page looks like: the set-up, the progress, the result and the error screens.

use egui::{Align, Color32, CornerRadius, Layout, RichText, ScrollArea, Sense, Stroke, Ui, vec2};

use super::convert::{
    Act, CHUNKER_URL, ConvertState, Look, MAX_UNMAPPED, OutputCheck, Pick, Screen, dimension_label, format_elapsed,
    free_name, number,
};
use super::icons::Icon;
use super::theme::*;
use super::widgets::*;
use crate::convert::backend::Edition;
use crate::tr;

/// How wide the page's content gets on a wide window.
const PAGE_WIDTH: f32 = 720.0;
/// The footer's height, kept free below the scrolling part.
const FOOTER: f32 = 30.0;
/// The height of the bar with the Convert button.
const START_BAR: f32 = 50.0;

/// The small capitals over a card.
fn label_over(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).strong().color(c_accent()));
    ui.add_space(2.0);
}

impl ConvertState {
    /// Draws the page; returns what it asks the window to do.
    pub(super) fn show(&mut self, ui: &mut Ui) -> Vec<Act> {
        let mut acts = Vec::new();
        ui.horizontal(|ui| {
            heading(ui, &if self.dest.is_some() { tr!("Add a Bedrock world") } else { tr!("Convert a Bedrock world") });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.screen != Screen::Running {
                    match &self.dest {
                        Some(d) => {
                            if ui.link(tr!("Back to {0}", &d.name)).clicked() {
                                acts.push(Act::BackToInstance(d.folder.clone()));
                            }
                        }
                        None => {
                            if ui.link(tr!("Back to Tools")).clicked() {
                                acts.push(Act::BackToTools);
                            }
                        }
                    }
                }
            });
        });
        match &self.dest {
            Some(dest) => muted(ui, tr!("Bring a Bedrock world to Java Edition and add it to {0}", &dest.name)),
            None => muted(ui, tr!("Turn a Bedrock Edition world into a Java Edition world")),
        }
        ui.add_space(10.0);
        // the Convert button stays in view under the scrolling cards
        let bar = self.screen == Screen::Setup && self.found().is_some();
        let height = (ui.available_height() - FOOTER - if bar { START_BAR } else { 0.0 }).max(120.0);
        ScrollArea::vertical().id_salt("convert-page").max_height(height).auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(ui.available_width().min(PAGE_WIDTH));
            match self.screen {
                Screen::Setup => self.setup(ui, &mut acts),
                Screen::Running => self.running(ui),
                Screen::Done => self.done(ui, &mut acts),
                Screen::Failed => self.failed(ui, &mut acts),
            }
            ui.add_space(8.0);
        });
        if bar {
            self.start_bar(ui, &mut acts);
        }
        self.footer(ui, &mut acts);
        if self.screen == Screen::Setup {
            drop_hint(ui);
        }
        acts
    }

    /// "Powered by Chunker by Hive Games (MIT)", the credit the converter owes its origin.
    fn footer(&self, ui: &mut Ui, acts: &mut Vec<Act>) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            small_muted(ui, tr!("Powered by"));
            if ui.link(RichText::new(tr!("Chunker by Hive Games")).size(12.0)).on_hover_text(CHUNKER_URL).clicked() {
                acts.push(Act::Link(CHUNKER_URL));
            }
            small_muted(ui, "(MIT)");
        });
    }

    // -----------------------------------------------------------------------------------------------------
    // Set up

    fn setup(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        self.world_card(ui, acts);
        if self.found().is_none() {
            return;
        }
        ui.add_space(8.0);
        self.target_card(ui);
        ui.add_space(8.0);
        self.output_card(ui, acts);
    }

    /// The Convert button, and why it cannot be pressed yet when that is so.
    fn start_bar(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let blocked = self.blocked();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if primary_enabled_icon(ui, Icon::Play, &tr!("Convert"), blocked.is_none()).clicked() {
                acts.push(Act::Start);
            }
            if let Some(why) = &blocked {
                ui.add(egui::Label::new(RichText::new(why).color(c_muted()).size(12.5)).wrap());
            }
        });
    }

    fn world_card(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        card(ui, |ui| {
            label_over(ui, &tr!("BEDROCK WORLD"));
            match &self.look {
                Look::Nothing => {
                    drop_zone(ui);
                    ui.add_space(8.0);
                }
                Look::Busy => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr!("Looking at the world..."));
                    });
                    self.source_path(ui);
                    ui.add_space(4.0);
                }
                Look::Unknown(why) => {
                    self.source_path(ui);
                    ui.add_space(2.0);
                    ui.add(egui::Label::new(RichText::new(why).color(c_danger())).wrap());
                    ui.add_space(6.0);
                }
                Look::Found(d) => {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_max_width((ui.available_width() - 120.0).max(160.0));
                            ui.add(egui::Label::new(RichText::new(&d.name).size(17.0).strong()).truncate());
                            ui.horizontal(|ui| {
                                tag(ui, d.edition.label(), edition_color(d.edition));
                                tag(ui, &d.version, c_muted());
                                small_muted(ui, human_size(d.size_bytes));
                            });
                            ui.add_space(2.0);
                            self.source_path(ui);
                        });
                        ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                            let change = secondary(ui, &tr!("Change..."));
                            egui::Popup::menu(&change).show(|ui| {
                                ui.set_min_width(150.0);
                                if ui.button(tr!("A world folder...")).clicked() {
                                    acts.push(Act::Pick(Pick::SourceFolder));
                                    ui.close();
                                }
                                if ui.button(tr!("A packed world...")).clicked() {
                                    acts.push(Act::Pick(Pick::SourceFile));
                                    ui.close();
                                }
                            });
                        });
                    });
                    return;
                }
            }
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!self.picking, |ui| {
                    let (folder, file) = match self.look {
                        Look::Nothing => (tr!("Choose a folder..."), tr!("Choose a file...")),
                        _ => (tr!("Another folder..."), tr!("Another file...")),
                    };
                    if secondary_icon(ui, Icon::Folder, &folder).clicked() {
                        acts.push(Act::Pick(Pick::SourceFolder));
                    }
                    if secondary(ui, &file).clicked() {
                        acts.push(Act::Pick(Pick::SourceFile));
                    }
                });
            });
        });
    }

    fn source_path(&self, ui: &mut Ui) {
        if let Some(src) = &self.source {
            ui.add(egui::Label::new(RichText::new(src.display().to_string()).color(c_muted()).size(12.5)).truncate());
        }
    }

    fn target_card(&mut self, ui: &mut Ui) {
        card(ui, |ui| {
            label_over(ui, &tr!("CONVERT TO"));
            egui::Grid::new("convert-target").num_columns(2).spacing([20.0, 10.0]).show(ui, |ui| {
                muted(ui, tr!("Game"));
                // the target is always Java Edition: there is nothing to choose
                ui.label(RichText::new(Edition::Java.label()).strong());
                ui.end_row();

                muted(ui, tr!("Version"));
                ui.horizontal(|ui| {
                    if self.versions.is_empty() {
                        small_muted(ui, tr!("No versions are available yet."));
                    } else {
                        egui::ComboBox::from_id_salt("convert-version")
                            .selected_text(&self.version)
                            .width(180.0)
                            .show_ui(ui, |ui| {
                                for v in &self.versions {
                                    ui.selectable_value(&mut self.version, v.clone(), v);
                                }
                            });
                    }
                });
                ui.end_row();

                muted(ui, tr!("Dimensions"));
                ui.horizontal_wrapped(|ui| {
                    for (name, on) in &mut self.dims {
                        ui.checkbox(on, dimension_label(name));
                    }
                });
                ui.end_row();
            });
            if let Some(dest) = &self.dest {
                ui.add_space(4.0);
                if self.version == dest.mc_version {
                    small_muted(ui, tr!("The same version as {0}.", &dest.name));
                } else if self.versions.contains(&dest.mc_version) {
                    wrapped(
                        ui,
                        tr!(
                            "{0} runs Minecraft {1}. A world made for another version may not open there.",
                            &dest.name,
                            &dest.mc_version
                        ),
                        c_warn(),
                    );
                } else {
                    wrapped(
                        ui,
                        tr!(
                            "{0} runs Minecraft {1}, which worlds cannot be written for. The world may not open there.",
                            &dest.name,
                            &dest.mc_version
                        ),
                        c_warn(),
                    );
                }
            }
        });
    }

    fn output_card(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let check = self.output_check();
        card(ui, |ui| {
            label_over(ui, &if self.dest.is_some() { tr!("ADD TO THE INSTANCE AS") } else { tr!("SAVE AS") });
            ui.horizontal(|ui| {
                let button_w = 150.0;
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.output)
                        .desired_width((ui.available_width() - button_w - 8.0).max(120.0))
                        .hint_text(tr!("A folder for the converted world"))
                        .margin(egui::Margin::symmetric(12, 8))
                        .background_color(c_field()),
                );
                if field.changed() {
                    self.output_edited();
                }
                ui.add_enabled_ui(!self.picking, |ui| {
                    if secondary_icon(ui, Icon::Folder, &tr!("Choose...")).clicked() {
                        acts.push(Act::Pick(Pick::OutputParent));
                    }
                });
            });
            ui.add_space(2.0);
            match check {
                OutputCheck::Ok => match &self.dest {
                    Some(dest) => small_muted(
                        ui,
                        tr!(
                            "The world goes into the saves folder of {0}. The original world is not changed.",
                            &dest.name
                        ),
                    ),
                    None => small_muted(ui, tr!("The original world is not changed.")),
                },
                OutputCheck::Empty => {
                    ui.label(RichText::new(tr!("Choose where to save the converted world.")).color(c_warn()));
                }
                OutputCheck::IsSource => {
                    wrapped(
                        ui,
                        tr!(
                            "That is the world you are converting. Choose another place so the original stays as it is."
                        ),
                        c_danger(),
                    );
                }
                OutputCheck::InsideSource => {
                    wrapped(ui, tr!("That is inside the world you are converting. Choose another place."), c_danger());
                }
                OutputCheck::IsFile => {
                    wrapped(ui, tr!("A file with that name is already there. Choose another name."), c_danger());
                }
                OutputCheck::NotEmpty => {
                    wrapped(
                        ui,
                        tr!(
                            "That folder already exists and has something in it. Choose another name so nothing gets mixed up or overwritten."
                        ),
                        c_warn(),
                    );
                    let free = free_name(std::path::Path::new(self.output.trim()));
                    let name = free.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    if secondary(ui, &tr!("Use {0}", name)).clicked() {
                        self.output = free.to_string_lossy().to_string();
                        self.output_edited();
                    }
                }
            }
        });
    }

    // -----------------------------------------------------------------------------------------------------
    // Running

    fn running(&mut self, ui: &mut Ui) {
        let name = self.found().map(|d| d.name.clone()).unwrap_or_default();
        let Some(run) = self.run.as_mut() else { return };
        // the clock on this page is the only thing that moves without an event
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        let mut stop = false;
        let mut confirmed = false;
        card(ui, |ui| {
            label_over(ui, &tr!("CONVERTING"));
            ui.add(egui::Label::new(RichText::new(&name).size(17.0).strong()).truncate());
            ui.horizontal(|ui| {
                tag(ui, &format!("{} {}", run.job.target.edition.label(), run.job.target.version), c_accent());
            });
            ui.add_space(8.0);
            let stage = if run.stopping { tr!("Stopping...") } else { run.stage.clone() };
            ui.add(egui::Label::new(stage).truncate());
            ui.add_space(4.0);
            let bar = egui::ProgressBar::new(run.progress).desired_height(14.0).animate(run.progress <= 0.0);
            ui.add(if run.progress > 0.0 { bar.show_percentage() } else { bar });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                small_muted(ui, tr!("Time passed: {0}", format_elapsed(run.started.elapsed())));
                let warnings = run.warnings.len() + run.more_warnings;
                if warnings > 0 {
                    small_muted(ui, tr_n_warnings(warnings));
                }
            });
            ui.add_space(8.0);
            ui.add_enabled_ui(!run.stopping, |ui| {
                if danger_icon(ui, Icon::Stop, &tr!("Cancel")).clicked() {
                    stop = true;
                }
            });
        });
        if run.confirm {
            match stop_modal(ui.ctx()) {
                Some(true) => confirmed = true,
                Some(false) => run.confirm = false,
                None => {}
            }
        }
        if confirmed {
            if let Some(run) = self.run.as_mut() {
                ConvertState::stop_now(run);
            }
        }
        if stop {
            self.ask_stop();
        }
    }

    // -----------------------------------------------------------------------------------------------------
    // The result

    fn done(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let Some(s) = self.summary.as_ref() else { return };
        let mut again = false;
        let dest = self.dest.clone();
        card(ui, |ui| {
            label_over(ui, &tr!("DONE"));
            match &dest {
                Some(d) => {
                    ui.label(RichText::new(tr!("Your world was added to {0}", &d.name)).size(19.0).strong());
                    ui.add_space(2.0);
                    muted(ui, tr!("It is in the instance's saves folder and shows up in Singleplayer."));
                }
                None => {
                    ui.label(RichText::new(tr!("Your world was converted")).size(19.0).strong());
                }
            }
            ui.add_space(6.0);
            egui::Grid::new("convert-summary").num_columns(2).spacing([24.0, 8.0]).show(ui, |ui| {
                muted(ui, tr!("Columns converted"));
                ui.label(number(s.columns));
                ui.end_row();
                muted(ui, tr!("Time taken"));
                ui.label(format_elapsed(s.elapsed));
                ui.end_row();
                muted(ui, tr!("Blocks without a match"));
                let kinds = s.unmapped_blocks.len();
                ui.label(if kinds == 0 {
                    tr!("None")
                } else {
                    crate::i18n::tr_n("{0} kind", "{0} kinds", kinds as u64, &[kinds.into()])
                });
                ui.end_row();
                muted(ui, tr!("Saved in"));
                ui.add(egui::Label::new(s.output.display().to_string()).truncate());
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                match &dest {
                    Some(d) => {
                        if primary_icon(ui, Icon::Folder, &tr!("Open the saves folder")).clicked() {
                            acts.push(Act::Open(d.saves.clone()));
                        }
                    }
                    None => {
                        if primary_icon(ui, Icon::Folder, &tr!("Open folder")).clicked() {
                            acts.push(Act::Open(s.output.clone()));
                        }
                    }
                }
                let another = if dest.is_some() { tr!("Add another") } else { tr!("Convert another") };
                if secondary(ui, &another).clicked() {
                    again = true;
                }
            });
        });
        if !s.warnings.is_empty() {
            ui.add_space(8.0);
            card(ui, |ui| {
                egui::CollapsingHeader::new(RichText::new(tr_n_warnings(s.warnings.len())).strong())
                    .id_salt("convert-warnings")
                    .default_open(false)
                    .show(ui, |ui| {
                        ScrollArea::vertical().max_height(180.0).auto_shrink([false, true]).show(ui, |ui| {
                            for w in &s.warnings {
                                ui.horizontal_top(|ui| {
                                    ui.label(RichText::new("\u{2022}").color(c_warn()));
                                    ui.add(egui::Label::new(w).wrap());
                                });
                            }
                        });
                    });
            });
        }
        if !s.unmapped_blocks.is_empty() {
            ui.add_space(8.0);
            card(ui, |ui| {
                ui.label(RichText::new(tr!("Blocks without an exact match")).strong());
                small_muted(ui, tr!("These blocks have no exact equivalent in the new version."));
                ui.add_space(4.0);
                unmapped_row(ui, &tr!("Block"), &tr!("Count"), true, false);
                for (i, (block, count)) in s.unmapped_blocks.iter().take(MAX_UNMAPPED).enumerate() {
                    unmapped_row(ui, block, &number(*count), false, i % 2 == 0);
                }
                let more = s.unmapped_blocks.len().saturating_sub(MAX_UNMAPPED);
                if more > 0 {
                    small_muted(ui, tr!("and {0} more", more));
                }
            });
        }
        if again {
            self.start_over();
        }
    }

    fn failed(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        card(ui, |ui| {
            ui.label(RichText::new(tr!("THE WORLD COULD NOT BE CONVERTED")).size(11.0).strong().color(c_danger()));
            ui.add_space(2.0);
            let message = self.error.clone().unwrap_or_default();
            ui.add(egui::Label::new(RichText::new(message).size(15.0)).wrap());
            ui.add_space(4.0);
            small_muted(
                ui,
                tr!("The original world is not changed. \"Copy details\" copies what to send when you ask for help."),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if primary(ui, &tr!("Try again")).clicked() {
                    self.screen = Screen::Setup;
                    self.error = None;
                }
                if secondary(ui, &tr!("Copy details")).clicked() {
                    acts.push(Act::Copy(self.details()));
                }
                if secondary(ui, &tr!("Choose another world")).clicked() {
                    self.start_over();
                }
            });
        });
    }
}

/// "Stop the conversion?": `Some(true)` to stop, `Some(false)` to go on, `None` while it is asked.
fn stop_modal(ctx: &egui::Context) -> Option<bool> {
    let mut answer = None;
    egui::Modal::new(egui::Id::unique("convert-stop")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
        ui.set_width(super::services::modal_width(ctx, 420.0));
        ui.label(RichText::new(tr!("Cancel the conversion?")).size(19.0).strong());
        ui.add_space(4.0);
        muted(ui, tr!("The new world is not finished and will not work properly. The original world is not changed."));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if danger(ui, &tr!("Cancel conversion")).clicked() {
                answer = Some(true);
            }
            if secondary(ui, &tr!("Keep converting")).clicked() {
                answer = Some(false);
            }
        });
    });
    if answer.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        answer = Some(false);
    }
    answer
}

/// A row of the table of blocks without a match: the name on the left, the count on the right.
fn unmapped_row(ui: &mut Ui, name: &str, count: &str, header: bool, shaded: bool) {
    let fill = if shaded { c_field() } else { Color32::TRANSPARENT };
    egui::Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let size = vec2(ui.available_width(), 18.0);
            ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
                if header {
                    ui.label(RichText::new(count).color(c_muted()).size(12.0));
                } else {
                    ui.label(RichText::new(count).monospace().size(12.5));
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    if header {
                        ui.label(RichText::new(name).color(c_muted()).size(12.0));
                    } else {
                        ui.add(egui::Label::new(RichText::new(name).monospace().size(12.5)).truncate());
                    }
                });
            });
        });
}

fn tr_n_warnings(n: usize) -> String {
    crate::i18n::tr_n("{0} warning", "{0} warnings", n as u64, &[n.into()])
}

fn edition_color(e: Edition) -> Color32 {
    match e {
        Edition::Java => c_accent(),
        Edition::Bedrock => c_warn(),
    }
}

/// A wrapped line of coloured text.
fn wrapped(ui: &mut Ui, text: String, color: Color32) {
    ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
}

/// The dashed box a world can be dropped on.
fn drop_zone(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 100.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(10), c_field());
    let r = rect.shrink(1.0);
    let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    ui.painter().extend(egui::Shape::dashed_line(&corners, Stroke::new(1.5, c_border()), 6.0, 5.0));
    let wrap = rect.width() - 32.0;
    let title = ui.painter().layout(tr!("Drop a world here"), egui::FontId::proportional(17.0), c_text(), wrap);
    let hint = ui.painter().layout(
        tr!("A Bedrock world folder, or a world packed as .mcworld or .zip"),
        egui::FontId::proportional(14.0),
        c_muted(),
        wrap,
    );
    let total = title.size().y + 4.0 + hint.size().y;
    let top = rect.center().y - total / 2.0;
    let title_pos = egui::pos2(rect.center().x - title.size().x / 2.0, top);
    let hint_pos = egui::pos2(rect.center().x - hint.size().x / 2.0, top + title.size().y + 4.0);
    ui.painter().galley(title_pos, title, c_text());
    ui.painter().galley(hint_pos, hint, c_muted());
}

/// Shown over the page while files are dragged above the window.
fn drop_hint(ui: &mut Ui) {
    if ui.ctx().input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    let rect = ui.max_rect();
    let painter =
        ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, ui.scope_id().with("convert-drop")));
    painter.rect_filled(rect, CornerRadius::same(12), c_accent_soft());
    painter.rect_stroke(
        rect.shrink(4.0),
        CornerRadius::same(12),
        Stroke::new(2.0, c_accent()),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        tr!("Drop the world to convert it"),
        egui::FontId::proportional(20.0),
        c_text(),
    );
}
