//! The egui renderer of the Markdown model (`textview::MdDocument`): headings,
//! paragraphs with inline styles and clickable links, nested lists and task
//! lists, quotes and GitHub alerts, highlighted code blocks, tables with
//! alignment, rules, images and footnotes. Links: http(s)/mailto open in the
//! browser, relative and local paths open the file in the text view.
use eframe::egui::{self, text::LayoutJob, Color32, RichText, TextFormat};

use crate::textview::{MdAlignment, MdBlock, MdQuoteKind, MdRun, MdRunKind};

use super::md_images::{ImageState, MdImages};
use super::md_links::{heading_slug, resolve_link, LinkTarget};
use super::text_tools::TextPlace;
use super::text_view_style::{mono, styled_job};

/// What a click in the rendered document asks the window for.
pub(in crate::app) enum MdAction {
    OpenFile(String),
    /// `#heading`: scroll to the heading with this slug.
    Anchor(String),
}

pub(in crate::app) struct MdRenderer<'a> {
    pub(in crate::app) base_dir: &'a str,
    pub(in crate::app) place: &'a TextPlace,
    pub(in crate::app) images: &'a mut MdImages,
    pub(in crate::app) actions: Vec<MdAction>,
    /// Slug of the heading to scroll to in this frame.
    pub(in crate::app) scroll_to: Option<String>,
    counter: usize,
}

fn quote_label(kind: MdQuoteKind) -> &'static str {
    match kind {
        MdQuoteKind::Note => "Hinweis",
        MdQuoteKind::Tip => "Tipp",
        MdQuoteKind::Important => "Wichtig",
        MdQuoteKind::Warning => "Warnung",
        MdQuoteKind::Caution => "Vorsicht",
    }
}

impl<'a> MdRenderer<'a> {
    pub(in crate::app) fn new(
        base_dir: &'a str,
        place: &'a TextPlace,
        images: &'a mut MdImages,
    ) -> Self {
        Self {
            base_dir,
            place,
            images,
            actions: Vec::new(),
            scroll_to: None,
            counter: 0,
        }
    }

    fn next_id(&mut self) -> usize {
        self.counter += 1;
        self.counter
    }

    pub(in crate::app) fn blocks(&mut self, ui: &mut egui::Ui, blocks: &[MdBlock], depth: usize) {
        for block in blocks {
            self.block(ui, block, depth);
        }
    }

    fn block(&mut self, ui: &mut egui::Ui, block: &MdBlock, depth: usize) {
        let body = egui::TextStyle::Body.resolve(ui.style()).size;
        let muted = super::theme::muted(ui);
        match block {
            MdBlock::Heading { level, runs } => {
                let scale = match level {
                    1 => 1.9,
                    2 => 1.6,
                    3 => 1.35,
                    4 => 1.2,
                    5 => 1.1,
                    _ => 1.0,
                };
                ui.add_space(body * 0.4);
                let rect = ui
                    .scope(|ui| self.runs(ui, runs, Some(body * scale), true))
                    .response
                    .rect;
                let text: String = runs.iter().map(|run| run.text.as_str()).collect();
                if self.scroll_to.as_deref() == Some(heading_slug(&text).as_str()) {
                    ui.scroll_to_rect(rect, Some(egui::Align::TOP));
                    self.scroll_to = None;
                }
                if *level <= 2 {
                    ui.separator();
                }
            }
            MdBlock::Paragraph { runs } => {
                self.runs(ui, runs, None, false);
                ui.add_space(body * 0.3);
            }
            MdBlock::List { start, items } => {
                for (index, item) in items.iter().enumerate() {
                    let marker = match (item.task, start) {
                        (Some(true), _) => "[x]".to_string(),
                        (Some(false), _) => "[ ]".to_string(),
                        (None, Some(first)) => format!("{}.", first + index as u64),
                        (None, None) if depth % 2 == 0 => "•".to_string(),
                        (None, None) => "–".to_string(),
                    };
                    ui.horizontal_top(|ui| {
                        ui.label(RichText::new(marker).monospace());
                        ui.vertical(|ui| self.blocks(ui, &item.blocks, depth + 1));
                    });
                }
                ui.add_space(body * 0.3);
            }
            MdBlock::Quote { kind, blocks } => {
                let color = match kind {
                    Some(MdQuoteKind::Warning | MdQuoteKind::Caution) => super::theme::warning(ui),
                    Some(_) => super::theme::accent(ui),
                    None => muted,
                };
                let frame = egui::Frame::none().inner_margin(egui::Margin {
                    left: 12.0,
                    right: 4.0,
                    top: 2.0,
                    bottom: 2.0,
                });
                let rect = frame
                    .show(ui, |ui| {
                        if let Some(kind) = kind {
                            ui.label(RichText::new(quote_label(*kind)).strong().color(color));
                        }
                        self.blocks(ui, blocks, depth + 1);
                    })
                    .response
                    .rect;
                ui.painter().line_segment(
                    [
                        rect.left_top() + egui::vec2(3.0, 0.0),
                        rect.left_bottom() + egui::vec2(3.0, 0.0),
                    ],
                    egui::Stroke::new(3.0, color),
                );
            }
            MdBlock::Code {
                language,
                highlighted_as,
                lines,
            } => {
                let id = self.next_id();
                let font = mono(ui);
                let fallback = ui.visuals().text_color();
                egui::Frame::none()
                    .fill(ui.visuals().code_bg_color)
                    .inner_margin(6.0)
                    .rounding(3.0)
                    .show(ui, |ui| {
                        if let Some(language) = language {
                            ui.label(RichText::new(language).small().color(muted))
                                .on_hover_text(format!("Hervorgehoben als {highlighted_as}"));
                        }
                        egui::ScrollArea::horizontal()
                            .id_salt(("md_code", id))
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                for line in lines {
                                    let job = styled_job(
                                        line,
                                        &font,
                                        fallback,
                                        &[],
                                        Color32::TRANSPARENT,
                                    );
                                    ui.add(egui::Label::new(job).extend());
                                }
                            });
                    });
                ui.add_space(body * 0.3);
            }
            MdBlock::Table {
                alignments,
                header,
                rows,
            } => {
                let id = self.next_id();
                let columns = alignments.len().max(header.len()).max(1);
                egui::Grid::new(("md_table", id))
                    .striped(true)
                    .num_columns(columns)
                    .show(ui, |ui| {
                        for (column, cell) in header.iter().enumerate() {
                            self.cell(ui, cell, alignments.get(column).copied(), true);
                        }
                        ui.end_row();
                        for row in rows {
                            for (column, cell) in row.iter().enumerate() {
                                self.cell(ui, cell, alignments.get(column).copied(), false);
                            }
                            ui.end_row();
                        }
                    });
                ui.add_space(body * 0.3);
            }
            MdBlock::Rule => {
                ui.separator();
            }
            MdBlock::Image { url, title, alt } => {
                self.image(ui, url, alt, title);
                ui.add_space(body * 0.3);
            }
            MdBlock::Html { text } => {
                ui.label(RichText::new(text).monospace().color(muted));
            }
            MdBlock::FootnoteDefinition { label, blocks } => {
                ui.horizontal_top(|ui| {
                    ui.label(RichText::new(format!("[{label}]")).small().color(muted));
                    ui.vertical(|ui| self.blocks(ui, blocks, depth + 1));
                });
            }
        }
    }

    /// Inline runs of a paragraph or heading, wrapped; every run its own
    /// widget so links stay clickable.
    fn runs(&mut self, ui: &mut egui::Ui, runs: &[MdRun], size: Option<f32>, strong: bool) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for run in runs {
                self.run(ui, run, size, strong);
            }
        });
    }

    fn run(&mut self, ui: &mut egui::Ui, run: &MdRun, size: Option<f32>, strong: bool) {
        let muted = super::theme::muted(ui);
        let mut text = match &run.kind {
            MdRunKind::LineBreak => {
                ui.end_row();
                return;
            }
            MdRunKind::Image { url, title } => {
                self.image(ui, url, &run.text, title);
                return;
            }
            MdRunKind::Code => RichText::new(&run.text).code(),
            MdRunKind::Html => RichText::new(&run.text).monospace().color(muted),
            MdRunKind::FootnoteRef => RichText::new(format!("[{}]", run.text)).small(),
            MdRunKind::Text => RichText::new(&run.text),
        };
        if let Some(size) = size {
            text = text.size(size);
        }
        if run.strong || strong {
            text = text.strong();
        }
        if run.emphasis {
            text = text.italics();
        }
        if run.strikethrough {
            text = text.strikethrough();
        }
        let Some(link) = &run.link else {
            ui.label(text);
            return;
        };
        let text = text.color(ui.visuals().hyperlink_color).underline();
        let response = ui
            .add(egui::Label::new(text).sense(egui::Sense::click()))
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(link);
        if response.clicked() {
            self.follow(ui, link);
        }
    }

    fn follow(&mut self, ui: &egui::Ui, link: &str) {
        match resolve_link(self.base_dir, link) {
            LinkTarget::Web(url) => ui.ctx().open_url(egui::OpenUrl::new_tab(url)),
            LinkTarget::File(path) => self.actions.push(MdAction::OpenFile(path)),
            LinkTarget::Anchor(name) => self.actions.push(MdAction::Anchor(name)),
            LinkTarget::Unsupported => {}
        }
    }

    /// A table cell: one layout with the run styles; a cell with a link
    /// follows its first link when clicked.
    fn cell(
        &mut self,
        ui: &mut egui::Ui,
        runs: &[MdRun],
        align: Option<MdAlignment>,
        header: bool,
    ) {
        let font = egui::TextStyle::Body.resolve(ui.style());
        let normal = ui.visuals().text_color();
        let strong = ui.visuals().strong_text_color();
        let link_color = ui.visuals().hyperlink_color;
        let mut job = LayoutJob::default();
        for run in runs {
            let mut format = TextFormat::simple(
                font.clone(),
                if header || run.strong { strong } else { normal },
            );
            format.italics = run.emphasis;
            if run.strikethrough {
                format.strikethrough = egui::Stroke::new(1.0, format.color);
            }
            if run.kind == MdRunKind::Code {
                format.font_id = mono(ui);
                format.background = ui.visuals().code_bg_color;
            }
            if run.link.is_some() {
                format.color = link_color;
                format.underline = egui::Stroke::new(1.0, link_color);
            }
            job.append(&run.text, 0.0, format);
        }
        let link = runs.iter().find_map(|run| run.link.clone());
        let layout = match align {
            Some(MdAlignment::Right) => egui::Layout::right_to_left(egui::Align::Center),
            Some(MdAlignment::Center) => egui::Layout::top_down(egui::Align::Center),
            _ => egui::Layout::left_to_right(egui::Align::Center),
        };
        ui.with_layout(layout, |ui| {
            let label = egui::Label::new(job).sense(if link.is_some() {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            });
            let response = ui.add(label);
            if let Some(link) = &link {
                let response = response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(link);
                if response.clicked() {
                    self.follow(ui, link);
                }
            }
        });
    }

    fn image(&mut self, ui: &mut egui::Ui, url: &str, alt: &str, title: &str) {
        let muted = super::theme::muted(ui);
        let hover = if title.is_empty() { alt } else { title };
        let LinkTarget::File(path) = resolve_link(self.base_dir, url) else {
            ui.label(RichText::new(format!("[Bild: {alt}]")).color(muted))
                .on_hover_text(url);
            return;
        };
        match self.images.get(ui.ctx(), self.place, &path) {
            ImageState::Ready(texture) => {
                let size = texture.size_vec2();
                let scale = (ui.available_width().max(16.0) / size.x.max(1.0)).min(1.0);
                let sized = egui::load::SizedTexture::new(texture.id(), size * scale);
                ui.add(egui::Image::new(sized)).on_hover_text(hover);
            }
            ImageState::Loading => {
                ui.label(RichText::new(format!("[Bild wird geladen: {alt}]")).color(muted));
            }
            ImageState::Failed(error) => {
                ui.label(RichText::new(format!("[Bild: {alt}]")).color(muted))
                    .on_hover_text(error.as_str());
            }
        }
    }
}
