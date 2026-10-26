use super::prelude::*;
use super::*;
use crate::app::theme;

impl App {
    pub(in crate::app) fn ui_rename_dialog(&mut self, ctx: &egui::Context) {
        let mut confirm = false;
        let mut cancel = false;
        let mut focus = self.rename_focus;
        if let Some((path, draft)) = self.rename_open.as_mut() {
            let title = path.rsplit('/').next().unwrap_or("").to_string();
            super::dialog_geometry::window(ctx, "dialogs.0", format!("Umbenennen: {}", title))
                .default_size([440.0, 140.0])
                .collapsible(false)
                .resizable(true)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    let resp =
                        ui.add(egui::TextEdit::singleline(draft).desired_width(f32::INFINITY));
                    if focus {
                        resp.request_focus();
                        focus = false;
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        confirm = true;
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cancel = true;
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(RichText::new("Umbenennen").strong()).clicked() {
                                confirm = true;
                            }
                            if ui.button("Abbrechen").clicked() {
                                cancel = true;
                            }
                        });
                    });
                });
        }
        self.rename_focus = focus;
        if confirm {
            self.confirm_rename();
        } else if cancel {
            self.rename_open = None;
        }
    }

    pub(in crate::app) fn ui_update_dialog(&mut self, ctx: &egui::Context) {
        let ready = match self.update_ready.clone() {
            Some(ready) if self.show_update_dialog => ready,
            None => return,
            Some(_) => return,
        };
        let version = ready.version().to_string();
        let mut restart = false;
        let mut later = false;
        let mut discard = false;
        super::dialog_geometry::window(ctx, "dialogs.1", "Update bereit")
            .id(egui::Id::new("Update bereit"))
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let message = match &ready {
                    ReadyUpdate::Staged(_) => format!(
                        "Version {version} wurde geprüft und bereitgestellt. Installiert wird sie erst nach Ihrer Bestätigung."
                    ),
                    ReadyUpdate::InstalledRollback { .. } => format!(
                        "Version {version} wurde für den Rollback eingesetzt. Zum Übernehmen ist ein Neustart nötig."
                    ),
                };
                ui.label(message);
                ui.colored_label(
                    theme::muted(ui),
                    "„Später“ behält ein gestagtes Update unverändert; beim nächsten Start wird erneut gefragt.",
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Jetzt neu starten").strong()).clicked() {
                        restart = true;
                    }
                    if ui.button("Später").clicked() {
                        later = true;
                    }
                    if matches!(&ready, ReadyUpdate::Staged(_))
                        && ui.button("Verwerfen").clicked()
                    {
                        discard = true;
                    }
                });
            });
        if restart {
            if self.desktop_sync_active() {
                self.request_sync_exit(super::sync_exit_gate::ExitIntent::Update);
            } else {
                self.apply_ready_update(ctx, ready);
            }
        } else if discard {
            let ReadyUpdate::Staged(bundle) = &ready else {
                return;
            };
            match crate::updater::discard_staged_update(bundle) {
                Ok(()) => {
                    self.update_ready = None;
                    self.show_update_dialog = false;
                    self.notice = Some((
                        format!("Gestagtes Update v{version} verworfen"),
                        std::time::Instant::now(),
                    ));
                }
                Err(error) => {
                    self.error_msg = Some(format!("Gestagtes Update verwerfen: {error}"));
                }
            }
        } else if later {
            self.show_update_dialog = false;
            if matches!(ready, ReadyUpdate::InstalledRollback { .. }) {
                self.update_ready = None;
            }
            self.notice = Some((
                format!("Update v{version} bleibt für später bereit"),
                std::time::Instant::now(),
            ));
        }
    }

    /// First-run liability notice. Modal-ish (foreground, dimmed backdrop);
    /// must be acknowledged once. The acceptance is recorded in appdata so it
    /// doesn't reappear.
    pub(in crate::app) fn ui_disclaimer(&mut self, ctx: &egui::Context) {
        if !self.show_disclaimer {
            return;
        }
        // Fill the viewport even though the main layout is intentionally not
        // rendered while this first-run gate is open.
        egui::Area::new(egui::Id::new("disclaimer_backdrop"))
            .order(egui::Order::Background)
            .show(ctx, |ui| {
                let r = ui.ctx().screen_rect();
                ui.painter()
                    .rect_filled(r, 0.0, Color32::from_black_alpha(200));
            });
        let mut accept = false;
        super::dialog_geometry::window(ctx, "dialogs.2", "Hinweis & Haftungsausschluss")
            .id(egui::Id::new("Hinweis & Haftungsausschluss"))
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(true)
            .default_size([560.0, 480.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().min_scrolled_height(0.0)
                    .max_height(super::dialog_geometry::remaining_height(ui, 2.0))
                    .show(ui, |ui| {
                        ui.label(DISCLAIMER_TEXT);
                    });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(
                            RichText::new("Verstanden — auf eigenes Risiko fortfahren").strong(),
                        )
                        .clicked()
                    {
                        accept = true;
                    }
                    if ui.button("Beenden").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        if accept {
            if let Err(error) = std::fs::write(appdata_file("disclaimer_ack.txt"), "1") {
                self.error_msg = Some(format!(
                    "Haftungshinweis konnte nicht dauerhaft bestätigt werden: {error}"
                ));
            }
            self.show_disclaimer = false;
        }
    }

    pub(in crate::app) fn ui_help_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.show_help;
        super::dialog_geometry::window(ctx, "dialogs.3", "Tastenkürzel")
            .id(egui::Id::new("Tastenkürzel"))
            .open(&mut open)
            .resizable(true)
            .default_size([600.0, 520.0])
            .max_size(theme::window_content_limit(ctx))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().min_scrolled_height(0.0).auto_shrink([false, false]).max_height(super::dialog_geometry::remaining_height(ui, 1.0))
                    .show(ui, |ui| {
                    let groups: &[(&str, &[(&str, &str)])] = &[
                        (
                            "Navigation",
                            &[
                                ("Alt+←/→", "Zurück / Vor"),
                                ("Alt+↑  ·  Backspace", "Eine Ebene hoch"),
                                ("Enter", "Öffnen (Ordner betreten / Datei öffnen)"),
                                ("F5", "Aktualisieren"),
                                ("Ctrl+L", "Pfad bearbeiten"),
                                ("Ctrl+R", "Rekursiv umschalten"),
                                ("Ctrl+F  ·  F3", "Suchleiste (Filter / Pfad / Befehl)"),
                                (
                                    "Suchleiste",
                                    "Tippen filtert · Pfad oder C:\\… öffnen · .. (…) hoch · ›  für Befehle",
                                ),
                                ("↑/↓ in der Leiste", "Vorschläge (Wurzeln, Ordnersprünge, Befehle)"),
                                (
                                    "Leiste → Enter",
                                    "1 Treffer: öffnen/betreten (Leiste bleibt aktiv); mehrere: in die Liste springen",
                                ),
                                (
                                    "Liste → Enter",
                                    "Öffnen; bei Ordner aus der Suche zurück zur Leiste",
                                ),
                                ("📊  ·  ›Analyse", "Speicher-Analyse: Treemap, größte Ordner/Dateien"),
                            ],
                        ),
                        (
                            "Tabs",
                            &[
                                ("Ctrl+T", "Neuer Tab"),
                                ("Ctrl+W", "Tab schließen"),
                                ("Ctrl+Tab / Ctrl+Shift+Tab", "Nächster / vorheriger Tab"),
                                ("Alt+1 … Alt+9", "Zu Tab 1 … 9 (Alt+9 = letzter)"),
                                (
                                    "Alt (tippen)",
                                    "Tastenkürzel einblenden: Buchstabe/Ziffer wählt das Bedienelement (Esc schließt)",
                                ),
                            ],
                        ),
                        (
                            "Auswahl",
                            &[
                                ("Klick / Ziehen", "Auswählen / Rechteck-Auswahl"),
                                ("Ctrl+Klick", "Einzeln hinzufügen/entfernen"),
                                ("Shift+Klick / Shift+Pfeile", "Bereich auswählen"),
                                ("Ctrl+A", "Alles auswählen"),
                                ("Ctrl+I", "Auswahl umkehren"),
                                ("Esc", "Auswahl aufheben"),
                                ("↑/↓ · PageUp/Down · Home/End", "Cursor bewegen"),
                                ("Tippen", "Zum Eintrag springen"),
                            ],
                        ),
                        (
                            "Dateiaktionen",
                            &[
                                ("Ctrl+C / Ctrl+X / Ctrl+V", "Kopieren / Ausschneiden / Einfügen"),
                                ("Ctrl+Shift+C", "Pfade als Text kopieren"),
                                (
                                    "Entf",
                                    "Papierkorb, sofern verfügbar; sonst endgültig mit Bestätigung",
                                ),
                                (
                                    "Shift+Entf",
                                    "Endgültig löschen, sofern die Quelle dies unterstützt",
                                ),
                                ("F2", "Umbenennen"),
                                ("Ctrl+Shift+N", "Neuer Ordner"),
                                ("Alt+Enter", "Eigenschaften"),
                                ("Ctrl+Shift+E", "Im Explorer anzeigen"),
                                ("Ctrl+B", "Aktuellen Ordner zu Favoriten"),
                            ],
                        ),
                        ("Sonstiges", &[("F1", "Diese Hilfe")]),
                    ];
                    for (title, rows) in groups {
                        ui.add_space(4.0);
                        ui.label(RichText::new(*title).strong().color(theme::accent(ui)));
                        egui::Grid::new(*title)
                            .num_columns(2)
                            .striped(true)
                            .spacing([16.0, 2.0])
                            .show(ui, |ui| {
                                for (k, d) in *rows {
                                    ui.label(RichText::new(*k).monospace());
                                    ui.label(*d);
                                    ui.end_row();
                                }
                            });
                    }
                });
            });
        self.show_help = open;
    }
}
