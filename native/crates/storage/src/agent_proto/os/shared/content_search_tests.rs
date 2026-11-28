//! The search core of "Suchen in Dateien" (plan docs/plaene/2026-11-27-dateien-suche-ansicht
//! section B) on a local tree: modes, case, whole words, lines across, the
//! backtracking fallback, binary, UTF-16 and ANSI files, cut lines,
//! `files_only`, `max_hits`, context lines and cancel.
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use super::content_extended::{can_match_newline, translate_boost, unescape_extended, Target};
use crate::agent_proto::{
    search_local_tree, search_reader, ContentEvent, ContentMatcher, ContentQuery, ContentSummary,
    LineHit, SkipReason, TextMode,
};

fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    std::fs::write(
        root.join("a.txt"),
        "Hello World\r\nsecond line foo\r\nthird FOO bar\r\n",
    )
    .unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(
        root.join("sub/b.rs"),
        "fn main() {\n    let x = foo_bar;\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("bin.dat"), b"foo\0binary foo").unwrap();
    let mut utf16 = vec![0xFF, 0xFE];
    for unit in "Größe foo\n".encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(root.join("utf16.txt"), utf16).unwrap();
    std::fs::write(root.join("ansi.txt"), b"Gr\xF6\xDFe im Text\n").unwrap();
    let long = format!("{}needle{}\n", "x".repeat(5000), "y".repeat(5000));
    std::fs::write(root.join("long.txt"), long).unwrap();
    dir
}

/// Every event and the totals of a search of `root`.
fn search(root: &Path, query: &ContentQuery) -> (Vec<ContentEvent>, ContentSummary) {
    let events = Mutex::new(Vec::new());
    let emit = |event: ContentEvent| {
        events.lock().unwrap().push(event);
        true
    };
    let cancel = AtomicBool::new(false);
    let summary = search_local_tree(root, query, &emit, &cancel).expect("search");
    (events.into_inner().unwrap(), summary)
}

/// The hit lines per file (context lines included), sorted by path.
fn hits(events: &[ContentEvent]) -> Vec<(String, Vec<LineHit>)> {
    let mut files: Vec<(String, Vec<LineHit>)> = Vec::new();
    for event in events {
        if let ContentEvent::Hits { rel, lines, .. } = event {
            match files.iter_mut().find(|(known, _)| known == rel) {
                Some((_, known)) => known.extend(lines.iter().cloned()),
                None => files.push((rel.clone(), lines.clone())),
            }
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

fn numbers(lines: &[LineHit]) -> Vec<u64> {
    lines.iter().map(|line| line.line).collect()
}

fn query(pattern: &str) -> ContentQuery {
    ContentQuery::text(pattern)
}

#[test]
fn content_search_task_extended_escapes_and_boost_spellings() {
    assert_eq!(
        unescape_extended(r"a\tb\x41ä\o101\d065\b01000001\\\q\n\r\0"),
        "a\tbA\u{e4}AAA\\\\q\n\r\0"
    );
    assert_eq!(unescape_extended(r"\x4"), r"\x4");
    assert_eq!(unescape_extended(r"end\"), r"end\");
    assert_eq!(
        translate_boost(r"\<foo\>", Target::Regex),
        r"\b{start}foo\b{end}"
    );
    let fancy = Target::Fancy {
        dot_matches_newline: false,
    };
    assert_eq!(
        translate_boost(r"\<a.$[.$]", fancy),
        r"\b(?=\w)a[^\r\n](?=\r\n|\n|\r|\z)[.$]"
    );
    assert!(translate_boost(r"a\Rb", Target::Regex).contains(r"\r\n|"));
    assert_eq!(translate_boost(r"\`x\'", Target::Regex), r"\Ax\z");
    for (pattern, across) in [
        ("foo", false),
        (r"\d+\.\w", false),
        ("a.b", false),
        (r"foo\sbar", true),
        ("[^a]", true),
        (r"\x0A", true),
        (r"\x{a}", true),
        ("(?s)a.b", true),
        (r"\W", true),
        ("[[:space:]]", true),
    ] {
        assert_eq!(can_match_newline(pattern), across, "{pattern}");
    }
}

#[test]
fn content_search_task_modes_case_and_whole_word() {
    let dir = tree();
    let (events, summary) = search(dir.path(), &query("foo"));
    let found = hits(&events);
    let names: Vec<&str> = found.iter().map(|(rel, _)| rel.as_str()).collect();
    assert_eq!(names, ["a.txt", "sub/b.rs", "utf16.txt"]);
    assert_eq!(numbers(&found[0].1), [2, 3]);
    let second = &found[0].1[0];
    assert_eq!(second.text, "second line foo");
    assert_eq!(second.ranges, [(12, 15)]);
    assert_eq!(second.offset, 13);
    assert!(events.iter().any(|event| matches!(
        event,
        ContentEvent::Skipped { rel, reason: SkipReason::Binary } if rel == "bin.dat"
    )));
    assert_eq!(summary.files_matched, 3);
    assert_eq!(summary.hits, 4);
    assert_eq!(summary.skipped, 1);
    assert!(!summary.limited);

    let mut exact = query("FOO");
    exact.match_case = true;
    let found = hits(&search(dir.path(), &exact).0);
    assert_eq!(found.len(), 1);
    assert_eq!(numbers(&found[0].1), [3]);

    let mut word = query("foo");
    word.whole_word = true;
    let found = hits(&search(dir.path(), &word).0);
    let names: Vec<&str> = found.iter().map(|(rel, _)| rel.as_str()).collect();
    assert_eq!(names, ["a.txt", "utf16.txt"]);

    let mut regex = query(r"^third\s+foo");
    regex.mode = TextMode::Regex;
    let found = hits(&search(dir.path(), &regex).0);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].1[0].ranges, [(0, 9)]);

    // `$` ends a CRLF line as in Notepad++.
    let mut end = query("foo$");
    end.mode = TextMode::Regex;
    let found = hits(&search(dir.path(), &end).0);
    assert_eq!(found[0].0, "a.txt");
    assert_eq!(numbers(&found[0].1), [2]);

    let mut filtered = query("foo");
    filtered.filters = "*.rs".into();
    let found = hits(&search(dir.path(), &filtered).0);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, "sub/b.rs");
}

#[test]
fn content_search_task_lines_across_and_backtracking_fallback() {
    let dir = tree();
    let mut across = query(r"foo\r?\nthird");
    across.mode = TextMode::Regex;
    let found = hits(&search(dir.path(), &across).0);
    assert_eq!(found.len(), 1);
    assert_eq!(numbers(&found[0].1), [2, 3]);
    assert_eq!(found[0].1[0].ranges, [(12, 15)]);
    assert_eq!(found[0].1[1].ranges, [(0, 5)]);

    let mut extended = query(r"line foo\r\nthird");
    extended.mode = TextMode::Extended;
    let found = hits(&search(dir.path(), &extended).0);
    assert_eq!(numbers(&found[0].1), [2, 3]);

    let mut behind = query("(?<=second line )foo");
    behind.mode = TextMode::Regex;
    assert!(ContentMatcher::new(&behind).unwrap().is_backtracking());
    let found = hits(&search(dir.path(), &behind).0);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].1[0].line, 2);
    assert_eq!(found[0].1[0].ranges, [(12, 15)]);

    let mut back_reference = query(r"(o)\1");
    back_reference.mode = TextMode::Regex;
    assert!(ContentMatcher::new(&back_reference)
        .unwrap()
        .is_backtracking());

    let mut invalid = query("(");
    invalid.mode = TextMode::Regex;
    let cancel = AtomicBool::new(false);
    let error = search_local_tree(dir.path(), &invalid, &|_| true, &cancel).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("Ungültiger"), "{error}");
    assert!(ContentMatcher::new(&query("")).is_err());
}

#[test]
fn content_search_task_utf16_and_ansi_files() {
    let dir = tree();
    let found = hits(&search(dir.path(), &query("größe")).0);
    let names: Vec<&str> = found.iter().map(|(rel, _)| rel.as_str()).collect();
    assert_eq!(names, ["ansi.txt", "utf16.txt"]);
    assert_eq!(found[0].1[0].text, "Größe im Text");
    assert_eq!(found[0].1[0].ranges, [(0, 7)]);
    assert_eq!(found[1].1[0].text, "Größe foo");

    // An ANSI line is shown decoded from Windows-1252 also for an ASCII hit.
    let found = hits(&search(dir.path(), &query("Text")).0);
    assert_eq!(found[0].1[0].text, "Größe im Text");
    assert_eq!(found[0].1[0].ranges, [(11, 15)]);
}

#[test]
fn content_search_task_cut_lines_files_only_limit_and_context() {
    let dir = tree();
    let mut cut = query("needle");
    cut.max_line_bytes = 100;
    let found = hits(&search(dir.path(), &cut).0);
    let line = &found[0].1[0];
    assert!(line.truncated);
    assert!(line.text.len() <= 100);
    let (start, end) = line.ranges[0];
    assert_eq!(&line.text[start as usize..end as usize], "needle");

    let mut names_only = query("foo");
    names_only.files_only = true;
    let (events, summary) = search(dir.path(), &names_only);
    for event in &events {
        if let ContentEvent::Hits { lines, last, .. } = event {
            assert_eq!(lines.len(), 1);
            assert!(*last);
        }
    }
    assert_eq!(hits(&events).len(), 3);
    assert_eq!(summary.hits, 3);

    let mut limited = query("o");
    limited.max_hits = 2;
    let (events, summary) = search(dir.path(), &limited);
    let lines: usize = hits(&events).iter().map(|(_, lines)| lines.len()).sum();
    assert_eq!(lines, 2);
    assert_eq!(summary.hits, 2);
    assert!(summary.limited);

    let mut context = query("second");
    context.context_lines = 1;
    let found = hits(&search(dir.path(), &context).0);
    let lines = &found[0].1;
    assert_eq!(numbers(lines), [1, 2, 3]);
    assert_eq!(
        lines.iter().map(|line| line.context).collect::<Vec<_>>(),
        [true, false, true]
    );
    assert_eq!(lines[0].text, "Hello World");
    assert!(lines[0].ranges.is_empty());
}

#[test]
fn content_search_task_cancel_and_closed_receiver_end_the_search() {
    let dir = tree();
    let canceled = AtomicBool::new(true);
    let error = search_local_tree(dir.path(), &query("foo"), &|_| true, &canceled).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);

    let running = AtomicBool::new(false);
    let error = search_local_tree(dir.path(), &query("foo"), &|_| false, &running).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);

    // A file as the root: only it is searched.
    let (events, summary) = search(&dir.path().join("a.txt"), &query("foo"));
    assert_eq!(hits(&events)[0].0, "a.txt");
    assert_eq!(summary.files_searched, 1);
}

/// One file through one reader (the Share host's own walk): its hits, its
/// bytes, the Windows-1252 pass from the kept bytes and a binary file as
/// `skipped` without an event.
#[test]
fn content_search_task_single_reader_keeps_bytes_for_the_ansi_pass() {
    let ansi = query("größe");
    let matcher = ContentMatcher::new(&ansi).unwrap();
    let mut events = Vec::new();
    let mut reader: &[u8] = b"x\nGr\xF6\xDFe\n";
    let outcome = search_reader(&matcher, &ansi, "a.txt", 9, 0, &mut reader, &mut |event| {
        events.push(event)
    })
    .unwrap();
    assert_eq!(outcome.hits, 1);
    assert_eq!(outcome.bytes, 8);
    assert!(!outcome.skipped);
    let found = hits(&events);
    assert_eq!(found[0].1[0].line, 2);
    assert_eq!(found[0].1[0].text, "Größe");

    let mut events = Vec::new();
    let mut binary: &[u8] = b"Gr\xC3\xB6\xC3\x9Fe\0";
    let outcome = search_reader(&matcher, &ansi, "b.bin", 9, 0, &mut binary, &mut |event| {
        events.push(event)
    })
    .unwrap();
    assert!(outcome.skipped);
    assert_eq!(outcome.hits, 0);
    assert!(events.is_empty());
}
