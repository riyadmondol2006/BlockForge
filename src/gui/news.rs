//! The launcher news on the home page, and the window that shows one article in full.

use egui::{Context, CornerRadius, RichText, ScrollArea, Sense, Ui};

use super::covers::{CoverSpec, CoverTheme};
use super::instance_menu::SplitRow;
use super::services::{SvcMsg, modal_scroll_height, modal_width};
use super::theme::*;
use super::widgets::*;
use super::{App, Msg};
use crate::news::{NewsItem, Span};

/// How many articles the home page lists (the Java edition shows six).
const SHOWN: usize = 6;

#[derive(Default)]
pub enum Status {
    #[default]
    Idle,
    Loading,
    Loaded,
    Failed(String),
}

/// One article as the page shows it, its text worked out once when the news arrives (not on every frame).
struct Shown {
    title: String,
    date: String,
    preview: String,
    spans: Vec<Span>,
}

impl Shown {
    fn of(item: &NewsItem) -> Shown {
        Shown { title: item.title.clone(), date: item.date(), preview: preview(item), spans: item.spans() }
    }
}

#[derive(Default)]
pub struct NewsState {
    items: Vec<Shown>,
    status: Status,
    /// The article open in its own window.
    open: Option<usize>,
}

impl NewsState {
    pub fn arrived(&mut self, result: Result<Vec<NewsItem>, String>) {
        match result {
            Ok(items) => {
                self.items = items.iter().take(SHOWN).map(Shown::of).collect();
                // the list changed under an open article: close it rather than show another one
                self.open = None;
                self.status = Status::Loaded;
            }
            Err(e) => self.status = Status::Failed(e),
        }
    }
}

/// Draw spans as wrapped lines: bold, italic and links keep their style, and `\n` starts a new line.
pub fn render_spans(ui: &mut Ui, spans: &[Span]) {
    let mut lines: Vec<Vec<(&str, &Span)>> = vec![Vec::new()];
    for s in spans {
        for (n, part) in s.text.split('\n').enumerate() {
            if n > 0 {
                lines.push(Vec::new());
            }
            if let (false, Some(line)) = (part.is_empty(), lines.last_mut()) {
                line.push((part, s));
            }
        }
    }
    for line in lines {
        if line.is_empty() {
            ui.add_space(8.0);
            continue;
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (text, span) in line {
                match &span.link {
                    Some(url) => {
                        ui.add(egui::Hyperlink::from_label_and_url(RichText::new(text).underline(), url));
                    }
                    None => {
                        let mut t = RichText::new(text).color(c_text());
                        if span.bold {
                            t = t.semibold();
                        }
                        if span.italic {
                            t = t.italics();
                        }
                        ui.label(t);
                    }
                }
            }
        });
    }
}

/// The article's text on one line, cut at a word (never inside one) with an ellipsis when it is long.
fn preview(item: &NewsItem) -> String {
    let flat = item.plain_text().split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 220 {
        return flat;
    }
    let cut: String = flat.chars().take(217).collect();
    // back up to the end of the last whole word, unless that would throw most of it away
    let whole = match cut.rfind(' ') {
        Some(i) if i > 120 => &cut[..i],
        _ => cut.as_str(),
    };
    format!("{}\u{2026}", whole.trim_end_matches([' ', ',', ';', ':', '.', '-']))
}

/// The least width of a news card on Home, and the gap between the cards.
const CARD_MIN: f32 = 300.0;
const CARD_GAP: f32 = 12.0;
const CARD_H: f32 = 76.0;
const THUMB: f32 = 52.0;

/// One article as a small card: its picture, the title, a line of its text and the date. It brightens under the
/// pointer and opens the article when clicked.
fn news_card(ui: &mut Ui, item: &Shown, thumb: &egui::TextureHandle, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, CARD_H), Sense::click());
    let t = super::motion::hover(ui, resp.id, resp.hovered());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let round = CornerRadius::same(R_CARD);
        p.rect_filled(rect, round, super::motion::mix(c_card(), c_raised(), t));
        let edge = super::motion::mix(c_border(), c_control().gamma_multiply(0.7), t);
        p.rect_stroke(rect, round, egui::Stroke::new(1.0, edge), egui::StrokeKind::Inside);
        glass_edge(p, rect, R_CARD);
        let pic = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 12.0, rect.center().y - THUMB / 2.0),
            egui::vec2(THUMB, THUMB),
        );
        egui::Image::from_texture(thumb).corner_radius(R_CONTROL).paint_at(ui, pic);
        let left = pic.right() + 14.0;
        let room = (rect.right() - 14.0 - left).max(0.0);
        let line = |text: &str, font: egui::FontId, color: egui::Color32| {
            let mut job =
                egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(font, color));
            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
            ui.painter().layout_job(job)
        };
        let title = line(&item.title, super::fonts::semibold(13.0), c_text());
        let preview = line(&item.preview, egui::FontId::proportional(FS_META), c_muted());
        let date = line(&item.date, super::fonts::mono(11.0), c_faint());
        let total = title.size().y + 3.0 + preview.size().y + 3.0 + date.size().y;
        let mut y = rect.center().y - total / 2.0;
        for (g, gap) in [(title, 3.0), (preview, 3.0), (date, 0.0)] {
            let h = g.size().y;
            p.galley(egui::pos2(left, y), g, c_text());
            y += h + gap;
        }
    }
    focus_ring(ui, &resp, rect, R_CARD);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &item.title));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

impl App {
    /// The news of the home page, as small cards: under the instances, or beside them in a wide window.
    pub(super) fn news_section(&mut self, ui: &mut Ui) {
        if matches!(self.svc.news.status, Status::Idle) {
            self.svc.news.status = Status::Loading;
            let (dirs, n) = (self.dirs.clone(), self.notifier());
            self.rt.spawn(async move {
                n.send(Msg::Svc(SvcMsg::News(crate::news::load(&dirs).await.map_err(|e| format!("{e:#}")))));
            });
        }
        // the title, and the way to hide the news on Home (it comes back with "Show news" at the bottom of the page)
        let mut hide = false;
        let mut head = SplitRow::new(ui, egui::Id::unique("home-news-head"), 32.0);
        head.left(ui, |ui| {
            ui.label(RichText::new("News").size(FS_SECTION).semibold());
        });
        head.right(ui, |ui| {
            hide = ghost(ui, "Hide").on_hover_text("Hide the news on Home").clicked();
        });
        head.finish(ui);
        if hide {
            self.settings.home_news = false;
            self.save_settings();
            return;
        }
        ui.add_space(2.0);
        match &self.svc.news.status {
            Status::Idle | Status::Loading => skeleton_rows(ui, 3, 52.0),
            Status::Failed(e) => {
                let message = format!("Couldn't load the news: {e}");
                if notice(ui, NoticeKind::Error, &message, Some("Try again")).is_some_and(|r| r.clicked()) {
                    self.svc.news.status = Status::Idle;
                }
            }
            Status::Loaded if self.svc.news.items.is_empty() => muted(ui, "No news right now."),
            Status::Loaded => {
                // small cards side by side, two rows at most: the news is the second thing on Home, not the first
                let (cols, width) = grid_columns(ui.available_width(), CARD_MIN, CARD_GAP);
                let shown = self.svc.news.items.len().min(if cols == 1 { 4 } else { cols * 2 });
                // each article gets a little voxel picture of its own, drawn from its title
                let thumbs: Vec<egui::TextureHandle> = (0..shown)
                    .map(|i| {
                        let spec = CoverSpec::new(&self.svc.news.items[i].title, "1.21", None, CoverTheme::Auto);
                        self.covers.texture(&self.ctx, &spec, 128, 128)
                    })
                    .collect();
                let mut open = None;
                ui.spacing_mut().item_spacing = egui::vec2(CARD_GAP, CARD_GAP);
                for (r, row) in self.svc.news.items[..shown].chunks(cols).enumerate() {
                    ui.horizontal(|ui| {
                        for (k, item) in row.iter().enumerate() {
                            let i = r * cols + k;
                            if news_card(ui, item, &thumbs[i], width).clicked() {
                                open = Some(i);
                            }
                        }
                    });
                }
                if open.is_some() {
                    self.svc.news.open = open;
                }
            }
        }
    }

    /// One article in a window of its own.
    pub(super) fn news_overlay(&mut self, ctx: &Context) {
        let Some(i) = self.svc.news.open else { return };
        let Some(item) = self.svc.news.items.get(i) else {
            self.svc.news.open = None;
            return;
        };
        let mut close = false;
        egui::Modal::new(egui::Id::unique("news-article")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(modal_width(ctx, 620.0));
            ui.add(egui::Label::new(RichText::new(&item.title).size(FS_SECTION).semibold()).wrap());
            if !item.date.is_empty() {
                small_muted(ui, &item.date);
            }
            ui.add_space(8.0);
            ScrollArea::vertical()
                .max_height(modal_scroll_height(ctx, 420.0, 140.0))
                .auto_shrink([false, true])
                .show(ui, |ui| render_spans(ui, &item.spans));
            ui.add_space(10.0);
            if secondary(ui, "Close").clicked() {
                close = true;
            }
        });
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.svc.news.open = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draw the real news text in a window-less egui, to be sure nothing panics and something is painted.
    #[test]
    fn an_article_draws_without_a_window() {
        let item = NewsItem {
            title: "T".into(),
            content: "Hello <b>bold</b> <i>it</i> and <a href=\"https://example.com\">a link</a>.<br/><br/><ul><li>one</li><li>two</li></ul>".into(),
            created_at: "2024-11-15T12:55:33.000Z".into(),
        };
        let ctx = egui::Context::default();
        let mut shapes = 0;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            render_spans(ui, &item.spans());
        });
        shapes += out.shapes.len();
        out.textures_delta.clear();
        assert!(shapes > 0, "something was painted");
        assert_eq!(
            preview(&item),
            "Hello bold it and a link.\n\n\u{2022} one\n\u{2022} two".split_whitespace().collect::<Vec<_>>().join(" ")
        );
    }
}
