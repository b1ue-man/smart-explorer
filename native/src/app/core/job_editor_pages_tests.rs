//! Every setting of the shared `JobEditor` is drawn on exactly one page of
//! the desktop sync-setup editor (rendered headless, labels read from the
//! painted shapes).
use super::gui_design_task_capture::Capture;
use super::job_editor_pages::{render_job_editor_page, JobEditorPage};
use super::JobEditor;
use eframe::egui;
use std::collections::BTreeSet;

/// The labels one page shows for `editor`, from a settled headless frame.
fn labels(page: JobEditorPage, editor: &mut JobEditor) -> BTreeSet<String> {
    let ctx = egui::Context::default();
    let mut capture = Capture::default();
    for _ in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 4000.0),
            )),
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut pick = None;
                render_job_editor_page(ui, page, &mut *editor, &mut pick);
            });
        });
        capture.record(&output);
    }
    capture
        .labels
        .iter()
        .map(|(text, _, _)| text.clone())
        .collect()
}

fn editor() -> JobEditor {
    let mut editor = JobEditor::blank("/a".into(), "/b".into());
    editor.direction = crate::bisync::Direction::AtoB;
    editor
}

/// Labels of each page over the variants that show conditional fields
/// (every trigger, every versioning scheme, one-way direction).
fn labels_by_page() -> Vec<(JobEditorPage, BTreeSet<String>)> {
    JobEditorPage::ALL
        .iter()
        .map(|page| {
            let mut seen = BTreeSet::new();
            let mut variants = vec![editor()];
            if *page == JobEditorPage::Schedule {
                variants = crate::syncjobs::Trigger::ALL
                    .iter()
                    .map(|trigger| JobEditor {
                        trigger: *trigger,
                        ..editor()
                    })
                    .collect();
            }
            if *page == JobEditorPage::Versions {
                variants = crate::bisync::VersioningScheme::ALL
                    .iter()
                    .map(|scheme| JobEditor {
                        versioning_scheme: *scheme,
                        ..editor()
                    })
                    .collect();
            }
            for mut editor in variants {
                seen.extend(labels(*page, &mut editor));
            }
            (*page, seen)
        })
        .collect()
}

#[test]
fn luecken_task_job_editor_pages_cover_every_field() {
    // One label per `JobEditor` field (two fields share the label of their
    // row: size, age, bandwidth/parallel, reliability, retries, active hours,
    // delete guard).
    let expected: [(&str, JobEditorPage); 33] = [
        ("Name", JobEditorPage::General),
        ("Quelle (A)", JobEditorPage::General),
        ("Ziel (B)", JobEditorPage::General),
        ("Richtung", JobEditorPage::General),
        ("Konflikte", JobEditorPage::General),
        ("Löschungen", JobEditorPage::General),
        ("Verschieben", JobEditorPage::General),
        ("Aktiv", JobEditorPage::General),
        ("Auslöser", JobEditorPage::Schedule),
        ("Intervall (min)", JobEditorPage::Schedule),
        ("Uhrzeit", JobEditorPage::Schedule),
        ("Wochentage", JobEditorPage::Schedule),
        ("Tag im Monat", JobEditorPage::Schedule),
        ("Nachholen", JobEditorPage::Schedule),
        ("Verzögerung (s)", JobEditorPage::Schedule),
        ("Höchstwartezeit (s)", JobEditorPage::Schedule),
        ("Abfrageintervall (s)", JobEditorPage::Schedule),
        ("Gerät/USB", JobEditorPage::Schedule),
        ("Kontroll-Lauf (s)", JobEditorPage::Schedule),
        ("Vollprüfung des Ziels (s)", JobEditorPage::Schedule),
        ("Aktive Zeiten", JobEditorPage::Schedule),
        ("Vergleich", JobEditorPage::Versions),
        ("Zeit-Toleranz (s)", JobEditorPage::Versions),
        ("Aufbewahrung (Tage)", JobEditorPage::Versions),
        ("Versionen behalten", JobEditorPage::Versions),
        ("Versionsort", JobEditorPage::Versions),
        ("Papierkorb", JobEditorPage::Versions),
        ("Lösch-Schutz", JobEditorPage::Versions),
        ("Eingehängte Dateisysteme", JobEditorPage::Filter),
        ("Ignorieren", JobEditorPage::Filter),
        ("Befehl davor", JobEditorPage::Transfer),
        ("Befehl danach", JobEditorPage::Transfer),
        ("Befehl bei Abbruch", JobEditorPage::Transfer),
    ];
    let pages = labels_by_page();
    for (label, home) in expected {
        let found: Vec<JobEditorPage> = pages
            .iter()
            .filter(|(_, seen)| seen.contains(label))
            .map(|(page, _)| *page)
            .collect();
        assert_eq!(found, [home], "{label:?} is drawn on {found:?}");
    }
    for (label, home) in [
        ("Versteckte Dateien", JobEditorPage::Filter),
        ("Größe (KB)", JobEditorPage::Filter),
        ("Alter (Tage)", JobEditorPage::Filter),
        ("Bandbreite", JobEditorPage::Transfer),
        ("Zuverlässigkeit", JobEditorPage::Transfer),
        ("Wiederholungen", JobEditorPage::Transfer),
    ] {
        assert!(
            pages
                .iter()
                .any(|(page, seen)| *page == home && seen.contains(label)),
            "{label:?} missing on {home:?}"
        );
    }
}

#[test]
fn luecken_task_job_editor_errors_point_to_their_page() {
    let (source, target) = ("/a", "/b");
    let mut editor = JobEditor::blank(source.into(), target.into());
    editor.rt_poll = "x".into();
    let error = editor.build_sync_job(None).unwrap_err();
    assert_eq!(JobEditorPage::for_error(&error), JobEditorPage::Schedule);
    let mut editor = JobEditor::blank(source.into(), target.into());
    editor.max_delete_min = "x".into();
    let error = editor.build_sync_job(None).unwrap_err();
    assert_eq!(JobEditorPage::for_error(&error), JobEditorPage::Versions);
    let mut editor = JobEditor::blank(source.into(), target.into());
    editor.filter_max_size_kb = "x".into();
    let error = editor.build_sync_job(None).unwrap_err();
    assert_eq!(JobEditorPage::for_error(&error), JobEditorPage::Filter);
    let mut editor = JobEditor::blank(source.into(), target.into());
    editor.retry_delay_secs = "x".into();
    let error = editor.build_sync_job(None).unwrap_err();
    assert_eq!(JobEditorPage::for_error(&error), JobEditorPage::Transfer);
    assert_eq!(
        JobEditorPage::for_error("Spiegel-Löschungen benötigen eine eindeutige Sync-Richtung."),
        JobEditorPage::General
    );
}
