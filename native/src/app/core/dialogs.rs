use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn ui_rename_dialog(&mut self, ctx: &egui::Context) {
        let mut confirm = false;
        let mut cancel = false;
        let mut focus = self.rename_focus;
        if let Some((path, draft)) = self.rename_open.as_mut() {
            let title = path.rsplit('/').next().unwrap_or("").to_string();
            egui::Window::new(format!("Umbenennen: {}", title))
                .fixed_size([420.0, 80.0])
                .collapsible(false)
                .resizable(false)
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
        egui::Window::new("Update bereit")
            .collapsible(false)
            .resizable(false)
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
                    Color32::from_gray(150),
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
            let preflight = match &ready {
                ReadyUpdate::Staged(bundle) => crate::updater::verify_staged_update(bundle),
                ReadyUpdate::InstalledRollback { .. } => Ok(()),
            };
            if let Err(error) = preflight {
                self.error_msg = Some(format!("Update-Staging ist nicht mehr gültig: {error}"));
                return;
            }
            if let Err(error) = self.prepare_for_update_apply() {
                self.error_msg = Some(format!(
                    "Neustart wurde nicht begonnen; laufende Arbeit konnte nicht sicher bewahrt werden: {error}"
                ));
                return;
            }
            let launch = match &ready {
                ReadyUpdate::Staged(bundle) => crate::updater::apply_staged_update(bundle),
                ReadyUpdate::InstalledRollback { executable, .. } => spawn_updated_app(executable)
                    .map_err(|error| format!("Rollback-Version starten: {error}")),
            };
            match launch {
                Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(error) => {
                    self.shutdown_prepared = false;
                    self.error_msg = Some(format!(
                        "Neustart-Helfer konnte nicht gestartet werden; das gestagte Update bleibt erhalten: {error}"
                    ));
                }
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

    pub(in crate::app) fn ui_connect_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_connect {
            return;
        }
        use crate::creds::Protocol;
        let mut do_connect = false;
        let mut close = false;
        let mut open = true;
        egui::Window::new("Verbinden (SFTP / FTP / Netzlaufwerk)")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fixed_size([440.0, 0.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let f = &mut self.connect_form;
                egui::ComboBox::from_label("Protokoll")
                    .selected_text(match f.protocol {
                        Protocol::Sftp => "SFTP",
                        Protocol::Ftp => "FTP",
                        Protocol::Ftps => "FTPS",
                        Protocol::Webdav => "WebDAV (HTTPS)",
                        Protocol::Share => "Netzlaufwerk (UNC)",
                    })
                    .show_ui(ui, |ui| {
                        for (p, lbl) in [
                            (Protocol::Sftp, "SFTP"),
                            (Protocol::Ftp, "FTP"),
                            (Protocol::Ftps, "FTPS"),
                            (Protocol::Webdav, "WebDAV (HTTPS)"),
                            (Protocol::Share, "Netzlaufwerk (UNC)"),
                        ] {
                            if ui.selectable_label(f.protocol == p, lbl).clicked() {
                                f.protocol = p;
                                if p != Protocol::Share && f.port.trim().is_empty() {
                                    f.port = p.default_port().to_string();
                                }
                            }
                        }
                    });
                ui.add_space(4.0);

                egui::Grid::new("connect_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        if f.protocol == Protocol::Share {
                            ui.label("Freigabe (UNC)");
                            ui.add(
                                egui::TextEdit::singleline(&mut f.unc)
                                    .hint_text(r"\\server\share")
                                    .desired_width(f32::INFINITY),
                            );
                            ui.end_row();
                            ui.label("Benutzer");
                            ui.add(
                                egui::TextEdit::singleline(&mut f.user)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.end_row();
                            ui.label("Passwort");
                            ui.add(
                                egui::TextEdit::singleline(&mut f.password)
                                    .password(true)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.end_row();
                        } else {
                            ui.label("Host");
                            ui.add(
                                egui::TextEdit::singleline(&mut f.host)
                                    .hint_text("host.example.com")
                                    .desired_width(f32::INFINITY),
                            );
                            ui.end_row();
                            ui.label("Port");
                            ui.add(egui::TextEdit::singleline(&mut f.port).desired_width(70.0));
                            ui.end_row();
                            ui.label("Benutzer");
                            ui.add(
                                egui::TextEdit::singleline(&mut f.user)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.end_row();
                            ui.label("Startpfad");
                            ui.add(
                                egui::TextEdit::singleline(&mut f.root)
                                    .hint_text("/")
                                    .desired_width(f32::INFINITY),
                            );
                            ui.end_row();
                        }
                    });

                if f.protocol == Protocol::Sftp {
                    ui.checkbox(&mut f.use_key, "Mit Schlüsseldatei anmelden");
                    ui.checkbox(&mut f.use_agent, "⚡ Remote-Agent (experimentell)")
                        .on_hover_text(
                            "Lädt beim Verbinden einen kleinen Helfer auf den Server und führt \
                             Erkundung/Analyse dort lokal aus (statt vieler Netzwerk-Roundtrips). \
                             Opt-in; fällt bei Problemen automatisch auf normales SFTP zurück. \
                             Noch keine Agent-Binaries gebündelt — siehe docs/SSH_AGENT_PLAN.md.",
                        );
                }
                if f.protocol == Protocol::Sftp && f.use_key {
                    ui.horizontal(|ui| {
                        ui.label("Schlüssel");
                        ui.add(egui::TextEdit::singleline(&mut f.keyfile).desired_width(220.0));
                        if ui.button("…").clicked() {
                            if let Some(p) = rfd::FileDialog::new().pick_file() {
                                f.keyfile = p.to_string_lossy().replace('\\', "/");
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Passphrase");
                        ui.add(
                            egui::TextEdit::singleline(&mut f.passphrase)
                                .password(true)
                                .desired_width(220.0),
                        );
                    });
                } else if f.protocol != Protocol::Share {
                    ui.horizontal(|ui| {
                        ui.label("Passwort");
                        ui.add(
                            egui::TextEdit::singleline(&mut f.password)
                                .password(true)
                                .desired_width(f32::INFINITY),
                        );
                    });
                }

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut f.save, "Speichern");
                    ui.add(
                        egui::TextEdit::singleline(&mut f.label)
                            .hint_text("Bezeichnung (optional)")
                            .desired_width(f32::INFINITY),
                    );
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if self.connecting {
                        ui.spinner();
                        ui.label("Verbinde…");
                        if ui.button("Abbrechen").clicked() {
                            close = true;
                        }
                    } else {
                        if ui.button(RichText::new("Verbinden").strong()).clicked() {
                            do_connect = true;
                        }
                        if ui.button("Abbrechen").clicked() {
                            close = true;
                        }
                    }
                });
            });
        if !open {
            close = true;
        }
        if do_connect {
            let form = self.connect_form.clone();
            self.begin_connect(form, None);
        } else if close {
            self.connect_rx = None;
            self.connecting = false;
            self.show_connect = false;
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
        egui::Window::new("Hinweis & Haftungsausschluss")
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(false)
            .fixed_size([560.0, 0.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(420.0)
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
        egui::Window::new("Tastenkürzel")
            .open(&mut open)
            .resizable(true)
            .default_size([520.0, 560.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
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
                        ui.label(RichText::new(*title).strong().color(Color32::from_rgb(120, 170, 255)));
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
