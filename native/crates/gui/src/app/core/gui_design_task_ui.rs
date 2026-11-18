//! Behavioral and visual acceptance through real egui frames on the task runner.
use super::{
    gui_design_task_capture::Capture,
    settings_ui::SettingsPage,
    ui_preferences::{Appearance, ColorMode, UiState},
    App,
};
use crate::types::{FileEntry, SortKey};
use eframe::egui;
use std::path::PathBuf;

struct Harness {
    app: App,
    ctx: egui::Context,
    capture: Capture,
    size: egui::Vec2,
    time: f64,
    hover_file: bool,
    fixture: tempfile::TempDir,
}

impl Harness {
    fn new(dark: bool, size: [f32; 2]) -> Self {
        assert_eq!(std::env::var("SMART_EXPLORER_GUI_TASK").as_deref(), Ok("1"));
        let fixture = tempfile::tempdir().unwrap();
        let mut app = App::new_for_gui_task();
        app.show_disclaimer = false;
        app.show_filters = false;
        app.show_summary = false;
        app.show_update_dialog = false;
        app.remote_versions = Some(Vec::new());
        app.share_discovery.initial_refresh_requested = true;
        app.share_profiles.auto_connect = false;
        app.share_status = "Bereit · keine aktive Verbindung".into();
        app.appearance = Appearance {
            mode: if dark {
                ColorMode::Dark
            } else {
                ColorMode::Light
            },
            ..Default::default()
        };
        app.home = fixture.path().to_path_buf();
        app.drive_info = vec![("Daten".into(), 300_000_000_000, 1_000_000_000_000)];
        app.favorites = vec!["/Projekte".into(), "/Dokumente".into()];
        app.recent = vec!["/Projekte".into(), "/Fotos".into(), "/Archiv".into()];
        app.saved_connections.clear();
        app.error_msg = None;
        app.app_errors.clear();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.configure_appearance(&ctx);
        Self {
            app,
            ctx,
            capture: Capture::default(),
            size: size.into(),
            time: 0.0,
            hover_file: false,
            fixture,
        }
    }

    fn workspace(&mut self) {
        self.app.root_path = self
            .fixture
            .path()
            .join("Projekte")
            .to_string_lossy()
            .into_owned();
        self.app.entries = [
            ("Ablage", "", true, 0),
            ("Bericht.md", "md", false, 24_576),
            ("Daten.csv", "csv", false, 128_000),
            (
                "Entwurf mit einem längeren Dateinamen.txt",
                "txt",
                false,
                4096,
            ),
        ]
        .into_iter()
        .map(|(name, ext, is_dir, size)| FileEntry {
            path: format!("{}/{name}", self.app.root_path).into(),
            parent: self.app.root_path.clone().into(),
            name: name.into(),
            ext: ext.into(),
            size,
            mtime_ms: 1_700_000_000_000,
            btime_ms: 1_699_000_000_000,
            is_dir,
            is_symlink: false,
            hidden: false,
            system: false,
            depth: 1,
            id: None,
        })
        .collect();
        self.app.progress.scanned = self.app.entries.len() as u64 + 1;
        self.app.progress.bytes = self.app.entries.iter().map(|entry| entry.size).sum();
        self.app.recursive = false;
        self.app.sort_key = SortKey::Name;
        self.app.recompute_view();
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
            hovered_files: if self.hover_file {
                vec![egui::HoveredFile::default()]
            } else {
                Vec::new()
            },
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

    fn click(&mut self, label: &str) -> String {
        let pos = self.capture.target(label);
        let mut copied = String::new();
        for pressed in [true, false] {
            let output = self.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            if !output.platform_output.copied_text.is_empty() {
                copied = output.platform_output.copied_text;
            }
        }
        self.settle();
        copied
    }

    fn key(&mut self, key: egui::Key) {
        self.key_with_modifiers(key, egui::Modifiers::NONE);
    }

    fn key_with_modifiers(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        self.frame(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]);
        self.frame(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        }]);
    }

    fn save(&mut self, name: &str) {
        let output = self.settle();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, self.size).expand(1.0);
        self.ctx.memory(|memory| {
            for layer in memory.areas().visible_layer_ids() {
                if layer.order == egui::Order::Middle {
                    if let Some(rect) = memory.area_rect(layer.id) {
                        assert!(
                            screen.contains_rect(rect),
                            "{name}: window {layer:?} outside viewport: {rect:?}"
                        );
                    }
                }
            }
        });
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
}

#[test]
#[ignore = "run only through the isolated remote GUI task entrypoint; Suite crates-task"]
fn gui_design_task_workspace_layout_selection_filters_and_split() {
    for dark in [false, true] {
        let mode = if dark { "dark" } else { "light" };
        for (name, size) in [("minimum", [900.0, 600.0]), ("desktop", [1400.0, 900.0])] {
            let mut h = Harness::new(dark, size);
            h.workspace();
            h.save(&format!("workspace-{name}-{mode}"));
            for label in [
                "Neu",
                "Verbindung",
                "Sync",
                "Ansicht",
                "Einstellungen",
                "Filter & Suche",
                "»",
                "Bericht.md",
            ] {
                h.assert_visible(label);
            }
            assert!(!h.capture.contains("Erstellt") && !h.capture.contains("Tiefe"));
            assert!(!h.capture.contains("Dateien suchen…"));
            let sidebar_labels: Vec<_> = [
                "Persönlicher Ordner",
                "Desktop",
                "Dokumente",
                "Downloads",
                "Bilder",
                "Musik",
                "Videos",
            ]
            .iter()
            .map(|label| {
                h.capture
                    .labels
                    .iter()
                    .rev()
                    .find(|(text, rect, clip)| {
                        text == label && rect.left() < 190.0 && clip.contains(rect.center())
                    })
                    .unwrap()
                    .1
                    .left()
            })
            .collect();
            assert!(
                sidebar_labels
                    .iter()
                    .all(|left| (left - sidebar_labels[0]).abs() < 1.0),
                "sidebar labels are not aligned: {sidebar_labels:?}"
            );
            assert!(
                h.capture.target("Bericht.md").y < 160.0,
                "toolbar crowds the file list"
            );
            let command_y = h.capture.target("Verbindung").y;
            for label in ["◀", "↑", "Neu", "Sync", "Einstellungen", "Ansicht", "»"] {
                assert!(
                    (h.capture.target(label).y - command_y).abs() < 3.0,
                    "{label} left the toolbar row"
                );
            }
            h.click("Bericht.md");
            h.click("»");
            h.assert_visible("Kopieren   Ctrl+C");
            if name == "minimum" {
                h.assert_visible("Share-Server");
            }
            h.key(egui::Key::Escape);
            h.settle();
            assert!(!h.capture.contains("Kopieren   Ctrl+C"));
            assert_eq!(
                h.app.selection.len(),
                1,
                "closing a menu cleared the selection"
            );
            h.click("Bericht.md");
            assert_eq!(h.app.selection.len(), 1);
            assert!(h.app.selection.contains(&h.app.entries[1].key()));
            h.save(&format!("workspace-{name}-{mode}"));
            h.hover_file = true;
            h.save(&format!("drag-{name}-{mode}"));
            h.hover_file = false;
            h.settle();

            h.key_with_modifiers(egui::Key::F, egui::Modifiers::COMMAND);
            h.settle();
            assert!(h.app.show_filters && h.ctx.wants_keyboard_input());
            h.frame(vec![egui::Event::Text("Bericht".into())]);
            h.app.flush_text_filter();
            h.settle();
            assert_eq!(h.app.view.len(), 1);
            h.assert_visible("Filter & Suche · aktiv");
            assert!(
                (h.capture.target("Zurücksetzen").y - h.capture.target("enthält").y).abs() < 3.0
            );
            h.save(&format!("filters-{name}-{mode}"));
            h.click("Filter & Suche · aktiv");
            assert!(!h.app.show_filters);
            assert_eq!(h.app.view.len(), 1);
            h.assert_visible("Filter & Suche · aktiv");
            h.click("Filter & Suche · aktiv");
            h.click("Zurücksetzen");
            assert_eq!(h.app.view.len(), 4);
            assert!(h.app.folder_search_rx.is_none() && h.app.filter_pending_at.is_none());
            h.app.show_filters = false;
            h.app.appearance.detailed_columns = true;
            h.settle();
            h.assert_visible("Pfad");
            h.save(&format!("details-{name}-{mode}"));
            h.app.appearance.detailed_columns = false;
            let mut other = super::TabState::default();
            other.root_path = h.app.root_path.clone();
            other.entries = h.app.entries.clone();
            other.view = h.app.view.clone();
            h.app.tabs.push(other);
            h.app.toggle_split();
            h.settle();
            assert_eq!(h.app.pane_rects.len(), 2);
            assert!(!h.app.pane_rects[0].1.intersects(h.app.pane_rects[1].1));
            h.save(&format!("split-{name}-{mode}"));
        }
    }
}

#[test]
#[ignore = "run only through the isolated remote GUI task entrypoint; Suite crates-task"]
fn gui_design_task_settings_persistence_shortcut_guard_and_dialogs() {
    for dark in [false, true] {
        let mode = if dark { "dark" } else { "light" };
        let mut h = Harness::new(dark, [900.0, 600.0]);
        h.workspace();
        h.settle();
        h.click("Bericht.md");
        h.click("Einstellungen");
        assert!(h.app.settings.open);
        // The settings open on „Dieses Gerät“; the colour scheme is on „Darstellung“.
        h.click("Darstellung");
        assert_eq!(h.app.settings.page, SettingsPage::Appearance);
        h.click(if dark { "Hell" } else { "Dunkel" });
        assert_eq!(
            UiState::load().appearance.mode,
            if dark {
                ColorMode::Light
            } else {
                ColorMode::Dark
            }
        );
        h.key(egui::Key::Delete);
        assert!(h.app.trash_worker.is_none() && h.app.trash_rx.is_none());
        assert_eq!(h.app.selection.len(), 1);
        h.click(if dark { "Dunkel" } else { "Hell" });
        h.save(&format!("settings-{mode}"));
        for (page, name) in [
            (SettingsPage::Connections, "connections"),
            (SettingsPage::Updates, "updates"),
            (SettingsPage::Storage, "storage"),
            (SettingsPage::Integration, "integration"),
        ] {
            h.app.settings.page = page;
            h.save(&format!("settings-{name}-{mode}"));
        }
        h.key(egui::Key::Escape);
        assert!(!h.app.settings.open);
        h.app.show_connect = true;
        h.save(&format!("connect-{mode}"));
        h.assert_visible("Verbinden");
        h.app.show_connect = false;
        h.app.copy_open = true;
        h.save(&format!("copy-{mode}"));
        h.assert_visible("Schließen");
        h.app.copy_open = false;
        h.app.show_share = true;
        for (index, name) in [(0, "devices"), (1, "rooms"), (2, "exports"), (4, "network")] {
            h.app.share_tab = index;
            h.save(&format!("share-{name}-{mode}"));
            h.assert_visible("Geräte");
        }
        assert!(h.app.update_rx.is_none() && h.app.share_open_rx.is_none());
    }
}

#[path = "gui_design_task_ui_visuals.rs"]
mod visuals;
