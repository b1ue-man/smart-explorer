//! „In Dateien suchen“, its results panel and the text view windows of the
//! desktop app (plan docs/plaene/2026-11-27-dateien-suche-ansicht, E;
//! surfaces docs/oberflaechen/dateisuche.md and textansicht.md). One state
//! value on `App`; the frame calls `ui_text_tools_panel` before the central
//! panel and `ui_text_tools_windows` after it. A pointer press decides which
//! of these surfaces owns the keyboard; while one does, the explorer's own
//! shortcuts stay off.
use super::content_search_dialog::{DialogAction, FindDialog};
use super::content_search_results::{RowKind, SearchRecord, SearchResults};
use super::content_search_results_ui::{PanelAction, ResultsPanel};
use super::content_search_run::SearchRun;
use super::prelude::*;
use super::settings_languages::LanguagesPage;
use super::text_view_ui::{theme_kind, ViewAction};
use super::text_view_window::TextViewWindow;
use crate::app::App;

/// Where files are read: this computer, or the backend of a remote tab.
#[derive(Clone)]
pub(in crate::app) struct TextPlace {
    pub(in crate::app) backend: Option<crate::vfs::BackendHandle>,
    pub(in crate::app) label: String,
}

impl Default for TextPlace {
    fn default() -> Self {
        Self::local()
    }
}

impl TextPlace {
    pub(in crate::app) fn local() -> Self {
        Self {
            backend: None,
            label: "Dieser Computer".to_string(),
        }
    }

    pub(in crate::app) fn remote(backend: crate::vfs::BackendHandle, label: String) -> Self {
        Self {
            backend: Some(backend),
            label,
        }
    }

    pub(in crate::app) fn same(&self, other: &TextPlace) -> bool {
        match (&self.backend, &other.backend) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// The backend a search of `root` runs on (local: one over that folder).
    pub(in crate::app) fn backend_for(&self, root: &str) -> crate::vfs::BackendHandle {
        self.backend
            .clone()
            .unwrap_or_else(|| Arc::new(crate::vfs::LocalBackend::new(root)))
    }
}

/// Which of the surfaces receives keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) enum KeyOwner {
    #[default]
    Explorer,
    Dialog,
    Results,
    View(u64),
}

#[derive(Default)]
pub(in crate::app) struct TextTools {
    pub(in crate::app) dialog: FindDialog,
    pub(in crate::app) results: SearchResults,
    pub(in crate::app) panel: ResultsPanel,
    pub(in crate::app) runs: Vec<SearchRun>,
    pub(in crate::app) views: Vec<TextViewWindow>,
    pub(in crate::app) languages: LanguagesPage,
    pub(in crate::app) owner: KeyOwner,
    next_view: u64,
}

impl TextTools {
    /// A press on a surface gives it the keyboard; a press elsewhere in the
    /// main window gives it back to the explorer. Popups keep the owner.
    pub(in crate::app) fn track_pointer(&mut self, ctx: &egui::Context) {
        let press = ctx.input(|input| {
            input
                .pointer
                .any_pressed()
                .then(|| input.pointer.interact_pos())
                .flatten()
        });
        let Some(position) = press else {
            return;
        };
        let Some(layer) = ctx.layer_id_at(position) else {
            self.owner = KeyOwner::Explorer;
            return;
        };
        if matches!(
            layer.order,
            egui::Order::Foreground | egui::Order::Tooltip | egui::Order::Debug
        ) {
            return;
        }
        self.owner = if layer.id == FindDialog::window_id() {
            KeyOwner::Dialog
        } else if let Some(view) = self.views.iter().find(|view| view.window_id() == layer.id) {
            KeyOwner::View(view.id)
        } else if layer.order == egui::Order::Background
            && self.panel.visible
            && self.panel.rect.is_some_and(|rect| rect.contains(position))
        {
            KeyOwner::Results
        } else {
            KeyOwner::Explorer
        };
    }

    pub(in crate::app) fn owns_keyboard(&self) -> bool {
        self.owner != KeyOwner::Explorer
    }

    pub(in crate::app) fn running(&self) -> bool {
        !self.runs.is_empty()
    }

    pub(in crate::app) fn cancel_searches(&mut self) {
        for run in &self.runs {
            run.cancel();
        }
    }

    /// Starts a search; the results panel shows it at the top.
    pub(in crate::app) fn start_search(
        &mut self,
        query: crate::agent_proto::ContentQuery,
        folder: String,
        place: TextPlace,
    ) -> Result<(), String> {
        if self.panel.clear_each {
            self.cancel_searches();
        }
        let id = self.results.next_id();
        let run = SearchRun::start(id, &place, folder.clone(), query.clone())?;
        self.results.begin(
            SearchRecord::new(id, &query, folder, place),
            self.panel.clear_each,
        );
        self.runs.push(run);
        self.panel.visible = true;
        self.panel.cursor = None;
        self.panel.selection.clear();
        Ok(())
    }

    /// Opens (or brings forward) the view of `path`; `line` is zero-based.
    pub(in crate::app) fn open_view(
        &mut self,
        ctx: &egui::Context,
        place: TextPlace,
        path: String,
        line: Option<(u64, Vec<(usize, usize)>)>,
    ) {
        if let Some(view) = self
            .views
            .iter_mut()
            .find(|view| view.same_file(&place, &path))
        {
            if let Some((line, marks)) = line {
                view.marks = marks;
                view.jump_to(line);
                if let Some(markdown) = &mut view.markdown {
                    markdown.rendered = false;
                }
            }
            let id = view.window_id();
            self.owner = KeyOwner::View(view.id);
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, id));
            return;
        }
        self.next_view += 1;
        let view = TextViewWindow::open(self.next_view, place, path, theme_kind(ctx), line);
        self.owner = KeyOwner::View(view.id);
        self.views.push(view);
    }

    /// Opens the view at a hit line (or the file of a file row).
    pub(in crate::app) fn open_row(
        &mut self,
        ctx: &egui::Context,
        row: super::content_search_results::Row,
    ) {
        let Some(record) = self.results.record(row.search) else {
            return;
        };
        let (file, at) = match row.kind {
            RowKind::Line(file, line) => {
                (file, record.files.get(file).and_then(|f| f.lines.get(line)))
            }
            RowKind::File(file) => (file, None),
            _ => return,
        };
        let Some(path) = record.full_path(file) else {
            return;
        };
        let line = at.map(|hit| {
            // The ranges of a cut line count from the cut, not from the
            // start of the line the view shows: the line alone is marked.
            let marks = if hit.truncated {
                Vec::new()
            } else {
                hit.ranges
                    .iter()
                    .map(|(start, end)| (*start as usize, *end as usize))
                    .collect()
            };
            (hit.line.saturating_sub(1), marks)
        });
        let place = record.place.clone();
        self.open_view(ctx, place, path, line);
    }

    /// F4 / Umschalt+F4: next or previous hit, opened in the view.
    pub(in crate::app) fn step_hit(&mut self, ctx: &egui::Context, forward: bool) {
        let Some(row) = self.results.next_hit(self.panel.cursor, forward) else {
            return;
        };
        self.results.reveal(row);
        self.panel.visible = true;
        self.panel.select(row, false, false);
        self.panel.scroll_to_cursor = true;
        let owner = self.owner;
        self.open_row(ctx, row);
        // F4 pressed in the panel keeps the keyboard there (Notepad++).
        if owner == KeyOwner::Results {
            self.owner = owner;
        }
    }

    fn apply_panel(&mut self, ctx: &egui::Context, actions: Vec<PanelAction>) {
        for action in actions {
            match action {
                PanelAction::Open(row) => self.open_row(ctx, row),
                PanelAction::OpenPaths(paths) => {
                    for (search, path) in paths {
                        if let Some(place) = self.results.record(search).map(|r| r.place.clone()) {
                            self.open_view(ctx, place, path, None);
                        }
                    }
                }
                PanelAction::CancelRuns => self.cancel_searches(),
                PanelAction::Clear => {
                    self.cancel_searches();
                    self.runs.clear();
                    self.results.clear();
                    self.panel.selection.clear();
                    self.panel.cursor = None;
                }
                PanelAction::SearchWithin(source, query) => {
                    if let Err(error) =
                        self.results
                            .search_within(source, &query, self.panel.clear_each)
                    {
                        if let Some(prompt) = &mut self.panel.within {
                            prompt.error = Some(error);
                        }
                    }
                }
            }
        }
    }

    /// Pumps the running searches and draws the results panel (before the
    /// central panel).
    pub(in crate::app) fn ui_panel(&mut self, ctx: &egui::Context) {
        let results = &mut self.results;
        self.runs.retain_mut(|run| !run.drain(results));
        if self.running() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if !self.panel.visible || self.results.records.is_empty() {
            self.panel.rect = None;
            return;
        }
        let running = self.running();
        let actions = self.panel.show(ctx, &mut self.results, running);
        self.apply_panel(ctx, actions);
    }

    /// Progress of the newest running search for the dialog.
    fn progress(&self) -> Option<(u64, u64, u64)> {
        let run = self.runs.last()?;
        let record = self.results.record(run.id)?;
        Some((record.progress.0, record.progress.1, record.hit_count()))
    }

    /// Keys of the owning surface, F4 everywhere (outside text fields).
    fn keys(&mut self, ctx: &egui::Context) -> bool {
        use egui::{Key, Modifiers};
        let typing = ctx.wants_keyboard_input();
        let (next, previous, find_dialog) = ctx.input_mut(|input| {
            let next = !typing && input.consume_key(Modifiers::NONE, Key::F4);
            let previous = !typing && input.consume_key(Modifiers::SHIFT, Key::F4);
            let dialog = self.owner != KeyOwner::Explorer
                && input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::F);
            (next, previous, dialog)
        });
        if next || previous {
            self.step_hit(ctx, next);
        }
        match self.owner {
            KeyOwner::View(id) => {
                if let Some(view) = self.views.iter_mut().find(|view| view.id == id) {
                    view.keys(ctx);
                }
            }
            KeyOwner::Results if !typing => {
                let actions = self.panel.keys(ctx, &mut self.results);
                self.apply_panel(ctx, actions);
            }
            KeyOwner::Dialog => {
                if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
                    self.dialog.open = false;
                }
            }
            _ => {}
        }
        find_dialog
    }
}

impl App {
    /// The place of the active tab (its backend when remote).
    pub(in crate::app) fn active_text_place(&self) -> TextPlace {
        match &self.remote {
            Some(remote) => TextPlace::remote(remote.backend.clone(), remote.label.clone()),
            None => TextPlace::local(),
        }
    }

    /// Strg+Umschalt+F and the menus: the dialog for `folder` (default: the
    /// folder of the active tab).
    pub(in crate::app) fn open_find_in_files(&mut self, folder: Option<String>) {
        let folder = folder.unwrap_or_else(|| self.root_path.clone());
        let place = self.active_text_place();
        self.text_tools.dialog.open_for(folder, place);
        self.text_tools.owner = KeyOwner::Dialog;
    }

    /// The one selected (or keyboard-focused) file of the table.
    pub(in crate::app) fn single_selected_file(&self) -> Option<usize> {
        let mut selected = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.selection.contains(&entry.key()));
        match (selected.next(), selected.next()) {
            (Some((index, entry)), None) if !entry.is_dir => Some(index),
            _ => None,
        }
    }

    /// „Ansehen“ (F3, context menus): the entry in the text view.
    pub(in crate::app) fn open_text_view_entry(&mut self, ctx: &egui::Context, index: usize) {
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        if entry.is_dir {
            return;
        }
        let path = entry.path.to_string();
        let place = self.active_text_place();
        self.text_tools.open_view(ctx, place, path, None);
    }

    pub(in crate::app) fn ui_text_tools_panel(&mut self, ctx: &egui::Context) {
        self.text_tools.ui_panel(ctx);
    }

    pub(in crate::app) fn ui_text_tools_windows(&mut self, ctx: &egui::Context) {
        if self.text_tools.keys(ctx) {
            self.open_find_in_files(None);
        }
        if self.text_tools.dialog.open {
            let progress = self.text_tools.progress();
            match self.text_tools.dialog.show(ctx, progress) {
                DialogAction::Search(query, folder, place) => {
                    if let Err(error) = self.text_tools.start_search(query, folder, place) {
                        self.text_tools.dialog.error = Some(error);
                    }
                }
                DialogAction::TakeActiveFolder => {
                    self.text_tools.dialog.form.folder = self.root_path.clone();
                    self.text_tools.dialog.place = self.active_text_place();
                }
                DialogAction::Cancel => self.text_tools.cancel_searches(),
                DialogAction::None => {}
            }
            if !self.text_tools.dialog.open && self.text_tools.owner == KeyOwner::Dialog {
                self.text_tools.owner = KeyOwner::Explorer;
            }
        }
        let mut opens = Vec::new();
        for view in &mut self.text_tools.views {
            for action in view.show(ctx) {
                match action {
                    ViewAction::OpenFile(path) => opens.push((view.place.clone(), path)),
                }
            }
        }
        let owner = self.text_tools.owner;
        self.text_tools.views.retain(|view| view.open);
        if let KeyOwner::View(id) = owner {
            if !self.text_tools.views.iter().any(|view| view.id == id) {
                self.text_tools.owner = KeyOwner::Explorer;
            }
        }
        for (place, path) in opens {
            self.text_tools.open_view(ctx, place, path, None);
        }
        if !self.text_tools.panel.visible && self.text_tools.owner == KeyOwner::Results {
            self.text_tools.owner = KeyOwner::Explorer;
        }
    }
}
