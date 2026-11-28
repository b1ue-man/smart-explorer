//! The text view window („Ansicht“): an `egui::Window` per file inside the
//! main window (several at once; same keyboard routing, textures and
//! renderer as every other window of the app). Toolbar (language, wrap,
//! Markdown Darstellung/Quelltext, find, go to), find bar, go-to bar, the
//! line body or the rendered Markdown, and the status line.
use super::md_links::parent_dir;
use super::md_render::{MdAction, MdRenderer};
use super::prelude::*;
use super::text_view_window::{TextViewWindow, ViewState};

/// What a window asks the app for.
pub(in crate::app) enum ViewAction {
    /// A link of the Markdown file: open this file (same place) in a view.
    OpenFile(String),
}

pub(in crate::app) fn theme_kind(ctx: &egui::Context) -> crate::textview::ThemeKind {
    if ctx.style().visuals.dark_mode {
        crate::textview::ThemeKind::Dark
    } else {
        crate::textview::ThemeKind::Light
    }
}

fn percent(done: u64, total: u64) -> u64 {
    if total == 0 {
        100
    } else {
        (done.saturating_mul(100) / total).min(100)
    }
}

impl TextViewWindow {
    pub(in crate::app) fn show(&mut self, ctx: &egui::Context) -> Vec<ViewAction> {
        self.poll(ctx);
        self.set_theme(theme_kind(ctx));
        let mut actions = Vec::new();
        let mut open = self.open;
        let limit = super::theme::window_content_limit(ctx);
        egui::Window::new(format!("{} – Ansicht", self.name))
            .id(self.window_id())
            .open(&mut open)
            .default_size(egui::vec2(820.0, 600.0).min(limit))
            .max_size(limit)
            .resizable(true)
            .vscroll(false)
            .collapsible(true)
            .show(ctx, |ui| self.contents(ui, &mut actions));
        self.open = open && self.open;
        if self.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        actions
    }

    fn contents(&mut self, ui: &mut egui::Ui, actions: &mut Vec<ViewAction>) {
        ui.label(
            RichText::new(format!("{} · {}", self.path, self.place.label))
                .small()
                .color(super::theme::muted(ui)),
        );
        match &self.state {
            ViewState::Loading => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Wird geöffnet …");
                });
                return;
            }
            ViewState::Failed(error) => {
                ui.add_space(12.0);
                ui.colored_label(super::theme::danger(ui), error.as_str());
                return;
            }
            ViewState::Ready => {}
        }
        self.toolbar(ui);
        if self.find.open {
            self.find_bar(ui);
        }
        if self.goto.open {
            self.goto_bar(ui);
        }
        let status = self.status_text();
        egui::TopBottomPanel::bottom(egui::Id::new(("text_view_status", self.id))).show_inside(
            ui,
            |ui| {
                ui.label(RichText::new(status).small());
            },
        );
        let rendered = self
            .markdown
            .as_ref()
            .is_some_and(|markdown| markdown.rendered);
        egui::CentralPanel::default().show_inside(ui, |ui| {
            if rendered {
                self.markdown_body(ui, actions);
            } else {
                self.body(ui);
            }
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let mut chosen = None;
            egui::ComboBox::from_id_salt(("text_view_language", self.id))
                .selected_text(self.language.as_str())
                .width(170.0)
                .show_ui(ui, |ui| {
                    let plain = crate::textview::PLAIN_TEXT;
                    if !self.languages.iter().any(|language| language.name == plain)
                        && ui.selectable_label(self.language == plain, plain).clicked()
                    {
                        chosen = Some(plain.to_string());
                    }
                    for language in &self.languages {
                        let response = ui
                            .selectable_label(self.language == language.name, &language.name)
                            .on_hover_text(language.extensions.join(", "));
                        if response.clicked() {
                            chosen = Some(language.name.clone());
                        }
                    }
                })
                .response
                .on_hover_text("Sprache der Hervorhebung");
            if let Some(language) = chosen {
                self.set_language(language);
            }
            if let Some(markdown) = &mut self.markdown {
                ui.separator();
                ui.selectable_value(&mut markdown.rendered, true, "Darstellung");
                ui.selectable_value(&mut markdown.rendered, false, "Quelltext");
            }
            ui.separator();
            ui.toggle_value(&mut self.wrap, "Umbruch")
                .on_hover_text("Lange Zeilen umbrechen");
            if ui
                .button("Suchen")
                .on_hover_text("In der Datei suchen (Strg+F)")
                .clicked()
            {
                self.open_find();
            }
            if ui
                .button("Gehe zu")
                .on_hover_text("Gehe zu Zeile (Strg+G)")
                .clicked()
            {
                self.open_goto();
            }
            if self.selection.is_some()
                && ui
                    .button("Zeilen kopieren")
                    .on_hover_text("Ausgewählte Zeilen kopieren (Strg+C)")
                    .clicked()
            {
                self.copy_selection();
            }
        });
    }

    pub(in crate::app) fn open_find(&mut self) {
        if let Some(markdown) = &mut self.markdown {
            markdown.rendered = false;
        }
        self.find.open = true;
        self.find.focus = true;
    }

    pub(in crate::app) fn open_goto(&mut self) {
        if let Some(markdown) = &mut self.markdown {
            markdown.rendered = false;
        }
        self.goto.open = true;
        self.goto.focus = true;
        self.goto.error = None;
    }

    fn find_bar(&mut self, ui: &mut egui::Ui) {
        let mut search = None;
        ui.horizontal_wrapped(|ui| {
            ui.label("Suchen:");
            let edit = ui.add(egui::TextEdit::singleline(&mut self.find.text).desired_width(220.0));
            if std::mem::take(&mut self.find.focus) {
                edit.request_focus();
            }
            if edit.changed() {
                self.find.current = None;
                self.find.status = None;
            }
            if edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                search = Some(!ui.input(|input| input.modifiers.shift));
                self.find.focus = true;
            }
            ui.radio_value(&mut self.find.regex, false, "Normal");
            ui.radio_value(&mut self.find.regex, true, "Regex");
            ui.checkbox(&mut self.find.match_case, "Groß/Klein");
            if ui
                .button("▲")
                .on_hover_text("Voriger Treffer (Umschalt+F3)")
                .clicked()
            {
                search = Some(false);
            }
            if ui
                .button("▼")
                .on_hover_text("Nächster Treffer (F3)")
                .clicked()
            {
                search = Some(true);
            }
            if let Some(status) = &self.find.status {
                ui.label(RichText::new(status).color(super::theme::muted(ui)));
            }
            if ui.button("×").on_hover_text("Schließen (Esc)").clicked() {
                self.find.open = false;
            }
        });
        if let Some(forward) = search {
            self.start_find(forward);
        }
    }

    fn goto_bar(&mut self, ui: &mut egui::Ui) {
        let mut apply = false;
        ui.horizontal_wrapped(|ui| {
            let range = if self.index_complete() {
                format!(
                    "Gehe zu Zeile (1–{}):",
                    super::content_search_results::group_thousands(self.known_lines())
                )
            } else {
                "Gehe zu Zeile:".to_string()
            };
            ui.label(range);
            let edit = ui.add(egui::TextEdit::singleline(&mut self.goto.text).desired_width(100.0));
            if std::mem::take(&mut self.goto.focus) {
                edit.request_focus();
            }
            apply |= edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            apply |= ui.button("Los").clicked();
            if let Some(error) = &self.goto.error {
                ui.colored_label(super::theme::danger(ui), error.as_str());
            }
            if ui.button("×").clicked() {
                self.goto.open = false;
            }
        });
        if apply {
            self.apply_goto();
        }
    }

    /// „UTF-8 · LF · 1.234 Zeilen · Rust · 12 KB“.
    pub(in crate::app) fn status_text(&self) -> String {
        let mut parts = Vec::new();
        let encoding = self
            .status
            .as_ref()
            .map(|status| status.encoding)
            .or_else(|| self.doc.as_ref().map(|doc| doc.encoding()));
        if let Some(encoding) = encoding {
            parts.push(encoding.label().to_string());
        }
        if let Some(status) = &self.status {
            let endings = match status.endings.dominant() {
                Some(ending) if status.endings.is_mixed() => {
                    format!("{} (gemischt)", ending.label())
                }
                Some(ending) => ending.label().to_string(),
                None => "ohne Zeilenende".to_string(),
            };
            parts.push(endings);
            let lines = super::content_search_results::group_thousands(status.lines);
            parts.push(if status.complete {
                format!("{lines} Zeilen")
            } else {
                format!(
                    "{lines} Zeilen bisher (Index {} %)",
                    percent(status.bytes_done, status.total_bytes)
                )
            });
        } else {
            parts.push("Zeilenindex wird erstellt …".to_string());
        }
        parts.push(self.language.clone());
        if let Some(doc) = &self.doc {
            parts.push(format_bytes(doc.len()));
        }
        if let Some(cursor) = self.cursor {
            parts.push(format!(
                "Zeile {}",
                super::content_search_results::group_thousands(cursor + 1)
            ));
        }
        if let Some(error) = &self.index_error {
            parts.push(format!("Zeilenindex: {error}"));
        }
        if let Some(message) = &self.message {
            parts.push(message.clone());
        }
        parts.join(" · ")
    }

    fn markdown_body(&mut self, ui: &mut egui::Ui, actions: &mut Vec<ViewAction>) {
        self.ensure_markdown();
        let base = parent_dir(&self.path);
        let place = self.place.clone();
        let id = self.id;
        let Some(markdown) = &mut self.markdown else {
            return;
        };
        let super::text_view_window::MdView {
            doc,
            images,
            anchor,
            ..
        } = markdown;
        let Some(doc) = doc.as_ref() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Markdown wird gelesen …");
            });
            return;
        };
        let mut renderer = MdRenderer::new(&base, &place, images);
        renderer.scroll_to = anchor.take();
        egui::ScrollArea::vertical()
            .id_salt(("text_view_md", id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width());
                renderer.blocks(ui, &doc.blocks, 0);
            });
        for action in renderer.actions {
            match action {
                MdAction::OpenFile(path) => actions.push(ViewAction::OpenFile(path)),
                MdAction::Anchor(name) => *anchor = Some(name.to_lowercase()),
            }
        }
    }

    /// Keys while this window owns the keyboard.
    pub(in crate::app) fn keys(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        let typing = ctx.wants_keyboard_input();
        let mut find = None;
        let mut open_find = false;
        let mut open_goto = false;
        let mut copy = false;
        let mut escape = false;
        let mut scroll: Option<i64> = None;
        let mut to_end: Option<bool> = None;
        let page = self.visible_rows.max(1) as i64;
        ctx.input_mut(|input| {
            if input.consume_key(Modifiers::COMMAND, Key::F) {
                open_find = true;
            }
            if input.consume_key(Modifiers::COMMAND, Key::G) {
                open_goto = true;
            }
            if input.consume_key(Modifiers::SHIFT, Key::F3) {
                find = Some(false);
            }
            if input.consume_key(Modifiers::NONE, Key::F3) {
                find = Some(true);
            }
            if input.consume_key(Modifiers::NONE, Key::Escape) {
                escape = true;
            }
            if typing {
                return;
            }
            if self.selection.is_some() && input.consume_key(Modifiers::COMMAND, Key::C) {
                copy = true;
            }
            for (key, lines) in [
                (Key::ArrowDown, 1),
                (Key::ArrowUp, -1),
                (Key::PageDown, page),
                (Key::PageUp, -page),
            ] {
                if input.consume_key(Modifiers::NONE, key) {
                    scroll = Some(lines);
                }
            }
            if input.consume_key(Modifiers::COMMAND, Key::Home) {
                to_end = Some(false);
            }
            if input.consume_key(Modifiers::COMMAND, Key::End) {
                to_end = Some(true);
            }
        });
        if open_find {
            self.open_find();
        }
        if open_goto {
            self.open_goto();
        }
        if let Some(forward) = find {
            if self
                .markdown
                .as_ref()
                .is_some_and(|markdown| markdown.rendered)
            {
                self.open_find();
            }
            self.start_find(forward);
        }
        if escape {
            if self.goto.open {
                self.goto.open = false;
            } else if self.find.open {
                self.find.open = false;
            } else {
                self.selection = None;
            }
        }
        if copy {
            self.copy_selection();
        }
        if let Some(lines) = scroll {
            self.scroll_by(lines);
        }
        match to_end {
            Some(true) => self.scroll_to(u64::MAX),
            Some(false) => self.scroll_to(0),
            None => {}
        }
    }
}
