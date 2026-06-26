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
        let (version, exe) = match self.update_ready.clone() {
            Some(v) => v,
            None => return,
        };
        let mut restart = false;
        let mut later = false;
        egui::Window::new("Update bereit")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "Version {} wurde installiert. Zum Übernehmen ist ein Neustart nötig.",
                    version
                ));
                ui.colored_label(
                    Color32::from_gray(150),
                    "„Später“ behält die laufende Version bei; das Update greift beim nächsten Start.",
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Jetzt neu starten").strong()).clicked() {
                        restart = true;
                    }
                    if ui.button("Später").clicked() {
                        later = true;
                    }
                });
            });
        if restart {
            // Empty exe = the worker path: it relaunches after we exit, so just
            // close. Otherwise the new binary is already in place — relaunch it.
            if !exe.as_os_str().is_empty() {
                spawn_updated_app(&exe);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if later {
            self.update_ready = None;
            self.notice = Some((
                format!("Update v{} greift beim nächsten Start", version),
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
        } else if close && !self.connecting {
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
        // Dim everything behind the notice.
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
            let _ = std::fs::write(appdata_file("disclaimer_ack.txt"), "1");
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
                                ("Entf", "In den Papierkorb"),
                                ("Shift+Entf", "Endgültig löschen"),
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

    pub(in crate::app) fn ui_copy_dialog(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let title = if self.copy_mode_pending == CopyMode::Copy {
            "Kopieren"
        } else {
            "Verschieben"
        };
        let running = matches!(&self.copy_progress, Some(p) if !p.done);
        let done = matches!(&self.copy_progress, Some(p) if p.done);

        egui::Window::new(title)
            .fixed_size([560.0, 280.0])
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("{} Einträge ausgewählt", self.selection.len()));
                ui.horizontal(|ui| {
                    ui.label("Modus:");
                    ui.radio_value(&mut self.copy_mode_pending, CopyMode::Copy, "kopieren");
                    ui.radio_value(&mut self.copy_mode_pending, CopyMode::Move, "verschieben");
                });
                ui.colored_label(
                    egui::Color32::from_gray(160),
                    "Ordner werden rekursiv expandiert; nur Dateien die dem aktuellen Filter entsprechen werden kopiert. Ordnerstruktur wird erhalten, leere Ordner weggelassen.",
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Ziel:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.copy_dest)
                            .desired_width(360.0)
                            .hint_text("Zielordner…"),
                    );
                    if ui.add_enabled(!running, egui::Button::new("Wählen…")).clicked() {
                        let init = self.copy_dest.clone();
                        self.open_picker(PickerPurpose::CopyDest, &init);
                    }
                });
                ui.checkbox(
                    &mut self.copy_preserve,
                    "Ordnerstruktur erhalten (leere Ordner werden weggelassen)",
                );
                ui.horizontal(|ui| {
                    ui.label("Bei Konflikt:");
                    ui.radio_value(&mut self.copy_conflict, Conflict::Rename, "umbenennen");
                    ui.radio_value(&mut self.copy_conflict, Conflict::Overwrite, "überschreiben");
                    ui.radio_value(&mut self.copy_conflict, Conflict::Skip, "überspringen");
                });

                if let Some(ref p) = self.copy_progress {
                    let frac = if p.bytes_total > 0 {
                        p.bytes_done as f32 / p.bytes_total as f32
                    } else if p.files_total > 0 {
                        p.files_done as f32 / p.files_total as f32
                    } else {
                        0.0
                    };
                    ui.add(egui::ProgressBar::new(frac).show_percentage());
                    ui.label(format!(
                        "{}/{} Dateien · {} / {} · {:.1}s{}",
                        p.files_done,
                        p.files_total,
                        format_bytes(p.bytes_done),
                        format_bytes(p.bytes_total),
                        p.elapsed_ms as f64 / 1000.0,
                        if p.errors > 0 {
                            format!(" · {} Fehler", p.errors)
                        } else {
                            String::new()
                        },
                    ));
                }

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !self.copy_dest.is_empty() && !running,
                                egui::Button::new(RichText::new("Start").strong()),
                            )
                            .clicked()
                        {
                            self.confirm_copy();
                        }
                        if ui.add_enabled(!running, egui::Button::new("Abbrechen")).clicked() {
                            close = true;
                        }
                    });
                });
            });

        if close || done {
            self.copy_open = false;
            if done && self.copy_mode_pending == CopyMode::Move {
                let removed: HashSet<Arc<str>> = self.selection.drain().collect();
                self.entries.retain(|e| !removed.contains(&e.key()));
                self.recompute_view();
            }
        }
    }
}
