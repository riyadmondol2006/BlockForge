//! What the Convert page looks like: the set-up, the progress, the result and the error screens.

use egui::{Align, Color32, CornerRadius, Layout, RichText, ScrollArea, Sense, Stroke, Ui, vec2};

use super::convert::{
    Act, CHUNKER_URL, ConvertState, Dest, Look, MAX_UNMAPPED, OutputCheck, Pick, Screen, dimension_label,
    format_elapsed, free_name, number,
};
use super::craft_fmt::{check_labeled, small_faint};
use super::icons::Icon;
use super::theme::*;
use super::widgets::*;
use crate::convert::backend::{Edition, EntityOptions, WorldPlayer};
use crate::convert::level::{Difficulty, EntityFilter, GameType, InventorySource, PlayerOptions};
use crate::tr;

/// How wide the page's content gets on a wide window: the rail of steps and the form beside it.
const PAGE_WIDTH: f32 = 960.0;
/// The rail of steps at the left, the room between it and the form, and the least room for both side by side.
const RAIL_W: f32 = 236.0;
const RAIL_GAP: f32 = 24.0;
const RAIL_BESIDE: f32 = 640.0;
/// The height of a step on the rail, and the room between two steps.
const RAIL_ROW: f32 = 56.0;
const RAIL_STEP_GAP: f32 = 14.0;
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
const ZONE_HEIGHT: f32 = 168.0;
/// The side of a step's badge.
const STEP_BADGE: f32 = 28.0;

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

/// How a step of the rail stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    /// Not done yet, nothing wrong.
    Todo,
    Done,
    /// It can go on, but something wants a look.
    Warn,
    /// It can't go on like this.
    Error,
    Busy,
    /// Not open until the world is chosen.
    Locked,
}

/// How the step that says where the world goes stands, from the check of the folder.
fn output_mark(check: OutputCheck) -> Mark {
    match check {
        OutputCheck::Ok => Mark::Done,
        OutputCheck::Empty | OutputCheck::NotAbsolute | OutputCheck::NotEmpty => Mark::Warn,
        OutputCheck::IsSource | OutputCheck::InsideSource | OutputCheck::IsFile | OutputCheck::NotWritable => {
            Mark::Error
        }
    }
}

/// How many of the things that come along by default were changed, for the line under "What comes along".
fn left_behind(e: &EntityOptions, p: &PlayerOptions, make_owner: bool, has_owner: bool) -> usize {
    let (de, dp) = (EntityOptions::default(), PlayerOptions::default());
    let entities = if e.convert {
        [e.filter != de.filter, e.item_frames != de.item_frames, e.drop_transient != de.drop_transient]
            .into_iter()
            .filter(|c| *c)
            .count()
    } else {
        1
    };
    let players = [
        has_owner && !make_owner,
        p.include_ender_chest != dp.include_ender_chest,
        p.include_stats != dp.include_stats,
        p.export_guest_profiles != dp.export_guest_profiles,
        p.inventory_source != dp.inventory_source,
    ]
    .into_iter()
    .filter(|c| *c)
    .count();
    entities + players
}

/// The round badge of step `n` on the rail: the brand gradient with a check mark when it is ready, amber or red with
/// an exclamation mark when it needs a look, a turning arc while it runs, and its number otherwise.
fn paint_mark(ui: &Ui, rect: egui::Rect, mark: Mark, n: usize, current: bool, id: egui::Id) {
    let p = ui.painter();
    let r = (rect.width() / 2.0) as u8;
    let c = rect.center();
    let t = super::motion::toward(ui.ctx(), id.with("done"), mark == Mark::Done, super::motion::MEDIUM);
    let (fill, line) = match mark {
        Mark::Warn => (c_warn_bg(), c_warn().gamma_multiply(0.7)),
        Mark::Error => (c_danger_bg(), c_danger_line()),
        Mark::Locked => (c_field(), c_border()),
        _ if current => (c_card(), c_accent()),
        _ => (c_card(), c_control()),
    };
    p.rect(rect, CornerRadius::same(r), fill, Stroke::new(1.5, line), egui::StrokeKind::Inside);
    if t > 0.0 {
        super::motion::gradient_rect(p, rect, r, c_accent().gamma_multiply(t), c_accent2().gamma_multiply(t));
    }
    match mark {
        Mark::Done => {
            let mark = Stroke::new(2.0, c_on_accent().gamma_multiply(t));
            p.line_segment([c + vec2(-5.0, 0.5), c + vec2(-1.5, 4.0)], mark);
            p.line_segment([c + vec2(-1.5, 4.0), c + vec2(5.0, -3.5)], mark);
        }
        Mark::Warn | Mark::Error => {
            let ink = if mark == Mark::Warn { c_warn() } else { c_danger() };
            p.line_segment([c + vec2(0.0, -6.0), c + vec2(0.0, 2.0)], Stroke::new(2.2, ink));
            p.circle_filled(c + vec2(0.0, 6.0), 1.4, ink);
        }
        Mark::Busy => {
            ui.ctx().request_repaint_after(ANIMATION_FRAME);
            let start = if super::motion::reduced(ui.ctx()) { 0.0 } else { ui.input(|i| i.time) as f32 * 4.0 };
            let points: Vec<egui::Pos2> = (0..=16)
                .map(|k| {
                    let a = start + k as f32 / 16.0 * 4.2;
                    c + vec2(a.cos(), a.sin()) * (rect.width() / 2.0 - 5.0)
                })
                .collect();
            p.add(egui::Shape::line(points, Stroke::new(2.0, c_accent())));
        }
        _ => {
            let ink = match mark {
                Mark::Locked => c_faint(),
                _ if current => c_text(),
                _ => c_muted(),
            };
            p.text(c, egui::Align2::CENTER_CENTER, n.to_string(), super::fonts::semibold(FS_META + 1.0), ink);
        }
    }
}

/// `text` on one line of at most `width`, cut short with an ellipsis when it is longer.
fn one_line(ui: &Ui, text: &str, font: egui::FontId, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(10.0));
    ui.painter().layout_job(job)
}

/// The hairline between two rows of a card, with the same room above and below it.
fn row_gap(ui: &mut Ui) {
    ui.add_space(9.0);
    rule(ui);
    ui.add_space(9.0);
}

/// The title of a card of the form, and a quiet line about it.
fn card_head(ui: &mut Ui, title: &str, hint: &str) {
    ui.label(RichText::new(title).size(FS_SECTION).semibold());
    if !hint.is_empty() {
        ui.add(egui::Label::new(RichText::new(hint).color(c_muted()).size(FS_META)).wrap());
    }
    ui.add_space(6.0);
}

/// The picture beside a world's name: a stack of layers in a soft brand tint.
fn world_badge(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL + 2), c_accent_soft());
    super::icons::paint(ui.painter(), Icon::Layers, rect.center(), 22.0, c_accent());
}

/// A round badge at the head of the result: the brand gradient with a check mark when the world was converted, the
/// danger tint with an exclamation mark when it was not.
fn result_badge(ui: &mut Ui, good: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
    let p = ui.painter();
    let c = rect.center();
    if good {
        super::motion::glow(p, c, 46.0, c_accent().gamma_multiply(if is_dark() { 0.22 } else { 0.18 }));
        super::motion::brand_rect(p, rect, 22);
        let mark = Stroke::new(2.6, c_on_accent());
        p.line_segment([c + vec2(-8.0, 0.5), c + vec2(-2.5, 6.0)], mark);
        p.line_segment([c + vec2(-2.5, 6.0), c + vec2(8.5, -6.0)], mark);
    } else {
        p.rect(
            rect,
            CornerRadius::same(22),
            c_danger_bg(),
            Stroke::new(1.0, c_danger_line()),
            egui::StrokeKind::Inside,
        );
        p.line_segment([c + vec2(0.0, -9.0), c + vec2(0.0, 3.0)], Stroke::new(2.6, c_danger()));
        p.circle_filled(c + vec2(0.0, 8.5), 1.7, c_danger());
    }
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

/// A row with a title, a line that says what it does, and a switch at the right.
fn switch_row(ui: &mut Ui, title: &str, hint: &str, on: &mut bool) {
    setting_row(ui, title, hint, |ui| {
        toggle(ui, on);
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
            let title = if self.via_instance { tr!("Add a Bedrock world") } else { tr!("World converter") };
            let subtitle = match &self.dest {
                Some(dest) => tr!("Bring a Bedrock world to Java Edition and add it to {0}", &dest.name),
                None => tr!("Turn a Bedrock Edition world into a Java Edition world"),
            };
            PageHeader::new(&title).subtitle(&subtitle).show(ui, |ui| {
                if self.screen != Screen::Running {
                    // the standalone converter is a page of its own in the sidebar: nowhere to go back to
                    if let Some(d) = self.dest.as_ref().filter(|_| self.via_instance) {
                        if back_link(ui, &tr!("Back to {0}", &d.name)).clicked() {
                            acts.push(Act::BackToInstance(d.folder.clone()));
                        }
                    }
                }
            });
        });
        // the world decides whether there is anything past the first step; a world that was just found moves the
        // page on to the next one
        let found = self.found().is_some();
        let key = egui::Id::unique("convert-found-before");
        let before = ui.data(|d| d.get_temp::<bool>(key)).unwrap_or(found);
        ui.data_mut(|d| d.insert_temp(key, found));
        if !found {
            self.step = 0;
        } else if !before && self.step == 0 {
            self.step = 1;
        }
        let marks = self.marks();
        let width = (ui.available_width() - 12.0).min(PAGE_WIDTH);
        let room = (ui.available_height() - FOOTER).max(160.0);
        if width >= RAIL_BESIDE {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = RAIL_GAP;
                ui.allocate_ui_with_layout(vec2(RAIL_W, room), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(RAIL_W);
                    self.rail(ui, &marks);
                });
                let form_w = width - RAIL_W - RAIL_GAP;
                ui.allocate_ui_with_layout(vec2(form_w, room), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(form_w);
                    self.form(ui, room, &mut acts);
                });
            });
        } else {
            // no room beside the form: the steps are a row of chips over it
            ui.scope(|ui| {
                ui.set_max_width(width);
                self.rail_chips(ui, &marks);
            });
            ui.add_space(10.0);
            let room = (ui.available_height() - FOOTER).max(160.0);
            ui.scope(|ui| {
                ui.set_max_width(width);
                self.form(ui, room, &mut acts);
            });
        }
        self.footer(ui, &mut acts);
        if self.screen == Screen::Setup && !matches!(self.look, Look::Nothing) {
            drop_hint(ui);
        }
        acts
    }

    /// How each step of the rail stands, the world first; the fifth is the conversion itself.
    fn marks(&mut self) -> [(Mark, String); 5] {
        let found = self.found().map(|d| d.name.clone());
        let world = match (&self.look, &found) {
            (_, Some(name)) => (Mark::Done, name.clone()),
            (Look::Busy, _) => (Mark::Busy, ellipsis(tr!("Looking at the world..."))),
            (Look::Unknown(_), _) => (Mark::Error, tr!("This can't be converted")),
            _ => (Mark::Todo, tr!("Choose a world")),
        };
        if found.is_none() {
            let locked = |text: String| (Mark::Locked, text);
            return [
                world,
                locked(tr!("Java Edition")),
                locked(tr!("Mobs, players and settings")),
                locked(if self.dest.is_some() { tr!("Into the instance") } else { tr!("Where it goes") }),
                locked(tr!("When the steps are ready")),
            ];
        }
        let dims = self.dims.iter().filter(|(_, on)| *on).count();
        let target = if self.version.is_empty() {
            (Mark::Warn, tr!("Choose a version"))
        } else if dims == 0 {
            (Mark::Warn, tr!("Choose a dimension"))
        } else {
            let what = crate::i18n::tr_n("{0} dimension", "{0} dimensions", dims as u64, &[dims.into()]);
            let mark =
                if self.dest.as_ref().is_some_and(|d| d.mc_version != self.version) { Mark::Warn } else { Mark::Done };
            (mark, format!("{} {} \u{00B7} {what}", Edition::Java.label(), self.version))
        };
        let changes = left_behind(&self.entities, &self.player_opts, self.make_owner, self.owner.is_some())
            + self.overrides_count();
        let keep = (
            Mark::Done,
            if changes == 0 {
                tr!("Everything comes along")
            } else {
                crate::i18n::tr_n("{0} change", "{0} changes", changes as u64, &[changes.into()])
            },
        );
        let check = if self.screen == Screen::Setup { self.output_check() } else { OutputCheck::Ok };
        let save_text = match (check, &self.dest) {
            (OutputCheck::Ok, Some(d)) => tr!("Into {0}", &d.name),
            (OutputCheck::Ok, None) => std::path::Path::new(self.output.trim())
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            (OutputCheck::Empty, _) => tr!("Choose where it goes"),
            (OutputCheck::NotEmpty, _) => tr!("That folder is in use"),
            (OutputCheck::NotAbsolute, _) => tr!("Write the whole path"),
            _ => tr!("Choose another place"),
        };
        let ready = self.blocked().is_none();
        let run = match self.screen {
            Screen::Setup if ready => (Mark::Todo, tr!("Ready when you are")),
            Screen::Setup => (Mark::Todo, tr!("When the steps are ready")),
            Screen::Running => (Mark::Busy, tr!("Converting")),
            Screen::Done => (Mark::Done, tr!("Done")),
            Screen::Failed => (Mark::Error, tr!("It didn't work")),
        };
        let mut marks = [world, target, keep, (output_mark(check), save_text), run];
        if self.screen != Screen::Setup {
            // the steps were ready when the conversion started: they stay ticked while it runs and after
            for m in marks.iter_mut().take(4) {
                m.0 = Mark::Done;
            }
        }
        marks
    }

    /// How many of the world's settings are forced rather than kept.
    fn overrides_count(&self) -> usize {
        let o = &self.overrides;
        [o.difficulty.is_some(), o.game_type.is_some(), o.allow_commands.is_some(), o.hardcore.is_some()]
            .into_iter()
            .filter(|c| *c)
            .count()
    }

    /// The titles of the steps.
    fn step_titles(&self) -> [String; 5] {
        [
            sentence(&tr!("BEDROCK WORLD")),
            sentence(&tr!("CONVERT TO")),
            sentence(&tr!("WHAT COMES ALONG")),
            if self.dest.is_some() { tr!("Add to the instance") } else { tr!("Save") },
            tr!("Convert"),
        ]
    }

    /// The steps down the left: a badge each (a check once the step is ready, a warning when it needs a look), the
    /// title and how it stands, joined by a line. A step that can be opened opens its form on the right.
    fn rail(&mut self, ui: &mut Ui, marks: &[(Mark, String); 5]) {
        let titles = self.step_titles();
        let setup = self.screen == Screen::Setup;
        let mut badges: Vec<(egui::Rect, Mark)> = Vec::new();
        ui.spacing_mut().item_spacing.y = 0.0;
        for (i, ((mark, line), title)) in marks.iter().zip(&titles).enumerate() {
            let current = setup && i == self.step;
            let openable = setup && i < 4 && *mark != Mark::Locked;
            let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), RAIL_ROW), Sense::click());
            let t = super::motion::hover(ui, resp.id, openable && resp.hovered());
            let s = super::motion::toward(ui.ctx(), resp.id.with("on"), current, super::motion::MEDIUM);
            let p = ui.painter();
            if t > 0.0 {
                p.rect_filled(row, CornerRadius::same(R_CONTROL), c_hover().gamma_multiply(t * (1.0 - s)));
            }
            if s > 0.0 {
                p.rect_filled(row, CornerRadius::same(R_CONTROL), c_accent_soft().gamma_multiply(s));
            }
            let badge = egui::Rect::from_center_size(
                egui::pos2(row.left() + 8.0 + STEP_BADGE / 2.0, row.top() + 10.0 + STEP_BADGE / 2.0),
                vec2(STEP_BADGE, STEP_BADGE),
            );
            paint_mark(ui, badge, *mark, i + 1, current, ui.scope_id().with(("convert-rail", i)));
            let x = badge.right() + 12.0;
            let width = row.right() - x - 6.0;
            let ink = if *mark == Mark::Locked { c_faint() } else { c_text() };
            let font = if current { super::fonts::semibold(FS_BODY) } else { super::fonts::medium(FS_BODY) };
            let title_g = one_line(ui, title, font, ink, width);
            ui.painter().galley(egui::pos2(x, row.top() + 10.0), title_g, ink);
            let sub = match mark {
                Mark::Warn => c_warn(),
                Mark::Error => c_danger(),
                Mark::Locked => c_faint(),
                _ => c_muted(),
            };
            let line_g = one_line(ui, line, egui::FontId::proportional(FS_META), sub, width);
            ui.painter().galley(egui::pos2(x, row.top() + 30.0), line_g, sub);
            if openable {
                focus_ring(ui, &resp, row, R_CONTROL);
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, current, title.as_str()));
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.step = i;
                }
            }
            badges.push((badge, *mark));
            ui.add_space(RAIL_STEP_GAP);
        }
        // the line between two badges: in the brand colour once the step above is ready
        for pair in badges.windows(2) {
            let ((a, mark), (b, _)) = (pair[0], pair[1]);
            let color = if mark == Mark::Done { c_accent().gamma_multiply(0.6) } else { c_border() };
            ui.painter().vline(a.center().x, a.bottom() + 4.0..=b.top() - 4.0, Stroke::new(2.0, color));
        }
    }

    /// The steps as a row of chips, for a narrow window.
    fn rail_chips(&mut self, ui: &mut Ui, marks: &[(Mark, String); 5]) {
        let titles = self.step_titles();
        let setup = self.screen == Screen::Setup;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (i, ((mark, _), title)) in marks.iter().zip(&titles).enumerate().take(4) {
                let label = format!(
                    "{}  {}",
                    if *mark == Mark::Done { "\u{2713}".to_string() } else { (i + 1).to_string() },
                    title
                );
                let r = chip(ui, &label, setup && i == self.step);
                if setup && *mark != Mark::Locked && r.clicked() {
                    self.step = i;
                }
            }
        });
    }

    /// The right side: the form of the open step in its cards, with Back, Next and Convert under it; or the
    /// conversion's progress, result or error.
    fn form(&mut self, ui: &mut Ui, room: f32, acts: &mut Vec<Act>) {
        // The buttons follow the last card. Only when the cards do not fit do they stay in view under the scrolling
        // part, with a hairline over them so the clipped edge of the cards shows.
        let has_bar = self.screen == Screen::Setup && self.found().is_some();
        let pinned = has_bar && self.bar_pinned;
        let height = if pinned { (room - START_BAR).max(120.0) } else { room };
        let mut needs_pin = false;
        let page = ScrollArea::vertical().id_salt("convert-page").max_height(height).auto_shrink([false, false]).show(
            ui,
            |ui| {
                ui.set_max_width(ui.available_width() - 12.0);
                ui.spacing_mut().item_spacing.y = 12.0;
                match self.screen {
                    Screen::Setup => match self.step {
                        0 => self.world_block(ui, acts),
                        1 => self.target_block(ui),
                        2 => self.keep_block(ui),
                        _ => self.output_block(ui, acts),
                    },
                    Screen::Running => self.running(ui),
                    Screen::Done => self.done(ui, acts),
                    Screen::Failed => self.failed(ui, acts),
                }
                if has_bar {
                    // the last card ends at the min rect; the layout adds its item spacing under it
                    needs_pin = ui.min_rect().height() + BAR_GAP + BUTTON_H > room;
                }
                if has_bar && !needs_pin {
                    ui.add_space(BAR_GAP - ui.spacing().item_spacing.y);
                    self.start_bar(ui, acts, false);
                } else {
                    ui.add_space(TAIL);
                }
            },
        );
        if has_bar && needs_pin != self.bar_pinned {
            // the page learned how it fits: draw it again at once with the buttons where they belong
            self.bar_pinned = needs_pin;
            ui.ctx().request_discard("the Convert button moved under or into the cards");
            ui.ctx().request_repaint();
        }
        if needs_pin {
            let y = page.inner_rect.bottom() + 0.5;
            ui.painter().hline(
                page.inner_rect.left()..=page.inner_rect.right() - 12.0,
                y,
                Stroke::new(1.0, c_border()),
            );
            self.start_bar(ui, acts, true);
        }
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

    /// Back and Next between the steps, and Convert (with why it can't be pressed yet, when that is so) at the
    /// right. `pinned`: it sits under the scrolling part, with a little more room above it.
    fn start_bar(&mut self, ui: &mut Ui, acts: &mut Vec<Act>, pinned: bool) {
        let blocked = self.blocked();
        if pinned {
            ui.add_space(4.0);
        }
        ui.horizontal(|ui| {
            // as wide as the cards over it (the scrolling part keeps a strip for its bar)
            ui.set_width(ui.available_width() - if pinned { 12.0 } else { 0.0 });
            if self.step > 0 && ghost(ui, &tr!("Back")).clicked() {
                self.step -= 1;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if primary_enabled_icon(ui, Icon::Convert, &tr!("Convert"), blocked.is_none()).clicked() {
                    acts.push(Act::Start);
                }
                if self.step < 3 && secondary(ui, &tr!("Next")).clicked() {
                    self.step += 1;
                }
                if let Some(why) = &blocked {
                    ui.add(egui::Label::new(RichText::new(why).color(c_faint()).size(FS_META)).truncate())
                        .on_hover_text(why);
                }
            });
        });
    }

    fn world_block(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
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
                    card_head(ui, &tr!("Select your Bedrock world"), "");
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 14.0;
                        world_badge(ui);
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
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ZONE_HEIGHT), Sense::hover());
        let dragging = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        // glass at rest; the hover fill fades in under the pointer, the brand tint while a file is dragged over
        let hot = super::motion::hover(ui, resp.id, ui.rect_contains_pointer(rect));
        let drag = super::motion::toward(ui.ctx(), resp.id.with("drag"), dragging, super::motion::FAST);
        let fill = super::motion::mix(super::motion::mix(c_card(), c_hover(), hot), c_accent_soft(), drag);
        let edge = super::motion::mix(c_control(), c_accent(), drag);
        ui.painter().add(card_shadow().as_shape(rect, CornerRadius::same(R_CARD)));
        ui.painter().rect_filled(rect, CornerRadius::same(R_CARD), fill);
        let outline = rounded_outline(rect.shrink(0.5), f32::from(R_CARD));
        ui.painter().extend(egui::Shape::dashed_line(&outline, Stroke::new(1.0 + drag, edge), 6.0, 4.0));
        let mut inner = ui.new_child(
            egui::UiBuilder::new().max_rect(rect.shrink2(vec2(16.0, 0.0))).layout(Layout::top_down(Align::Center)),
        );
        inner.spacing_mut().item_spacing.y = 4.0;
        inner.add_space(18.0);
        // the picture of the area: a soft badge that lights up in the brand gradient while a file is dragged over
        let (badge, _) = inner.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        inner.painter().rect_filled(badge, CornerRadius::same(R_CONTROL), c_accent_soft());
        if drag > 0.0 {
            super::motion::gradient_rect(
                inner.painter(),
                badge,
                R_CONTROL,
                c_accent().gamma_multiply(drag),
                c_accent2().gamma_multiply(drag),
            );
        }
        icons_arrow_down(inner.painter(), badge.center(), super::motion::mix(c_accent(), c_on_accent(), drag));
        inner.add_space(4.0);
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
        card(ui, |ui| {
            card_head(
                ui,
                &sentence(&tr!("CONVERT TO")),
                &tr!("The version of Java Edition the world is written for, and its dimensions"),
            );
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

    /// What comes along besides the blocks: the world's mobs and item frames, the players' things and who owns them,
    /// and the world's settings, a card each. Everything is kept by default.
    fn keep_block(&mut self, ui: &mut Ui) {
        card(ui, |ui| {
            // the title of a row sits close to its explanation; the rows are kept apart by hairlines
            ui.spacing_mut().item_spacing.y = 2.0;
            card_head(ui, &tr!("Mobs and things"), "");
            switch_row(
                ui,
                &tr!("Mobs, pets and villagers"),
                &tr!(
                    "Animals and monsters, tamed pets, villagers with their trades, armor stands, minecarts, boats and paintings"
                ),
                &mut self.entities.convert,
            );
            if self.entities.convert {
                row_gap(ui);
                option_row(ui, &tr!("Which mobs"), |ui| {
                    let label = |f: EntityFilter| match f {
                        EntityFilter::All => tr!("All of them"),
                        EntityFilter::NoHostile => tr!("No hostile mobs"),
                        EntityFilter::PetsAndNamedOnly => tr!("Only pets, named mobs and villagers"),
                        EntityFilter::None => tr!("None"),
                    };
                    egui::ComboBox::from_id_salt("convert-mobs")
                        .selected_text(label(self.entities.filter))
                        .width(250.0)
                        .show_ui(ui, |ui| {
                            for f in [EntityFilter::All, EntityFilter::NoHostile, EntityFilter::PetsAndNamedOnly] {
                                ui.selectable_value(&mut self.entities.filter, f, label(f));
                            }
                        });
                });
                row_gap(ui);
                switch_row(
                    ui,
                    &tr!("Item frames"),
                    &tr!("Item frames and glow item frames, with the item in them"),
                    &mut self.entities.item_frames,
                );
                row_gap(ui);
                switch_row(
                    ui,
                    &tr!("Leave out what lies around"),
                    &tr!("Dropped items, experience orbs, falling sand and arrows are not brought over"),
                    &mut self.entities.drop_transient,
                );
            }
        });
        card(ui, |ui| {
            // the title of a row sits close to its explanation; the rows are kept apart by hairlines
            ui.spacing_mut().item_spacing.y = 2.0;
            card_head(ui, &tr!("Players"), "");
            match self.owner.clone() {
                Some((name, _)) => switch_row(
                    ui,
                    &tr!("Make {0} the owner", &name),
                    &tr!("The host's character and pets become this account's, so you play on as them"),
                    &mut self.make_owner,
                ),
                None => {
                    option_row(ui, &tr!("Owner"), |ui| small_muted(ui, tr!("Add an account to become the owner")));
                }
            }
            row_gap(ui);
            switch_row(
                ui,
                &tr!("Ender chest"),
                &tr!("What the host kept in the ender chest"),
                &mut self.player_opts.include_ender_chest,
            );
            row_gap(ui);
            switch_row(
                ui,
                &tr!("Experience, health and food"),
                &tr!("Also effects and the bed or respawn anchor to wake up at"),
                &mut self.player_opts.include_stats,
            );
            let players = self.world_players().to_vec();
            if !players.is_empty() {
                row_gap(ui);
                switch_row(
                    ui,
                    &tr!("The other players"),
                    &tr!("Each player who joined the world gets their own player file with their things"),
                    &mut self.player_opts.export_guest_profiles,
                );
                row_gap(ui);
                self.inventory_choice(ui, &players);
            }
        });
        card(ui, |ui| {
            // the title of a row sits close to its explanation; the rows are kept apart by hairlines
            ui.spacing_mut().item_spacing.y = 2.0;
            card_head(ui, &tr!("World settings"), &tr!("Kept as the world had them unless you choose otherwise"));
            self.overrides_rows(ui);
        });
    }

    /// Whose inventory the owner starts with: the host's, another player's, or the host's with the other's items in
    /// the free slots.
    fn inventory_choice(&mut self, ui: &mut Ui, players: &[WorldPlayer]) {
        let describe = |p: &WorldPlayer| {
            let who = if p.signed_in { tr!("Signed-in player") } else { tr!("Player") };
            tr!(
                "{0} \u{00B7} level {1} \u{00B7} {2} items \u{00B7} at {3}, {4}, {5}",
                &who,
                p.level,
                p.items,
                p.position[0],
                p.position[1],
                p.position[2]
            )
        };
        let chosen = self.player_opts.inventory_source.profile_id().map(str::to_string);
        let current = chosen
            .as_deref()
            .and_then(|id| players.iter().find(|p| p.id.eq_ignore_ascii_case(id)))
            .map(describe)
            .unwrap_or_else(|| tr!("The host's"));
        let merge = matches!(self.player_opts.inventory_source, InventorySource::MergeHostAnd(_));
        option_row(ui, &tr!("Your inventory"), |ui| {
            egui::ComboBox::from_id_salt("convert-inventory").selected_text(current).width(320.0).show_ui(ui, |ui| {
                if ui.selectable_label(chosen.is_none(), tr!("The host's")).clicked() {
                    self.player_opts.inventory_source = InventorySource::Host;
                }
                for p in players {
                    let on = chosen.as_deref().is_some_and(|id| id.eq_ignore_ascii_case(&p.id));
                    if ui.selectable_label(on, describe(p)).clicked() {
                        self.player_opts.inventory_source = if merge {
                            InventorySource::MergeHostAnd(p.id.clone())
                        } else {
                            InventorySource::Profile(p.id.clone())
                        };
                    }
                }
            });
        });
        if let Some(id) = chosen {
            let mut keep_host = merge;
            if check_labeled(
                ui,
                &mut keep_host,
                &tr!("Keep the host's items too, with that player's in the free slots"),
            )
            .changed()
            {
                self.player_opts.inventory_source =
                    if keep_host { InventorySource::MergeHostAnd(id) } else { InventorySource::Profile(id) };
            }
        }
    }

    /// Difficulty, game mode, cheats and hardcore: kept as the world had them unless one is chosen.
    fn overrides_rows(&mut self, ui: &mut Ui) {
        let keep = tr!("As the world had it");
        option_row(ui, &tr!("Difficulty"), |ui| {
            let label = |d: Option<Difficulty>| match d {
                None => keep.clone(),
                Some(Difficulty::Peaceful) => tr!("Peaceful"),
                Some(Difficulty::Easy) => tr!("Easy"),
                Some(Difficulty::Normal) => tr!("Normal"),
                Some(Difficulty::Hard) => tr!("Hard"),
            };
            egui::ComboBox::from_id_salt("convert-difficulty")
                .selected_text(label(self.overrides.difficulty))
                .width(200.0)
                .show_ui(ui, |ui| {
                    for d in [
                        None,
                        Some(Difficulty::Peaceful),
                        Some(Difficulty::Easy),
                        Some(Difficulty::Normal),
                        Some(Difficulty::Hard),
                    ] {
                        ui.selectable_value(&mut self.overrides.difficulty, d, label(d));
                    }
                });
        });
        option_row(ui, &tr!("Game mode"), |ui| {
            let label = |g: Option<GameType>| match g {
                None => keep.clone(),
                Some(GameType::Survival) => tr!("Survival"),
                Some(GameType::Creative) => tr!("Creative"),
                Some(GameType::Adventure) => tr!("Adventure"),
                Some(GameType::Spectator) => tr!("Spectator"),
            };
            egui::ComboBox::from_id_salt("convert-gamemode")
                .selected_text(label(self.overrides.game_type))
                .width(200.0)
                .show_ui(ui, |ui| {
                    for g in [
                        None,
                        Some(GameType::Survival),
                        Some(GameType::Creative),
                        Some(GameType::Adventure),
                        Some(GameType::Spectator),
                    ] {
                        ui.selectable_value(&mut self.overrides.game_type, g, label(g));
                    }
                });
        });
        let on_off = |id: &str, title: String, value: &mut Option<bool>, ui: &mut Ui| {
            option_row(ui, &title, |ui| {
                let label = |v: Option<bool>| match v {
                    None => keep.clone(),
                    Some(true) => tr!("On"),
                    Some(false) => tr!("Off"),
                };
                egui::ComboBox::from_id_salt(id).selected_text(label(*value)).width(200.0).show_ui(ui, |ui| {
                    for v in [None, Some(true), Some(false)] {
                        ui.selectable_value(value, v, label(v));
                    }
                });
            });
        };
        on_off("convert-cheats", tr!("Cheats"), &mut self.overrides.allow_commands, ui);
        on_off("convert-hardcore", tr!("Hardcore"), &mut self.overrides.hardcore, ui);
    }

    /// "Put it in": an installed instance, where the world is added to the saves at once (converted for the
    /// instance's Minecraft version), or a folder of the player's choice. The choice made here is remembered.
    fn put_in_choice(&self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let label = |d: &Dest| tr!("{0} \u{00B7} Minecraft {1}", &d.name, &d.mc_version);
        let folder_text = tr!("A folder you choose");
        let current = self.dest.as_ref().map(label).unwrap_or_else(|| folder_text.clone());
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr!("Put it in")).color(c_muted()));
            egui::ComboBox::from_id_salt("convert-into")
                .selected_text(current)
                .width((ui.available_width() - 4.0).clamp(160.0, 360.0))
                .show_ui(ui, |ui| {
                    if ui.selectable_label(self.dest.is_none(), &folder_text).clicked() && self.dest.is_some() {
                        acts.push(Act::Into(None));
                    }
                    if !self.choices.is_empty() {
                        ui.separator();
                    }
                    for c in &self.choices {
                        let on = self.dest.as_ref().is_some_and(|d| d.folder == c.folder);
                        if ui.selectable_label(on, label(c)).clicked() && !on {
                            acts.push(Act::Into(Some(c.folder.clone())));
                        }
                    }
                })
                .response
                .on_hover_text(tr!(
                    "Choose an instance to add the converted world to its worlds straight away, ready to play"
                ));
        });
    }

    fn output_block(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let check = self.output_check();
        card(ui, |ui| {
            let title = if self.dest.is_some() { tr!("ADD TO THE INSTANCE AS") } else { tr!("SAVE AS") };
            card_head(ui, &sentence(&title), &tr!("A new folder: the original world is never changed"));
            self.put_in_choice(ui, acts);
            ui.add_space(6.0);
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
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                world_badge(ui);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.set_max_width((ui.available_width() - 90.0).max(120.0));
                    ui.add(egui::Label::new(RichText::new(&name).size(FS_SECTION).semibold()).truncate());
                    small_muted(ui, format!("{} {}", run.job.target.edition.label(), run.job.target.version));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // the figure that moves, large, in the brand colour
                    if run.progress > 0.0 {
                        ui.label(
                            RichText::new(format!("{:.0}%", run.progress * 100.0))
                                .font(super::fonts::semibold(FS_TITLE))
                                .color(c_accent()),
                        );
                    } else {
                        spinner_sized(ui, 24.0);
                    }
                });
            });
            ui.add_space(14.0);
            let stage = if run.stopping { ellipsis(tr!("Stopping...")) } else { run.stage.clone() };
            ui.add(egui::Label::new(RichText::new(stage).color(c_muted())).truncate());
            ui.add_space(4.0);
            progress_bar(ui, (run.progress > 0.0).then_some(run.progress), 10.0);
            ui.add_space(14.0);
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
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                result_badge(ui, true);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    match &dest {
                        Some(d) => {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(tr!("Your world was added to {0}", &d.name))
                                        .size(FS_FIGURE)
                                        .semibold(),
                                )
                                .wrap(),
                            );
                            small_muted(ui, tr!("It is in the instance's saves folder and shows up in Singleplayer."));
                        }
                        None => {
                            // one line: centred on the badge
                            ui.add_space(9.0);
                            ui.label(RichText::new(tr!("Your world was converted")).size(FS_FIGURE).semibold());
                        }
                    }
                });
            });
            ui.add_space(14.0);
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
                // put in an instance from the converter's own page: one click to it
                if let Some(d) = dest.as_ref().filter(|_| !self.via_instance) {
                    if secondary(ui, &tr!("Go to {0}", &d.name)).clicked() {
                        acts.push(Act::BackToInstance(d.folder.clone()));
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
        card(ui, |ui| {
            ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            result_badge(ui, false);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(
                    egui::Label::new(
                        RichText::new(sentence(&tr!("THE WORLD COULD NOT BE CONVERTED")))
                            .size(FS_FIGURE)
                            .semibold()
                            .color(c_danger()),
                    )
                    .wrap(),
                );
                small_muted(ui, tr!("The original world is not changed. \"Copy details\" copies what to send when you ask for help."));
            });
        });
            ui.add_space(12.0);
            notice(ui, NoticeKind::Error, &message, None);
            ui.add_space(6.0);
            egui::CollapsingHeader::new(RichText::new(tr!("Details")).color(c_muted()))
                .id_salt("convert-details")
                .show(ui, |ui| {
                    ScrollArea::vertical().max_height(160.0).auto_shrink([false, true]).show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(details).monospace().size(FS_META).color(c_muted())).wrap(),
                        );
                    });
                });
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

/// An arrow down into a tray: "drop it here".
fn icons_arrow_down(p: &egui::Painter, c: egui::Pos2, color: Color32) {
    let stroke = Stroke::new(1.8, color);
    let at = |x: f32, y: f32| c + vec2(x, y) * 9.0;
    p.line_segment([at(0.0, -0.95), at(0.0, 0.35)], stroke);
    p.add(egui::Shape::line(vec![at(-0.45, -0.1), at(0.0, 0.35), at(0.45, -0.1)], stroke));
    p.add(egui::Shape::line(vec![at(-0.95, 0.3), at(-0.95, 0.9), at(0.95, 0.9), at(0.95, 0.3)], stroke));
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
    use super::*;

    #[test]
    fn the_save_step_warns_what_can_be_fixed_and_stops_what_would_harm_the_world() {
        assert_eq!(output_mark(OutputCheck::Ok), Mark::Done);
        for c in [OutputCheck::Empty, OutputCheck::NotAbsolute, OutputCheck::NotEmpty] {
            assert_eq!(output_mark(c), Mark::Warn, "{c:?}");
        }
        for c in [OutputCheck::IsSource, OutputCheck::InsideSource, OutputCheck::IsFile, OutputCheck::NotWritable] {
            assert_eq!(output_mark(c), Mark::Error, "{c:?}");
        }
    }

    #[test]
    fn what_comes_along_counts_what_was_changed_from_everything() {
        let (e, p) = (EntityOptions::default(), PlayerOptions::default());
        assert_eq!(left_behind(&e, &p, true, true), 0, "everything comes along by default");
        // no account to own the world is not a change the player made
        assert_eq!(left_behind(&e, &p, false, false), 0);
        assert_eq!(left_behind(&e, &p, false, true), 1);
        // no mobs at all is one change, whatever the mob options under it say
        let none = EntityOptions { convert: false, item_frames: false, ..EntityOptions::default() };
        assert_eq!(left_behind(&none, &p, true, true), 1);
        let fewer = PlayerOptions { include_ender_chest: false, include_stats: false, ..PlayerOptions::default() };
        assert_eq!(left_behind(&e, &fewer, true, true), 2);
    }

    #[test]
    fn a_closing_three_dots_becomes_one_ellipsis() {
        assert_eq!(ellipsis("Choose...".to_string()), "Choose\u{2026}");
        assert_eq!(ellipsis("Change".to_string()), "Change");
        assert_eq!(ellipsis("A... b".to_string()), "A... b");
    }
}
