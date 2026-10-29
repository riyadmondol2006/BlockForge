//! The left column of the Crafting page: choose the item and the amount, and say what you already have.

use std::sync::Arc;

use egui::{Align, CornerRadius, FontId, Layout, RichText, Sense, Stroke, Ui, vec2};

use super::craft::{Book, CraftState, Dialog, Entry};
use super::craft_fmt::*;
use super::theme::*;
use super::widgets::*;
use crate::craft::{Category, ItemId};

const ROW_H: f32 = 40.0;

/// The entries matching `query` (and the category), best first; at most `limit`. An empty query matches all.
pub(super) fn search_entries(book: &Book, query: &str, cat: Option<Category>, limit: usize) -> Vec<u32> {
    // an id copied from the game or a wiki ("minecraft:oak_planks") finds its item too
    let q = query.trim().to_lowercase();
    let q = q.strip_prefix("minecraft:").unwrap_or(&q).to_string();
    let q_id = q.replace(' ', "_");
    let mut hits: Vec<(u8, u32)> = Vec::new();
    for (i, e) in book.entries.iter().enumerate() {
        if cat.is_some_and(|c| c != e.category) {
            continue;
        }
        let rank = if q.is_empty() || e.lname.starts_with(&q) {
            0
        } else if e.lname.split(' ').any(|w| w.starts_with(&q)) {
            1
        } else if e.lname.contains(&q) {
            2
        } else if e.lid.contains(&q_id) {
            3
        } else {
            continue;
        };
        hits.push((rank, i as u32));
        // an empty query is already in order; with a query, every hit is needed to rank them
        if q.is_empty() && hits.len() >= limit {
            break;
        }
    }
    hits.sort_by_key(|&(r, i)| (r, i));
    hits.into_iter().take(limit).map(|(_, i)| i).collect()
}

impl CraftState {
    /// The cards that pick the item and the amount: the amount of what was picked on top, the search under it.
    pub(super) fn left_pick(&mut self, ui: &mut Ui, book: &Arc<Book>, list_h: f32) {
        if self.target.is_some() {
            panel(ui, |ui| self.quantity_block(ui, book, false));
        }
        panel(ui, |ui| {
            card_title(ui, "What do you want to make?");
            ui.add_space(2.0);
            let field = ui.add_sized(
                [ui.available_width(), 32.0],
                egui::TextEdit::singleline(&mut self.search)
                    .id(egui::Id::unique("craft-search"))
                    .hint_text(RichText::new("Search items").color(c_faint()))
                    .margin(egui::Margin::symmetric(10, 7))
                    .background_color(c_field()),
            );
            if self.target.is_none() {
                // nothing is picked yet: typing can start at once
                focus_when_shown(ui, &field);
            }
            small_faint(ui, "Press / to search");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                if chip(ui, "All", self.category.is_none()).clicked() {
                    self.category = None;
                }
                for c in Category::ALL {
                    if chip(ui, c.label(), self.category == Some(c)).clicked() {
                        self.category = Some(c);
                    }
                }
            });
            let fresh = self
                .filtered_for
                .as_ref()
                .is_some_and(|(s, c, g)| *s == self.search && *c == self.category && *g == self.rev);
            if !fresh {
                self.filtered = search_entries(book, &self.search, self.category, usize::MAX);
                self.filtered_for = Some((self.search.clone(), self.category, self.rev));
            }
            let n = self.filtered.len();
            small_muted(
                ui,
                if self.search.trim().is_empty() && self.category.is_none() {
                    format!("{} items", thousands(n as u64))
                } else if n == 1 {
                    "1 match".to_string()
                } else {
                    format!("{} matches", thousands(n as u64))
                },
            );
            let mut pick: Option<ItemId> = None;
            if n == 0 {
                ui.add_space(16.0);
                // a category can be what hides the item: say so, and offer to look in all of them
                let in_all = self.category.is_some() && !search_entries(book, &self.search, None, 1).is_empty();
                ui.vertical_centered(|ui| {
                    if in_all {
                        muted(ui, "Nothing in this category matches that.");
                        if ghost(ui, "Search all categories").clicked() {
                            self.category = None;
                        }
                    } else {
                        muted(ui, "Nothing matches that. Try fewer letters.");
                    }
                });
                ui.add_space(16.0);
            } else {
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical()
                    .id_salt("craft-items")
                    .max_height(list_h)
                    .auto_shrink([false, false])
                    .show_rows(ui, ROW_H, n, |ui, range| {
                        for k in range {
                            let e = &book.entries[self.filtered[k] as usize];
                            let selected = self.target.as_deref() == Some(e.id.as_str());
                            if self.item_row(ui, e, selected) {
                                pick = Some(e.id.clone());
                            }
                        }
                    });
            }
            if let Some(id) = pick {
                self.set_target(&id);
            }
        });
    }

    /// One row of the item list: picture, name, and a small line under it. True when clicked.
    fn item_row(&mut self, ui: &mut Ui, e: &Entry, selected: bool) -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
        if !ui.is_rect_visible(rect) {
            return false;
        }
        // the hover fill fades in; the chosen row has the selection fill and a short bar of the brand gradient
        let t = super::motion::hover(ui, resp.id, resp.hovered() || selected);
        if t > 0.0 {
            let fill = if selected { c_selection() } else { c_hover() };
            ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), fill.gamma_multiply(t));
        }
        if selected {
            let bar = egui::Rect::from_min_size(rect.left_top() + vec2(0.0, 10.0), vec2(3.0, ROW_H - 20.0));
            super::motion::gradient_rect(ui.painter(), bar, 1, c_accent(), c_accent2());
        }
        let icon = egui::Rect::from_min_size(rect.left_top() + vec2(8.0, (ROW_H - 32.0) / 2.0), vec2(32.0, 32.0));
        self.paint_slot(ui, icon, &e.id, &e.name);
        let short = e.id.rsplit(':').next().unwrap_or(&e.id);
        let sub = if e.stack > 1 { format!("{short}  \u{00B7}  stacks of {}", e.stack) } else { short.to_string() };
        let clip = rect.shrink2(vec2(2.0, 0.0));
        let p = ui.painter().with_clip_rect(clip);
        let font = if selected { super::fonts::semibold(FS_BODY) } else { FontId::proportional(FS_BODY) };
        p.text(rect.left_top() + vec2(50.0, 4.0), egui::Align2::LEFT_TOP, &e.name, font, c_text());
        // faint text never sits on the selection fill
        let sub_color = if selected { c_muted() } else { c_faint() };
        p.text(
            rect.left_top() + vec2(50.0, 22.0),
            egui::Align2::LEFT_TOP,
            sub,
            FontId::proportional(FS_META),
            sub_color,
        );
        focus_ring(ui, &resp, rect, R_CONTROL);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &e.name));
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    }

    /// What was picked and how many of it.
    pub(super) fn quantity_block(&mut self, ui: &mut Ui, book: &Arc<Book>, change: bool) {
        let Some(target) = self.target.clone() else { return };
        let item = book.ds.item(&target);
        let name = item.map_or_else(|| book.ds.name(&target), |i| i.name.clone());
        let stack = item.map_or(64, |i| i.stack);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            self.slot_icon(ui, &target, &name, 44.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new(&name).size(FS_SECTION).semibold());
                small_muted(ui, if stack > 1 { format!("Stacks of {stack}") } else { "Does not stack".to_string() });
            });
            if change {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if secondary(ui, "Change item").clicked() {
                        self.picking = true;
                    }
                });
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new("How many?").semibold());
        let parsed = parse_qty(&self.qty_text);
        ui.horizontal(|ui| {
            let mut edit = egui::TextEdit::singleline(&mut self.qty_text)
                .id(egui::Id::unique("craft-qty"))
                .margin(egui::Margin::symmetric(10, 7))
                .background_color(c_field());
            if parsed.is_err() {
                edit = edit.text_color(c_danger());
            }
            let r = ui.add_sized([140.0, 32.0], edit);
            if r.changed() {
                self.touch_plan();
            }
            if let Ok(q) = &parsed {
                small_muted(ui, format!("= {}", thousands(*q)));
            }
        });
        if let Err(e) = &parsed {
            ui.label(RichText::new(e).color(c_danger()).size(FS_META));
        }
        let current = parsed.as_ref().ok().copied();
        let st = u64::from(stack);
        let options: Vec<(String, u64)> = if stack > 1 {
            vec![
                ("1".to_string(), 1),
                (format!("1 stack = {}", thousands(st)), st),
                (format!("27 stacks = {}", thousands(27 * st)), 27 * st),
                (format!("54 stacks = {}", thousands(54 * st)), 54 * st),
            ]
        } else {
            vec![("1".to_string(), 1), ("9".to_string(), 9), ("27".to_string(), 27), ("54".to_string(), 54)]
        };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (label, v) in options {
                if chip(ui, &label, current == Some(v)).clicked() {
                    self.qty_text = thousands(v);
                    self.touch_plan();
                }
            }
        });
        if stack > 1 {
            small_muted(ui, "27 stacks fill a chest, 54 a double chest.");
        }
    }

    /// The card with what the player already has.
    pub(super) fn left_stock(&mut self, ui: &mut Ui, book: &Arc<Book>) {
        let version = self.version.clone().unwrap_or_default();
        panel(ui, |ui| {
            let count = self.stock().map_or(0, |s| s.len());
            ui.horizontal(|ui| {
                card_title(ui, "Items I already have");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if count > 0 && secondary(ui, "Clear").clicked() {
                        self.dialog = Some(Dialog::ClearStock);
                    }
                });
            });
            small_muted(ui, format!("Kept for Minecraft {version} and taken off the list."));
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.set_min_height(32.0);
                ui.label("Use my items in the plan");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut on = self.use_stock;
                    if toggle(ui, &mut on).changed() {
                        self.use_stock = on;
                        self.touch_plan();
                    }
                });
            });
            ui.add_space(2.0);
            // the rows
            let ids: Vec<ItemId> = self.stock().map(|s| s.keys().cloned().collect()).unwrap_or_default();
            let mut remove: Option<ItemId> = None;
            for id in &ids {
                let name = book.ds.name(id);
                let value = self.stock().and_then(|s| s.get(id)).copied().unwrap_or(0);
                ui.horizontal(|ui| {
                    self.slot_icon(ui, id, &name, 32.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if x_button(ui, "Remove") {
                            remove = Some(id.clone());
                        }
                        let buf = self.stock_bufs.entry(id.clone()).or_insert_with(|| value.to_string());
                        let bad = parse_qty(buf).is_err();
                        let mut edit = egui::TextEdit::singleline(buf)
                            .margin(egui::Margin::symmetric(8, 5))
                            .background_color(c_field());
                        if bad {
                            edit = edit.text_color(c_danger());
                        }
                        let r = ui.add_sized([88.0, 30.0], edit);
                        let typed = parse_qty(buf);
                        if r.changed() {
                            if let Ok(q) = typed {
                                self.set_stock(id, q);
                                self.touch_store();
                                if self.use_stock {
                                    self.touch_plan();
                                }
                            }
                        } else if !r.has_focus() {
                            *self.stock_bufs.entry(id.clone()).or_default() = value.to_string();
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.add(egui::Label::new(name.as_str()).truncate());
                        });
                    });
                });
            }
            if let Some(id) = remove {
                self.set_stock(&id, 0);
                self.stock_bufs.remove(&id);
                self.touch_store();
                if self.use_stock {
                    self.touch_plan();
                }
            }
            if ids.is_empty() {
                small_muted(ui, "Nothing yet. Add what is already in your chests.");
            }
            ui.add_space(4.0);
            self.stock_adder(ui, book);
        });
    }

    /// The small form that adds an item to the stock.
    fn stock_adder(&mut self, ui: &mut Ui, book: &Arc<Book>) {
        ui.add_sized(
            [ui.available_width(), 32.0],
            egui::TextEdit::singleline(&mut self.stock_search)
                .hint_text(RichText::new("Add an item you have").color(c_faint()))
                .margin(egui::Margin::symmetric(10, 7))
                .background_color(c_field()),
        );
        if self.stock_search != self.stock_hits_for {
            self.stock_hits = if self.stock_search.trim().is_empty() {
                Vec::new()
            } else {
                search_entries(book, &self.stock_search, None, 6)
            };
            self.stock_hits_for = self.stock_search.clone();
        }
        let mut chosen: Option<ItemId> = None;
        if !self.stock_search.trim().is_empty() {
            if self.stock_hits.is_empty() {
                small_muted(ui, "Nothing matches that.");
            }
            for k in 0..self.stock_hits.len() {
                let e = &book.entries[self.stock_hits[k] as usize];
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                let t = super::motion::hover(ui, resp.id, resp.hovered());
                if t > 0.0 {
                    ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), c_hover().gamma_multiply(t));
                }
                let icon = egui::Rect::from_min_size(rect.left_top() + vec2(4.0, 5.0), vec2(24.0, 24.0));
                self.paint_icon(ui, icon, &e.id, &e.name);
                ui.painter().with_clip_rect(rect).text(
                    rect.left_center() + vec2(36.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &e.name,
                    FontId::proportional(FS_BODY),
                    c_text(),
                );
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    chosen = Some(e.id.clone());
                }
            }
        }
        if let Some(id) = chosen {
            self.stock_pick = Some(id);
            self.stock_search.clear();
            self.stock_hits.clear();
            self.stock_hits_for.clear();
        }
        if let Some(id) = self.stock_pick.clone() {
            let name = book.ds.name(&id);
            let parsed = parse_qty(&self.stock_qty);
            let mut add = false;
            let mut cancel = false;
            ui.horizontal(|ui| {
                self.slot_icon(ui, &id, &name, 32.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if x_button(ui, "Cancel") {
                        cancel = true;
                    }
                    if primary_enabled(ui, "Add", parsed.is_ok()).clicked() {
                        add = true;
                    }
                    let mut edit = egui::TextEdit::singleline(&mut self.stock_qty)
                        .margin(egui::Margin::symmetric(8, 5))
                        .background_color(c_field());
                    if parsed.is_err() {
                        edit = edit.text_color(c_danger());
                    }
                    ui.add_sized([76.0, 30.0], edit);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(egui::Label::new(name.as_str()).truncate());
                    });
                });
            });
            if add {
                if let Ok(q) = parsed {
                    let have = self.stock().and_then(|s| s.get(&id)).copied().unwrap_or(0);
                    self.set_stock(&id, have.saturating_add(q));
                    self.stock_bufs.remove(&id);
                    self.stock_pick = None;
                    self.touch_store();
                    if self.use_stock {
                        self.touch_plan();
                    }
                }
            }
            if cancel {
                self.stock_pick = None;
            }
        }
    }

    /// The items the player will bring, as chips to take away again.
    pub(super) fn left_supplied(&mut self, ui: &mut Ui, book: &Arc<Book>) {
        if self.supplied.is_empty() {
            return;
        }
        panel(ui, |ui| {
            card_title(ui, "Items I will bring");
            small_muted(ui, "The plan stops at these and lists them to gather.");
            ui.add_space(2.0);
            let items: Vec<ItemId> = self.supplied.iter().cloned().collect();
            let mut remove: Option<ItemId> = None;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for id in &items {
                    let name = book.ds.name(id);
                    egui::Frame::new()
                        .fill(c_field())
                        .stroke(Stroke::new(1.0, c_border()))
                        .corner_radius(CornerRadius::same(16))
                        .inner_margin(egui::Margin { left: 5, right: 3, top: 3, bottom: 3 })
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.horizontal(|ui| {
                                self.icon(ui, id, &name, 22.0);
                                ui.label(RichText::new(&name).size(FS_BODY));
                                if x_button(ui, "Take away") {
                                    remove = Some(id.clone());
                                }
                            });
                        });
                }
            });
            if let Some(id) = remove {
                self.supplied.remove(&id);
                self.touch_plan();
            }
        });
    }
}
