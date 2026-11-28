//! „In Dateien suchen“ without a window: the result tree from search events
//! (header, totals, folding, clearing), F4 across files, „In diesen
//! Ergebnissen suchen“, the copy texts, wrapped rows and the dialog checks.
use crate::agent_proto::{
    ContentEvent, ContentQuery, ContentSummary, LineHit, SkipReason, TextMode,
};

use super::content_search_dialog::{normalize_folder, FindForm};
use super::content_search_nav::WithinQuery;
use super::content_search_results::{
    group_thousands, RecordState, Row, RowKind, SearchRecord, SearchResults,
};
use super::content_search_results_ui::chunk_ranges;
use super::text_tools::TextPlace;

fn hit(line: u64, text: &str, ranges: &[(u32, u32)], context: bool) -> LineHit {
    LineHit {
        line,
        offset: 0,
        text: text.to_string(),
        ranges: ranges.to_vec(),
        truncated: false,
        context,
    }
}

fn hits(rel: &str, lines: Vec<LineHit>) -> ContentEvent {
    ContentEvent::Hits {
        rel: rel.to_string(),
        size: 10,
        mtime_ms: 0,
        lines,
        last: true,
    }
}

/// A finished search of „foo“ below /p: a.txt (2 hits, 1 context line),
/// b/c.rs (1 hit), one binary file.
fn sample(results: &mut SearchResults, query: &ContentQuery) -> u64 {
    let id = results.next_id();
    results.begin(
        SearchRecord::new(id, query, "/p".into(), TextPlace::local()),
        false,
    );
    results.apply(
        id,
        hits(
            "a.txt",
            vec![
                hit(1, "foo one", &[(0, 3)], false),
                hit(2, "between", &[], true),
            ],
        ),
    );
    results.apply(
        id,
        ContentEvent::Progress {
            files: 7,
            bytes: 99,
            hits: 1,
        },
    );
    results.apply(
        id,
        hits("b/c.rs", vec![hit(7, "let foo = 1;", &[(4, 7)], false)]),
    );
    // A second event of the same file joins its entry.
    results.apply(
        id,
        hits("a.txt", vec![hit(3, "foo three", &[(0, 3)], false)]),
    );
    results.apply(
        id,
        ContentEvent::Skipped {
            rel: "bin.dat".into(),
            reason: SkipReason::Binary,
        },
    );
    id
}

#[test]
fn content_search_task_results_tree_counts_header_folding_and_clearing() {
    assert_eq!(group_thousands(0), "0");
    assert_eq!(group_thousands(999), "999");
    assert_eq!(group_thousands(1_234_567), "1.234.567");
    let mut query = ContentQuery::text("foo");
    query.match_case = true;
    query.whole_word = true;
    let mut results = SearchResults::default();
    let id = sample(&mut results, &query);
    let record = results.record(id).unwrap();
    assert!(record
        .header()
        .starts_with("Suche „foo“ (läuft: 3 Treffer in 2 Dateien, 7 durchsucht)"));
    assert_eq!(record.files.len(), 2);
    assert_eq!(record.files[0].hits, 2, "context lines are not hits");
    assert_eq!(record.file_label(1), "/p/b/c.rs (1 Treffer)");
    results.finish(
        id,
        RecordState::Done(ContentSummary {
            files_searched: 1234,
            files_matched: 2,
            hits: 3,
            ..ContentSummary::default()
        }),
    );
    assert_eq!(
        results.record(id).unwrap().header(),
        "Suche „foo“ (3 Treffer in 2 Dateien von 1.234 durchsuchten) [Normal: Groß/Klein/Wort]"
    );
    let rows = results.rows();
    let kinds: Vec<RowKind> = rows.iter().map(|row| row.kind).collect();
    assert_eq!(
        kinds,
        vec![
            RowKind::Header,
            RowKind::File(0),
            RowKind::Line(0, 0),
            RowKind::Line(0, 1),
            RowKind::Line(0, 2),
            RowKind::File(1),
            RowKind::Line(1, 0),
            RowKind::SkippedHeader,
        ]
    );
    assert_eq!(results.row_text(rows[2]), "Zeile 1: foo one");
    assert_eq!(results.row_text(rows[7]), "Ausgelassen (1)");
    results.toggle(rows[7]);
    assert!(results
        .rows()
        .iter()
        .any(|row| row.kind == RowKind::Skipped(0)));
    assert!(results
        .row_text(Row {
            search: id,
            kind: RowKind::Skipped(0)
        })
        .starts_with("/p/bin.dat – Binärdatei"));
    results.toggle(rows[1]);
    assert_eq!(results.rows().len(), 6, "a folded file hides its lines");

    // A new search goes on top; the older one folds to its header.
    let mut regex = ContentQuery::text("ba.");
    regex.mode = TextMode::Regex;
    regex.dot_matches_newline = true;
    let newer = sample(&mut results, &regex);
    assert_eq!(results.records[0].id, newer);
    assert!(results.records[0]
        .options
        .starts_with("[Regex: . findet Zeilenumbruch]"));
    assert!(!results.records[1].expanded);
    let rows = results.rows();
    assert_eq!(
        rows.last().map(|row| (row.search, row.kind)),
        Some((id, RowKind::Header))
    );
    results.set_all_expanded(true);
    assert!(results.records.iter().all(|record| record.expanded));

    // „Bei jeder Suche leeren“ keeps only the new search; „Leeren“ all.
    let third = results.next_id();
    results.begin(
        SearchRecord::new(third, &query, "/q".into(), TextPlace::local()),
        true,
    );
    assert_eq!(results.records.len(), 1);
    results.finish(third, RecordState::Failed("weg".into()));
    assert_eq!(
        results.records[0].header(),
        "Suche „foo“ fehlgeschlagen: weg"
    );
    results.clear();
    assert!(results.rows().is_empty());
}

#[test]
fn content_search_task_f4_moves_across_files_skips_context_and_wraps() {
    let mut results = SearchResults::default();
    let id = sample(&mut results, &ContentQuery::text("foo"));
    let line = |file, line| Row {
        search: id,
        kind: RowKind::Line(file, line),
    };
    assert_eq!(results.next_hit(None, true), Some(line(0, 0)));
    assert_eq!(results.next_hit(None, false), Some(line(1, 0)));
    assert_eq!(
        results.next_hit(Some(line(0, 0)), true),
        Some(line(0, 2)),
        "context line 1 is passed over"
    );
    assert_eq!(
        results.next_hit(Some(line(0, 2)), true),
        Some(line(1, 0)),
        "into the next file"
    );
    assert_eq!(
        results.next_hit(Some(line(1, 0)), true),
        Some(line(0, 0)),
        "wraps at the end"
    );
    assert_eq!(
        results.next_hit(Some(line(0, 0)), false),
        Some(line(1, 0)),
        "wraps at the start"
    );
    let file_b = Row {
        search: id,
        kind: RowKind::File(1),
    };
    assert_eq!(results.next_hit(Some(file_b), true), Some(line(1, 0)));
    assert_eq!(results.next_hit(Some(file_b), false), Some(line(0, 2)));
    // A hit in a folded file or search is unfolded when reached.
    results.toggle(file_b);
    results.toggle(Row {
        search: id,
        kind: RowKind::Header,
    });
    results.reveal(line(1, 0));
    assert!(results.rows().contains(&line(1, 0)));
    assert_eq!(SearchResults::default().next_hit(None, true), None);
}

#[test]
fn content_search_task_search_within_results_and_copy_texts() {
    let mut results = SearchResults::default();
    let id = sample(&mut results, &ContentQuery::text("foo"));
    let within = WithinQuery {
        pattern: "THREE".into(),
        regex: false,
        match_case: false,
    };
    let derived = results.search_within(id, &within, false).unwrap();
    let record = results.record(derived).unwrap();
    assert_eq!(record.files.len(), 1);
    assert_eq!(record.files[0].lines[0].ranges, vec![(4, 9)]);
    assert!(record.header().starts_with(
        "Suche „THREE“ in den Ergebnissen von „foo“ (1 Treffer in 1 Datei von 2 durchsuchten)"
    ));
    let regex = WithinQuery {
        pattern: r"^let\s".into(),
        regex: true,
        match_case: true,
    };
    let derived = results.search_within(id, &regex, false).unwrap();
    assert_eq!(results.record(derived).unwrap().files[0].rel, "b/c.rs");
    let broken = WithinQuery {
        pattern: "(".into(),
        regex: true,
        match_case: false,
    };
    assert!(results
        .search_within(id, &broken, false)
        .unwrap_err()
        .starts_with("Ungültiger regulärer Ausdruck"));
    let rows = [
        Row {
            search: id,
            kind: RowKind::Line(0, 0),
        },
        Row {
            search: id,
            kind: RowKind::Line(1, 0),
        },
    ];
    assert_eq!(results.copy_lines(&rows), "foo one\nlet foo = 1;");
    assert_eq!(
        results.copy_rows(&rows),
        "Zeile 1: foo one\nZeile 7: let foo = 1;"
    );
    assert_eq!(
        results.row_paths(&rows),
        vec!["/p/a.txt".to_string(), "/p/b/c.rs".to_string()]
    );
    let header = [Row {
        search: id,
        kind: RowKind::Header,
    }];
    assert_eq!(results.row_paths(&header).len(), 2);
    // Wrapped rows: the first row full, continuations indented by six.
    assert_eq!(chunk_ranges(8, 10), vec![(0, 8)]);
    assert_eq!(
        chunk_ranges(25, 10),
        vec![(0, 10), (10, 14), (14, 18), (18, 22), (22, 25)]
    );
}

#[test]
fn content_search_task_dialog_validates_and_locks_whole_word_for_regex() {
    let mut form = FindForm::default();
    assert!(form.subfolders && !form.hidden && !form.match_case && !form.whole_word);
    assert_eq!(
        form.validate().unwrap_err(),
        "Bitte einen Suchtext eingeben."
    );
    form.pattern = "foo".into();
    assert_eq!(form.validate().unwrap_err(), "Bitte einen Ordner angeben.");
    form.folder = "C:\\Daten\\".into();
    form.whole_word = true;
    form.filters = " *.rs !\\target ".into();
    let (query, folder) = form.validate().unwrap();
    assert_eq!(folder, "C:/Daten");
    assert!(query.whole_word);
    assert_eq!(query.filters, "*.rs !\\target");
    assert_eq!(query.folder_include, None);
    // Regular expressions: whole word is locked off, ". findet Zeilenumbruch" on.
    form.mode = TextMode::Regex;
    form.dot_matches_newline = true;
    assert!(!form.whole_word_enabled() && form.dot_enabled());
    let query = form.query();
    assert!(!query.whole_word && query.dot_matches_newline);
    form.pattern = "(".into();
    assert!(
        !form.validate().unwrap_err().is_empty(),
        "an invalid regex starts no search"
    );
    form.mode = TextMode::Normal;
    assert!(
        !form.query().dot_matches_newline,
        ". findet Zeilenumbruch only for regexes"
    );
    form.pattern = "foo".into();
    form.folder_include = "([".into();
    assert!(
        form.validate().is_err(),
        "an invalid folder regex starts no search"
    );
    form.folder_include = r"^src(/|$)".into();
    form.folder_exclude = "  ".into();
    let (query, _) = form.validate().unwrap();
    assert_eq!(query.folder_include.as_deref(), Some(r"^src(/|$)"));
    assert_eq!(query.folder_exclude, None);
    assert_eq!(normalize_folder("/"), "/");
    assert_eq!(normalize_folder("C:/"), "C:/");
    assert_eq!(normalize_folder(" /home/u/ "), "/home/u");
}
