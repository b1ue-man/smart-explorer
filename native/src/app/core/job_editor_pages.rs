//! The pages of the sync-setup editor: every field of the shared `JobEditor`
//! on exactly one page (Allgemein, Zeitplan, Vergleich & Versionen, Filter,
//! Übertragung & Befehle), so a setup does not show all its details at once.
//! The window, validation and saving stay in `job_editor_ui.rs`.
use super::prelude::*;
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum JobEditorPage {
    #[default]
    General,
    Schedule,
    Versions,
    Filter,
    Transfer,
}

impl JobEditorPage {
    pub(super) const ALL: [Self; 5] = [
        Self::General,
        Self::Schedule,
        Self::Versions,
        Self::Filter,
        Self::Transfer,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::General => "Allgemein",
            Self::Schedule => "Zeitplan",
            Self::Versions => "Vergleich & Versionen",
            Self::Filter => "Filter",
            Self::Transfer => "Übertragung & Befehle",
        }
    }

    /// The page that holds the field a `JobEditor` message is about.
    pub(super) fn for_error(message: &str) -> Self {
        const RULES: [(&str, JobEditorPage); 21] = [
            ("aktiven Zeit", JobEditorPage::Schedule),
            ("Intervall", JobEditorPage::Schedule),
            ("Uhrzeit", JobEditorPage::Schedule),
            ("Tag im Monat", JobEditorPage::Schedule),
            ("Verzögerung", JobEditorPage::Schedule),
            ("Höchstwartezeit", JobEditorPage::Schedule),
            ("Abfrageintervall", JobEditorPage::Schedule),
            ("Kontroll-Lauf", JobEditorPage::Schedule),
            ("Vollprüfung", JobEditorPage::Schedule),
            ("Zeit-Toleranz", JobEditorPage::Versions),
            ("Aufbewahrung", JobEditorPage::Versions),
            ("Versionen behalten", JobEditorPage::Versions),
            ("Lösch-Schutz", JobEditorPage::Versions),
            ("Mindestzahl an Löschungen", JobEditorPage::Versions),
            ("Ignoriermuster", JobEditorPage::Filter),
            ("größe", JobEditorPage::Filter),
            ("Mindestalter", JobEditorPage::Filter),
            ("Maximales Alter", JobEditorPage::Filter),
            ("Bandbreite", JobEditorPage::Transfer),
            ("Parallele Übertragungen", JobEditorPage::Transfer),
            ("Wiederholung", JobEditorPage::Transfer),
        ];
        RULES
            .iter()
            .find(|(needle, _)| message.contains(needle))
            .map_or(Self::General, |(_, page)| *page)
    }
}

/// Renders one page of the editor into `ui`. A "…" click is reported through
/// `pick` (field and start value), as before the split into pages.
pub(super) fn render_job_editor_page(
    ui: &mut egui::Ui,
    page: JobEditorPage,
    ed: &mut JobEditor,
    pick: &mut Option<(PickerPurpose, String)>,
) {
    // A vault job (U29-9) has the pages „Tresor“ (the first one) and
    // „Zeitplan“; its other values are fixed (C5.6) and only named.
    let vault = crate::app::vault_modules::vault_sync_editor_ui::is_vault_draft(ed);
    egui::Grid::new(("job_editor_grid", page as u8))
        .num_columns(2)
        .spacing([10.0, 8.0])
        .show(ui, |ui| match page {
            JobEditorPage::General if vault => {
                crate::app::vault_modules::vault_sync_editor_ui::tresor_page(ui, ed)
            }
            JobEditorPage::Versions | JobEditorPage::Filter | JobEditorPage::Transfer if vault => {
                crate::app::vault_modules::vault_sync_editor_ui::fixed_page(ui)
            }
            JobEditorPage::General => general(ui, ed, pick),
            JobEditorPage::Schedule => schedule(ui, ed),
            JobEditorPage::Versions => versions(ui, ed),
            JobEditorPage::Filter => filter(ui, ed),
            JobEditorPage::Transfer => transfer(ui, ed),
        });
}

fn number(ui: &mut egui::Ui, label: &str, hover: &str, value: &mut String) {
    ui.label(label).on_hover_text(hover);
    ui.add(egui::TextEdit::singleline(value).desired_width(80.0));
    ui.end_row();
}

fn general(ui: &mut egui::Ui, ed: &mut JobEditor, pick: &mut Option<(PickerPurpose, String)>) {
    ui.label("Name");
    ui.add(
        egui::TextEdit::singleline(&mut ed.name)
            .hint_text("z. B. Dokumente sichern")
            .desired_width(360.0),
    );
    ui.end_row();

    for source_side in [true, false] {
        let (label, value) = if source_side {
            ("Quelle (A)", &mut ed.source)
        } else {
            ("Ziel (B)", &mut ed.target)
        };
        ui.label(label);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(value)
                    .hint_text("lokaler Ordner / Netzlaufwerk / Verbindung")
                    .desired_width(280.0),
            );
            if ui
                .button("…")
                .on_hover_text(
                    "Im Explorer wählen — lokale Laufwerke oder gespeicherte Verbindungen",
                )
                .clicked()
            {
                let purpose = if source_side {
                    PickerPurpose::SyncSource
                } else {
                    PickerPurpose::SyncTarget
                };
                *pick = Some((purpose, value.clone()));
            }
        });
        ui.end_row();
    }

    ui.label("Richtung")
        .on_hover_text("Methode: in welche Richtung abgeglichen wird");
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

    ui.label("Konflikte")
        .on_hover_text("Was passiert, wenn beide Seiten geändert wurden");
    egui::ComboBox::from_id_salt("job_conf")
        .selected_text(ed.conflict.label())
        .show_ui(ui, |ui| {
            for c in crate::bisync::ConflictMode::ALL {
                ui.selectable_value(&mut ed.conflict, c, c.label());
            }
        });
    ui.end_row();

    ui.label("Löschungen")
        .on_hover_text("Wie mit Dateien umgegangen wird, die auf einer Seite fehlen");
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
        ui.label("Verschieben")
            .on_hover_text("Einseitig: Quelle nach erfolgreicher Kopie löschen (Move)");
        ui.checkbox(&mut ed.move_files, "Dateien verschieben statt kopieren");
        ui.end_row();
    }

    ui.label("Aktiv");
    ui.checkbox(&mut ed.enabled, "Zeitplan aktiv");
    ui.end_row();
}

fn schedule(ui: &mut egui::Ui, ed: &mut JobEditor) {
    ui.label("Auslöser")
        .on_hover_text("Wann dieser Sync automatisch läuft");
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
            number(
                ui,
                "Intervall (min)",
                "Alle N Minuten ausführen",
                &mut ed.interval_min,
            );
        }
        crate::syncjobs::Trigger::Calendar => {
            ui.label("Uhrzeit").on_hover_text("Startzeit HH:MM");
            ui.add(egui::TextEdit::singleline(&mut ed.cal_time).desired_width(80.0));
            ui.end_row();

            ui.label("Wochentage")
                .on_hover_text("Keiner markiert = täglich");
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

            number(
                ui,
                "Tag im Monat",
                "1–31 = monatlich; 0 = Wochentage verwenden",
                &mut ed.cal_monthday,
            );

            ui.label("Nachholen").on_hover_text(
                "Verpasste geplante Läufe nachholen, statt auf den nächsten Termin zu warten",
            );
            ui.checkbox(&mut ed.catch_up, "verpasste Läufe nachholen");
            ui.end_row();
        }
        crate::syncjobs::Trigger::RealTime => {
            number(ui, "Verzögerung (s)", "Wartezeit nach der letzten Änderung, bevor synchronisiert wird (entprellt). Echtzeit beobachtet die lokale Seite.", &mut ed.rt_debounce);
            number(ui, "Höchstwartezeit (s)", "Längste Wartezeit nach der ersten Änderung, solange weitere Änderungen kommen (0 = automatisch: fünffache Verzögerung, mindestens 300 s)", &mut ed.rt_max_latency);
            number(ui, "Abfrageintervall (s)", "So oft werden Seiten ohne Änderungsereignisse abgefragt, z. B. Netzlaufwerke und Remote-Seiten (0 = nie)", &mut ed.rt_poll);
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

    number(ui, "Kontroll-Lauf (s)", "Abstand der Kontroll-Läufe, die die beobachtete(n) Seite(n) vollständig durchgehen (0 = keine)", &mut ed.verify_interval);
    number(ui, "Vollprüfung des Ziels (s)", "Abstand der vollständigen Prüfung einer reinen Zielseite; dazwischen prüft der Spiegel nur geänderte Pfade (0 = keine)", &mut ed.verify_target);

    ui.label("Aktive Zeiten")
        .on_hover_text("Nur in diesem Zeitfenster ausführen (von = bis ⇒ immer)");
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut ed.active_from).desired_width(64.0));
        ui.label("–");
        ui.add(egui::TextEdit::singleline(&mut ed.active_to).desired_width(64.0));
    });
    ui.end_row();
}

fn versions(ui: &mut egui::Ui, ed: &mut JobEditor) {
    ui.label("Vergleich")
        .on_hover_text("Wie entschieden wird, ob zwei Dateien gleich sind");
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
        number(ui, "Zeit-Toleranz (s)", "Zeitstempel-Unterschiede bis zu N Sekunden als gleich werten (FAT/exFAT, Sommerzeit: 1–2)", &mut ed.modify_window);
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
            number(
                ui,
                "Aufbewahrung (Tage)",
                "0 = für immer behalten",
                &mut ed.retain_days,
            );
        }
        crate::bisync::VersioningScheme::Count => {
            number(
                ui,
                "Versionen behalten",
                "Anzahl der neuesten Versions-Schnappschüsse (0 = alle)",
                &mut ed.retain_count,
            );
        }
        _ => {}
    }

    ui.label("Versionsort")
        .on_hover_text("Wo Versionen ersetzter und gelöschter Dateien liegen");
    egui::ComboBox::from_id_salt("job_ver_place")
        .selected_text(ed.versions_location.label())
        .show_ui(ui, |ui| {
            for place in crate::bisync::VersionsLocation::ALL {
                ui.selectable_value(&mut ed.versions_location, place, place.label());
            }
        });
    ui.end_row();

    ui.label("Papierkorb").on_hover_text(
        "Gelöschte Dateien in den Papierkorb verschieben statt entfernen (nur lokale Pfade)",
    );
    ui.checkbox(&mut ed.use_recycle_bin, "Löschungen in den Papierkorb");
    ui.end_row();

    ui.label("Lösch-Schutz").on_hover_text("Abbrechen, wenn ein Lauf mehr als so viele Dateien bzw. Prozent einer Seite löschen würde (0 = aus); der Prozentwert greift ab der genannten Zahl an Löschungen. Schützt vor versehentlichem Massenlöschen, z. B. wenn ein Laufwerk nicht verbunden ist.");
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut ed.max_delete).desired_width(70.0));
        ui.label("Dateien /");
        ui.add(egui::TextEdit::singleline(&mut ed.max_delete_pct).desired_width(50.0));
        ui.label("% ab");
        ui.add(egui::TextEdit::singleline(&mut ed.max_delete_min).desired_width(50.0));
        ui.label("gelöschten Dateien");
    });
    ui.end_row();
}

fn filter(ui: &mut egui::Ui, ed: &mut JobEditor) {
    ui.label("Versteckte Dateien");
    ui.checkbox(&mut ed.include_hidden, "einbeziehen");
    ui.end_row();

    ui.label("Ignorieren")
        .on_hover_text("Glob-Muster, eines pro Zeile (z. B. **/*.tmp, node_modules/**)");
    ui.vertical(|ui| {
        ui.add(
            egui::TextEdit::multiline(&mut ed.ignore)
                .hint_text("**/*.tmp\nnode_modules/**")
                .desired_rows(3)
                .desired_width(360.0),
        );
        if ui
            .small_button("＋ Standard-Ausschlüsse")
            .on_hover_text("Übliche temporäre/System-Dateien ergänzen")
            .clicked()
        {
            const DEFAULTS: &[&str] = &[
                "**/*.tmp",
                "**/~$*",
                "**/desktop.ini",
                "**/Thumbs.db",
                "**/.DS_Store",
                "**/System Volume Information/**",
            ];
            let mut lines: Vec<String> = ed
                .ignore
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            for d in DEFAULTS {
                if !lines.iter().any(|l| l == d) {
                    lines.push((*d).to_string());
                }
            }
            ed.ignore = lines.join("\n");
        }
    });
    ui.end_row();

    ui.label("Größe (KB)")
        .on_hover_text("Nur Dateien in diesem Größenbereich (0 = keine Grenze)");
    ui.horizontal(|ui| {
        ui.label("min");
        ui.add(egui::TextEdit::singleline(&mut ed.filter_min_size_kb).desired_width(70.0));
        ui.label("max");
        ui.add(egui::TextEdit::singleline(&mut ed.filter_max_size_kb).desired_width(70.0));
    });
    ui.end_row();

    ui.label("Alter (Tage)")
        .on_hover_text("Nur Dateien, die jünger als / älter als N Tage sind (0 = aus)");
    ui.horizontal(|ui| {
        ui.label("jünger als");
        ui.add(egui::TextEdit::singleline(&mut ed.filter_max_age_days).desired_width(60.0));
        ui.label("älter als");
        ui.add(egui::TextEdit::singleline(&mut ed.filter_min_age_days).desired_width(60.0));
    });
    ui.end_row();

    ui.label("Eingehängte Dateisysteme").on_hover_text("Andere Dateisysteme, die innerhalb einer Seite eingehängt sind, mit synchronisieren (Linux). Links und Junctions bleiben geschützt ausgelassen.");
    ui.checkbox(&mut ed.cross_mounts, "einbeziehen");
    ui.end_row();
}

fn transfer(ui: &mut egui::Ui, ed: &mut JobEditor) {
    ui.label("Bandbreite")
        .on_hover_text("Übertragungsrate begrenzen (KB/s, 0 = unbegrenzt)");
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut ed.bwlimit_kbps).desired_width(80.0));
        ui.label("KB/s · max");
        ui.add(egui::TextEdit::singleline(&mut ed.max_transfers).desired_width(50.0));
        ui.label("parallel");
    });
    ui.end_row();

    ui.label("Zuverlässigkeit")
        .on_hover_text("Sichere Kopien (temporär + umbenennen), Größe nach dem Kopieren prüfen");
    ui.horizontal(|ui| {
        ui.checkbox(&mut ed.atomic_copy, "Sichere Kopien");
        ui.checkbox(&mut ed.verify, "Überprüfen");
    });
    ui.end_row();

    ui.label("Wiederholungen")
        .on_hover_text("Fehlgeschlagene Übertragungen N-mal wiederholen, mit Pause dazwischen");
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut ed.retries).desired_width(50.0));
        ui.label("× / Pause");
        ui.add(egui::TextEdit::singleline(&mut ed.retry_delay_secs).desired_width(50.0));
        ui.label("s");
    });
    ui.end_row();

    for (label, hover, value) in [
        ("Befehl davor", "Vor manuellem oder geplantem Lauf und vor dem Verbinden ausführen. Ein Fehler bricht den Sync ab; es werden keine Dateien verändert.", &mut ed.run_before),
        ("Befehl danach", "Nach jedem gestarteten Sync ausführen, auch bei Sync-Fehlern oder Konflikten (nicht nach Abbruch). Fehler werden im Ergebnis protokolliert.", &mut ed.run_after),
        ("Befehl bei Abbruch", "Nach einem abgebrochenen oder fehlgeschlagenen Lauf ausführen (Aufräumen), wie „Befehl danach“.", &mut ed.run_cleanup),
    ] {
        ui.label(label).on_hover_text(hover);
        ui.add(egui::TextEdit::singleline(value).hint_text("optional").desired_width(360.0));
        ui.end_row();
    }
}
