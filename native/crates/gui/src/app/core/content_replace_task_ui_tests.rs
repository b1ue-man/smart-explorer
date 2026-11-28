//! „In Dateien ersetzen“ in real egui frames on the GUI task runner (Suite
//! crates-task, part „GUI layout harness“): Strg+Umschalt+H, the dry run,
//! the confirmation with „Abbrechen“ preselected (Enter cancels and writes
//! nothing), the run with its record and „Rückgängig“, then „Aus dem
//! Ergebnis entfernen“ (Entf) and „Alle verbleibenden ersetzen…“, which
//! leaves the dismissed hit alone. Both colour schemes, one visuals dump per
//! view as `gui_design_task_find_in_files_results_and_text_view` writes them.
use crate::app::content_search_results::{Row, RowKind};
use crate::app::content_search_results_ui::PanelAction;
use crate::app::gui_design_task_capture::Capture;
use crate::app::text_tools::content_replace_confirm::PreviewState;
use crate::app::text_tools::content_replace_results::UndoState;
use crate::app::text_tools::KeyOwner;
use crate::app::ui_preferences::{Appearance, ColorMode};
use crate::app::App;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct Harness {
    app: App,
    ctx: egui::Context,
    capture: Capture,
    size: egui::Vec2,
    time: f64,
    root: PathBuf,
    _fixture: tempfile::TempDir,
}

impl Harness {
    fn new(dark: bool) -> Self {
        assert_eq!(std::env::var("SMART_EXPLORER_GUI_TASK").as_deref(), Ok("1"));
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("Projekte");
        std::fs::create_dir_all(&root).unwrap();
        let mut app = App::new_for_gui_task();
        app.show_disclaimer = false;
        app.show_update_dialog = false;
        app.remote_versions = Some(Vec::new());
        app.share_discovery.initial_refresh_requested = true;
        app.share_profiles.auto_connect = false;
        app.appearance = Appearance {
            mode: if dark {
                ColorMode::Dark
            } else {
                ColorMode::Light
            },
            ..Default::default()
        };
        app.home = fixture.path().to_path_buf();
        app.root_path = root.to_string_lossy().into_owned();
        app.error_msg = None;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.configure_appearance(&ctx);
        Self {
            app,
            ctx,
            capture: Capture::default(),
            size: egui::vec2(1000.0, 700.0),
            time: 0.0,
            root,
            _fixture: fixture,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.2;
        let modifiers = events
            .iter()
            .find_map(|event| match event {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            modifiers,
            focused: true,
            ..Default::default()
        };
        let app = &mut self.app;
        let output = self.ctx.run(input, |ctx| {
            app.update_keyboard(ctx);
            app.update_layout(ctx);
        });
        self.capture.record(&output);
        output
    }

    fn settle(&mut self) -> egui::FullOutput {
        self.frame(Vec::new());
        self.frame(Vec::new());
        self.frame(Vec::new())
    }

    fn click(&mut self, label: &str) {
        let pos = self.capture.target(label);
        for pressed in [true, false] {
            self.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        self.settle();
    }

    fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        for pressed in [true, false] {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            }]);
        }
        self.settle();
    }

    /// Frames until `done` holds (background threads write the files).
    fn until(&mut self, what: &str, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !done(&self.app) {
            assert!(Instant::now() < deadline, "{what} did not end");
            std::thread::sleep(Duration::from_millis(20));
            self.frame(Vec::new());
        }
        self.settle();
    }

    fn save(&mut self, name: &str) {
        let output = self.settle();
        let directory = PathBuf::from(std::env::var("SMART_EXPLORER_GUI_VISUALS").unwrap());
        std::fs::create_dir_all(&directory).unwrap();
        self.capture.save(
            &self.ctx,
            output,
            &directory.join(format!("{name}.json")),
            self.size,
        );
    }

    fn assert_visible(&self, label: &str) {
        let position = self.capture.target(label);
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO, self.size).contains(position),
            "{label} outside window"
        );
    }

    fn preview_ready(&mut self) -> (u64, u64) {
        self.until("the dry run", |app| {
            matches!(
                app.text_tools.replace.confirm.as_ref().map(|c| &c.state),
                Some(PreviewState::Ready(_) | PreviewState::Failed(_))
            )
        });
        match self
            .app
            .text_tools
            .replace
            .confirm
            .as_ref()
            .map(|c| &c.state)
        {
            Some(PreviewState::Ready(totals)) => (totals.replacements, totals.files_changed),
            other => panic!("dry run: {other:?}"),
        }
    }
}

fn read(root: &Path, name: &str) -> String {
    std::fs::read_to_string(root.join(name)).unwrap()
}

const ORIGINAL: &str = "Bericht alt\nnoch ein Bericht\n";

#[test]
#[ignore = "run only through the isolated remote GUI task entrypoint; Suite crates-task"]
fn replace_ui_task_dialog_preview_confirm_run_undo_and_dismiss() {
    for dark in [false, true] {
        let mode = if dark { "dark" } else { "light" };
        let mut h = Harness::new(dark);
        std::fs::write(h.root.join("Bericht.md"), ORIGINAL).unwrap();
        std::fs::write(h.root.join("Daten.csv"), "Bericht\n").unwrap();
        h.settle();
        h.key(
            egui::Key::H,
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        );
        assert!(
            h.app.text_tools.dialog.open,
            "Strg+Umschalt+H opens the dialog"
        );
        assert_eq!(h.app.text_tools.dialog.form.folder, h.app.root_path);
        {
            let form = &mut h.app.text_tools.dialog.form;
            form.pattern = "Bericht".into();
            form.replacement = "Report".into();
            form.filters = "*.md".into();
        }
        h.save(&format!("replace-dialog-{mode}"));
        h.assert_visible("Ersetzen durch:");
        h.assert_visible("Ignorierdateien beachten (.gitignore)");

        // The dry run counts; „Abbrechen“ is preselected: Enter writes nothing.
        h.click("In Dateien ersetzen");
        assert_eq!(h.preview_ready(), (2, 1));
        assert_eq!(read(&h.root, "Bericht.md"), ORIGINAL);
        h.save(&format!("replace-confirm-{mode}"));
        h.assert_visible("Ersetzen");
        h.assert_visible("Abbrechen");
        h.key(egui::Key::Enter, egui::Modifiers::NONE);
        assert!(h.app.text_tools.replace.confirm.is_none());
        assert_eq!(read(&h.root, "Bericht.md"), ORIGINAL);

        // „Ersetzen“: the run, its record and „Rückgängig“.
        h.click("In Dateien ersetzen");
        assert_eq!(h.preview_ready(), (2, 1));
        h.click("Ersetzen");
        h.until("the run", |app| app.text_tools.replace.runs.is_empty());
        assert_eq!(read(&h.root, "Bericht.md"), "Report alt\nnoch ein Report\n");
        assert_eq!(read(&h.root, "Daten.csv"), "Bericht\n", "the filter holds");
        let record = &h.app.text_tools.results.records[0];
        let info = record.replace.as_ref().expect("replacement record");
        assert_eq!(info.replacements, 2, "{}", record.header());
        assert!(matches!(info.undo, UndoState::Available(_)));
        assert_eq!(record.files[0].rel, "Bericht.md");
        h.save(&format!("replace-result-{mode}"));
        h.assert_visible("Rückgängig");
        h.click("Rückgängig");
        h.until("the undo", |app| {
            !matches!(
                app.text_tools.results.records[0]
                    .replace
                    .as_ref()
                    .map(|i| &i.undo),
                Some(UndoState::Running)
            )
        });
        let undo = &h.app.text_tools.results.records[0]
            .replace
            .as_ref()
            .unwrap()
            .undo;
        assert_eq!(undo, &UndoState::Done);
        assert_eq!(read(&h.root, "Bericht.md"), ORIGINAL);

        // A search, Entf on its first hit, then „Alle verbleibenden ersetzen…“.
        h.click("Alle suchen");
        h.until("the search", |app| app.text_tools.runs.is_empty());
        let search = h.app.text_tools.results.records[0].id;
        let first = Row {
            search,
            kind: RowKind::Line(0, 0),
        };
        h.app.text_tools.panel.selection = [first].into_iter().collect();
        h.app.text_tools.panel.cursor = Some(first);
        h.app.text_tools.owner = KeyOwner::Results;
        h.key(egui::Key::Delete, egui::Modifiers::NONE);
        let record = h.app.text_tools.results.record(search).unwrap();
        assert_eq!(record.files[0].lines.len(), 1, "Entf dismissed the hit");
        assert!(h
            .app
            .text_tools
            .apply_replace_action(&PanelAction::ReplaceRemaining(search)));
        h.settle();
        h.save(&format!("replace-ask-{mode}"));
        h.click("Weiter …");
        assert_eq!(h.preview_ready(), (1, 1), "the dismissed hit stays");
        h.click("Ersetzen");
        h.until("the second run", |app| {
            app.text_tools.replace.runs.is_empty()
        });
        assert_eq!(
            read(&h.root, "Bericht.md"),
            "Bericht alt\nnoch ein Report\n"
        );
        assert!(
            h.app
                .text_tools
                .results
                .record(search)
                .unwrap()
                .files
                .is_empty(),
            "replaced files leave their search"
        );
    }
}
