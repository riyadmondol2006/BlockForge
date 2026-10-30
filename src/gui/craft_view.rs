//! The right column of the Crafting page: the plan, in four tabs, and the windows for saved plans.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Align, Layout, RichText, Ui, vec2};

use super::craft::{Act, Book, CraftState, Dialog, Env, LEFT_W, Load, PlanView, TWO_COLUMNS, Tab};
use super::craft_fmt::*;
use super::services::{confirm_modal, modal_scroll_height, modal_width};
use super::theme::*;
use super::widgets::*;
use crate::craft::saved;
use crate::craft::{self, ItemId, Kind, Node, Recipe, Stop, Storage};

/// How wide the plan gets on a wide window: past this the amounts are too far from their names.
pub(super) const PLAN_WIDTH: f32 = 860.0;
/// The width of the card with the recipe's grid, beside the item at the head of the plan.
const RECIPE_CARD_W: f32 = 228.0;
/// The least room for the item and the recipe's grid side by side; with less the grid goes under the item.
const GRID_BESIDE: f32 = 540.0;
/// The height of the band at the head of the plan: a full 3 x 3 grid of the recipe and a little room above and below
/// it, so the plan's answer starts high on the page.
const BAND_H: f32 = 112.0;
/// The side of a slot of the recipe's grid, and the room between two.
const SLOT: f32 = 32.0;
const SLOT_GAP: f32 = 4.0;
/// The height of the line of the plan's state, totals and actions over its tabs.
const TOOLBAR_H: f32 = 34.0;
/// With a plan at least this wide, Saved plans and Export are buttons of their own; narrower, they are in a menu.
const TOOLS_APART: f32 = 760.0;
/// How far each level of the tree is indented, and the height of its rows.
const TREE_INDENT: f32 = 26.0;
const TREE_ROW_H: f32 = 40.0;
/// How far the lines of a step are indented: past its tick box and its station, under its title.
const STEP_INDENT: f32 = 80.0;

impl CraftState {
    /// Both columns (or one under the other when the window is narrow).
    pub(super) fn body(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let Some(book) = self.book.clone() else { return };
        if self.load != Load::Ready {
            return;
        }
        let total_w = ui.available_width();
        let h = ui.available_height();
        if total_w >= TWO_COLUMNS {
            let right_w = total_w - LEFT_W - 16.0;
            // the list takes the height of the page, less its search and the line under it
            let list_h = (h - 118.0).max(220.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                ui.allocate_ui_with_layout(vec2(LEFT_W, h), Layout::top_down(Align::Min), |ui| {
                    egui::ScrollArea::vertical().id_salt("craft-left").auto_shrink([false, false]).show(ui, |ui| {
                        ui.set_max_width(ui.available_width() - 12.0);
                        ui.spacing_mut().item_spacing.y = 12.0;
                        self.left_pick(ui, &book, list_h);
                        self.left_supplied(ui, &book);
                        ui.add_space(8.0);
                    });
                });
                ui.allocate_ui_with_layout(vec2(right_w, h), Layout::top_down(Align::Min), |ui| {
                    egui::ScrollArea::vertical().id_salt("craft-right").auto_shrink([false, false]).show(ui, |ui| {
                        ui.set_max_width((ui.available_width() - 12.0).min(PLAN_WIDTH));
                        ui.spacing_mut().item_spacing.y = 12.0;
                        ui.spacing_mut().item_spacing.y = 8.0;
                        self.item_band(ui, &book, false);
                        self.right(ui, &book, acts, false);
                        ui.add_space(8.0);
                    });
                });
            });
        } else {
            egui::ScrollArea::vertical().id_salt("craft-one").auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(ui.available_width() - 12.0);
                ui.spacing_mut().item_spacing.y = 12.0;
                let picked = self.target.is_some() && !self.picking;
                if picked {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    self.item_band(ui, &book, true);
                } else {
                    self.left_pick(ui, &book, 260.0);
                }
                // the line of the plan's state and actions joins its tabs here, so the answer starts as high
                self.right(ui, &book, acts, picked);
                ui.spacing_mut().item_spacing.y = 12.0;
                self.left_supplied(ui, &book);
                ui.add_space(8.0);
            });
        }
    }

    /// The head of the plan, one low band: the item and how many of it on the left, the recipe that makes it on the
    /// right (under it when there is no room beside it). `change` offers to pick another item (the one-column page
    /// folds the list away once something is picked). Nothing until an item is picked.
    fn item_band(&mut self, ui: &mut Ui, book: &Arc<Book>, change: bool) {
        let Some(target) = self.target.clone() else { return };
        if ui.available_width() < GRID_BESIDE {
            let w = ui.available_width();
            band_card(ui, w, egui::Margin::symmetric(12, 10), |ui| self.quantity_block(ui, book, change));
            band_card(ui, w, egui::Margin::symmetric(12, 4), |ui| self.recipe_card(ui, book, &target));
            return;
        }
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let left_w = ui.available_width() - RECIPE_CARD_W - 12.0;
            band_card(ui, left_w, egui::Margin::symmetric(12, 10), |ui| self.quantity_block(ui, book, change));
            band_card(ui, RECIPE_CARD_W, egui::Margin::symmetric(12, 4), |ui| self.recipe_card(ui, book, &target));
        });
    }

    /// The recipe that makes the picked item, as the game shows it: the grid at the left, and beside it the station
    /// over the arrow and the result, with how many other ways there are under them.
    fn recipe_card(&mut self, ui: &mut Ui, book: &Arc<Book>, target: &str) {
        // the recipe the plan uses (the player's choice, else the default), with the ingredients it picked
        let plan = self.plan.as_ref().and_then(|p| p.as_ref().ok()).cloned();
        let recipe_id = plan
            .as_ref()
            .and_then(|v| v.plan.tree.recipe.clone())
            .or_else(|| self.recipe_choices.get(target).cloned())
            .or_else(|| book.ds.recipes_for(target).next().map(|r| r.id.clone()));
        let recipe = recipe_id.as_deref().and_then(|id| book.ds.recipe(id)).cloned();
        let choices: Vec<(String, ItemId)> = plan
            .as_ref()
            .and_then(|v| recipe_id.as_ref().and_then(|id| v.plan.steps.iter().find(|s| &s.recipe == id)))
            .map(|s| s.choices.clone())
            .unwrap_or_default();
        let ways = book.ds.recipes_for(target).count();
        match recipe {
            None => {
                ui.add_space(8.0);
                card_title(ui, "Recipe");
                small_muted(ui, "Gathered, not made: there is no recipe for it.");
            }
            Some(r) => self.paint_grid(ui, book, &recipe_grid(&r, &choices), &r, ways),
        }
    }

    /// The slots of a recipe, an arrow and the slot of what it makes, with the counts in the corners like the game;
    /// the station over the arrow and, when there are other recipes, "1 of n" under it. Takes the whole height of
    /// the band.
    fn paint_grid(&mut self, ui: &mut Ui, book: &Arc<Book>, grid: &Grid, r: &Recipe, ways: usize) {
        let rows = grid.cells.len().div_ceil(grid.cols.max(1));
        let grid_w = grid.cols as f32 * SLOT + (grid.cols as f32 - 1.0) * SLOT_GAP;
        let grid_h = rows as f32 * SLOT + (rows as f32 - 1.0) * SLOT_GAP;
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        // the grid's column is as wide as a full grid, so the arrow does not move between recipes
        let full = 3.0 * SLOT + 2.0 * SLOT_GAP;
        let left = rect.left() + ((full - grid_w) / 2.0).floor();
        let top = rect.center().y - grid_h / 2.0;
        for (k, cell) in grid.cells.iter().enumerate() {
            let (col, row) = ((k % grid.cols) as f32, (k / grid.cols) as f32);
            let at = egui::Rect::from_min_size(
                egui::pos2(left + col * (SLOT + SLOT_GAP), top + row * (SLOT + SLOT_GAP)),
                vec2(SLOT, SLOT),
            );
            match cell {
                Some((id, n)) => {
                    let name = book.ds.name(id);
                    self.paint_slot(ui, at, id, &name);
                    if *n > 1 {
                        corner_count(ui, at, *n as u64);
                    }
                    ui.interact(at, ui.scope_id().with(("craft-grid", k)), egui::Sense::hover()).on_hover_text(name);
                }
                None => {
                    ui.painter().rect(
                        at,
                        egui::CornerRadius::same(R_TAG + 2),
                        c_field().gamma_multiply(0.6),
                        egui::Stroke::new(1.0, c_rule()),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }
        // beside the grid: the station over the arrow and the result, the other ways under them
        let side = egui::Rect::from_min_max(egui::pos2(rect.left() + full + 10.0, rect.top()), rect.max);
        let mid = rect.center().y;
        let station = egui::text::LayoutJob::simple_singleline(
            r.kind.station().to_string(),
            super::fonts::medium(FS_META),
            c_muted(),
        );
        let mut station = station;
        station.wrap = egui::text::TextWrapping::truncate_at_width(side.width());
        let galley = ui.painter().layout_job(station);
        ui.painter().galley(egui::pos2(side.left(), side.top() + 4.0), galley, c_muted());
        let ax = side.left();
        let stroke = egui::Stroke::new(1.8, c_muted());
        ui.painter().line_segment([egui::pos2(ax, mid), egui::pos2(ax + 20.0, mid)], stroke);
        ui.painter().line_segment([egui::pos2(ax + 14.0, mid - 5.0), egui::pos2(ax + 20.0, mid)], stroke);
        ui.painter().line_segment([egui::pos2(ax + 14.0, mid + 5.0), egui::pos2(ax + 20.0, mid)], stroke);
        let out = egui::Rect::from_min_size(egui::pos2(ax + 30.0, mid - SLOT / 2.0), vec2(SLOT, SLOT));
        let name = book.ds.name(&r.output);
        // the result's slot is lit in the brand tint
        ui.painter().rect(
            out.expand(3.0),
            egui::CornerRadius::same(R_CONTROL + 1),
            c_accent_soft(),
            egui::Stroke::new(1.0, c_accent().gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
        self.paint_slot(ui, out, &r.output, &name);
        if r.count > 1 {
            corner_count(ui, out, u64::from(r.count));
        }
        ui.interact(out, ui.scope_id().with("craft-grid-out"), egui::Sense::hover()).on_hover_text(format!(
            "{} {} \u{00B7} {}",
            r.count,
            name,
            r.kind.station()
        ));
        if ways > 1 {
            let text = format!("1 of {ways}");
            let galley = ui.painter().layout_no_wrap(text, super::fonts::mono(FS_META), c_faint());
            let at = egui::Rect::from_min_size(
                egui::pos2(side.left(), side.bottom() - galley.size().y - 4.0),
                galley.size(),
            );
            ui.painter().galley(at.min, galley, c_faint());
            ui.interact(at, ui.scope_id().with("craft-grid-ways"), egui::Sense::hover())
                .on_hover_text("There are other ways to make it: choose one in the Steps or Tree tab");
        }
    }

    /// The plan under the band: its state and actions, its tabs and the open tab. `merged` puts the state and the
    /// actions on the line of the tabs (the one-column page, where height is short).
    fn right(&mut self, ui: &mut Ui, book: &Arc<Book>, acts: &mut Vec<Act>, merged: bool) {
        match self.plan.clone() {
            None => {
                let bad = self.target.is_some() && parse_qty(&self.qty_text).is_err();
                ui.add_space(64.0);
                ui.vertical_centered(|ui| {
                    // a soft badge with the crafting grid over the sentence, in the warning tint when the amount is wrong
                    let (rect, _) = ui.allocate_exact_size(vec2(56.0, 56.0), egui::Sense::hover());
                    let (fill, fg) = if bad { (c_warn_bg(), c_warn()) } else { (c_accent_soft(), c_accent()) };
                    ui.painter().rect_filled(rect, egui::CornerRadius::same(R_CARD), fill);
                    super::icons::paint(ui.painter(), super::icons::Icon::Craft, rect.center(), 24.0, fg);
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(if bad { "Check the amount" } else { "Pick something to build" })
                            .size(FS_SECTION)
                            .semibold(),
                    );
                    ui.add_space(2.0);
                    ui.scope(|ui| {
                        ui.set_max_width(380.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(if bad {
                                    "The plan appears as soon as the amount is a whole number."
                                } else {
                                    "Search for an item on the left. The materials, the steps and the storage it needs show up here."
                                })
                                .color(c_muted()),
                            )
                            .wrap(),
                        );
                    });
                });
            }
            Some(Err(e)) => {
                let mut reset = false;
                note_box(ui, NoticeKind::Error, |ui| {
                    ui.label(RichText::new("This can't be worked out").semibold().color(c_danger()));
                    ui.label(e);
                    // a recipe picked earlier can be what stands in the way: the way back is here too
                    if !self.recipe_choices.is_empty() || !self.ingredient_choices.is_empty() {
                        ui.add_space(4.0);
                        reset = secondary(ui, "Reset my recipe choices").clicked();
                    }
                });
                if reset {
                    self.recipe_choices.clear();
                    self.ingredient_choices.clear();
                    self.touch_plan();
                }
            }
            Some(Ok(view)) => self.result(ui, book, &view, acts, merged),
        }
    }

    fn result(&mut self, ui: &mut Ui, book: &Arc<Book>, v: &Arc<PlanView>, acts: &mut Vec<Act>, merged: bool) {
        if !merged {
            self.toolbar(ui, book, v, acts);
        }
        if !v.plan.warnings.is_empty() || !v.plan.complete {
            note_box(ui, NoticeKind::Warn, |ui| {
                ui.label(RichText::new("Worth a look").semibold().color(c_warn()));
                for w in &v.plan.warnings {
                    ui.label(w);
                }
                if v.plan.warnings.is_empty() {
                    ui.label("Some branches stop early, so the lists below are not the whole story.");
                }
            });
        }
        let mut pill_shown = !merged;
        tab_bar(ui, |ui| {
            for (tab_id, label) in
                [(Tab::Materials, "Materials"), (Tab::Steps, "Steps"), (Tab::Tree, "Tree"), (Tab::Storage, "Storage")]
            {
                if tab(ui, label, self.tab == tab_id).clicked() {
                    self.tab = tab_id;
                }
            }
            if merged {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    self.plan_actions(ui, book, v, acts, false);
                    let (good, text) = plan_state(v);
                    if ui.available_width() >= status_width(ui, text) + 12.0 {
                        status_text(ui, good, text);
                        pill_shown = true;
                    }
                });
            }
        });
        // in a small window the totals give their line to the answer: the number to gather is on the shopping list's
        // head, and the steps, operations and storage have tabs of their own
        if merged && !pill_shown {
            ui.horizontal(|ui| {
                let (good, text) = plan_state(v);
                status_text(ui, good, text);
            });
        }
        ui.spacing_mut().item_spacing.y = 12.0;
        match self.tab {
            Tab::Materials => self.tab_materials(ui, book, v, acts),
            Tab::Steps => self.tab_steps(ui, book, v),
            Tab::Tree => self.tab_tree(ui, book, v),
            Tab::Storage => self.tab_storage(ui, book, v),
        }
    }

    /// One line over the tabs: how the plan stands, its totals (those that fit) and what can be done with it. (The
    /// item, the amount and what your items change are in the band above.)
    fn toolbar(&mut self, ui: &mut Ui, book: &Arc<Book>, v: &Arc<PlanView>, acts: &mut Vec<Act>) {
        let apart = ui.available_width() >= TOOLS_APART;
        let w = ui.available_width();
        ui.allocate_ui_with_layout(vec2(w, TOOLBAR_H), Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_size(vec2(w, TOOLBAR_H));
            ui.spacing_mut().item_spacing.x = 12.0;
            let (good, text) = plan_state(v);
            status_text(ui, good, text);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                self.plan_actions(ui, book, v, acts, apart);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| self.figures(ui, v));
            });
        });
    }

    /// Save, and the plans saved before and the ways to export: buttons of their own when `apart`, else in a menu.
    /// Laid out right to left.
    fn plan_actions(&mut self, ui: &mut Ui, book: &Arc<Book>, v: &Arc<PlanView>, acts: &mut Vec<Act>, apart: bool) {
        let plan = &v.plan;
        let name = book.ds.name(&plan.request.item);
        let n = self.store.plans.len();
        let saved_label = if n > 0 { format!("Saved plans ({n})") } else { "Saved plans".to_string() };
        if apart {
            let export = secondary(ui, "Export");
            egui::Popup::menu(&export).show(|ui| {
                ui.set_min_width(200.0);
                self.export_items(ui, book, plan, acts);
            });
            if secondary(ui, &saved_label).clicked() {
                self.dialog = Some(Dialog::Saved);
            }
        } else {
            let more = more_button(ui);
            egui::Popup::menu(&more).show(|ui| {
                ui.set_min_width(220.0);
                if ui.button(format!("{saved_label}\u{2026}")).clicked() {
                    self.dialog = Some(Dialog::Saved);
                    ui.close();
                }
                ui.separator();
                small_faint(ui, "Export");
                self.export_items(ui, book, plan, acts);
            });
        }
        let label = if self.active.is_some() { "Update plan" } else { "Save plan" };
        let mut save = primary(ui, label);
        if let Some(p) = self.active.and_then(|id| self.store.get(id)) {
            let note =
                format!("Saved as \u{201C}{}\u{201D}{}", p.name, if self.unsaved { ", changed since" } else { "" });
            save = save.on_hover_text(note);
        }
        if save.clicked() {
            let fallback = format!("{} \u{00D7} {}", thousands(plan.request.quantity), name);
            let current = self.active.and_then(|id| self.store.get(id)).map(|p| p.name.clone());
            self.dialog = Some(Dialog::Save(current.unwrap_or(fallback)));
        }
    }

    /// The entries of the Export menu: the shopping list to copy, the plan as a file, and the way back from the
    /// player's recipe choices.
    fn export_items(&mut self, ui: &mut Ui, book: &Arc<Book>, plan: &crate::craft::Plan, acts: &mut Vec<Act>) {
        let name = book.ds.name(&plan.request.item);
        let slug = file_name(&name, plan.request.quantity);
        if ui.button("Copy the shopping list").clicked() {
            acts.push(Act::Copy(saved::clipboard_list(&book.ds, plan)));
            ui.close();
        }
        ui.separator();
        if ui.button("Text file (.txt)").clicked() {
            acts.push(Act::Export { name: format!("{slug}.txt"), content: saved::to_text(&book.ds, plan), ext: "txt" });
            ui.close();
        }
        if ui
            .button("JSON data (.json)")
            .on_hover_text(
                "For spreadsheets and scripts. To open this plan again later, save it, then use Saved plans > Export.",
            )
            .clicked()
        {
            acts.push(Act::Export {
                name: format!("{slug}.json"),
                content: saved::to_json(&book.ds, plan),
                ext: "json",
            });
            ui.close();
        }
        if !self.recipe_choices.is_empty() || !self.ingredient_choices.is_empty() {
            ui.separator();
            if ui.button("Reset my recipe choices").clicked() {
                self.recipe_choices.clear();
                self.ingredient_choices.clear();
                self.touch_plan();
                ui.close();
            }
        }
    }

    /// The plan's totals as figures in the monospace face, as many as fit on the line, the most useful first; all of
    /// them are under the pointer. The name of an open saved plan comes first.
    fn figures(&mut self, ui: &mut Ui, v: &Arc<PlanView>) {
        let plan = &v.plan;
        let gather: u64 = plan.materials.iter().map(|(_, n)| *n).sum();
        let mut items: Vec<(String, String)> = Vec::new();
        if let Some(p) = self.active.and_then(|id| self.store.get(id)) {
            let note = format!(
                "Saved as \u{201C}{}\u{201D}{}",
                clip(&p.name, 20),
                if self.unsaved { ", changed" } else { "" }
            );
            items.push((String::new(), note));
        }
        for (n, many, one) in [
            (gather, "items to gather", "item to gather"),
            (plan.steps.len() as u64, "steps", "step"),
            (plan.operations, "operations", "operation"),
            (v.storage[0].slots, "storage slots", "storage slot"),
        ] {
            items.push((thousands(n), (if n == 1 { one } else { many }).to_string()));
        }
        const GAP: f32 = 14.0;
        let widths: Vec<f32> = items
            .iter()
            .map(|(n, label)| {
                let a = if n.is_empty() {
                    0.0
                } else {
                    ui.painter().layout_no_wrap(n.clone(), super::fonts::mono(FS_BODY), c_text()).size().x + 5.0
                };
                a + ui.painter().layout_no_wrap(label.clone(), egui::FontId::proportional(FS_META), c_muted()).size().x
            })
            .collect();
        let shown = fit_count(&widths, GAP, ui.available_width() - 4.0);
        let all =
            items.iter().map(|(n, l)| if n.is_empty() { l.clone() } else { format!("{n} {l}") }).collect::<Vec<_>>();
        let r = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for (n, label) in items.iter().take(shown) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        if !n.is_empty() {
                            ui.label(RichText::new(n).font(super::fonts::mono(FS_BODY)).color(c_text()));
                        }
                        ui.label(RichText::new(label).color(c_muted()).size(FS_META));
                    });
                }
            })
            .response;
        if shown > 0 {
            ui.interact(r.rect, ui.scope_id().with("craft-figures"), egui::Sense::hover())
                .on_hover_text(all.join("\n"));
        }
    }

    // ----------------------------------------------------------------------------------------------------
    // Materials

    fn tab_materials(&mut self, ui: &mut Ui, book: &Arc<Book>, v: &Arc<PlanView>, acts: &mut Vec<Act>) {
        let plan = &v.plan;
        // the shopping list: what to gather
        panel(ui, |ui| {
            let total = plan.materials.len();
            let ticked = plan.materials.iter().filter(|(i, _)| self.done.contains(&format!("gather:{i}"))).count();
            // one line: the title, how many kinds and how many in all, how far the ticking got, and Copy list; a thin
            // bar under it shows the ticking too
            let gather: u64 = plan.materials.iter().map(|(_, n)| *n).sum();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                card_title(ui, "To gather");
                let kinds = if total == 1 { "1 kind".to_string() } else { format!("{total} kinds") };
                ui.label(RichText::new(format!("({kinds} \u{00B7} {} in all)", thousands(gather))).color(c_muted()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if total > 0 && secondary(ui, "Copy list").on_hover_text("Copy the shopping list as text").clicked()
                    {
                        acts.push(Act::Copy(saved::clipboard_list(&book.ds, plan)));
                    }
                    if total > 0 {
                        small_muted(ui, format!("{ticked} of {total} ticked"));
                    }
                });
            });
            if total == 0 {
                muted(ui, "Nothing to gather: everything comes from your items or from what you bring.");
                return;
            }
            progress_bar(ui, Some(ticked as f32 / total as f32), 3.0);
            ui.add_space(4.0);
            for (k, (id, qty)) in plan.materials.iter().enumerate() {
                let key = format!("gather:{id}");
                let mut on = self.done.contains(&key);
                let before = on;
                let supplied = self.supplied.contains(id);
                let mut unbring = false;
                // what the player's own items already cover, right on the row, so the shorter list explains itself
                let from_stock = plan.ledger.iter().find(|r| &r.item == id).map(|r| r.from_stock).unwrap_or(0);
                self.list_row(ui, book, id, *qty, Some(&mut on), before, k + 1 == total, |ui| {
                    if supplied {
                        if x_button(ui, "No longer bringing this") {
                            unbring = true;
                        }
                        small_faint(ui, "you bring");
                    }
                    if from_stock > 0 {
                        small_faint(ui, format!("+{} from your items", thousands(from_stock)));
                    }
                });
                if on != before {
                    if on {
                        self.done.insert(key);
                    } else {
                        self.done.remove(&key);
                    }
                    self.done_changed();
                }
                if unbring {
                    self.supplied.remove(id);
                    self.touch_plan();
                }
            }
        });
        // what comes out of the player's own items: why the lists are shorter than the recipe says
        let owned: Vec<(&str, u64)> =
            plan.ledger.iter().filter(|r| r.from_stock > 0).map(|r| (r.item.as_str(), r.from_stock)).collect();
        if !owned.is_empty() {
            panel(ui, |ui| {
                ui.horizontal(|ui| {
                    card_title(ui, "Taken from your items");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if secondary(ui, "Edit").on_hover_text("Edit the items you already have").clicked() {
                            self.dialog = Some(Dialog::Stock);
                        }
                    });
                });
                small_muted(ui, "Already in your chests, so not gathered or made again.");
                ui.add_space(4.0);
                let total = owned.len();
                for (k, (id, used)) in owned.into_iter().enumerate() {
                    let have = self.stock().and_then(|s| s.get(id)).copied().unwrap_or(used);
                    self.list_row(ui, book, id, used, None::<&mut bool>, false, k + 1 == total, |ui| {
                        small_faint(ui, format!("you have {}", thousands(have)));
                    });
                }
            });
        }
        // what is made on the way
        if !plan.intermediates.is_empty() {
            panel(ui, |ui| {
                card_title(ui, "Made on the way");
                small_muted(ui, "Crafted or smelted from the list above. Bring one yourself to skip making it.");
                ui.add_space(4.0);
                let total = plan.intermediates.len();
                for (k, (id, qty)) in plan.intermediates.iter().enumerate() {
                    let mut bring = false;
                    self.list_row(ui, book, id, *qty, None::<&mut bool>, false, k + 1 == total, |ui| {
                        let r = secondary(ui, "I'll bring this");
                        if r.on_hover_text("Skip making it: it moves to the list of things to gather").clicked() {
                            bring = true;
                        }
                    });
                    if bring {
                        self.supplied.insert(id.clone());
                        self.touch_plan();
                    }
                }
            });
        }
        // what is left
        let spare: Vec<(&str, u64)> =
            plan.ledger.iter().filter(|r| r.surplus > 0).map(|r| (r.item.as_str(), r.surplus)).collect();
        if !spare.is_empty() {
            panel(ui, |ui| {
                card_title(ui, "Left over");
                small_muted(ui, "Made by whole batches, so a little more than needed.");
                ui.add_space(4.0);
                let total = spare.len();
                for (k, (id, qty)) in spare.into_iter().enumerate() {
                    self.list_row(ui, book, id, qty, None::<&mut bool>, false, k + 1 == total, |_| {});
                }
            });
        }
        // fuel
        if !plan.fuel.is_empty() {
            panel(ui, |ui| {
                card_title(ui, "Fuel for the furnaces");
                ui.label(
                    RichText::new(format!(
                        "The smelting takes about {} in one furnace. That burns about one of these:",
                        ticks_text(plan.cook_ticks)
                    ))
                    .color(c_muted()),
                );
                ui.add_space(4.0);
                let total = plan.fuel.len();
                for (k, f) in plan.fuel.iter().enumerate() {
                    self.list_row(ui, book, &f.item, f.count, None::<&mut bool>, false, k + 1 == total, |_| {});
                }
                small_muted(
                    ui,
                    "An estimate for one furnace running the whole time. It is not added to the list above.",
                );
            });
        }
        // containers that come back
        let mut back: BTreeMap<&str, u64> = BTreeMap::new();
        for s in &plan.steps {
            for (id, n) in &s.byproducts {
                *back.entry(id.as_str()).or_default() += n;
            }
        }
        if !back.is_empty() {
            panel(ui, |ui| {
                card_title(ui, "Containers that come back");
                small_muted(ui, "Empty buckets and bottles you get back while crafting. They are not counted above.");
                ui.add_space(4.0);
                let total = back.len();
                for (k, (id, n)) in back.into_iter().enumerate() {
                    self.list_row(ui, book, id, n, None::<&mut bool>, false, k + 1 == total, |_| {});
                }
            });
        }
    }

    /// One line of a list of items: [tick box] picture, name, and on the right the amount over its stacks; `tail`
    /// sits before the amount. A hairline divides it from the next line unless it is the `last`.
    fn list_row(
        &mut self,
        ui: &mut Ui,
        book: &Arc<Book>,
        id: &str,
        qty: u64,
        check: Option<&mut bool>,
        struck: bool,
        last: bool,
        tail: impl FnOnce(&mut Ui),
    ) {
        let name = book.ds.name(id);
        let sub = stack_note(qty, book.ds.stack(id));
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.horizontal(|ui| {
                ui.set_min_height(48.0);
                ui.spacing_mut().item_spacing.x = 10.0;
                if let Some(on) = check {
                    check_box(ui, on, true, &name);
                }
                self.slot_icon(ui, id, &name, 36.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.allocate_ui_with_layout(vec2(104.0, 38.0), Layout::top_down(Align::Max), |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        if sub.is_empty() {
                            // one line: centre it on the row
                            ui.add_space(9.0);
                        }
                        let color = if struck { c_muted() } else { c_text() };
                        ui.label(RichText::new(thousands(qty)).font(super::fonts::mono(FS_BODY)).color(color));
                        if !sub.is_empty() {
                            small_muted(ui, sub.as_str());
                        }
                    });
                    tail(ui);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let mut t = RichText::new(&name);
                        if struck {
                            t = t.strikethrough().color(c_muted());
                        }
                        ui.add(egui::Label::new(t).truncate());
                    });
                });
            });
            if !last {
                rule(ui);
            }
        });
    }

    // ----------------------------------------------------------------------------------------------------
    // Steps

    fn tab_steps(&mut self, ui: &mut Ui, book: &Arc<Book>, v: &Arc<PlanView>) {
        let plan = &v.plan;
        if plan.steps.is_empty() {
            panel(ui, |ui| {
                card_title(ui, "Steps");
                muted(ui, "Nothing to craft: everything is gathered or comes from your items.");
            });
            return;
        }
        let n = plan.steps.len();
        let ticked = plan.steps.iter().filter(|s| self.done.contains(&format!("step:{}", s.recipe))).count();
        panel(ui, |ui| {
            ui.horizontal(|ui| {
                card_title(ui, "Steps in order");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    small_muted(ui, format!("{ticked} of {n} done"))
                });
            });
            small_muted(ui, "Each step needs the ones that make its ingredients to be ticked first.");
            ui.add_space(2.0);
            progress_bar(ui, Some(ticked as f32 / n as f32), 4.0);
            for i in 0..n {
                ui.add_space(10.0);
                rule(ui);
                ui.add_space(10.0);
                let step = &v.plan.steps[i];
                let next = !self.done.contains(&format!("step:{}", step.recipe))
                    && v.deps[i].iter().all(|&j| self.done.contains(&format!("step:{}", v.plan.steps[j].recipe)));
                let rect = ui.scope(|ui| self.step_card(ui, book, v, i)).response.rect;
                // a step that can be done now has a short bar of the brand gradient in the card's margin beside it
                let t = super::motion::toward(
                    ui.ctx(),
                    ui.scope_id().with(("craft-step-next", i)),
                    next,
                    super::motion::MEDIUM,
                );
                if t > 0.0 {
                    let bar = egui::Rect::from_min_max(
                        egui::pos2(rect.left() - 12.0, rect.top() + 2.0),
                        egui::pos2(rect.left() - 9.0, rect.bottom() - 2.0),
                    );
                    super::motion::gradient_rect(
                        ui.painter(),
                        bar,
                        2,
                        c_accent().gamma_multiply(t),
                        c_accent2().gamma_multiply(t),
                    );
                }
            }
        });
    }

    /// One step of the plan, drawn straight into the list of steps.
    fn step_card(&mut self, ui: &mut Ui, book: &Arc<Book>, v: &Arc<PlanView>, i: usize) {
        let step = &v.plan.steps[i];
        let key = format!("step:{}", step.recipe);
        let ticked = self.done.contains(&key);
        let ready = v.deps[i].iter().all(|&j| self.done.contains(&format!("step:{}", v.plan.steps[j].recipe)));
        let recipe = book.ds.recipe(&step.recipe);
        let out_name = book.ds.name(&step.item);
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.horizontal(|ui| {
            // the tick box (24), the station (36) and the gaps add up to the indent of the lines under the title
            ui.spacing_mut().item_spacing.x = 10.0;
            let mut on = ticked;
            let r = check_box(ui, &mut on, ready || ticked, &format!("Step {}", i + 1));
            if !ready && !ticked {
                r.on_hover_text("Tick the steps that make its ingredients first");
            }
            if on != ticked {
                if on {
                    self.done.insert(key.clone());
                } else {
                    // taking a step back takes back everything built on it
                    let mut stack = vec![i];
                    while let Some(k) = stack.pop() {
                        if self.done.remove(&format!("step:{}", v.plan.steps[k].recipe)) {
                            stack.extend(v.dependants[k].iter().copied());
                        }
                    }
                }
                self.done_changed();
            }
            let station = station_item(step.kind);
            self.slot_icon(ui, station, step.kind.station(), 36.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let title = RichText::new(format!("{} {} \u{00D7}", step.kind.verb(), thousands(step.operations)))
                    .font(super::fonts::medium(FS_BODY));
                ui.horizontal(|ui| {
                    ui.label(if ticked { title.color(c_muted()) } else { title });
                    ui.label(RichText::new(format!("at {}", step.kind.station())).color(c_muted()));
                });
                small_muted(ui, format!("Makes {} {}", thousands(step.output), out_name));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                small_muted(ui, format!("Step {}", i + 1));
            });
        });
        // what goes in and what comes out, lined up under the name
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
            ui.add_space(STEP_INDENT);
            for (k, (id, qty)) in step.inputs.iter().enumerate() {
                if k > 0 {
                    ui.label(RichText::new("+").color(c_muted()));
                }
                let name = book.ds.name(id);
                self.icon(ui, id, &name, 24.0);
                ui.label(format!("{} {}", thousands(*qty), name));
            }
            arrow(ui);
            self.icon(ui, &step.item, &out_name, 24.0);
            ui.label(RichText::new(format!("{} {}", thousands(step.output), out_name)).semibold());
        });
        // notes
        let has_notes = recipe.is_some_and(|r| r.small_grid && matches!(r.kind, Kind::Shaped | Kind::Shapeless))
            || step.ticks.is_some()
            || recipe.and_then(|r| r.manual.as_ref()).is_some();
        if has_notes {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.add_space(STEP_INDENT);
                if recipe.is_some_and(|r| r.small_grid && matches!(r.kind, Kind::Shaped | Kind::Shapeless)) {
                    small_faint(ui, "Fits the inventory grid");
                }
                if let Some(t) = step.ticks {
                    small_muted(ui, format!("About {} in one {}", ticks_text(t), step.kind.station().to_lowercase()));
                }
                if let Some(m) = recipe.and_then(|r| r.manual.as_ref()) {
                    ui.label(RichText::new(m).color(c_warn()).size(FS_META));
                }
            });
        }
        if !step.byproducts.is_empty() {
            let text: Vec<String> =
                step.byproducts.iter().map(|(id, n)| format!("{} {}", thousands(*n), book.ds.name(id))).collect();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.add_space(STEP_INDENT);
                small_muted(ui, format!("You get back: {}", text.join(", ")));
            });
        }
        // the choices
        let mut any = false;
        if book.ds.recipes_for(&step.item).nth(1).is_some() {
            any = true;
        }
        if let Some(r) = recipe {
            if r.ingredients.iter().any(|g| g.alternatives.len() > 1) {
                any = true;
            }
        }
        if any {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                ui.add_space(STEP_INDENT);
                self.recipe_combo(ui, book, &step.item, &step.recipe, i as u64);
                if let Some(r) = recipe {
                    for (key, current) in &step.choices {
                        let Some(g) = r.ingredients.iter().find(|g| &g.key == key) else { continue };
                        if g.alternatives.len() < 2 {
                            continue;
                        }
                        small_muted(ui, "Use");
                        self.alternative_combo(ui, book, &r.id, key, current, &g.alternatives);
                    }
                }
            });
        }
    }

    /// A drop-down that picks how an item is made, when the game has more than one way.
    fn recipe_combo(&mut self, ui: &mut Ui, book: &Arc<Book>, item: &str, current: &str, salt: u64) {
        let list: Vec<&Recipe> = book.ds.recipes_for(item).collect();
        if list.len() < 2 {
            return;
        }
        let shown =
            list.iter().find(|r| r.id == current).map_or_else(|| "Recipe".to_string(), |r| recipe_label(&book.ds, r));
        let mut pick: Option<String> = None;
        let width = ui.available_width().clamp(120.0, 300.0);
        egui::ComboBox::from_id_salt(("craft-recipe", item, salt))
            .selected_text(clip(&shown, ((width - 44.0) / 6.5) as usize))
            .width(width)
            .show_ui(ui, |ui| {
                ui.set_min_width(340.0);
                for r in &list {
                    if ui.selectable_label(r.id == current, recipe_label(&book.ds, r)).clicked() && r.id != current {
                        pick = Some(r.id.clone());
                    }
                }
            })
            .response
            .on_hover_text("Choose how this is made");
        if let Some(r) = pick {
            self.choose_recipe(item, &r);
        }
    }

    /// A drop-down that picks which of several items an ingredient slot uses (any plank, any wool).
    fn alternative_combo(
        &mut self,
        ui: &mut Ui,
        book: &Arc<Book>,
        recipe: &str,
        key: &str,
        current: &str,
        alternatives: &[String],
    ) {
        let mut pick: Option<String> = None;
        egui::ComboBox::from_id_salt(("craft-alt", recipe, key))
            .selected_text(clip(&book.ds.name(current), 26))
            .width(190.0)
            .show_ui(ui, |ui| {
                ui.set_min_width(210.0);
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    for a in alternatives {
                        if ui.selectable_label(a == current, book.ds.name(a)).clicked() && a != current {
                            pick = Some(a.clone());
                        }
                    }
                });
            })
            .response
            .on_hover_text("Any of these works for this slot. Choose the one you have.");
        if let Some(a) = pick {
            self.choose_ingredient(recipe, key, &a);
        }
    }

    // ----------------------------------------------------------------------------------------------------
    // Tree

    fn tab_tree(&mut self, ui: &mut Ui, book: &Arc<Book>, v: &Arc<PlanView>) {
        panel(ui, |ui| {
            ui.horizontal(|ui| {
                card_title(ui, "What it is made of");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if v.nodes <= 400 {
                        if ghost(ui, "Collapse all").clicked() {
                            self.tree_epoch += 1;
                            self.tree_all = false;
                        }
                        if ghost(ui, "Expand all").clicked() {
                            self.tree_epoch += 1;
                            self.tree_all = true;
                        }
                    }
                });
            });
            small_muted(ui, "Each item over what goes into it. Open one to follow it down to what you gather.");
            ui.add_space(6.0);
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                self.tree_node(ui, book, &v.plan.tree, 1, 0);
            });
        });
    }

    /// One item of the tree and, when it is open, the items under it, joined to it by guide lines. Returns where its
    /// line starts and the height of its middle, for the arm from the item above.
    fn tree_node(&mut self, ui: &mut Ui, book: &Arc<Book>, node: &Node, path: u64, depth: usize) -> (f32, f32) {
        let id = ui.make_persistent_id(("craft-tree", self.tree_epoch, path));
        let has_children = !node.children.is_empty();
        let default_open = depth == 0 || (depth <= 1 && node.children.len() <= 6) || self.tree_all;
        let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(default_open);
        let indent = depth as f32 * TREE_INDENT;
        let (start, toggled, slot) = self.tree_row(ui, book, node, path, indent, has_children, open);
        if toggled {
            open = !open;
            ui.data_mut(|d| d.insert_temp(id, open));
        }
        if has_children && open {
            let mut children = Vec::new();
            for (i, c) in node.children.iter().enumerate() {
                children.push(self.tree_node(ui, book, c, path.wrapping_mul(31).wrapping_add(i as u64 + 1), depth + 1));
            }
            // a line down from this item's slot, with a short arm to each item under it (to its fold arrow, when it
            // has one)
            let x = slot.center().x;
            let stroke = egui::Stroke::new(1.0, c_control().gamma_multiply(0.55));
            if let Some(&(_, last_y)) = children.last() {
                ui.painter().vline(x, slot.bottom() + 2.0..=last_y, stroke);
            }
            for &(start, y) in &children {
                ui.painter().hline(x..=start - 3.0, y, stroke);
            }
        }
        (start, slot.center().y)
    }

    /// The line of one item in the tree: the fold arrow, the slot, the name and how many are needed, how it is
    /// come by, and at the right how it is made. Returns where it starts (its arrow, or its slot when it has none),
    /// whether the arrow was pressed, and the slot.
    #[allow(clippy::too_many_arguments)]
    fn tree_row(
        &mut self,
        ui: &mut Ui,
        book: &Arc<Book>,
        n: &Node,
        path: u64,
        indent: f32,
        has_children: bool,
        open: bool,
    ) -> (f32, bool, egui::Rect) {
        let name = book.ds.name(&n.item);
        let mut toggled = false;
        let mut slot = egui::Rect::NOTHING;
        let mut start = 0.0;
        ui.horizontal(|ui| {
            ui.set_min_height(TREE_ROW_H);
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add_space(indent);
            // the fold arrow, or room for it
            let (arrow, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::click());
            start = arrow.left();
            if has_children {
                let t = super::motion::hover(ui, resp.id, resp.hovered());
                let fg = super::motion::mix(c_muted(), c_text(), t);
                let c = arrow.center();
                let a = if open { std::f32::consts::FRAC_PI_2 } else { 0.0 };
                let rot = |x: f32, y: f32| c + vec2(x * a.cos() - y * a.sin(), x * a.sin() + y * a.cos());
                ui.painter().add(egui::Shape::line(
                    vec![rot(-2.0, -4.0), rot(2.5, 0.0), rot(-2.0, 4.0)],
                    egui::Stroke::new(1.6, fg),
                ));
                focus_ring(ui, &resp, arrow, R_TAG);
                let label = if open { "Fold" } else { "Open" };
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::DisclosureTriangle, true, open, label));
                toggled = resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
            }
            let (rect, _) = ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::hover());
            self.paint_slot(ui, rect, &n.item, &name);
            slot = rect;
            if !has_children {
                start = rect.left();
            }
            ui.label(RichText::new(&name).font(super::fonts::medium(FS_BODY)));
            ui.label(
                RichText::new(format!("\u{00D7}{}", thousands(n.required)))
                    .font(super::fonts::mono(FS_META))
                    .color(c_muted()),
            );
            match n.stop {
                Some(Stop::Inventory) => tag(ui, "from your items", c_accent()),
                Some(Stop::Raw) => tag(ui, "gather", c_secondary()),
                Some(Stop::Supplied) => tag(ui, "you bring", c_secondary()),
                Some(Stop::NoRecipe) => tag(ui, "no recipe", c_warn()),
                Some(Stop::Manual) => tag(ui, "by hand", c_warn()),
                Some(Stop::Cycle) => tag(ui, "cycle", c_danger()),
                None => {}
            }
            if n.from_stock > 0 && n.stop != Some(Stop::Inventory) {
                small_muted(ui, format!("{} of yours", thousands(n.from_stock)));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(r) = &n.recipe {
                    self.recipe_combo(ui, book, &n.item, r, path);
                }
                if n.operations > 0 {
                    ui.label(
                        RichText::new(format!(
                            "{} op{}",
                            thousands(n.operations),
                            if n.operations == 1 { "" } else { "s" }
                        ))
                        .font(super::fonts::mono(FS_META))
                        .color(c_faint()),
                    );
                }
            });
        });
        (start, toggled, slot)
    }

    // ----------------------------------------------------------------------------------------------------
    // Storage

    fn tab_storage(&mut self, ui: &mut Ui, book: &Arc<Book>, v: &Arc<PlanView>) {
        small_muted(
            ui,
            "Three separate estimates, not storage you need all at once: pick the one that matches how you work.",
        );
        let parts = [
            ("The things to gather", "What you carry home before you start crafting."),
            ("The finished product", "Only what you asked for, once it is made."),
            ("What is made on the way", "The in-between items, if you keep them all."),
        ];
        for (k, (title, blurb)) in parts.into_iter().enumerate() {
            self.storage_card(ui, book, title, blurb, &v.storage[k]);
        }
    }

    fn storage_card(&mut self, ui: &mut Ui, book: &Arc<Book>, title: &str, blurb: &str, s: &Storage) {
        panel(ui, |ui| {
            ui.horizontal(|ui| {
                card_title(ui, title);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    small_muted(ui, format!("{} slot{}", thousands(s.slots), if s.slots == 1 { "" } else { "s" }));
                });
            });
            small_muted(ui, blurb);
            if s.rows.is_empty() {
                ui.add_space(2.0);
                muted(ui, "Nothing to store here.");
                return;
            }
            ui.add_space(4.0);
            facts_rows(
                ui,
                &[
                    ("Single chests", thousands(s.single_chests.count)),
                    ("Double chests", thousands(s.double_chests.count)),
                    ("Shulker boxes", thousands(s.shulker_boxes.count)),
                    ("Inventories", thousands(s.player_inventories.count)),
                ],
            );
            small_muted(ui, "A chest holds 27 stacks, a double chest 54, a shulker box 27, an inventory 36.");
            ui.add_space(6.0);
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let total = s.rows.len();
                for (k, r) in s.rows.iter().enumerate() {
                    let name = book.ds.name(&r.item);
                    let text = if r.stack <= 1 {
                        format!("{} slot{}", thousands(r.slots), if r.slots == 1 { "" } else { "s" })
                    } else if r.partial > 0 {
                        format!("{} full + {} spare, {} slots", thousands(r.full_stacks), r.partial, thousands(r.slots))
                    } else {
                        format!("{} full stacks, {} slots", thousands(r.full_stacks), thousands(r.slots))
                    };
                    ui.horizontal(|ui| {
                        ui.set_min_height(44.0);
                        self.slot_icon(ui, &r.item, &name, 32.0);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let (dr, _) = ui.allocate_exact_size(vec2(190.0, 20.0), egui::Sense::hover());
                            ui.painter().text(
                                dr.right_center(),
                                egui::Align2::RIGHT_CENTER,
                                text,
                                egui::FontId::proportional(FS_META),
                                c_muted(),
                            );
                            let (qr, _) = ui.allocate_exact_size(vec2(72.0, 20.0), egui::Sense::hover());
                            ui.painter().text(
                                qr.right_center(),
                                egui::Align2::RIGHT_CENTER,
                                thousands(r.quantity),
                                super::fonts::medium(FS_BODY),
                                c_text(),
                            );
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.add(egui::Label::new(name.as_str()).truncate());
                            });
                        });
                    });
                    if k + 1 < total {
                        rule(ui);
                    }
                }
            });
        });
    }

    // ----------------------------------------------------------------------------------------------------
    // Windows

    pub(super) fn dialogs(&mut self, ctx: &egui::Context, env: &Env, acts: &mut Vec<Act>) {
        let Some(d) = self.dialog.take() else { return };
        let keep = match d {
            Dialog::GetVersion => self.get_dialog(ctx, env, acts).then_some(Dialog::GetVersion),
            Dialog::Saved => self.saved_dialog(ctx, acts).then_some(Dialog::Saved),
            Dialog::Stock => self.stock_dialog(ctx).then_some(Dialog::Stock),
            Dialog::Save(mut name) => self.save_dialog(ctx, &mut name).then_some(Dialog::Save(name)),
            Dialog::Rename(id, mut name) => self.rename_dialog(ctx, id, &mut name).then_some(Dialog::Rename(id, name)),
            Dialog::Delete(id) => {
                let label = self.store.get(id).map(|p| p.name.clone()).unwrap_or_default();
                match confirm_modal(
                    ctx,
                    "craft-delete",
                    "Delete this plan?",
                    &format!("\"{label}\" is removed from your saved plans. This can't be undone."),
                    "Delete",
                    true,
                ) {
                    Some(true) => {
                        self.store.delete(id);
                        if self.active == Some(id) {
                            self.active = None;
                            self.active_sig = None;
                            self.unsaved = false;
                        }
                        self.touch_store();
                        Some(Dialog::Saved)
                    }
                    Some(false) => Some(Dialog::Saved),
                    None => Some(Dialog::Delete(id)),
                }
            }
            Dialog::ClearStock => {
                match confirm_modal(
                    ctx,
                    "craft-clear",
                    "Clear your items?",
                    "Everything in \"Your items\" for this version is removed.",
                    "Clear",
                    true,
                ) {
                    Some(true) => {
                        self.clear_stock();
                        self.stock_bufs.clear();
                        self.touch_store();
                        self.touch_plan();
                        Some(Dialog::Stock)
                    }
                    Some(false) => Some(Dialog::Stock),
                    None => Some(Dialog::ClearStock),
                }
            }
        };
        if self.dialog.is_none() {
            self.dialog = keep;
        }
    }

    fn get_dialog(&mut self, ctx: &egui::Context, env: &Env, acts: &mut Vec<Act>) -> bool {
        let mut open = true;
        let m = egui::Modal::new(egui::Id::unique("craft-get")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(modal_width(ctx, 460.0));
            ui.label(RichText::new("Get another version").size(FS_SECTION).semibold());
            ui.add_space(2.0);
            muted(ui, "Only the game file is downloaded, to read its recipes. You can play it later or never.");
            ui.add_space(8.0);
            self.get_versions(ui, env, acts, modal_scroll_height(ctx, 340.0, 170.0));
            ui.add_space(10.0);
            if secondary(ui, "Close").clicked() {
                open = false;
            }
        });
        open && !m.should_close()
    }

    fn save_dialog(&mut self, ctx: &egui::Context, name: &mut String) -> bool {
        let mut open = true;
        let mut action: Option<bool> = None;
        let existing = self.active.is_some();
        let m = egui::Modal::new(egui::Id::unique("craft-save")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(modal_width(ctx, 420.0));
            ui.label(
                RichText::new(if existing { "Update your saved plan" } else { "Save this plan" })
                    .size(FS_SECTION)
                    .semibold(),
            );
            ui.add_space(2.0);
            muted(ui, "It keeps the item, the amount, your choices and what you ticked.");
            ui.add_space(8.0);
            let r = ui.add_sized(
                [ui.available_width(), 36.0],
                egui::TextEdit::singleline(name)
                    .hint_text("A name for the plan")
                    .margin(egui::Margin::symmetric(12, 8))
                    .background_color(c_field()),
            );
            focus_when_shown(ui, &r);
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ok = !name.trim().is_empty();
                if primary_enabled(ui, if existing { "Update" } else { "Save" }, ok).clicked() || (enter && ok) {
                    action = Some(!existing);
                }
                if existing && secondary(ui, "Save as new").clicked() && ok {
                    action = Some(true);
                }
                if secondary(ui, "Cancel").clicked() {
                    open = false;
                }
            });
        });
        if let Some(as_new) = action {
            self.save_plan(name, as_new);
            return false;
        }
        open && !m.should_close()
    }

    fn rename_dialog(&mut self, ctx: &egui::Context, id: u64, name: &mut String) -> bool {
        let mut open = true;
        let mut done = false;
        let m = egui::Modal::new(egui::Id::unique("craft-rename")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(modal_width(ctx, 420.0));
            ui.label(RichText::new("Rename the plan").size(FS_SECTION).semibold());
            ui.add_space(8.0);
            let r = ui.add_sized(
                [ui.available_width(), 36.0],
                egui::TextEdit::singleline(name).margin(egui::Margin::symmetric(12, 8)).background_color(c_field()),
            );
            focus_when_shown(ui, &r);
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ok = !name.trim().is_empty();
                if primary_enabled(ui, "Rename", ok).clicked() || (enter && ok) {
                    done = true;
                }
                if secondary(ui, "Cancel").clicked() {
                    open = false;
                }
            });
        });
        if done {
            self.store.rename(id, name, saved::now_ms());
            self.touch_store();
            return false;
        }
        open && !m.should_close()
    }

    /// "Your items": what the player already has, which the plan takes off what to gather and make. It opens from
    /// the line under the amount, so it is reached without scrolling the item list. Returns whether it stays open.
    fn stock_dialog(&mut self, ctx: &egui::Context) -> bool {
        let Some(book) = self.book.clone() else { return false };
        let mut open = true;
        let m = egui::Modal::new(egui::Id::unique("craft-stock")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(modal_width(ctx, 420.0));
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(RichText::new("Your items").size(FS_SECTION).semibold());
            egui::ScrollArea::vertical()
                .max_height(modal_scroll_height(ctx, 420.0, 200.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_max_width(ui.available_width() - 8.0);
                    self.stock_editor(ui, &book);
                });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if primary(ui, "Done").clicked() {
                    open = false;
                }
                let count = self.stock().map_or(0, |s| s.len());
                if count > 0 && ghost(ui, "Clear all").clicked() {
                    self.dialog = Some(Dialog::ClearStock);
                }
            });
        });
        open && !m.should_close()
    }

    fn saved_dialog(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) -> bool {
        enum Row {
            Open(usize),
            Rename(usize),
            Copy(usize),
            Export(usize),
            Delete(usize),
        }
        let mut open = true;
        let mut row: Option<Row> = None;
        let mut import = false;
        let book = self.book.clone();
        let m = egui::Modal::new(egui::Id::unique("craft-saved")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(modal_width(ctx, 600.0));
            ui.label(RichText::new("Saved plans").size(FS_SECTION).semibold());
            ui.add_space(2.0);
            muted(ui, "Plans live on this computer. Open one to carry on where you stopped.");
            ui.add_space(12.0);
            if self.store.plans.is_empty() {
                ui.add_space(8.0);
                muted(ui, "No saved plans yet. Make a plan, then press Save plan.");
                ui.add_space(8.0);
            } else {
                egui::ScrollArea::vertical()
                    .max_height(modal_scroll_height(ctx, 380.0, 190.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let mut order: Vec<usize> = (0..self.store.plans.len()).collect();
                        order.sort_by_key(|&k| std::cmp::Reverse(self.store.plans[k].updated_ms));
                        for k in order {
                            let p = &self.store.plans[k];
                            let item = book
                                .as_ref()
                                .filter(|b| b.ds.version == p.version)
                                .map_or_else(|| craft::pretty_name(&p.request.item), |b| b.ds.name(&p.request.item));
                            let active = self.active == Some(p.id);
                            ui.horizontal(|ui| {
                                ui.set_min_height(52.0);
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    kebab_menu(ui, |ui| {
                                        if ui.button("Rename").clicked() {
                                            row = Some(Row::Rename(k));
                                            ui.close();
                                        }
                                        if ui.button("Duplicate").clicked() {
                                            row = Some(Row::Copy(k));
                                            ui.close();
                                        }
                                        if ui.button("Export to a file").clicked() {
                                            row = Some(Row::Export(k));
                                            ui.close();
                                        }
                                        if ui.button(RichText::new("Delete").color(c_danger())).clicked() {
                                            row = Some(Row::Delete(k));
                                            ui.close();
                                        }
                                    });
                                    if secondary(ui, "Open").clicked() {
                                        row = Some(Row::Open(k));
                                    }
                                    if active {
                                        small_muted(ui, "Open now");
                                    }
                                    ui.vertical(|ui| {
                                        ui.set_width(ui.available_width());
                                        ui.spacing_mut().item_spacing.y = 1.0;
                                        ui.add(egui::Label::new(RichText::new(&p.name).semibold()).truncate());
                                        small_muted(
                                            ui,
                                            format!(
                                                "{} \u{00D7} {} \u{00B7} Minecraft {} \u{00B7} {}",
                                                thousands(p.request.quantity),
                                                item,
                                                p.version,
                                                changed_text(p.updated_ms)
                                            ),
                                        );
                                    });
                                });
                            });
                            rule(ui);
                        }
                    });
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if secondary(ui, "Import a plan file").clicked() {
                    import = true;
                }
                if secondary(ui, "Close").clicked() {
                    open = false;
                }
            });
        });
        if import {
            acts.push(Act::ImportPlan);
        }
        if let Some(r) = row {
            match r {
                Row::Open(k) => {
                    if let Some(p) = self.store.plans.get(k).cloned() {
                        self.open_plan(p, acts);
                        return false;
                    }
                }
                Row::Rename(k) => {
                    if let Some(p) = self.store.plans.get(k) {
                        self.dialog = Some(Dialog::Rename(p.id, p.name.clone()));
                    }
                }
                Row::Copy(k) => {
                    if let Some(id) = self.store.plans.get(k).map(|p| p.id) {
                        self.store.duplicate(id, saved::now_ms());
                        self.touch_store();
                    }
                }
                Row::Export(k) => {
                    if let Some(p) = self.store.plans.get(k) {
                        acts.push(Act::Export {
                            name: format!("{}.json", file_name(&p.name, 0)),
                            content: saved::export_plan(p),
                            ext: "json",
                        });
                    }
                }
                Row::Delete(k) => {
                    if let Some(p) = self.store.plans.get(k) {
                        self.dialog = Some(Dialog::Delete(p.id));
                    }
                }
            }
        }
        open && !m.should_close()
    }
}

/// How an item's recipe reads in a list: "Crafting table: 2 Wheat + 1 Cocoa Beans → 8 Cookie".
fn recipe_label(ds: &craft::Dataset, r: &Recipe) -> String {
    let inputs: Vec<String> = r
        .ingredients
        .iter()
        .map(|g| {
            let first = g.alternatives.first().map(|a| ds.name(a)).unwrap_or_default();
            if g.alternatives.len() > 1 {
                // "Oak Planks", "Birch Planks" -> "any Planks"
                let last = |n: &str| n.rsplit(' ').next().unwrap_or("").to_string();
                let word = last(&first);
                if g.alternatives.iter().all(|a| last(&ds.name(a)) == word) {
                    format!("{} any {}", g.count, word)
                } else {
                    format!("{} {} or similar", g.count, first)
                }
            } else {
                format!("{} {}", g.count, first)
            }
        })
        .collect();
    let inputs = if inputs.is_empty() { "by hand".to_string() } else { inputs.join(" + ") };
    format!("{}: {} \u{2192} {} {}", r.kind.station(), inputs, r.count, ds.name(&r.output))
}

/// The block or item that stands for the station of a step.
fn station_item(kind: Kind) -> &'static str {
    match kind {
        Kind::Shaped | Kind::Shapeless | Kind::Transmute => "minecraft:crafting_table",
        Kind::Smelting => "minecraft:furnace",
        Kind::Blasting => "minecraft:blast_furnace",
        Kind::Smoking => "minecraft:smoker",
        Kind::Campfire => "minecraft:campfire",
        Kind::Stonecutting => "minecraft:stonecutter",
        Kind::Smithing => "minecraft:smithing_table",
    }
}

/// A recipe's slots as the game shows them: `cols` across, each empty or holding an item and how many of it.
#[derive(Debug, PartialEq)]
pub(super) struct Grid {
    pub cols: usize,
    pub cells: Vec<Option<(ItemId, u32)>>,
}

/// The slots of `r`, with the item the plan chose for each ingredient (`chosen`, by key; the first alternative for
/// one it doesn't name). Crafting fills the 3 by 3 grid: a shaped recipe as its pattern lays it out, in the middle,
/// a shapeless one slot by slot. A furnace and a stonecutter have one slot, smithing three in a row, and a recipe
/// that puts something on an item two.
pub(super) fn recipe_grid(r: &Recipe, chosen: &[(String, ItemId)]) -> Grid {
    let item_of = |g: &craft::Ingredient| {
        chosen
            .iter()
            .find(|(k, _)| *k == g.key)
            .map(|(_, i)| i.clone())
            .or_else(|| g.alternatives.first().cloned())
            .unwrap_or_default()
    };
    match r.kind {
        Kind::Shaped if !r.shape.is_empty() => {
            let mut cells = vec![None; 9];
            let h = r.shape.len().min(3);
            let w = r.shape.iter().map(|row| row.chars().count()).max().unwrap_or(0).min(3);
            let (top, left) = ((3 - h) / 2, (3 - w) / 2);
            for (y, row) in r.shape.iter().take(3).enumerate() {
                for (x, c) in row.chars().take(3).enumerate() {
                    if let Some(g) = r.ingredients.iter().find(|g| g.key.chars().eq(std::iter::once(c))) {
                        cells[(top + y) * 3 + left + x] = Some((item_of(g), 1));
                    }
                }
            }
            Grid { cols: 3, cells }
        }
        Kind::Shaped | Kind::Shapeless => {
            let mut cells: Vec<Option<(ItemId, u32)>> = r
                .ingredients
                .iter()
                .flat_map(|g| std::iter::repeat_n(Some((item_of(g), 1)), g.count as usize))
                .take(9)
                .collect();
            cells.resize(9, None);
            Grid { cols: 3, cells }
        }
        Kind::Smithing => Grid { cols: 3, cells: r.ingredients.iter().map(|g| Some((item_of(g), g.count))).collect() },
        Kind::Transmute => Grid { cols: 2, cells: r.ingredients.iter().map(|g| Some((item_of(g), g.count))).collect() },
        _ => Grid { cols: 1, cells: r.ingredients.iter().take(1).map(|g| Some((item_of(g), g.count))).collect() },
    }
}

/// How many of an item a slot holds, in the slot's lower right corner like the game shows it.
fn corner_count(ui: &Ui, slot: egui::Rect, n: u64) {
    let text = thousands(n);
    let font = super::fonts::mono(FS_META);
    let at = slot.right_bottom() - vec2(4.0, 2.0);
    // a soft dark edge keeps the number readable over a light picture
    ui.painter().text(
        at + vec2(1.0, 1.0),
        egui::Align2::RIGHT_BOTTOM,
        &text,
        font.clone(),
        egui::Color32::from_black_alpha(160),
    );
    ui.painter().text(at, egui::Align2::RIGHT_BOTTOM, text, font, egui::Color32::WHITE);
}

/// A file name for a plan: "Diamond Pickaxe" and 64 -> "diamond-pickaxe-64".
fn file_name(name: &str, quantity: u64) -> String {
    let mut s: String = name.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-').to_string();
    let s = if s.is_empty() { "crafting-plan".to_string() } else { s };
    if quantity > 0 { format!("{s}-{quantity}") } else { s }
}

/// How the plan stands, for its pill: whether every recipe on the way was found. It says nothing of the player's
/// own progress, which the ticks of the lists keep.
fn plan_state(v: &PlanView) -> (bool, &'static str) {
    let good = v.plan.complete && v.plan.warnings.is_empty();
    (good, if good { "All recipes found" } else { "Needs a look" })
}

/// The width [`status_text`] takes for `text`.
fn status_width(ui: &Ui, text: &str) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(FS_META), c_text()).size().x + 30.0
}

/// A square button with three dots that opens a menu, as tall as the buttons beside it.
fn more_button(ui: &mut Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), egui::Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let t = super::motion::hover(ui, resp.id, resp.hovered() || open);
    let fill = super::motion::mix(c_field(), c_field_hover(), t);
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(R_CONTROL),
        fill,
        egui::Stroke::new(1.0, c_border()),
        egui::StrokeKind::Inside,
    );
    super::icons::paint(
        ui.painter(),
        super::icons::Icon::Kebab,
        rect.center(),
        16.0,
        super::motion::mix(c_muted(), c_text(), t),
    );
    focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "More"));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Saved plans and export")
}

/// A card of the band at the head of the plan: `w` wide and exactly [`BAND_H`] tall, with `margin` inside.
fn band_card(ui: &mut Ui, w: f32, margin: egui::Margin, add: impl FnOnce(&mut Ui)) {
    let out = card_frame().inner_margin(margin).show(ui, |ui| {
        let inner = vec2(w - margin.sum().x - 2.0, BAND_H - margin.sum().y - 2.0);
        ui.set_min_size(inner);
        ui.set_max_size(inner);
        // its lines go down the card, whatever way the band lays out the cards
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            // the lines are as tall as what is in them, not the page's taller rows of controls
            ui.spacing_mut().interact_size.y = 16.0;
            add(ui);
        });
    });
    glass_edge(ui.painter(), out.response.rect, R_CARD);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ing(key: &str, count: u32, items: &[&str]) -> craft::Ingredient {
        craft::Ingredient { key: key.into(), count, alternatives: items.iter().map(|i| i.to_string()).collect() }
    }

    fn rec(kind: Kind, shape: &[&str], ingredients: Vec<craft::Ingredient>) -> Recipe {
        Recipe {
            id: "r".into(),
            kind,
            output: "out".into(),
            count: 1,
            ingredients,
            manual: None,
            ticks: None,
            small_grid: false,
            shape: shape.iter().map(|r| r.to_string()).collect(),
        }
    }

    fn names(g: &Grid) -> Vec<&str> {
        g.cells.iter().map(|c| c.as_ref().map_or(".", |(i, _)| i.as_str())).collect()
    }

    #[test]
    fn a_shaped_recipe_is_laid_out_as_its_pattern_in_the_middle_of_the_grid() {
        let pickaxe =
            rec(Kind::Shaped, &["XXX", " # ", " # "], vec![ing("X", 3, &["diamond"]), ing("#", 2, &["stick"])]);
        let g = recipe_grid(&pickaxe, &[]);
        assert_eq!(g.cols, 3);
        assert_eq!(names(&g), ["diamond", "diamond", "diamond", ".", "stick", ".", ".", "stick", "."]);
        // two rows of one: the middle column, from the top
        let stick = rec(Kind::Shaped, &["#", "#"], vec![ing("#", 2, &["birch_planks", "oak_planks"])]);
        let g = recipe_grid(&stick, &[("#".into(), "oak_planks".into())]);
        assert_eq!(names(&g), [".", "oak_planks", ".", ".", "oak_planks", ".", ".", ".", "."]);
    }

    #[test]
    fn other_recipes_fill_the_slots_their_station_has() {
        let shapeless = rec(Kind::Shapeless, &[], vec![ing("#0", 2, &["wheat"]), ing("#1", 1, &["cocoa_beans"])]);
        assert_eq!(
            names(&recipe_grid(&shapeless, &[])),
            ["wheat", "wheat", "cocoa_beans", ".", ".", ".", ".", ".", "."]
        );
        let smelt = rec(Kind::Smelting, &[], vec![ing("input", 1, &["raw_iron"])]);
        assert_eq!(recipe_grid(&smelt, &[]), Grid { cols: 1, cells: vec![Some(("raw_iron".into(), 1))] });
        let smith = rec(
            Kind::Smithing,
            &[],
            vec![
                ing("template", 1, &["upgrade"]),
                ing("base", 1, &["diamond_pickaxe"]),
                ing("addition", 1, &["netherite_ingot"]),
            ],
        );
        let g = recipe_grid(&smith, &[]);
        assert_eq!((g.cols, names(&g)), (3, vec!["upgrade", "diamond_pickaxe", "netherite_ingot"]));
    }

    #[test]
    fn plan_files_get_plain_names() {
        assert_eq!(file_name("Diamond Pickaxe", 64), "diamond-pickaxe-64");
        assert_eq!(file_name("  ", 0), "crafting-plan");
        assert_eq!(file_name("My plan: 2nd!", 0), "my-plan-2nd");
    }
}
