//! The MCP `grep` tool over a local folder in its three outputs, and the
//! checks of its arguments (plan 2026-11-27 section C); ignore files and
//! masked hits (plan 2026-11-28, M5).
use super::{request, work, GrepControl, HitMasker};
use crate::agent_proto::TextMode;
use crate::cli::mcp::args::Args;
use crate::cli::mcp::jobs::{JobKind, JobRegistry, JobStatus};
use crate::cli::mcp::targets::Targets;
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn args(value: Value) -> serde_json::Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

/// Runs one grep job over `root` and returns its output text and result.
fn grep(root: &str, call: Value) -> (String, Value) {
    grep_masked(root, call, None)
}

/// [`grep`] with the hits masked by `masks`.
fn grep_masked(
    root: &str,
    call: Value,
    masks: Option<Arc<crate::masks::MaskSet>>,
) -> (String, Value) {
    let map = args(call);
    let request = request(&Args::new(Some(&map))).unwrap();
    let targets = Targets::default();
    let target = targets.resolve(root).unwrap();
    let jobs_dir = tempfile::tempdir().unwrap();
    let registry = JobRegistry::new(jobs_dir.path());
    let stream = if request.json { "hits" } else { "matches" };
    let job = registry
        .create(JobKind::Grep, "grep test".into(), &[stream])
        .unwrap();
    let control = GrepControl {
        cancel: Arc::new(AtomicBool::new(false)),
        files: Arc::default(),
        bytes: Arc::default(),
        hits: Arc::default(),
    };
    let masker = masks.and_then(|set| HitMasker::new(set, target.backend.clone(), &target.path));
    let end = work(
        &job,
        (&target, masker),
        &request,
        stream,
        &control,
        &targets,
    );
    assert_eq!(end.status, JobStatus::Completed, "{:?}", end.error);
    let spool = job.stream(stream).unwrap();
    let bytes = spool.read(0, usize::MAX).unwrap();
    (String::from_utf8(bytes).unwrap(), end.result)
}

#[test]
fn mcp_root_task_grep_searches_a_local_folder_in_three_outputs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("a.txt"),
        "eins\nzwei Treffer\ndrei\nvier Treffer\n",
    )
    .unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("b.rs"),
        "// Treffer\nfn main() {}\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("bin.dat"), b"Treffer\0binary").unwrap();
    let root = dir.path().to_string_lossy().into_owned();

    let (text, result) = grep(&root, json!({"pattern": "treffer"}));
    let mut lines = text.lines().collect::<Vec<_>>();
    lines.sort_unstable();
    assert_eq!(
        lines,
        [
            "a.txt:2:zwei Treffer",
            "a.txt:4:vier Treffer",
            "src/b.rs:1:// Treffer"
        ]
    );
    assert_eq!(result["hits"], 3);
    assert_eq!(result["files_matched"], 2);
    assert_eq!(result["searched_at"], "local");
    assert_eq!(result["output"], "content");
    assert_eq!(result["mode"], "normal");
    // The binary file is named as skipped.
    assert!(
        result["skipped_items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["rel"] == "bin.dat" && item["reason"] == "binary"),
        "{result}"
    );

    let (text, _) = grep(&root, json!({"pattern": "treffer", "output": "files"}));
    let mut files = text.lines().collect::<Vec<_>>();
    files.sort_unstable();
    assert_eq!(files, ["a.txt", "src/b.rs"]);

    let (text, _) = grep(
        &root,
        json!({"pattern": "Treffer", "output": "count", "match_case": true}),
    );
    let mut counts = text.lines().collect::<Vec<_>>();
    counts.sort_unstable();
    assert_eq!(counts, ["a.txt:2", "src/b.rs:1"]);

    // Context lines, a regex, a filter, and JSON lines with ranges.
    let (text, _) = grep(
        &root,
        json!({"pattern": "^zwei", "mode": "regex", "context_lines": 1, "filters": "*.txt"}),
    );
    assert_eq!(text, "a.txt-1-eins\na.txt:2:zwei Treffer\na.txt-3-drei\n");
    let (text, _) = grep(
        &root,
        json!({"pattern": "main", "format": "json", "folders": "^src$"}),
    );
    let hit: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(
        hit,
        json!({"rel": "src/b.rs", "line": 2, "text": "fn main() {}", "ranges": [[3, 7]]})
    );
}

#[test]
fn mcp_root_task_grep_checks_its_arguments_before_a_job_starts() {
    let parse = |value: Value| {
        let map = args(value);
        request(&Args::new(Some(&map)))
    };
    let invalid = parse(json!({"pattern": "(", "mode": "regex"}))
        .err()
        .unwrap();
    assert!(invalid.starts_with("invalid search"), "{invalid}");
    assert!(parse(json!({"pattern": "x", "exclude_folders": "("})).is_err());
    assert!(parse(json!({"pattern": ""})).is_err());
    assert!(parse(json!({"pattern": "x", "mode": "glob"})).is_err());
    assert!(parse(json!({"pattern": "x", "output": "lines"})).is_err());
    assert!(parse(json!({"pattern": "x", "format": "xml"})).is_err());
    let defaults = parse(json!({"pattern": "x"})).unwrap();
    assert_eq!(defaults.query.mode, TextMode::Normal);
    assert!(defaults.query.subfolders && !defaults.query.hidden && !defaults.json);
    let files = parse(json!({"pattern": "x", "output": "files", "context_lines": 3, "subfolders": false, "max_results": 7})).unwrap();
    assert!(files.query.files_only && !files.query.subfolders);
    assert_eq!((files.query.context_lines, files.query.max_hits), (0, 7));
}

#[test]
fn mcp_coding_task_grep_applies_ignore_files() {
    let dir = tempfile::tempdir().unwrap();
    let write = |rel: &str, text: &str| {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    write(".gitignore", "target/\n");
    write("src/a.rs", "needle\n");
    write("target/b.rs", "needle\n");
    let root = dir.path().to_string_lossy().into_owned();
    let (text, result) = grep(
        &root,
        json!({"pattern": "needle", "output": "files", "gitignore": true}),
    );
    assert_eq!(text, "src/a.rs\n");
    assert_eq!(result["gitignore"], true);
    let (text, result) = grep(&root, json!({"pattern": "needle", "output": "files"}));
    assert_eq!(text.lines().count(), 2, "without `gitignore` every file");
    assert!(
        result.get("gitignore").is_none(),
        "the former result fields only"
    );
    let parsed = {
        let map = args(json!({"pattern": "x", "gitignore": true}));
        request(&Args::new(Some(&map))).unwrap()
    };
    assert!(parsed.query.respect_ignore_files);
}

#[test]
fn mcp_coding_task_grep_hits_come_from_the_masked_file() {
    use crate::masks::{Actor, MaskStore, NewEntry};
    let store_dir = tempfile::tempdir().unwrap();
    let store = MaskStore::open_at(store_dir.path(), [7u8; 32]).unwrap();
    let list = store.create_list(Actor::Gui, "Projekt").unwrap();
    store
        .add_entries(
            Actor::Gui,
            list,
            vec![
                NewEntry::literal("Geheimprojekt").with_label("Projekt"),
                NewEntry::regex(r"Konto\s+\d+").with_label("Konto"),
            ],
        )
        .unwrap();
    let set = store.active().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Geheimprojekt.txt"),
        "x Geheimprojekt y\nKonto\n4711 Ende\n",
    )
    .unwrap();
    let root = dir.path().to_string_lossy().into_owned();

    // A value over two lines is masked although the hit shows one of them.
    let (text, _) = grep_masked(
        &root,
        json!({"pattern": "Ende", "format": "json"}),
        Some(set.clone()),
    );
    let hit: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    let line = hit["text"].as_str().unwrap();
    assert_eq!(hit["line"], 3, "line numbers stay those of the file");
    assert!(!line.contains("4711"), "{line}");
    let range = &hit["ranges"][0];
    let (start, end) = (
        range[0].as_u64().unwrap() as usize,
        range[1].as_u64().unwrap() as usize,
    );
    assert_eq!(&line[start..end], "Ende");
    let rel = hit["rel"].as_str().unwrap();
    assert!(!rel.contains("Geheimprojekt") && rel.contains('⟦'), "{rel}");

    // A hit on the value itself covers its whole placeholder.
    let (text, _) = grep_masked(
        &root,
        json!({"pattern": "Geheimprojekt", "format": "json", "context_lines": 1}),
        Some(set.clone()),
    );
    assert!(
        !text.contains("Geheimprojekt") && !text.contains("4711"),
        "{text}"
    );
    let hit: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    let line = hit["text"].as_str().unwrap();
    let range = &hit["ranges"][0];
    let (start, end) = (
        range[0].as_u64().unwrap() as usize,
        range[1].as_u64().unwrap() as usize,
    );
    assert!(
        line[start..end].starts_with('⟦') && line[start..end].ends_with('⟧'),
        "{line}"
    );
    assert!(line.starts_with("x ") && line.ends_with(" y"), "{line}");

    // Without an active list the output is that of an unmasked search.
    let empty_dir = tempfile::tempdir().unwrap();
    let empty = MaskStore::open_at(empty_dir.path(), [7u8; 32])
        .unwrap()
        .active()
        .unwrap();
    assert_eq!(
        grep_masked(&root, json!({"pattern": "Ende"}), Some(empty)).0,
        grep(&root, json!({"pattern": "Ende"})).0
    );
}
