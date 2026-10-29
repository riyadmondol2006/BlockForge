//! What the Convert page looks like: the set-up, the progress, the result and the error screens.

use egui::{Align, Color32, CornerRadius, Layout, RichText, ScrollArea, Sense, Stroke, Ui, vec2};

use super::convert::{
    Act, CHUNKER_URL, ConvertState, Look, MAX_UNMAPPED, OutputCheck, Pick, Screen, dimension_label, format_elapsed,
    free_name, number,
};
use super::craft_fmt::{check_labeled, small_faint};
use super::icons::Icon;
use super::theme::*;
use super::widgets::*;
use crate::convert::backend::Edition;
use crate::tr;

/// How wide the page's content gets on a wide window.
const PAGE_WIDTH: f32 = 720.0;
/// The footer's height, kept free below the scrolling part.
const FOOTER: f32 = 30.0;
/// The height the pinned bar takes under the scrolling part: the gap, the room above the button, the button.
const START_BAR: f32 = 46.0;
/// The gap between the last block and the Convert button when the button follows the blocks.
const BAR_GAP: f32 = 24.0;
/// The height of the Convert button.
const BUTTON_H: f32 = 32.0;
/// The room below the blocks inside the scrolling part.
const TAIL: f32 = 8.0;
/// The height of the dashed area that waits for a world.
const ZONE_HEIGHT: f32 = 112.0;

/// A text of a button or a wait with its closing "..." typeset as one ellipsis character. The text in `tr!` stays the
/// key of the translations as it was written; a translation that ends in three dots is shown the same way.
fn ellipsis(text: String) -> String {
    match text.strip_suffix("...") {
        Some(head) => format!("{head}\u{2026}"),
        None => text,
    }
}

/// A block's label in sentence case. The texts are written in capitals in the source (they are the keys of the
/// translations), so a text that is all capitals is read as a sentence: "BEDROCK WORLD" -> "Bedrock world".
fn sentence(text: &str) -> String {
    let all_caps = text.chars().any(|c| c.is_alphabetic()) && !text.chars().any(|c| c.is_lowercase());
    if !all_caps {
        return text.to_string();
    }
    let lower = text.to_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The title of a block of the page, with the room under it.
fn block_title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(sentence(text)).size(FS_SECTION).semibold());
    ui.add_space(2.0);
}

/// One option of a card: its name on the left, its control on the right, both centred on the row. (The shared
/// `setting_row` puts a title without a hint at the top of the row, so it is not used here.)
fn option_row(ui: &mut Ui, title: &str, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(32.0);
        ui.label(RichText::new(title).semibold());
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
}

/// The tick boxes of the dimensions in a row, first to last, so Tab visits them in the order they are read. The group
/// is measured first, as it sits at the right of its row.
fn dimension_checks(ui: &mut Ui, dims: &mut [(String, bool)]) {
    let spacing = ui.spacing().item_spacing.x;
    let labels: Vec<String> = dims.iter().map(|(name, _)| dimension_label(name)).collect();
    let width: f32 = labels
        .iter()
        .map(|l| ui.painter().layout_no_wrap(l.clone(), egui::FontId::proportional(FS_BODY), c_text()).size().x + 28.0)
        .sum::<f32>()
        + spacing * labels.len().saturating_sub(1) as f32;
    ui.allocate_ui_with_layout(vec2(width, 32.0), Layout::left_to_right(Align::Center), |ui| {
        for ((_, on), label) in dims.iter_mut().zip(&labels) {
            check_labeled(ui, on, label);
        }
    });
}

impl ConvertState {
    /// Draws the page; returns what it asks the window to do.
    pub(super) fn show(&mut self, ui: &mut Ui) -> Vec<Act> {
        let mut acts = Vec::new();
        ui.scope(|ui| {
            ui.set_max_width((ui.available_width() - 12.0).min(PAGE_WIDTH));
            ui.horizontal(|ui| {
                heading(ui, &if self.dest.is_some() { tr!("Add a Bedrock world") } else { tr!("World converter") });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.screen != Screen::Running {
                        // the standalone converter is a page of its own in the sidebar: nowhere to go back to
                        if let Some(d) = &self.dest {
                            if ui.link(RichText::new(tr!("Back to {0}", &d.name)).color(c_muted())).clicked() {
                                acts.push(Act::BackToInstance(d.folder.clone()));
                            }
                        }
                    }
                });
            });
            match &self.dest {
                Some(dest) => muted(ui, tr!("Bring a Bedrock world to Java Edition and add it to {0}", &dest.name)),
                None => muted(ui, tr!("Turn a Bedrock Edition world into a Java Edition world")),
            }
        });
        ui.add_space(12.0);
        // The Convert button follows the last block. Only when the blocks do not fit the panel does it stay in view
        // under the scrolling part, with a hairline over it so the clipped edge of the blocks shows.
        let has_bar = self.screen == Screen::Setup && self.found().is_some();
        let room = (ui.available_height() - FOOTER).max(120.0);
        let pinned = has_bar && self.bar_pinned;
        let height = if pinned { (room - START_BAR).max(120.0) } else { room };
        let mut needs_pin = false;
        let page = ScrollArea::vertical().id_salt("convert-page").max_height(height).auto_shrink([false, false]).show(
            ui,
            |ui| {
                ui.set_max_width((ui.available_width() - 12.0).min(PAGE_WIDTH));
                match self.screen {
                    Screen::Setup => self.setup(ui, &mut acts),
                    Screen::Running => self.running(ui),
                    Screen::Done => self.done(ui, &mut acts),
                    Screen::Failed => self.failed(ui, &mut acts),
                }
                if has_bar {
                    // the last block ends at the min rect; the layout adds its item spacing under it
                    needs_pin = ui.min_rect().height() + BAR_GAP + BUTTON_H > room;
                }
                if has_bar && !needs_pin {
                    ui.add_space(BAR_GAP - ui.spacing().item_spacing.y);
                    self.start_bar(ui, &mut acts, false);
                } else {
                    ui.add_space(TAIL);
                }
            },
        );
        if has_bar && needs_pin != self.bar_pinned {
            // the page learned how it fits: draw it again at once with the button where it belongs
            self.bar_pinned = needs_pin;
            ui.ctx().request_discard("the Convert button moved under or into the blocks");
            ui.ctx().request_repaint();
        }
        if needs_pin {
            // a hairline where the blocks are cut off, so the clipped edge reads as an edge
            let y = page.inner_rect.bottom() + 0.5;
            ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, c_border()));
            self.start_bar(ui, &mut acts, true);
        }
        self.footer(ui, &mut acts);
        if self.screen == Screen::Setup && !matches!(self.look, Look::Nothing) {
            drop_hint(ui);
        }
        acts
    }

    /// "Powered by Chunker by Hive Games (MIT)", the credit the converter owes its origin: one quiet line.
    fn footer(&self, ui: &mut Ui, acts: &mut Vec<Act>) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            small_faint(ui, tr!("Powered by"));
            if ui
                .link(RichText::new(tr!("Chunker by Hive Games")).size(FS_META).color(c_faint()).underline())
                .on_hover_text(CHUNKER_URL)
                .clicked()
            {
                acts.push(Act::Link(CHUNKER_URL));
            }
            small_faint(ui, "(MIT)");
        });
    }

    // -----------------------------------------------------------------------------------------------------
    // Set up

    fn setup(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        self.world_block(ui, acts);
        if self.found().is_none() {
            return;
        }
        ui.add_space(16.0);
        self.target_block(ui);
        ui.add_space(16.0);
        self.output_block(ui, acts);
    }

    /// The Convert button, and why it cannot be pressed yet when that is so. `pinned`: it sits under the scrolling
    /// part, with a little more room above it.
    fn start_bar(&mut self, ui: &mut Ui, acts: &mut Vec<Act>, pinned: bool) {
        let blocked = self.blocked();
        if pinned {
            ui.add_space(4.0);
        }
        ui.horizontal(|ui| {
            if primary_enabled_icon(ui, Icon::Convert, &tr!("Convert"), blocked.is_none()).clicked() {
                acts.push(Act::Start);
            }
            if let Some(why) = &blocked {
                ui.add(egui::Label::new(RichText::new(why).color(c_faint()).size(FS_META)).wrap());
            }
        });
    }

    fn world_block(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        block_title(ui, &tr!("BEDROCK WORLD"));
        match &self.look {
            Look::Nothing => {
                self.drop_zone(ui, acts);
                return;
            }
            Look::Busy => {
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        spinner(ui);
                        ui.label(ellipsis(tr!("Looking at the world...")));
                    });
                    self.source_path(ui);
                });
            }
            Look::Unknown(why) => {
                card(ui, |ui| {
                    self.source_path(ui);
                    ui.add_space(4.0);
                    notice(ui, NoticeKind::Error, why, None);
                });
            }
            Look::Found(d) => {
                card(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_max_width((ui.available_width() - 120.0).max(160.0));
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.add(egui::Label::new(RichText::new(&d.name).size(FS_SECTION).semibold()).truncate());
                            small_muted(
                                ui,
                                format!(
                                    "{} \u{00B7} {} \u{00B7} {}",
                                    d.edition.label(),
                                    d.version,
                                    human_size(d.size_bytes)
                                ),
                            );
                            self.source_path(ui);
                        });
                        ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                            let change = secondary(ui, &ellipsis(tr!("Change...")));
                            egui::Popup::menu(&change).show(|ui| {
                                ui.set_min_width(150.0);
                                if ui.button(ellipsis(tr!("A world folder..."))).clicked() {
                                    acts.push(Act::Pick(Pick::SourceFolder));
                                    ui.close();
                                }
                                if ui.button(ellipsis(tr!("A packed world..."))).clicked() {
                                    acts.push(Act::Pick(Pick::SourceFile));
                                    ui.close();
                                }
                            });
                        });
                    });
                });
                return;
            }
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!self.picking, |ui| {
                if secondary_icon(ui, Icon::Folder, &ellipsis(tr!("Another folder..."))).clicked() {
                    acts.push(Act::Pick(Pick::SourceFolder));
                }
                if secondary(ui, &ellipsis(tr!("Another file..."))).clicked() {
                    acts.push(Act::Pick(Pick::SourceFile));
                }
            });
        });
    }

    /// The calm dashed area that waits for a world: one sentence, and the two ways to choose one. It takes the hover
    /// fill under the pointer and the selection fill while a file is dragged over the window.
    fn drop_zone(&self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ZONE_HEIGHT), Sense::hover());
        let dragging = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        let (fill, edge) = if dragging {
            (Some(c_selection()), c_text())
        } else if ui.rect_contains_pointer(rect) {
            (Some(c_hover()), c_control())
        } else {
            (None, c_control())
        };
        if let Some(fill) = fill {
            ui.painter().rect_filled(rect, CornerRadius::same(R_CARD), fill);
        }
        let outline = rounded_outline(rect.shrink(0.5), f32::from(R_CARD));
        ui.painter().extend(egui::Shape::dashed_line(&outline, Stroke::new(1.0, edge), 5.0, 4.0));
        let mut inner = ui.new_child(
            egui::UiBuilder::new().max_rect(rect.shrink2(vec2(16.0, 0.0))).layout(Layout::top_down(Align::Center)),
        );
        inner.spacing_mut().item_spacing.y = 4.0;
        inner.add_space(14.0);
        inner.label(RichText::new(tr!("Drop a world here")).font(super::fonts::medium(FS_BODY)));
        inner.add(
            egui::Label::new(
                RichText::new(tr!("A Bedrock world folder, or a world packed as .mcworld or .zip"))
                    .color(c_muted())
                    .size(FS_META),
            )
            .wrap(),
        );
        inner.add_space(8.0);
        inner.add_enabled_ui(!self.picking, |ui| {
            // two buttons, centred as a pair
            let folder = ellipsis(tr!("Choose a folder..."));
            let file = ellipsis(tr!("Choose a file..."));
            let est = |t: &str| {
                ui.painter().layout_no_wrap(t.to_string(), super::fonts::medium(FS_BODY), c_text()).size().x + 28.0
            };
            let pair = est(&folder) + 22.0 + est(&file) + ui.spacing().item_spacing.x;
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - pair) / 2.0).max(0.0));
                if secondary_icon(ui, Icon::Folder, &folder).clicked() {
                    acts.push(Act::Pick(Pick::SourceFolder));
                }
                if secondary(ui, &file).clicked() {
                    acts.push(Act::Pick(Pick::SourceFile));
                }
            });
        });
    }

    fn source_path(&self, ui: &mut Ui) {
        if let Some(src) = &self.source {
            ui.add(
                egui::Label::new(RichText::new(src.display().to_string()).color(c_faint()).size(FS_META)).truncate(),
            );
        }
    }

    fn target_block(&mut self, ui: &mut Ui) {
        block_title(ui, &tr!("CONVERT TO"));
        card(ui, |ui| {
            // the target is always Java Edition: there is nothing to choose
            option_row(ui, &tr!("Game"), |ui| {
                ui.label(RichText::new(Edition::Java.label()).semibold());
            });
            rule(ui);
            option_row(ui, &tr!("Version"), |ui| {
                if self.versions.is_empty() {
                    small_muted(ui, tr!("No versions are available yet."));
                } else {
                    egui::ComboBox::from_id_salt("convert-version").selected_text(&self.version).width(160.0).show_ui(
                        ui,
                        |ui| {
                            for v in &self.versions {
                                ui.selectable_value(&mut self.version, v.clone(), v);
                            }
                        },
                    );
                }
            });
            rule(ui);
            option_row(ui, &tr!("Dimensions"), |ui| dimension_checks(ui, &mut self.dims));
            if let Some(dest) = &self.dest {
                ui.add_space(6.0);
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

    fn output_block(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let check = self.output_check();
        block_title(ui, &if self.dest.is_some() { tr!("ADD TO THE INSTANCE AS") } else { tr!("SAVE AS") });
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let button_w = 130.0;
                let field = ui.add_sized(
                    [(ui.available_width() - button_w - 8.0).max(120.0), 32.0],
                    egui::TextEdit::singleline(&mut self.output)
                        .hint_text(RichText::new(tr!("A folder for the converted world")).color(c_faint()))
                        .margin(egui::Margin::symmetric(10, 7))
                        .background_color(c_field()),
                );
                if field.changed() {
                    self.output_edited();
                }
                ui.add_enabled_ui(!self.picking, |ui| {
                    if secondary_icon(ui, Icon::Folder, &ellipsis(tr!("Choose..."))).clicked() {
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
                    wrapped(ui, tr!("Choose where to save the converted world."), c_warn());
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
                OutputCheck::NotAbsolute => {
                    wrapped(
                        ui,
                        tr!("Write the whole path, starting from a drive or your home folder (~), or use Choose."),
                        c_warn(),
                    );
                }
                OutputCheck::NotWritable => {
                    wrapped(ui, tr!("BlockForge isn't allowed to write there. Choose another place."), c_danger());
                }
                OutputCheck::NotEmpty => {
                    wrapped(
                        ui,
                        tr!(
                            "That folder already exists and has something in it. Choose another name so nothing gets mixed up or overwritten."
                        ),
                        c_warn(),
                    );
                    ui.add_space(4.0);
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
        block_title(ui, &tr!("CONVERTING"));
        card(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(&name).size(FS_SECTION).semibold()).truncate());
            small_muted(ui, format!("{} {}", run.job.target.edition.label(), run.job.target.version));
            ui.add_space(12.0);
            let stage = if run.stopping { ellipsis(tr!("Stopping...")) } else { run.stage.clone() };
            ui.add(egui::Label::new(RichText::new(stage).color(c_muted())).truncate());
            ui.add_space(2.0);
            progress_bar(ui, (run.progress > 0.0).then_some(run.progress), 6.0);
            ui.add_space(12.0);
            // "Time passed: 0:42" is one text in the translations; its label and value are told apart again here
            let passed = tr!("Time passed: {0}", format_elapsed(run.started.elapsed()));
            let (passed_label, passed_value) = match passed.split_once(": ") {
                Some((l, v)) => (l.to_string(), v.to_string()),
                None => (tr!("Time passed"), passed.clone()),
            };
            let warnings = run.warnings.len() + run.more_warnings;
            let progress = if run.progress > 0.0 { format!("{:.0}%", run.progress * 100.0) } else { "\u{2013}".into() };
            let (progress_label, warnings_label) = (tr!("Progress"), tr!("Warnings"));
            facts(
                ui,
                &[
                    (progress_label.as_str(), progress),
                    (passed_label.as_str(), passed_value),
                    (warnings_label.as_str(), number(warnings as u64)),
                ],
            );
            ui.add_space(12.0);
            ui.add_enabled_ui(!run.stopping, |ui| {
                if secondary(ui, &tr!("Cancel")).clicked() {
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
            match &dest {
                Some(d) => {
                    ui.label(RichText::new(tr!("Your world was added to {0}", &d.name)).size(FS_SECTION).semibold());
                    small_muted(ui, tr!("It is in the instance's saves folder and shows up in Singleplayer."));
                }
                None => {
                    ui.label(RichText::new(tr!("Your world was converted")).size(FS_SECTION).semibold());
                }
            }
            ui.add_space(12.0);
            let kinds = s.unmapped_blocks.len();
            let (columns_label, time_label, unmatched_label) =
                (tr!("Columns converted"), tr!("Time taken"), tr!("Blocks without a match"));
            facts(
                ui,
                &[
                    (columns_label.as_str(), number(s.columns)),
                    (time_label.as_str(), format_elapsed(s.elapsed)),
                    (
                        unmatched_label.as_str(),
                        if kinds == 0 {
                            tr!("None")
                        } else {
                            crate::i18n::tr_n("{0} kind", "{0} kinds", kinds as u64, &[kinds.into()])
                        },
                    ),
                ],
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                small_muted(ui, tr!("Saved in"));
                ui.add(egui::Label::new(RichText::new(s.output.display().to_string()).size(FS_META)).truncate());
            });
            ui.add_space(12.0);
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
            ui.add_space(12.0);
            card(ui, |ui| {
                egui::CollapsingHeader::new(RichText::new(tr_n_warnings(s.warnings.len())).semibold())
                    .id_salt("convert-warnings")
                    .default_open(false)
                    .show(ui, |ui| {
                        ScrollArea::vertical().max_height(180.0).auto_shrink([false, true]).show(ui, |ui| {
                            for w in &s.warnings {
                                ui.horizontal_top(|ui| {
                                    ui.label(RichText::new("\u{2022}").color(c_muted()));
                                    ui.add(egui::Label::new(RichText::new(w).color(c_muted())).wrap());
                                });
                            }
                        });
                    });
            });
        }
        if !s.unmapped_blocks.is_empty() {
            ui.add_space(12.0);
            card(ui, |ui| {
                ui.label(RichText::new(tr!("Blocks without an exact match")).semibold());
                small_muted(ui, tr!("These blocks have no exact equivalent in the new version."));
                ui.add_space(6.0);
                unmapped_row(ui, &tr!("Block"), &tr!("Count"), true);
                for (block, count) in s.unmapped_blocks.iter().take(MAX_UNMAPPED) {
                    unmapped_row(ui, block, &number(*count), false);
                }
                let more = s.unmapped_blocks.len().saturating_sub(MAX_UNMAPPED);
                if more > 0 {
                    ui.add_space(4.0);
                    small_muted(ui, tr!("and {0} more", more));
                }
            });
        }
        if again {
            self.start_over();
        }
    }

    fn failed(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let message = self.error.clone().unwrap_or_default();
        let details = self.details();
        notice(ui, NoticeKind::Error, &sentence(&tr!("THE WORLD COULD NOT BE CONVERTED")), None);
        ui.add_space(8.0);
        ui.add(egui::Label::new(RichText::new(message).size(FS_BODY).color(c_text())).wrap());
        ui.add_space(8.0);
        small_muted(
            ui,
            tr!("The original world is not changed. \"Copy details\" copies what to send when you ask for help."),
        );
        ui.add_space(4.0);
        egui::CollapsingHeader::new(RichText::new(tr!("Details")).color(c_muted())).id_salt("convert-details").show(
            ui,
            |ui| {
                ScrollArea::vertical().max_height(160.0).auto_shrink([false, true]).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(details).monospace().size(FS_META).color(c_muted())).wrap());
                });
            },
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if self.error_final {
                // the world itself is the problem: going back to its settings can't help
                if secondary(ui, &tr!("Choose another world")).clicked() {
                    self.start_over();
                }
            } else if secondary(ui, &tr!("Back to the settings")).clicked() {
                // the check of the output folder is made afresh: what made the run fail may have changed
                self.back_to_setup();
            }
            if ghost(ui, &tr!("Copy details")).clicked() {
                acts.push(Act::Copy(self.details()));
            }
            if !self.error_final && ghost(ui, &tr!("Choose another world")).clicked() {
                self.start_over();
            }
        });
    }
}

/// "Stop the conversion?": `Some(true)` to stop, `Some(false)` to go on, `None` while it is asked.
fn stop_modal(ctx: &egui::Context) -> Option<bool> {
    let mut answer = None;
    egui::Modal::new(egui::Id::unique("convert-stop")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
        ui.set_width(super::services::modal_width(ctx, 420.0));
        ui.label(RichText::new(tr!("Cancel the conversion?")).size(FS_SECTION).semibold());
        ui.add_space(4.0);
        muted(ui, tr!("What was written of the new world so far is deleted. The original world is not changed."));
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

/// A row of the table of blocks without a match: the name on the left, the count on the right, a hairline under it.
fn unmapped_row(ui: &mut Ui, name: &str, count: &str, header: bool) {
    let height = if header { 22.0 } else { 28.0 };
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter();
    let (color, font_name, font_count) = if header {
        (c_muted(), egui::FontId::proportional(FS_META), egui::FontId::proportional(FS_META))
    } else {
        (c_text(), egui::FontId::monospace(FS_META), egui::FontId::proportional(FS_BODY))
    };
    painter.with_clip_rect(rect.shrink2(vec2(0.0, 0.0)).with_max_x(rect.right() - 80.0)).text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        name,
        font_name,
        color,
    );
    painter.text(rect.right_center(), egui::Align2::RIGHT_CENTER, count, font_count, color);
    painter.hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, c_rule()));
}

fn tr_n_warnings(n: usize) -> String {
    crate::i18n::tr_n("{0} warning", "{0} warnings", n as u64, &[n.into()])
}

/// A wrapped line of coloured small text, for what is said under a field.
fn wrapped(ui: &mut Ui, text: String, color: Color32) {
    ui.add(egui::Label::new(RichText::new(text).color(color).size(FS_META)).wrap());
}

/// Shown over the page while files are dragged above the window, when no drop area is on show: the selection fill and a
/// plain edge, not colour.
fn drop_hint(ui: &mut Ui) {
    if ui.ctx().input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    let rect = ui.max_rect();
    let painter =
        ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, ui.scope_id().with("convert-drop")));
    painter.rect_filled(rect, CornerRadius::same(R_CARD), c_selection());
    painter.rect_stroke(
        rect.shrink(4.0),
        CornerRadius::same(R_CARD),
        Stroke::new(2.0, c_text()),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        tr!("Drop the world to convert it"),
        super::fonts::semibold(FS_TITLE),
        c_text(),
    );
}

/// The outline of a rectangle with round corners, as points to draw a dashed line along.
fn rounded_outline(rect: egui::Rect, radius: f32) -> Vec<egui::Pos2> {
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (egui::pos2(rect.right() - r, rect.top() + r), -std::f32::consts::FRAC_PI_2),
        (egui::pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (egui::pos2(rect.left() + r, rect.bottom() - r), std::f32::consts::FRAC_PI_2),
        (egui::pos2(rect.left() + r, rect.top() + r), std::f32::consts::PI),
    ];
    let mut points = Vec::new();
    for (centre, start) in corners {
        for k in 0..=6 {
            let a = start + std::f32::consts::FRAC_PI_2 * k as f32 / 6.0;
            points.push(centre + vec2(a.cos(), a.sin()) * r);
        }
    }
    points.push(points[0]);
    points
}

#[cfg(test)]
mod tests {
    use super::ellipsis;

    #[test]
    fn a_closing_three_dots_becomes_one_ellipsis() {
        assert_eq!(ellipsis("Choose...".to_string()), "Choose\u{2026}");
        assert_eq!(ellipsis("Change".to_string()), "Change");
        assert_eq!(ellipsis("A... b".to_string()), "A... b");
    }
}
