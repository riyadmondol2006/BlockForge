//! The right column of the Crafting page: the plan, in four tabs, and the windows for saved plans.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Align, CornerRadius, Layout, RichText, Ui, vec2};

use super::craft::{Act, Book, CraftState, Dialog, Env, LEFT_W, Load, PlanView, TWO_COLUMNS, Tab};
use super::craft_fmt::*;
use super::services::{confirm_modal, modal_scroll_height, modal_width};
use super::theme::*;
use super::widgets::*;
use crate::craft::saved;
use crate::craft::{self, Kind, Node, Recipe, Stop, Storage};

/// How wide the plan gets on a wide window: past this the amounts are too far from their names.
const PLAN_WIDTH: f32 = 780.0;

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
            let list_h = (h - 420.0).clamp(160.0, 420.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                ui.allocate_ui_with_layout(vec2(LEFT_W, h), Layout::top_down(Align::Min), |ui| {
                    egui::ScrollArea::vertical().id_salt("craft-left").auto_shrink([false, false]).show(ui, |ui| {
                        ui.set_max_width(ui.available_width() - 12.0);
                        ui.spacing_mut().item_spacing.y = 12.0;
                        self.left_pick(ui, &book, list_h);
                        self.left_stock(ui, &book);
                        self.left_supplied(ui, &book);
                        ui.add_space(8.0);
                    });
                });
                ui.allocate_ui_with_layout(vec2(right_w, h), Layout::top_down(Align::Min), |ui| {
                    egui::ScrollArea::vertical().id_salt("craft-right").auto_shrink([false, false]).show(ui, |ui| {
                        ui.set_max_width((ui.available_width() - 12.0).min(PLAN_WIDTH));
                        ui.spacing_mut().item_spacing.y = 12.0;
                        self.right(ui, &book, acts);
                        ui.add_space(8.0);
                    });
                });
            });
        } else {
            egui::ScrollArea::vertical().id_salt("craft-one").auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(ui.available_width() - 12.0);
                ui.spacing_mut().item_spacing.y = 12.0;
                if self.target.is_some() && !self.picking {
                    panel(ui, |ui| self.quantity_block(ui, &book, true));
                } else {
                    self.left_pick(ui, &book, 230.0);
                }
                self.right(ui, &book, acts);
                self.left_stock(ui, &book);
                self.left_supplied(ui, &book);
                ui.add_space(8.0);
            });
        }
    }

    fn right(&mut self, ui: &mut Ui, book: &Arc<Book>, acts: &mut Vec<Act>) {
        match self.plan.clone() {
            None => {
                let bad = self.target.is_some() && parse_qty(&self.qty_text).is_err();
                ui.add_space(72.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(if bad { "Check the amount" } else { "Pick something to build" })
                            .size(16.0)
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
                note_box(ui, c_danger(), |ui| {
                    ui.label(RichText::new("This can't be worked out").semibold().color(c_danger()));
                    ui.label(e);
                });
            }
            Some(Ok(view)) => self.result(ui, book, &view, acts),
        }
    }

    fn result(&mut self, ui: &mut Ui, book: &Arc<Book>, v: &Arc<PlanView>, acts: &mut Vec<Act>) {
        self.result_header(ui, book, v, acts);
        if !v.plan.warnings.is_empty() || !v.plan.complete {
            note_box(ui, c_warn(), |ui| {
                ui.label(RichText::new("Worth a look").semibold().color(c_warn()));
                for w in &v.plan.warnings {
                    ui.label(w);
                }
                if v.plan.warnings.is_empty() {
                    ui.label("Some branches stop early, so the lists below are not the whole story.");
                }
            });
        }
        tab_bar(ui, |ui| {
            for (tab_id, label) in
                [(Tab::Materials, "Materials"), (Tab::Steps, "Steps"), (Tab::Tree, "Tree"), (Tab::Storage, "Storage")]
            {
                if tab(ui, label, self.tab == tab_id).clicked() {
                    self.tab = tab_id;
                }
            }
        });
        match self.tab {
            Tab::Materials => self.tab_materials(ui, book, v),
            Tab::Steps => self.tab_steps(ui, book, v),
            Tab::Tree => self.tab_tree(ui, book, v),
            Tab::Storage => self.tab_storage(ui, book, v),
        }
    }

    /// The item and amount as the title of the plan, the totals as type, and what can be done with the plan.
    fn result_header(&mut self, ui: &mut Ui, book: &Arc<Book>, v: &Arc<PlanView>, acts: &mut Vec<Act>) {
        let plan = &v.plan;
        let name = book.ds.name(&plan.request.item);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            self.icon(ui, &plan.request.item, &name, 44.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("{} x {}", thousands(plan.request.quantity), name)).size(22.0).semibold(),
                    )
                    .truncate(),
                );
                ui.horizontal_wrapped(|ui| {
                    if plan.complete && plan.warnings.is_empty() {
                        tag(ui, "Complete plan", c_accent());
                    } else {
                        tag(ui, "Needs a look", c_warn());
                    }
                    if let Some(p) = self.active.and_then(|id| self.store.get(id)) {
                        tag(ui, &format!("Saved: {}", clip(&p.name, 24)), c_muted());
                        if self.unsaved {
                            tag(ui, "Changed since saved", c_warn());
                        }
                    }
                });
            });
        });
        ui.add_space(4.0);
        let gather: u64 = plan.materials.iter().map(|(_, n)| *n).sum();
        stat_strip(
            ui,
            &[
                (thousands(plan.operations), "Operations"),
                (thousands(gather), "Items to gather"),
                (thousands(plan.steps.len() as u64), "Steps"),
                (thousands(v.storage[0].slots), "Storage slots"),
            ],
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            if secondary(ui, "Copy shopping list").clicked() {
                acts.push(Act::Copy(saved::clipboard_list(&book.ds, plan)));
            }
            let export = secondary(ui, "Export");
            let slug = file_name(&name, plan.request.quantity);
            egui::Popup::menu(&export).show(|ui| {
                ui.set_min_width(180.0);
                if ui.button("Text file (.txt)").clicked() {
                    acts.push(Act::Export {
                        name: format!("{slug}.txt"),
                        content: saved::to_text(&book.ds, plan),
                        ext: "txt",
                    });
                    ui.close();
                }
                if ui.button("JSON file (.json)").clicked() {
                    acts.push(Act::Export {
                        name: format!("{slug}.json"),
                        content: saved::to_json(&book.ds, plan),
                        ext: "json",
                    });
                    ui.close();
                }
            });
            let label = if self.active.is_some() { "Update plan" } else { "Save plan" };
            if secondary(ui, label).clicked() {
                let fallback = format!("{} x {}", thousands(plan.request.quantity), name);
                let current = self.active.and_then(|id| self.store.get(id)).map(|p| p.name.clone());
                self.dialog = Some(Dialog::Save(current.unwrap_or(fallback)));
            }
            let n = self.store.plans.len();
            if secondary(ui, &if n > 0 { format!("Saved plans ({n})") } else { "Saved plans".to_string() }).clicked() {
                self.dialog = Some(Dialog::Saved);
            }
            if (!self.recipe_choices.is_empty() || !self.ingredient_choices.is_empty())
                && secondary(ui, "Reset my recipe choices").clicked()
            {
                self.recipe_choices.clear();
                self.ingredient_choices.clear();
                self.touch_plan();
            }
        });
    }

    // ----------------------------------------------------------------------------------------------------
    // Materials

    fn tab_materials(&mut self, ui: &mut Ui, book: &Arc<Book>, v: &Arc<PlanView>) {
        let plan = &v.plan;
        // what to gather
        panel(ui, |ui| {
            let total = plan.materials.len();
            let ticked = plan.materials.iter().filter(|(i, _)| self.done.contains(&format!("gather:{i}"))).count();
            ui.horizontal(|ui| {
                card_title(ui, "To gather");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if total > 0 {
                        small_muted(ui, format!("{ticked} of {total} ticked"));
                    }
                });
            });
            if total == 0 {
                muted(ui, "Nothing to gather: everything comes from your items or from what you bring.");
                return;
            }
            progress_bar(ui, ticked as f32 / total as f32);
            ui.add_space(4.0);
            for (k, (id, qty)) in plan.materials.iter().enumerate() {
                let key = format!("gather:{id}");
                let mut on = self.done.contains(&key);
                let before = on;
                let supplied = self.supplied.contains(id);
                let mut unbring = false;
                self.list_row(ui, book, id, *qty, Some(&mut on), before, k + 1 == total, |ui| {
                    if supplied {
                        if x_button(ui, "No longer bringing this") {
                            unbring = true;
                        }
                        tag(ui, "you bring", c_muted());
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
                ui.set_min_height(44.0);
                if let Some(on) = check {
                    check_box(ui, on, true);
                }
                self.icon(ui, id, &name, 28.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.allocate_ui_with_layout(
                        vec2(104.0, 38.0),
                        Layout::top_down(Align::Max).with_main_align(Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let color = if struck { c_muted() } else { c_text() };
                            ui.label(RichText::new(thousands(qty)).font(super::fonts::medium(15.0)).color(color));
                            if !sub.is_empty() {
                                small_muted(ui, sub.as_str());
                            }
                        },
                    );
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
            progress_bar(ui, ticked as f32 / n as f32);
            for i in 0..n {
                ui.add_space(10.0);
                rule(ui);
                ui.add_space(10.0);
                self.step_card(ui, book, v, i);
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
            let mut on = ticked;
            let r = check_box(ui, &mut on, ready || ticked);
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
            self.icon(ui, station, step.kind.station(), 28.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let title = RichText::new(format!("{} {}x", step.kind.verb(), thousands(step.operations)))
                    .size(15.0)
                    .semibold();
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
            ui.add_space(36.0);
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
                ui.add_space(36.0);
                if recipe.is_some_and(|r| r.small_grid && matches!(r.kind, Kind::Shaped | Kind::Shapeless)) {
                    tag(ui, "Fits the inventory grid", c_muted());
                }
                if let Some(t) = step.ticks {
                    small_muted(ui, format!("About {} in one {}", ticks_text(t), step.kind.station().to_lowercase()));
                }
                if let Some(m) = recipe.and_then(|r| r.manual.as_ref()) {
                    ui.label(RichText::new(m).color(c_warn()).size(12.5));
                }
            });
        }
        if !step.byproducts.is_empty() {
            let text: Vec<String> =
                step.byproducts.iter().map(|(id, n)| format!("{} {}", thousands(*n), book.ds.name(id))).collect();
            ui.horizontal(|ui| {
                ui.add_space(36.0);
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
                ui.add_space(36.0);
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
                card_title(ui, "Everything it is made of");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if v.nodes <= 400 {
                        if secondary(ui, "Collapse all").clicked() {
                            self.tree_epoch += 1;
                            self.tree_all = false;
                        }
                        if secondary(ui, "Expand all").clicked() {
                            self.tree_epoch += 1;
                            self.tree_all = true;
                        }
                    }
                });
            });
            small_muted(ui, "Open an item to see what goes into it.");
            ui.add_space(4.0);
            self.tree_node(ui, book, &v.plan.tree, 1, 0);
        });
    }

    fn tree_node(&mut self, ui: &mut Ui, book: &Arc<Book>, node: &Node, path: u64, depth: usize) {
        if node.children.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(ui.spacing().indent);
                self.tree_row(ui, book, node, path);
            });
            return;
        }
        let id = ui.make_persistent_id(("craft-tree", self.tree_epoch, path));
        let open = depth <= 1 || self.tree_all;
        let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open);
        state.show_header(ui, |ui| self.tree_row(ui, book, node, path)).body(|ui| {
            for (i, c) in node.children.iter().enumerate() {
                self.tree_node(ui, book, c, path.wrapping_mul(31).wrapping_add(i as u64 + 1), depth + 1);
            }
        });
    }

    fn tree_row(&mut self, ui: &mut Ui, book: &Arc<Book>, n: &Node, path: u64) {
        let name = book.ds.name(&n.item);
        ui.set_min_height(30.0);
        self.icon(ui, &n.item, &name, 24.0);
        ui.label(RichText::new(&name).semibold());
        small_muted(ui, format!("need {}", thousands(n.required)));
        match n.stop {
            Some(Stop::Inventory) => tag(ui, "from your items", c_accent()),
            Some(Stop::Raw) => tag(ui, "gather", c_accent()),
            Some(Stop::Supplied) => tag(ui, "you bring", c_muted()),
            Some(Stop::NoRecipe) => tag(ui, "no recipe", c_warn()),
            Some(Stop::Manual) => tag(ui, "manual", c_warn()),
            Some(Stop::Cycle) => tag(ui, "cycle", c_danger()),
            None => {}
        }
        if n.from_stock > 0 && n.stop != Some(Stop::Inventory) {
            small_muted(ui, format!("{} of yours", thousands(n.from_stock)));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if n.operations > 0 {
                small_muted(ui, format!("{} op{}", thousands(n.operations), if n.operations == 1 { "" } else { "s" }));
            }
            if let Some(r) = &n.recipe {
                self.recipe_combo(ui, book, &n.item, r, path);
            }
        });
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
            stat_strip(
                ui,
                &[
                    (thousands(s.single_chests.count), "Single chests"),
                    (thousands(s.double_chests.count), "Double chests"),
                    (thousands(s.shulker_boxes.count), "Shulker boxes"),
                    (thousands(s.player_inventories.count), "Inventories"),
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
                        ui.set_min_height(38.0);
                        self.icon(ui, &r.item, &name, 24.0);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let (dr, _) = ui.allocate_exact_size(vec2(190.0, 20.0), egui::Sense::hover());
                            ui.painter().text(
                                dr.right_center(),
                                egui::Align2::RIGHT_CENTER,
                                text,
                                egui::FontId::proportional(12.5),
                                c_muted(),
                            );
                            let (qr, _) = ui.allocate_exact_size(vec2(72.0, 20.0), egui::Sense::hover());
                            ui.painter().text(
                                qr.right_center(),
                                egui::Align2::RIGHT_CENTER,
                                thousands(r.quantity),
                                super::fonts::medium(14.0),
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
                    "Everything in \"Items I already have\" for this version is removed.",
                    "Clear",
                    true,
                ) {
                    Some(true) => {
                        self.clear_stock();
                        self.stock_bufs.clear();
                        self.touch_store();
                        self.touch_plan();
                        None
                    }
                    Some(false) => None,
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
            ui.label(RichText::new("Get another version").size(19.0).semibold());
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
                RichText::new(if existing { "Update your saved plan" } else { "Save this plan" }).size(19.0).semibold(),
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
            ui.label(RichText::new("Rename the plan").size(19.0).semibold());
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
            ui.label(RichText::new("Saved plans").size(19.0).semibold());
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
                                        tag(ui, "Open now", c_accent());
                                    }
                                    ui.vertical(|ui| {
                                        ui.set_width(ui.available_width());
                                        ui.spacing_mut().item_spacing.y = 1.0;
                                        ui.add(egui::Label::new(RichText::new(&p.name).semibold()).truncate());
                                        small_muted(
                                            ui,
                                            format!(
                                                "{} x {} \u{00B7} Minecraft {} \u{00B7} {}",
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

/// How an item's recipe reads in a list: "Crafting table: 2 Wheat + 1 Cocoa Beans -> 8 Cookie".
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
    format!("{}: {} -> {} {}", r.kind.station(), inputs, r.count, ds.name(&r.output))
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

/// A thin bar showing how much is done.
fn progress_bar(ui: &mut Ui, fraction: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(2), c_field_hover());
    if fraction > 0.0 {
        let mut fill = rect;
        fill.set_width((rect.width() * fraction.clamp(0.0, 1.0)).max(4.0));
        ui.painter().rect_filled(fill, CornerRadius::same(2), c_accent());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_files_get_plain_names() {
        assert_eq!(file_name("Diamond Pickaxe", 64), "diamond-pickaxe-64");
        assert_eq!(file_name("  ", 0), "crafting-plan");
        assert_eq!(file_name("My plan: 2nd!", 0), "my-plan-2nd");
    }
}
