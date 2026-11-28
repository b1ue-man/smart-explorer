//! The dialog „In Dateien suchen“ (Strg+Umschalt+F) with the fields of
//! Notepad++'s „Find in Files“ (plan docs/plaene/2026-11-27-dateien-suche-ansicht,
//! B1) plus the two folder-path regexes the user asked for, and „Ersetzen
//! durch“ with „In Dateien ersetzen“ (Strg+Umschalt+H; plan
//! docs/plaene/2026-11-28-ersetzen-masken-shell, part C). The form turns
//! into a `ContentQuery`; an invalid pattern, filter or regex is reported in
//! the dialog and starts no search.
use super::prelude::*;
use crate::agent_proto::{ContentQuery, TextMode};

use super::text_tools::TextPlace;

/// Tooltip of the filter field: Notepad++'s syntax with its examples.
pub(in crate::app) const FILTER_TIP: &str = "Muster durch Leerzeichen getrennt, ohne \
Anführungszeichen; * und ? gelten nur für den Dateinamen.\n\
!muster schließt Dateien aus (gewinnt immer), !\\muster schließt Ordner der ersten Ebene \
nach dem Namen aus, !+\\muster Ordner jeder Ebene. Leer = *.*\n\
Beispiele:\n  *.cpp *.cxx *.h *.hxx *.hpp\n  *.* !*.exe !*.obj !*.log\n  *.* !\\tests !\\bin*\n  \
*.* !+\\log*";
/// Tooltip of the two folder-path regex fields.
pub(in crate::app) const FOLDER_REGEX_TIP: &str = "Regulärer Ausdruck auf den Ordnerpfad \
relativ zum gewählten Ordner, mit / getrennt, ohne / am Anfang und Ende (der Ordner selbst \
ist leer). Zusätzlich zu Notepad++.\nBeispiel: ^src/(core|os)(/|$)";
/// Tooltip of „Ersetzen durch“: the syntax follows the search mode.
pub(in crate::app) const REPLACE_TIP: &str =
    "Normal: wörtlich. Erweitert: \\n, \\r, \\t, \\0, \\x… \
wie im Suchtext. Regulärer Ausdruck: $0 oder $&, $1 oder ${1}, $+{name}, \\U…\\E, \\L, \\u, \\l, \
Bedingungen ?1ja:nein (wie Notepad++). Leer löscht die Treffer.";
/// Tooltip of „Ignorierdateien beachten“.
pub(in crate::app) const IGNORE_TIP: &str =
    "Überspringt, was .gitignore (in einem Git-Arbeitsbaum), \
.ignore und .git/info/exclude ausschließen, wie ripgrep. Aus = wie Notepad++.";
const HISTORY_FILE: &str = "find_in_files_history.json";
const HISTORY_LEN: usize = 20;

/// The fields of the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct FindForm {
    pub(in crate::app) pattern: String,
    pub(in crate::app) replacement: String,
    pub(in crate::app) filters: String,
    pub(in crate::app) folder: String,
    pub(in crate::app) folder_include: String,
    pub(in crate::app) folder_exclude: String,
    pub(in crate::app) subfolders: bool,
    pub(in crate::app) hidden: bool,
    pub(in crate::app) whole_word: bool,
    pub(in crate::app) match_case: bool,
    pub(in crate::app) mode: TextMode,
    pub(in crate::app) dot_matches_newline: bool,
    pub(in crate::app) context_lines: u32,
    /// `ContentQuery::respect_ignore_files` (off as in Notepad++).
    pub(in crate::app) respect_ignore_files: bool,
}

impl Default for FindForm {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            replacement: String::new(),
            filters: String::new(),
            folder: String::new(),
            folder_include: String::new(),
            folder_exclude: String::new(),
            subfolders: true,
            hidden: false,
            whole_word: false,
            match_case: false,
            mode: TextMode::Normal,
            dot_matches_newline: false,
            context_lines: 0,
            respect_ignore_files: false,
        }
    }
}

fn optional(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// A folder typed by hand: `\` as `/`, no trailing `/` except for a root.
pub(in crate::app) fn normalize_folder(folder: &str) -> String {
    let mut path = folder.trim().replace('\\', "/");
    while path.len() > 1 && path.ends_with('/') && !path.ends_with(":/") {
        path.pop();
    }
    path
}

impl FindForm {
    /// Notepad++ disables „Match whole word only“ for regular expressions.
    pub(in crate::app) fn whole_word_enabled(&self) -> bool {
        self.mode != TextMode::Regex
    }

    /// „. findet Zeilenumbruch“ exists only for regular expressions.
    pub(in crate::app) fn dot_enabled(&self) -> bool {
        self.mode == TextMode::Regex
    }

    pub(in crate::app) fn query(&self) -> ContentQuery {
        let mut query = ContentQuery::text(self.pattern.clone());
        query.mode = self.mode;
        query.match_case = self.match_case;
        query.whole_word = self.whole_word && self.whole_word_enabled();
        query.dot_matches_newline = self.dot_matches_newline && self.dot_enabled();
        query.filters = self.filters.trim().to_string();
        query.folder_include = optional(&self.folder_include);
        query.folder_exclude = optional(&self.folder_exclude);
        query.subfolders = self.subfolders;
        query.hidden = self.hidden;
        query.context_lines = self.context_lines;
        query.respect_ignore_files = self.respect_ignore_files;
        query
    }

    /// The query and the folder, or the German message that keeps the search
    /// from starting (empty text, invalid pattern/regex/filter, no folder).
    pub(in crate::app) fn validate(&self) -> Result<(ContentQuery, String), String> {
        if self.pattern.is_empty() {
            return Err("Bitte einen Suchtext eingeben.".to_string());
        }
        let folder = normalize_folder(&self.folder);
        if folder.is_empty() {
            return Err("Bitte einen Ordner angeben.".to_string());
        }
        let query = self.query();
        crate::agent_proto::prepare_content_search(&query).map_err(|error| error.to_string())?;
        Ok((query, folder))
    }
}

/// Recently searched texts and filters, newest first, kept in the app data.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(in crate::app) struct FindHistory {
    #[serde(default)]
    pub(in crate::app) patterns: Vec<String>,
    #[serde(default)]
    pub(in crate::app) filters: Vec<String>,
    /// Texts of „Ersetzen durch“ (a file without the key reads as empty).
    #[serde(default)]
    pub(in crate::app) replacements: Vec<String>,
}

fn remember_in(list: &mut Vec<String>, value: &str) {
    if value.trim().is_empty() {
        return;
    }
    list.retain(|entry| entry != value);
    list.insert(0, value.to_string());
    list.truncate(HISTORY_LEN);
}

impl FindHistory {
    pub(in crate::app) fn remember(&mut self, form: &FindForm) {
        remember_in(&mut self.patterns, &form.pattern);
        remember_in(&mut self.filters, &form.filters);
    }

    /// Also keeps the replacement (an empty one deletes, and is not kept).
    pub(in crate::app) fn remember_replacement(&mut self, form: &FindForm) {
        self.remember(form);
        remember_in(&mut self.replacements, &form.replacement);
    }

    fn load() -> Self {
        std::fs::read(super::support_paths::app_data_file(HISTORY_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        std::fs::write(super::support_paths::app_data_file(HISTORY_FILE), bytes)
            .map_err(|error| format!("Suchverlauf speichern: {error}"))
    }
}

/// What the dialog asks the app for.
pub(in crate::app) enum DialogAction {
    None,
    Search(ContentQuery, String, TextPlace),
    /// „In Dateien ersetzen“: query, folder, place, replacement text.
    Replace(ContentQuery, String, TextPlace, String),
    /// „<<“: take the folder and place of the active tab.
    TakeActiveFolder,
    Cancel,
}

#[derive(Default)]
pub(in crate::app) struct FindDialog {
    pub(in crate::app) open: bool,
    pub(in crate::app) form: FindForm,
    pub(in crate::app) place: TextPlace,
    pub(in crate::app) error: Option<String>,
    pub(in crate::app) focus_pattern: bool,
    /// Strg+Umschalt+H: focus „Ersetzen durch“ on the next frame.
    pub(in crate::app) focus_replacement: bool,
    history: Option<FindHistory>,
}

/// Key of the dialog window (its layer id, for keyboard ownership).
pub(in crate::app) const DIALOG_KEY: &str = "content_search.dialog";

impl FindDialog {
    pub(in crate::app) fn open_for(&mut self, folder: String, place: TextPlace) {
        if self.history.is_none() {
            self.history = Some(FindHistory::load());
        }
        self.form.folder = folder;
        self.place = place;
        self.error = None;
        self.open = true;
        self.focus_pattern = true;
    }

    /// Strg+Umschalt+H: the same dialog with „Ersetzen durch“ focused (the
    /// pattern field while it is still empty).
    pub(in crate::app) fn open_for_replace(&mut self, folder: String, place: TextPlace) {
        self.open_for(folder, place);
        if !self.form.pattern.is_empty() {
            self.focus_pattern = false;
            self.focus_replacement = true;
        }
    }

    /// A replacement typed outside the dialog (the panel's „Ersetzen…“)
    /// becomes the field's text and joins its history.
    pub(in crate::app) fn remember_replacement_text(&mut self, text: &str) {
        self.form.replacement = text.to_string();
        let history = self.history.get_or_insert_with(FindHistory::load);
        remember_in(&mut history.replacements, text);
        if let Err(error) = history.save() {
            self.error = Some(error);
        }
    }

    pub(in crate::app) fn window_id() -> egui::Id {
        egui::Id::new(DIALOG_KEY)
    }

    /// Checks the form; on success remembers it in the history.
    fn submit(&mut self) -> DialogAction {
        self.checked(false)
    }

    /// „In Dateien ersetzen“: the same checks, the replacement kept as well.
    pub(in crate::app) fn submit_replace(&mut self) -> DialogAction {
        self.checked(true)
    }

    fn checked(&mut self, replace: bool) -> DialogAction {
        match self.form.validate() {
            Ok((query, folder)) => {
                self.error = None;
                let history = self.history.get_or_insert_with(FindHistory::default);
                if replace {
                    history.remember_replacement(&self.form);
                } else {
                    history.remember(&self.form);
                }
                if let Err(error) = history.save() {
                    self.error = Some(error);
                }
                if replace {
                    let text = self.form.replacement.clone();
                    DialogAction::Replace(query, folder, self.place.clone(), text)
                } else {
                    DialogAction::Search(query, folder, self.place.clone())
                }
            }
            Err(error) => {
                self.error = Some(error);
                DialogAction::None
            }
        }
    }

    /// Draws the dialog; `progress` is the running search (files, bytes,
    /// hits), `replacing` the line of a running replacement.
    pub(in crate::app) fn show(
        &mut self,
        ctx: &egui::Context,
        progress: Option<(u64, u64, u64)>,
        replacing: Option<String>,
    ) -> DialogAction {
        let mut open = self.open;
        let mut action = DialogAction::None;
        super::dialog_geometry::window(ctx, DIALOG_KEY, "In Dateien suchen")
            .open(&mut open)
            .default_size([560.0, 470.0])
            .show(ctx, |ui| action = self.ui(ui, progress, replacing));
        self.open = open && self.open;
        action
    }

    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        progress: Option<(u64, u64, u64)>,
        replacing: Option<String>,
    ) -> DialogAction {
        let mut action = DialogAction::None;
        let mut enter = false;
        let history = self.history.clone().unwrap_or_default();
        egui::Grid::new("content_search_fields")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Suchen nach:");
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.form.pattern).desired_width(340.0),
                    );
                    if self.focus_pattern {
                        edit.request_focus();
                        self.focus_pattern = false;
                    }
                    enter |= edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    history_menu(
                        ui,
                        "find_pattern_history",
                        &history.patterns,
                        &mut self.form.pattern,
                    );
                });
                ui.end_row();
                ui.label("Ersetzen durch:");
                ui.horizontal(|ui| {
                    let edit = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.form.replacement)
                                .desired_width(340.0),
                        )
                        .on_hover_text(REPLACE_TIP);
                    if std::mem::take(&mut self.focus_replacement) {
                        edit.request_focus();
                    }
                    history_menu(
                        ui,
                        "find_replacement_history",
                        &history.replacements,
                        &mut self.form.replacement,
                    );
                });
                ui.end_row();
                ui.label("Filter:");
                ui.horizontal(|ui| {
                    let edit = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.form.filters)
                                .hint_text("*.*")
                                .desired_width(340.0),
                        )
                        .on_hover_text(FILTER_TIP);
                    enter |= edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    history_menu(
                        ui,
                        "find_filter_history",
                        &history.filters,
                        &mut self.form.filters,
                    );
                });
                ui.end_row();
                ui.label("Ordner:");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.form.folder).desired_width(300.0));
                    if ui
                        .button("<<")
                        .on_hover_text("Ordner des aktiven Tabs übernehmen")
                        .clicked()
                    {
                        action = DialogAction::TakeActiveFolder;
                    }
                });
                ui.end_row();
                ui.label("");
                ui.label(
                    RichText::new(format!("auf: {}", self.place.label))
                        .color(super::theme::muted(ui)),
                );
                ui.end_row();
                ui.label("Ordnerpfad (Regex):");
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.folder_include).desired_width(340.0),
                )
                .on_hover_text(FOLDER_REGEX_TIP);
                ui.end_row();
                ui.label("Ordnerpfad ausschließen (Regex):");
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.folder_exclude).desired_width(340.0),
                )
                .on_hover_text(FOLDER_REGEX_TIP);
                ui.end_row();
            });
        ui.separator();
        self.ui_options(ui);
        ui.separator();
        if let Some(error) = &self.error {
            ui.colored_label(super::theme::danger(ui), error);
        }
        if let Some((files, bytes, hits)) = progress {
            ui.label(format!(
                "Läuft: {} Dateien, {}, {} Treffer",
                super::content_search_results::group_thousands(files),
                format_bytes(bytes),
                super::content_search_results::group_thousands(hits)
            ));
        }
        let busy = progress.is_some() || replacing.is_some();
        if let Some(line) = replacing {
            ui.label(line);
        }
        ui.horizontal(|ui| {
            if ui.button("Alle suchen").clicked() || enter {
                action = self.submit();
            }
            if ui
                .button("In Dateien ersetzen")
                .on_hover_text("Erst zählt eine Vorschau die Stellen, dann fragt eine Bestätigung.")
                .clicked()
            {
                action = self.submit_replace();
            }
            if busy && ui.button("Abbrechen").clicked() {
                action = DialogAction::Cancel;
            }
            if ui.button("Schließen").clicked() {
                self.open = false;
            }
        });
        action
    }

    fn ui_options(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.form.subfolders, "In allen Unterordnern");
            ui.checkbox(&mut self.form.hidden, "In versteckten Ordnern")
                .on_hover_text("Versteckte Dateien werden immer durchsucht, wie in Notepad++.");
            let word_enabled = self.form.whole_word_enabled();
            ui.add_enabled(
                word_enabled,
                egui::Checkbox::new(&mut self.form.whole_word, "Nur ganze Wörter"),
            )
            .on_disabled_hover_text("Bei regulären Ausdrücken gesperrt (wie in Notepad++).");
            ui.checkbox(&mut self.form.match_case, "Groß-/Kleinschreibung beachten");
            ui.checkbox(
                &mut self.form.respect_ignore_files,
                "Ignorierdateien beachten (.gitignore)",
            )
            .on_hover_text(IGNORE_TIP);
        });
        ui.horizontal(|ui| {
            ui.label("Kontextzeilen:");
            ui.add(egui::DragValue::new(&mut self.form.context_lines).range(0..=50));
        });
        ui.label("Suchmodus:");
        ui.horizontal_wrapped(|ui| {
            ui.radio_value(&mut self.form.mode, TextMode::Normal, "Normal");
            ui.radio_value(
                &mut self.form.mode,
                TextMode::Extended,
                "Erweitert (\\n, \\r, \\t, \\0, \\x…)",
            );
            ui.radio_value(&mut self.form.mode, TextMode::Regex, "Regulärer Ausdruck");
            let dot_enabled = self.form.dot_enabled();
            ui.add_enabled(
                dot_enabled,
                egui::Checkbox::new(&mut self.form.dot_matches_newline, ". findet Zeilenumbruch"),
            );
        });
    }
}

fn history_menu(ui: &mut egui::Ui, id: &str, entries: &[String], target: &mut String) {
    ui.add_enabled_ui(!entries.is_empty(), |ui| {
        ui.push_id(id, |ui| {
            ui.menu_button("▾", |ui| {
                for entry in entries {
                    if ui.button(entry).clicked() {
                        *target = entry.clone();
                        ui.close_menu();
                    }
                }
            })
            .response
            .on_hover_text("Verlauf");
        });
    });
}
