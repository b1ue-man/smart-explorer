//! The commands of the results panel of „In Dateien suchen“: its context
//! menu (Notepad++'s commands of „Search results“ plus VS Code's „Replace“,
//! „Replace All“ and „Dismiss“), the prompt of „In diesen Ergebnissen
//! suchen“ and the keys while the panel owns the keyboard.
use super::content_search_nav::WithinQuery;
use super::content_search_results::{Row, RowKind, SearchResults};
use super::content_search_results_ui::{
    foldable, MenuChoice, PanelAction, ResultsPanel, WithinPrompt,
};
use super::prelude::*;
use super::text_tools::content_replace_results::UndoState;

/// What the context menu offers for the current rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct MenuFlags {
    pub(in crate::app) wrap: bool,
    pub(in crate::app) clear_each: bool,
    /// The chosen rows are hits of one finished search.
    pub(in crate::app) replace: bool,
    /// The search of the chosen rows (or the newest) can be replaced.
    pub(in crate::app) remaining: bool,
    pub(in crate::app) dismiss: bool,
    pub(in crate::app) undo: bool,
}

impl ResultsPanel {
    /// The search the commands without own rows act on: that of the first
    /// chosen row, else the newest.
    fn target_search(&self, results: &SearchResults, chosen: &[Row]) -> Option<u64> {
        chosen
            .first()
            .map(|row| row.search)
            .or_else(|| results.records.first().map(|record| record.id))
    }

    pub(in crate::app) fn menu_flags(&self, results: &SearchResults, rows: &[Row]) -> MenuFlags {
        let chosen = self.chosen_in(rows);
        let replaceable = |search: Option<u64>| {
            search
                .and_then(|id| results.record(id))
                .is_some_and(|record| record.replaceable().is_ok())
        };
        let same_search = chosen
            .iter()
            .all(|row| Some(row.search) == chosen.first().map(|r| r.search));
        MenuFlags {
            wrap: self.wrap,
            clear_each: self.clear_each,
            replace: !chosen.is_empty()
                && same_search
                && replaceable(chosen.first().map(|r| r.search)),
            remaining: replaceable(self.target_search(results, &chosen)),
            dismiss: !chosen.is_empty(),
            undo: self.undo_target(results).is_some(),
        }
    }

    /// The replacement record „Rückgängig“ acts on (that of the cursor, else
    /// the newest) and the hover text naming it.
    pub(in crate::app) fn undo_target(&self, results: &SearchResults) -> Option<(u64, String)> {
        let search = self
            .cursor
            .map(|row| row.search)
            .or_else(|| results.records.first().map(|record| record.id))?;
        let record = results.record(search)?;
        let info = record.replace.as_ref()?;
        matches!(info.undo, UndoState::Available(_)).then(|| {
            (
                search,
                format!(
                    "Ersetzen „{}“ durch „{}“ rückgängig machen",
                    record.pattern, info.replacement
                ),
            )
        })
    }

    pub(in crate::app) fn apply_choice(
        &mut self,
        ctx: &egui::Context,
        results: &mut SearchResults,
        choice: MenuChoice,
        actions: &mut Vec<PanelAction>,
    ) {
        let chosen = self.chosen();
        match choice {
            MenuChoice::ExpandAll => results.set_all_expanded(true),
            MenuChoice::CollapseAll => results.set_all_expanded(false),
            MenuChoice::Copy => ctx.copy_text(results.copy_rows(&chosen)),
            MenuChoice::CopyLines => ctx.copy_text(results.copy_lines(&chosen)),
            MenuChoice::CopyPaths => ctx.copy_text(results.row_paths(&chosen).join("\n")),
            MenuChoice::OpenPaths => {
                let mut paths = Vec::new();
                for row in &chosen {
                    for path in results.row_paths(&[*row]) {
                        if !paths.iter().any(|(_, known)| *known == path) {
                            paths.push((row.search, path));
                        }
                    }
                }
                actions.push(PanelAction::OpenPaths(paths));
            }
            MenuChoice::SelectAll => self.selection = self.rows().iter().copied().collect(),
            MenuChoice::Clear => actions.push(PanelAction::Clear),
            MenuChoice::SearchWithin => {
                let source = chosen
                    .first()
                    .map(|row| row.search)
                    .or_else(|| results.records.first().map(|record| record.id));
                if let Some(source) = source {
                    self.within = Some(WithinPrompt {
                        source,
                        query: WithinQuery {
                            pattern: String::new(),
                            regex: false,
                            match_case: false,
                        },
                        error: None,
                        focus: true,
                    });
                }
            }
            MenuChoice::ToggleWrap => {
                self.wrap = !self.wrap;
                self.invalidate();
            }
            MenuChoice::ToggleClearEach => self.clear_each = !self.clear_each,
            MenuChoice::Replace => actions.push(PanelAction::Replace(chosen)),
            MenuChoice::ReplaceRemaining => {
                if let Some(search) = self.target_search(results, &chosen) {
                    actions.push(PanelAction::ReplaceRemaining(search));
                }
            }
            MenuChoice::Dismiss => {
                if results.dismiss(&chosen) {
                    self.selection.clear();
                    self.cursor = None;
                    self.anchor = None;
                    self.invalidate();
                }
            }
            MenuChoice::Undo => {
                if let Some((search, _)) = self.undo_target(results) {
                    actions.push(PanelAction::Undo(search));
                }
            }
        }
    }

    pub(in crate::app) fn show_within(
        &mut self,
        ctx: &egui::Context,
        actions: &mut Vec<PanelAction>,
    ) {
        let Some(prompt) = &mut self.within else {
            return;
        };
        let mut open = true;
        let mut submit = false;
        egui::Window::new("In diesen Ergebnissen suchen")
            .id(egui::Id::new("content_search.within"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut prompt.query.pattern).desired_width(300.0),
                );
                if std::mem::take(&mut prompt.focus) {
                    edit.request_focus();
                }
                submit |= edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut prompt.query.regex, false, "Normal");
                    ui.radio_value(&mut prompt.query.regex, true, "Regulärer Ausdruck");
                    ui.checkbox(
                        &mut prompt.query.match_case,
                        "Groß-/Kleinschreibung beachten",
                    );
                });
                if let Some(error) = &prompt.error {
                    ui.colored_label(super::theme::danger(ui), error);
                }
                submit |= ui.button("Suchen").clicked();
            });
        if submit {
            match super::content_search_nav::line_regex(
                &prompt.query.pattern,
                prompt.query.regex,
                prompt.query.match_case,
            ) {
                Ok(_) => {
                    actions.push(PanelAction::SearchWithin(
                        prompt.source,
                        prompt.query.clone(),
                    ));
                    self.within = None;
                    return;
                }
                Err(error) => prompt.error = Some(error),
            }
        }
        if !open {
            self.within = None;
        }
    }

    /// Keys while the panel owns the keyboard and no text field is focused.
    pub(in crate::app) fn keys(
        &mut self,
        ctx: &egui::Context,
        results: &mut SearchResults,
    ) -> Vec<PanelAction> {
        use egui::{Key, Modifiers};
        let mut actions = Vec::new();
        let rows = self.rows().to_vec();
        if rows.is_empty() {
            return actions;
        }
        let position = self
            .cursor
            .and_then(|cursor| rows.iter().position(|row| *row == cursor));
        let mut moved = None;
        let mut choice = None;
        let mut activate = false;
        let mut fold: Option<bool> = None;
        let shift = ctx.input(|input| input.modifiers.shift);
        ctx.input_mut(|input| {
            for modifiers in [Modifiers::NONE, Modifiers::SHIFT] {
                if input.consume_key(modifiers, Key::ArrowDown) {
                    moved = Some(position.map_or(0, |p| (p + 1).min(rows.len() - 1)));
                }
                if input.consume_key(modifiers, Key::ArrowUp) {
                    moved = Some(position.map_or(0, |p| p.saturating_sub(1)));
                }
                if input.consume_key(modifiers, Key::Home) {
                    moved = Some(0);
                }
                if input.consume_key(modifiers, Key::End) {
                    moved = Some(rows.len() - 1);
                }
            }
            if input.consume_key(Modifiers::NONE, Key::Enter) {
                activate = true;
            }
            if input.consume_key(Modifiers::NONE, Key::ArrowLeft) {
                fold = Some(false);
            }
            if input.consume_key(Modifiers::NONE, Key::ArrowRight) {
                fold = Some(true);
            }
            if input.consume_key(Modifiers::COMMAND, Key::A) {
                choice = Some(MenuChoice::SelectAll);
            }
            if input.consume_key(Modifiers::COMMAND, Key::C) {
                choice = Some(MenuChoice::Copy);
            }
            if input.consume_key(Modifiers::NONE, Key::Escape) {
                self.selection.clear();
            }
            // Entf: „Aus dem Ergebnis entfernen“ (VS Code „Dismiss“).
            if input.consume_key(Modifiers::NONE, Key::Delete) {
                choice = Some(MenuChoice::Dismiss);
            }
        });
        if let Some(index) = moved {
            self.select(rows[index], false, shift);
            self.scroll_to_cursor = true;
        }
        if let Some(cursor) = self.cursor {
            if activate {
                if matches!(cursor.kind, RowKind::Line(..)) {
                    actions.push(PanelAction::Open(cursor));
                } else {
                    results.toggle(cursor);
                }
            }
            if let Some(expand) = fold {
                let open = match cursor.kind {
                    RowKind::Header => results.record(cursor.search).is_some_and(|r| r.expanded),
                    RowKind::File(file) => results
                        .record(cursor.search)
                        .and_then(|r| r.files.get(file))
                        .is_some_and(|f| f.expanded),
                    RowKind::SkippedHeader => results
                        .record(cursor.search)
                        .is_some_and(|r| r.skipped_open),
                    _ => expand,
                };
                if foldable(cursor.kind) && open != expand {
                    results.toggle(cursor);
                }
            }
        }
        if let Some(choice) = choice {
            self.apply_choice(ctx, results, choice, &mut actions);
        }
        actions
    }
}

pub(in crate::app) fn menu(ui: &mut egui::Ui, flags: MenuFlags, choice: &mut Option<MenuChoice>) {
    let MenuFlags {
        wrap, clear_each, ..
    } = flags;
    let mut pick = |ui: &mut egui::Ui, label: &str, value: MenuChoice| {
        if ui.button(label).clicked() {
            *choice = Some(value);
            ui.close_menu();
        }
    };
    pick(ui, "Alle aufklappen", MenuChoice::ExpandAll);
    pick(ui, "Alle zuklappen", MenuChoice::CollapseAll);
    ui.separator();
    pick(ui, "Kopieren", MenuChoice::Copy);
    pick(ui, "Ausgewählte Zeilen kopieren", MenuChoice::CopyLines);
    pick(ui, "Pfade kopieren", MenuChoice::CopyPaths);
    pick(ui, "Pfade öffnen (Textansicht)", MenuChoice::OpenPaths);
    ui.separator();
    pick(ui, "Alles auswählen", MenuChoice::SelectAll);
    pick(ui, "Leeren", MenuChoice::Clear);
    ui.separator();
    pick(
        ui,
        "In diesen Ergebnissen suchen…",
        MenuChoice::SearchWithin,
    );
    ui.separator();
    let replace_items = [
        (flags.replace, "Ersetzen…", MenuChoice::Replace),
        (
            flags.remaining,
            "Alle verbleibenden ersetzen…",
            MenuChoice::ReplaceRemaining,
        ),
        (
            flags.dismiss,
            "Aus dem Ergebnis entfernen (Entf)",
            MenuChoice::Dismiss,
        ),
        (flags.undo, "Ersetzen rückgängig machen", MenuChoice::Undo),
    ];
    for (enabled, label, value) in replace_items {
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            *choice = Some(value);
            ui.close_menu();
        }
    }
    ui.separator();
    let mut wrapped = wrap;
    if ui
        .checkbox(&mut wrapped, "Lange Zeilen umbrechen")
        .clicked()
    {
        *choice = Some(MenuChoice::ToggleWrap);
        ui.close_menu();
    }
    let mut each = clear_each;
    if ui.checkbox(&mut each, "Bei jeder Suche leeren").clicked() {
        *choice = Some(MenuChoice::ToggleClearEach);
        ui.close_menu();
    }
}
