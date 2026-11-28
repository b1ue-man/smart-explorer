//! „In Dateien ersetzen“ on the desktop (plan
//! docs/plaene/2026-11-28-ersetzen-masken-shell, M3): which hits a
//! selection takes, „Aus dem Ergebnis entfernen“, the confirmation text, the
//! header of a run with „Rückgängig“, the dialog's replacement history and
//! the core request; and in real egui frames the dialog, the dry run, the
//! confirmation with „Abbrechen“ preselected, the run and its undo.
use crate::agent_proto::{
    ContentEvent, ContentQuery, ContentSummary, LineHit, ReplaceSelection, SkipReason,
};
use crate::app::content_search_dialog::{FindForm, FindHistory};
use crate::app::content_search_results::{RecordState, Row, RowKind, SearchRecord, SearchResults};

use super::content_replace_confirm::{confirm_lines, PreviewTotals};
use super::content_replace_results::{JobSelection, ReplaceJob, SelectedHit, UndoState};
use super::content_replace_run::request;
use super::TextPlace;

fn hit(line: u64, text: &str, ranges: &[(u32, u32)], context: bool, truncated: bool) -> LineHit {
    LineHit {
        line,
        offset: 0,
        text: text.to_string(),
        ranges: ranges.to_vec(),
        truncated,
        context,
    }
}

/// A finished search of „foo“ below /p: a.txt (lines 1 and 3, context
/// line 2), b.txt (one cut line), one binary file.
fn sample(results: &mut SearchResults) -> u64 {
    let id = results.next_id();
    let mut query = ContentQuery::text("foo");
    query.filters = "*.txt".into();
    results.begin(
        SearchRecord::new(id, &query, "/p".into(), TextPlace::local()),
        false,
    );
    results.apply(
        id,
        ContentEvent::Hits {
            rel: "a.txt".into(),
            size: 20,
            mtime_ms: 5,
            lines: vec![
                hit(1, "foo", &[(0, 3)], false, false),
                hit(2, "davor", &[], true, false),
                hit(3, "foo foo", &[(0, 3), (4, 7)], false, false),
            ],
            last: true,
        },
    );
    results.apply(
        id,
        ContentEvent::Hits {
            rel: "b.txt".into(),
            size: 10,
            mtime_ms: 9,
            lines: vec![hit(7, "…foo…", &[(3, 6)], false, true)],
            last: true,
        },
    );
    results.apply(
        id,
        ContentEvent::Skipped {
            rel: "bild.dat".into(),
            reason: SkipReason::Binary,
        },
    );
    results.finish(id, RecordState::Done(ContentSummary::default()));
    id
}

fn row(search: u64, kind: RowKind) -> Row {
    Row { search, kind }
}

fn hits_of(job: &ReplaceJob) -> &[SelectedHit] {
    match &job.selection {
        JobSelection::Hits(hits) => hits,
        JobSelection::All => panic!("a selection names hits"),
    }
}

#[test]
fn replace_ui_task_selection_takes_hit_lines_of_one_finished_search() {
    let mut results = SearchResults::default();
    let id = sample(&mut results);
    // A line row takes that line; the context line between is passed over.
    let job = results
        .replace_selected(
            &[row(id, RowKind::Line(0, 2)), row(id, RowKind::Line(0, 1))],
            "bar",
        )
        .unwrap();
    assert_eq!(
        hits_of(&job),
        &[SelectedHit {
            rel: "a.txt".into(),
            line: 3,
            ranges: vec![(0, 3), (4, 7)],
            size: 20,
            mtime_ms: 5,
        }]
    );
    assert_eq!(
        (job.root.as_str(), job.replacement.as_str(), job.source),
        ("/p", "bar", Some(id))
    );
    assert_eq!(job.query.filters, "*.txt", "the query of the search");
    // A file row takes all of its hit lines; a cut line names no ranges
    // (its ranges count from the cut): every match of that line.
    let job = results
        .replace_selected(&[row(id, RowKind::File(1))], "bar")
        .unwrap();
    assert_eq!(hits_of(&job)[0].ranges, Vec::<(u32, u32)>::new());
    assert_eq!(job.chosen_count(), Some(1));
    let all = results.replace_remaining(id, "").unwrap();
    assert_eq!(
        hits_of(&all)
            .iter()
            .map(|hit| (hit.rel.as_str(), hit.line))
            .collect::<Vec<_>>(),
        vec![("a.txt", 1), ("a.txt", 3), ("b.txt", 7)]
    );
    assert_eq!(all.chosen_count(), Some(4));
    // Context and skipped rows alone name no hit.
    assert!(results
        .replace_selected(&[row(id, RowKind::Line(0, 1))], "bar")
        .is_err());
    assert!(results.replace_selected(&[], "bar").is_err());
    // Rows of two searches, a running search and a filtered record refuse.
    let second = sample(&mut results);
    assert!(results
        .replace_selected(
            &[row(id, RowKind::File(0)), row(second, RowKind::File(0))],
            "x"
        )
        .is_err());
    results.record_mut(second).unwrap().state = RecordState::Running;
    assert_eq!(
        results.replace_remaining(second, "x").err().as_deref(),
        Some("Die Suche läuft noch.")
    );
    results.record_mut(second).unwrap().query = None;
    results.record_mut(second).unwrap().state = RecordState::Cancelled;
    assert!(results.replace_remaining(second, "x").is_err());
    // The core request carries the hits and the dry-run flag.
    let core = request(&job, true);
    assert!(core.dry_run);
    assert_eq!(core.replacement, "bar");
    let ReplaceSelection::Hits(refs) = core.selection else {
        panic!("hits");
    };
    assert_eq!(
        (
            refs[0].rel.as_str(),
            refs[0].line,
            refs[0].size,
            refs[0].mtime_ms
        ),
        ("b.txt", 7, 10, 9)
    );
}

#[test]
fn replace_ui_task_dismiss_removes_hits_files_searches_and_skips() {
    let mut results = SearchResults::default();
    let id = sample(&mut results);
    assert!(results.dismiss(&[row(id, RowKind::Line(0, 0))]));
    let record = results.record(id).unwrap();
    assert_eq!(record.files[0].hits, 1);
    assert_eq!(record.files[0].lines.len(), 2, "context line stays");
    // The last hit line of a file takes the file with it.
    assert!(results.dismiss(&[row(id, RowKind::Line(0, 1))]));
    let record = results.record(id).unwrap();
    assert_eq!(record.files.len(), 1);
    assert_eq!(record.files[0].rel, "b.txt");
    assert_eq!(record.file_slot("b.txt"), Some(0), "index rebuilt");
    assert!(results.dismiss(&[row(id, RowKind::SkippedHeader)]));
    assert!(results.record(id).unwrap().skipped.is_empty());
    // What is gone is never replaced.
    let job = results.replace_remaining(id, "x").unwrap();
    assert_eq!(hits_of(&job).len(), 1);
    assert!(!results.dismiss(&[row(id, RowKind::Line(5, 0))]));
    assert!(results.dismiss(&[row(id, RowKind::Header)]));
    assert!(results.records.is_empty());
}

fn job(selection: JobSelection, replacement: &str) -> ReplaceJob {
    let mut query = ContentQuery::text("foo");
    query.respect_ignore_files = true;
    ReplaceJob {
        query,
        root: "/p".into(),
        place: TextPlace::local(),
        replacement: replacement.into(),
        selection,
        source: None,
    }
}

#[test]
fn replace_ui_task_confirmation_and_run_header_with_undo_states() {
    let totals = PreviewTotals {
        files_searched: 10,
        files_changed: 2,
        replacements: 1_234,
        skipped: vec![("c.txt".into(), "schreibgeschützt".into())],
    };
    assert_eq!(
        confirm_lines(&job(JobSelection::All, "bar"), &totals),
        vec![
            "Wirklich alle Vorkommen ersetzen in:",
            "/p",
            "auf: Dieser Computer",
            "Für Dateitypen: *.*",
            "Ignorierdateien (.gitignore) werden beachtet.",
            "1.234 Stellen in 2 Dateien durch „bar“ ersetzen?",
            "1 übersprungen (Gründe unten).",
            "Vor dem Überschreiben wird jede Datei gesichert; „Rückgängig“ im Ergebnis stellt sie \
             wieder her.",
        ]
    );
    let one = PreviewTotals {
        files_changed: 1,
        replacements: 1,
        ..PreviewTotals::default()
    };
    let lines = confirm_lines(&job(JobSelection::Hits(Vec::new()), ""), &one);
    assert_eq!(lines[0], "Wirklich die ausgewählten Treffer ersetzen in:");
    assert_eq!(
        lines[5],
        "1 Stelle in 1 Datei löschen (Ersetzen durch nichts)?"
    );

    let mut results = SearchResults::default();
    let id = results.next_id();
    results.begin(
        SearchRecord::new_replace(id, &job(JobSelection::All, "bar")),
        false,
    );
    let record = results.record_mut(id).unwrap();
    record.progress.0 = 4;
    assert_eq!(
        record.header(),
        "Ersetzen „foo“ durch „bar“ (läuft: 0 Ersetzungen in 0 Dateien, 4 durchsucht) [Normal]"
    );
    assert!(record.replaceable().is_err(), "a run is not replaced again");
    let info = record.replace.as_mut().unwrap();
    info.replacements = 1;
    info.files_searched = 7;
    info.undo = UndoState::Available("r1".into());
    record.state = RecordState::Done(ContentSummary::default());
    record
        .skipped
        .push(("c.txt".into(), "seit der Suche geändert".into()));
    assert_eq!(
        record.header(),
        "Ersetzen „foo“ durch „bar“ (1 Ersetzung in 0 Dateien von 7 durchsuchten, 1 \
         übersprungen) [Normal]"
    );
    record.replace.as_mut().unwrap().undo = UndoState::Done;
    assert!(record.header().ends_with(" – rückgängig gemacht"));
    record.replace.as_mut().unwrap().undo = UndoState::Partly(1);
    assert!(record
        .header()
        .ends_with(" – teilweise rückgängig gemacht, 1 nicht zurückgesetzt"));
    record.state = RecordState::Cancelled;
    record.replace.as_mut().unwrap().undo = UndoState::Failed("weg".into());
    assert!(record
        .header()
        .contains("(abgebrochen: 1 Ersetzung in 0 Dateien, 1 übersprungen)"));
    assert!(record
        .header()
        .ends_with(" – Rückgängig fehlgeschlagen: weg"));
}

#[test]
fn replace_ui_task_dialog_form_history_and_place_location() {
    let form = FindForm {
        pattern: "foo".into(),
        replacement: "bar".into(),
        folder: "/p".into(),
        respect_ignore_files: true,
        ..FindForm::default()
    };
    let (query, folder) = form.validate().unwrap();
    assert!(query.respect_ignore_files);
    assert_eq!(folder, "/p");
    assert!(
        !FindForm::default().respect_ignore_files,
        "off as in Notepad++"
    );
    let mut history = FindHistory::default();
    history.remember_replacement(&form);
    history.remember_replacement(&FindForm {
        replacement: String::new(),
        ..form.clone()
    });
    assert_eq!(
        history.replacements,
        vec!["bar"],
        "an empty text is not kept"
    );
    assert_eq!(history.patterns, vec!["foo"]);
    // A history file of an older version has no replacements.
    let old: FindHistory = serde_json::from_str(r#"{"patterns":["x"],"filters":[]}"#).unwrap();
    assert!(old.replacements.is_empty());

    assert_eq!(TextPlace::local().location("/p/q"), "/p/q");
    let backend: crate::vfs::BackendHandle =
        std::sync::Arc::new(crate::vfs::LocalBackend::new("/"));
    let remote = TextPlace::remote(backend, "Server".into());
    assert_eq!(remote.location("/srv/x"), "@Server:/srv/x");
    let endpoint = TextPlace {
        endpoint: Some("sftp://ich@host:22".into()),
        ..remote
    };
    assert_eq!(endpoint.location("/srv/x"), "sftp://ich@host:22/srv/x");
}

#[path = "content_replace_task_ui_tests.rs"]
mod ui;
