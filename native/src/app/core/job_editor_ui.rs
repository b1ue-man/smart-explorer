use super::prelude::*;
use super::*;

impl App {
    /// Add/edit dialog for a single sync setup (the "rich" setup menu: source,
    /// target, method = direction + conflict handling, retention, schedule,
    /// hidden-file handling, ignore globs).
    pub(in crate::app) fn ui_job_editor(&mut self, ctx: &egui::Context) {
        let mut ed = match self.job_editor.take() {
            Some(e) => e,
            None => return,
        };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        // Set when a "Durchsuchen" button is clicked → open the in-app picker
        // after `ed` is restored to self.job_editor (so the picker can write
        // back into it). Carries the field + its current value as a start point.
        let mut pick: Option<(PickerPurpose, String)> = None;
        let title = if ed.id.is_some() {
            "✎ Sync-Setup bearbeiten"
        } else {
            "＋ Neues Sync-Setup"
        };
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([600.0, 650.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| {
                egui::Grid::new("job_editor_grid")
                    .num_columns(2)
                    .spacing([10.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.add(egui::TextEdit::singleline(&mut ed.name).hint_text("z. B. Dokumente sichern").desired_width(360.0));
                        ui.end_row();

                        ui.label("Quelle (A)");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut ed.source).hint_text("lokaler Ordner / Netzlaufwerk / Verbindung").desired_width(280.0));
                            if ui
                                .button("📂")
                                .on_hover_text("Im Explorer wählen — lokale Laufwerke oder gespeicherte Verbindungen")
                                .clicked()
                            {
                                pick = Some((PickerPurpose::SyncSource, ed.source.clone()));
                            }
                        });
                        ui.end_row();

                        ui.label("Ziel (B)");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut ed.target).hint_text("lokaler Ordner / Netzlaufwerk / Verbindung").desired_width(280.0));
                            if ui
                                .button("📂")
                                .on_hover_text("Im Explorer wählen — lokale Laufwerke oder gespeicherte Verbindungen")
                                .clicked()
                            {
                                pick = Some((PickerPurpose::SyncTarget, ed.target.clone()));
                            }
                        });
                        ui.end_row();

                        ui.label("Richtung").on_hover_text("Methode: in welche Richtung abgeglichen wird");
                        egui::ComboBox::from_id_salt("job_dir")
                            .selected_text(ed.direction.label())
                            .show_ui(ui, |ui| {
                                for d in [
                                    crate::bisync::Direction::Both,
                                    crate::bisync::Direction::AtoB,
                                    crate::bisync::Direction::BtoA,
                                ] {
                                    ui.selectable_value(&mut ed.direction, d, d.label());
                                }
                            });
                        ui.end_row();

                        ui.label("Konflikte").on_hover_text("Was passiert, wenn beide Seiten geändert wurden");
                        egui::ComboBox::from_id_salt("job_conf")
                            .selected_text(ed.conflict.label())
                            .show_ui(ui, |ui| {
                                for c in crate::bisync::ConflictMode::ALL {
                                    ui.selectable_value(&mut ed.conflict, c, c.label());
                                }
                            });
                        ui.end_row();

                        ui.label("Löschungen").on_hover_text("Wie mit Dateien umgegangen wird, die auf einer Seite fehlen");
                        egui::ComboBox::from_id_salt("job_del")
                            .selected_text(ed.delete_policy.label())
                            .show_ui(ui, |ui| {
                                for d in [
                                    crate::bisync::DeletePolicy::Propagate,
                                    crate::bisync::DeletePolicy::Mirror,
                                    crate::bisync::DeletePolicy::NoDelete,
                                ] {
                                    ui.selectable_value(&mut ed.delete_policy, d, d.label());
                                }
                            });
                        ui.end_row();

                        if ed.direction != crate::bisync::Direction::Both {
                            ui.label("Verschieben").on_hover_text("Einseitig: Quelle nach erfolgreicher Kopie löschen (Move)");
                            ui.checkbox(&mut ed.move_files, "Dateien verschieben statt kopieren");
                            ui.end_row();
                        }

                        ui.label("Vergleich").on_hover_text("Wie entschieden wird, ob zwei Dateien gleich sind");
                        egui::ComboBox::from_id_salt("job_cmp")
                            .selected_text(ed.compare.label())
                            .show_ui(ui, |ui| {
                                for c in [
                                    crate::bisync::CompareMode::MtimeSize,
                                    crate::bisync::CompareMode::SizeOnly,
                                    crate::bisync::CompareMode::Checksum,
                                ] {
                                    ui.selectable_value(&mut ed.compare, c, c.label());
                                }
                            });
                        ui.end_row();

                        if ed.compare == crate::bisync::CompareMode::MtimeSize {
                            ui.label("Zeit-Toleranz (s)").on_hover_text("Zeitstempel-Unterschiede bis zu N Sekunden als gleich werten (FAT/exFAT, Sommerzeit: 1–2)");
                            ui.add(egui::TextEdit::singleline(&mut ed.modify_window).desired_width(80.0));
                            ui.end_row();
                        }

                        ui.label("Versionen").on_hover_text("Wie lange/viele reversible Sicherungen überschriebener & gelöschter Dateien aufbewahrt werden");
                        egui::ComboBox::from_id_salt("job_ver")
                            .selected_text(ed.versioning_scheme.label())
                            .show_ui(ui, |ui| {
                                for v in crate::bisync::VersioningScheme::ALL {
                                    ui.selectable_value(&mut ed.versioning_scheme, v, v.label());
                                }
                            });
                        ui.end_row();

                        match ed.versioning_scheme {
                            crate::bisync::VersioningScheme::Days => {
                                ui.label("Aufbewahrung (Tage)").on_hover_text("0 = für immer behalten");
                                ui.add(egui::TextEdit::singleline(&mut ed.retain_days).desired_width(80.0));
                                ui.end_row();
                            }
                            crate::bisync::VersioningScheme::Count => {
                                ui.label("Versionen behalten").on_hover_text("Anzahl der neuesten Versions-Schnappschüsse (0 = alle)");
                                ui.add(egui::TextEdit::singleline(&mut ed.retain_count).desired_width(80.0));
                                ui.end_row();
                            }
                            _ => {}
                        }

                        ui.label("Papierkorb").on_hover_text("Gelöschte Dateien in den Windows-Papierkorb verschieben statt entfernen (nur lokale Pfade)");
                        ui.checkbox(&mut ed.use_recycle_bin, "Löschungen in den Papierkorb");
                        ui.end_row();

                        ui.label("Lösch-Schutz").on_hover_text("Abbrechen, wenn ein Lauf mehr als so viele Dateien löschen würde (0 = aus). Schützt vor versehentlichem Massenlöschen, z. B. wenn ein Laufwerk nicht verbunden ist.");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut ed.max_delete).desired_width(70.0));
                            ui.label("Dateien /");
                            ui.add(egui::TextEdit::singleline(&mut ed.max_delete_pct).desired_width(50.0));
                            ui.label("%");
                        });
                        ui.end_row();

                        ui.label("Auslöser").on_hover_text("Wann dieser Sync automatisch läuft");
                        egui::ComboBox::from_id_salt("job_trigger")
                            .selected_text(ed.trigger.label())
                            .show_ui(ui, |ui| {
                                for t in crate::syncjobs::Trigger::ALL {
                                    ui.selectable_value(&mut ed.trigger, t, t.label());
                                }
                            });
                        ui.end_row();

                        match ed.trigger {
                            crate::syncjobs::Trigger::Interval => {
                                ui.label("Intervall (min)").on_hover_text("Alle N Minuten ausführen");
                                ui.add(egui::TextEdit::singleline(&mut ed.interval_min).desired_width(80.0));
                                ui.end_row();
                            }
                            crate::syncjobs::Trigger::Calendar => {
                                ui.label("Uhrzeit").on_hover_text("Startzeit HH:MM");
                                ui.add(egui::TextEdit::singleline(&mut ed.cal_time).desired_width(80.0));
                                ui.end_row();

                                ui.label("Wochentage").on_hover_text("Keiner markiert = täglich");
                                ui.horizontal(|ui| {
                                    const DAYS: [&str; 7] = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"];
                                    for (i, d) in DAYS.iter().enumerate() {
                                        let on = (ed.cal_weekdays >> i) & 1 == 1;
                                        if ui.selectable_label(on, *d).clicked() {
                                            ed.cal_weekdays ^= 1 << i;
                                        }
                                    }
                                });
                                ui.end_row();

                                ui.label("Tag im Monat").on_hover_text("1–31 = monatlich; 0 = Wochentage verwenden");
                                ui.add(egui::TextEdit::singleline(&mut ed.cal_monthday).desired_width(80.0));
                                ui.end_row();
                            }
                            crate::syncjobs::Trigger::RealTime => {
                                ui.label("Verzögerung (s)").on_hover_text("Wartezeit nach der letzten Änderung, bevor synchronisiert wird (entprellt). Echtzeit beobachtet die lokale Seite.");
                                ui.add(egui::TextEdit::singleline(&mut ed.rt_debounce).desired_width(80.0));
                                ui.end_row();
                            }
                            crate::syncjobs::Trigger::OnConnect => {
                                ui.label("Gerät/USB").on_hover_text("Laufwerksbezeichnung, Seriennummer oder Buchstabe; Platzhalter * ? erlaubt; leer = jedes Wechselmedium");
                                ui.add(
                                    egui::TextEdit::singleline(&mut ed.connect_match)
                                        .hint_text("z. B. BACKUP* oder E:")
                                        .desired_width(220.0),
                                );
                                ui.end_row();
                            }
                            _ => {}
                        }

                        ui.label("Aktive Zeiten").on_hover_text("Nur in diesem Zeitfenster ausführen (von = bis ⇒ immer)");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut ed.active_from).desired_width(64.0));
                            ui.label("–");
                            ui.add(egui::TextEdit::singleline(&mut ed.active_to).desired_width(64.0));
                        });
                        ui.end_row();

                        if matches!(ed.trigger, crate::syncjobs::Trigger::Calendar) {
                            ui.label("Nachholen").on_hover_text("Verpasste geplante Läufe nachholen, statt auf den nächsten Termin zu warten");
                            ui.checkbox(&mut ed.catch_up, "verpasste Läufe nachholen");
                            ui.end_row();
                        }

                        ui.label("Versteckte Dateien");
                        ui.checkbox(&mut ed.include_hidden, "einbeziehen");
                        ui.end_row();

                        ui.label("Ignorieren").on_hover_text("Glob-Muster, eines pro Zeile (z. B. **/*.tmp, node_modules/**)");
                        ui.vertical(|ui| {
                            ui.add(egui::TextEdit::multiline(&mut ed.ignore).hint_text("**/*.tmp\nnode_modules/**").desired_rows(3).desired_width(360.0));
                            if ui.small_button("＋ Standard-Ausschlüsse").on_hover_text("Übliche temporäre/System-Dateien ergänzen").clicked() {
                                const DEFAULTS: &[&str] = &[
                                    "**/*.tmp", "**/~$*", "**/desktop.ini", "**/Thumbs.db",
                                    "**/.DS_Store", "**/System Volume Information/**",
                                ];
                                let mut lines: Vec<String> = ed.ignore.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
                                for d in DEFAULTS {
                                    if !lines.iter().any(|l| l == d) {
                                        lines.push((*d).to_string());
                                    }
                                }
                                ed.ignore = lines.join("\n");
                            }
                        });
                        ui.end_row();

                        ui.label("Größe (KB)").on_hover_text("Nur Dateien in diesem Größenbereich (0 = keine Grenze)");
                        ui.horizontal(|ui| {
                            ui.label("min");
                            ui.add(egui::TextEdit::singleline(&mut ed.filter_min_size_kb).desired_width(70.0));
                            ui.label("max");
                            ui.add(egui::TextEdit::singleline(&mut ed.filter_max_size_kb).desired_width(70.0));
                        });
                        ui.end_row();

                        ui.label("Alter (Tage)").on_hover_text("Nur Dateien, die jünger als / älter als N Tage sind (0 = aus)");
                        ui.horizontal(|ui| {
                            ui.label("jünger als");
                            ui.add(egui::TextEdit::singleline(&mut ed.filter_max_age_days).desired_width(60.0));
                            ui.label("älter als");
                            ui.add(egui::TextEdit::singleline(&mut ed.filter_min_age_days).desired_width(60.0));
                        });
                        ui.end_row();

                        ui.label("Bandbreite").on_hover_text("Übertragungsrate begrenzen (KB/s, 0 = unbegrenzt)");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut ed.bwlimit_kbps).desired_width(80.0));
                            ui.label("KB/s · max");
                            ui.add(egui::TextEdit::singleline(&mut ed.max_transfers).desired_width(50.0));
                            ui.label("parallel");
                        });
                        ui.end_row();

                        ui.label("Zuverlässigkeit").on_hover_text("Sichere Kopien (temporär + umbenennen), Größe nach dem Kopieren prüfen");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut ed.atomic_copy, "Sichere Kopien");
                            ui.checkbox(&mut ed.verify, "Überprüfen");
                        });
                        ui.end_row();

                        ui.label("Wiederholungen").on_hover_text("Fehlgeschlagene Übertragungen N-mal wiederholen, mit Pause dazwischen");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut ed.retries).desired_width(50.0));
                            ui.label("× / Pause");
                            ui.add(egui::TextEdit::singleline(&mut ed.retry_delay_secs).desired_width(50.0));
                            ui.label("s");
                        });
                        ui.end_row();

                        ui.label("Befehl davor").on_hover_text("Vor dem Lauf ausführen (nur Hintergrund-Dienst)");
                        ui.add(egui::TextEdit::singleline(&mut ed.run_before).hint_text("optional").desired_width(360.0));
                        ui.end_row();
                        ui.label("Befehl danach").on_hover_text("Nach dem Lauf ausführen (nur Hintergrund-Dienst)");
                        ui.add(egui::TextEdit::singleline(&mut ed.run_after).hint_text("optional").desired_width(360.0));
                        ui.end_row();

                        ui.label("Aktiv");
                        ui.checkbox(&mut ed.enabled, "Zeitplan aktiv");
                        ui.end_row();
                    });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("✔ Speichern").clicked() {
                        save = true;
                    }
                    if ui.button("Abbrechen").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel || !open {
            // Dropped (taken at top) — leaving job_editor as None closes it.
            return;
        }
        if save {
            if ed.source.trim().is_empty() || ed.target.trim().is_empty() {
                self.error_msg = Some("Quelle und Ziel dürfen nicht leer sein.".to_string());
                self.job_editor = Some(ed); // keep the dialog open
                return;
            }
            let name = if ed.name.trim().is_empty() {
                let base = ed
                    .source
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or("Sync");
                base.to_string()
            } else {
                ed.name.trim().to_string()
            };
            let ignore: Vec<String> = ed
                .ignore
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            let mut job = match &ed.id {
                // Editing: preserve id + last_run from the stored job.
                Some(id) => {
                    let mut j = self
                        .sync_jobs
                        .iter()
                        .find(|j| &j.id == id)
                        .cloned()
                        .unwrap_or_else(|| {
                            crate::syncjobs::SyncJob::new(
                                name.clone(),
                                ed.source.clone(),
                                ed.target.clone(),
                            )
                        });
                    j.name = name.clone();
                    j.source = ed.source.trim().to_string();
                    j.target = ed.target.trim().to_string();
                    j
                }
                None => crate::syncjobs::SyncJob::new(
                    name.clone(),
                    ed.source.trim().to_string(),
                    ed.target.trim().to_string(),
                ),
            };
            job.direction = ed.direction;
            job.conflict = ed.conflict;
            job.retain_days = ed.retain_days.trim().parse().unwrap_or(30);
            job.interval_min = ed.interval_min.trim().parse().unwrap_or(0);
            job.include_hidden = ed.include_hidden;
            job.ignore = ignore;
            job.enabled = ed.enabled;
            job.trigger = ed.trigger;
            if let Some(m) = hm_to_min(&ed.cal_time) {
                job.cal_time_min = m;
            }
            job.cal_weekdays = ed.cal_weekdays;
            job.cal_monthday = ed.cal_monthday.trim().parse().unwrap_or(0).min(31);
            job.rt_debounce_secs = ed.rt_debounce.trim().parse().unwrap_or(10);
            job.connect_match = ed.connect_match.trim().to_string();
            job.active_from_min = hm_to_min(&ed.active_from).unwrap_or(0);
            job.active_to_min = hm_to_min(&ed.active_to).unwrap_or(0);
            job.catch_up = ed.catch_up;
            job.delete_policy = ed.delete_policy;
            job.move_files = ed.move_files && ed.direction != crate::bisync::Direction::Both;
            job.compare = ed.compare;
            job.modify_window_sec = ed.modify_window.trim().parse().unwrap_or(0);
            job.versioning_scheme = ed.versioning_scheme;
            job.retain_count = ed.retain_count.trim().parse().unwrap_or(0);
            job.use_recycle_bin = ed.use_recycle_bin;
            job.max_delete = ed.max_delete.trim().parse().unwrap_or(0);
            job.max_delete_pct = ed.max_delete_pct.trim().parse().unwrap_or(0);
            job.filter_min_size_kb = ed.filter_min_size_kb.trim().parse().unwrap_or(0);
            job.filter_max_size_kb = ed.filter_max_size_kb.trim().parse().unwrap_or(0);
            job.filter_max_age_days = ed.filter_max_age_days.trim().parse().unwrap_or(0);
            job.filter_min_age_days = ed.filter_min_age_days.trim().parse().unwrap_or(0);
            job.bwlimit_kbps = ed.bwlimit_kbps.trim().parse().unwrap_or(0);
            job.max_transfers = ed.max_transfers.trim().parse().unwrap_or(0);
            job.atomic_copy = ed.atomic_copy;
            job.verify = ed.verify;
            job.retries = ed.retries.trim().parse().unwrap_or(0);
            job.retry_delay_secs = ed.retry_delay_secs.trim().parse().unwrap_or(2);
            job.run_before = ed.run_before.trim().to_string();
            job.run_after = ed.run_after.trim().to_string();
            match crate::syncjobs::upsert(&job) {
                Ok(_) => {
                    self.sync_jobs = crate::syncjobs::load();
                    self.notice = Some((
                        format!("✓ Setup „{}“ gespeichert", job.name),
                        std::time::Instant::now(),
                    ));
                }
                Err(e) => {
                    self.error_msg = Some(format!("Setup speichern: {}", e));
                    self.job_editor = Some(ed);
                }
            }
            return;
        }
        // Still open, nothing pressed — keep the editor for the next frame.
        self.job_editor = Some(ed);
        // Now that job_editor is restored, the picker can write back into it.
        if let Some((purpose, initial)) = pick {
            self.open_picker(purpose, &initial);
        }
    }
}
