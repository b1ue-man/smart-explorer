//! The MCP `replace` tool on a local folder: its arguments, a dry run with
//! previews, a run that changes files (new etag, `undo_id`, original in the
//! journal), and masked streams while a mask list is active.
use super::run::replace;
use super::{call, Call, Choice, ReplaceControl};
use crate::cli::mcp::args::Args;
use crate::cli::mcp::grep_tool::mask::HitMasker;
use crate::cli::mcp::jobs::{JobKind, JobRegistry, JobStatus};
use crate::cli::mcp::targets::Targets;
use crate::masks::MaskSet;
use crate::vfs::{ReplaceJournal, ReplaceRequest, ReplaceSelection};
use serde_json::{json, Value};
use std::sync::Arc;

fn parse(value: Value, masked: bool) -> Result<(Call, bool), String> {
    let map = value.as_object().cloned().unwrap_or_default();
    call(&Args::new(Some(&map)), masked)
}

/// Runs one replacement over `root`; returns the streams and the result.
fn run(
    root: &str,
    arguments: Value,
    masks: Option<Arc<MaskSet>>,
    journal: &ReplaceJournal,
) -> (String, String, Value) {
    let (call, _) = parse(arguments, masks.as_ref().is_some_and(|set| !set.is_empty())).unwrap();
    let Call::Replace {
        query,
        replacement,
        choice,
        dry_run,
    } = call
    else {
        panic!("not a replacement");
    };
    let targets = Targets::default();
    let target = targets.resolve(root).unwrap();
    let jobs_dir = tempfile::tempdir().unwrap();
    let registry = JobRegistry::new(jobs_dir.path());
    let job = registry
        .create(
            JobKind::Replace,
            "replace test".into(),
            &["changes", "previews"],
        )
        .unwrap();
    let control = ReplaceControl {
        cancel: Arc::default(),
        files: Arc::default(),
        changed: Arc::default(),
        replacements: Arc::default(),
    };
    let masker = masks.and_then(|set| HitMasker::new(set, target.backend.clone(), &target.path));
    let request = ReplaceRequest {
        query,
        replacement,
        selection: ReplaceSelection::All,
        dry_run,
    };
    let end = replace(
        &job,
        (&target, masker, Some(journal)),
        request,
        choice,
        &control,
        &targets,
    );
    assert_eq!(end.status, JobStatus::Completed, "{:?}", end.error);
    let stream = |name: &str| {
        String::from_utf8(job.stream(name).unwrap().read(0, usize::MAX).unwrap()).unwrap()
    };
    (stream("changes"), stream("previews"), end.result)
}

#[test]
fn mcp_coding_task_replace_checks_its_arguments_before_a_job_starts() {
    assert!(matches!(
        parse(json!({"history": true}), false),
        Ok((Call::History, false))
    ));
    assert!(matches!(parse(json!({"undo": "r1"}), false), Ok((Call::Undo(id), _)) if id == "r1"));
    assert!(parse(json!({"pattern": "x"}), false)
        .unwrap_err()
        .contains("replacement"));
    assert!(parse(json!({"pattern": "", "replacement": "y"}), false).is_err());
    assert!(parse(
        json!({"pattern": "(", "mode": "regex", "replacement": "y"}),
        false
    )
    .is_err());
    assert!(parse(
        json!({"pattern": "x", "replacement": "y", "files": ["a"], "hits": [{"rel": "a", "line": 1}]}),
        false
    )
    .is_err());
    assert!(parse(
        json!({"pattern": "x", "replacement": "y", "hits": [{"rel": "a", "line": 0}]}),
        false
    )
    .is_err());
    assert!(parse(json!({"pattern": "x", "replacement": "y", "hits": [{"rel": "a", "line": 1, "ranges": [[3, 1]]}]}), false).is_err());
    let hits = json!({"pattern": "x", "replacement": "y", "hits": [{"rel": "a", "line": 2, "ranges": [[0, 1]]}]});
    let (
        Call::Replace {
            choice: Choice::Hits(list),
            ..
        },
        ignored,
    ) = parse(hits.clone(), false).unwrap()
    else {
        panic!("hits");
    };
    assert_eq!(
        (list[0].line, list[0].ranges.clone(), ignored),
        (2, vec![(0, 1)], false)
    );
    // Ranges of a masked grep point into masked lines: not used.
    let (
        Call::Replace {
            choice: Choice::Hits(list),
            ..
        },
        ignored,
    ) = parse(hits, true).unwrap()
    else {
        panic!("hits");
    };
    assert!(list[0].ranges.is_empty() && ignored);
}

#[test]
fn mcp_coding_task_replace_previews_changes_and_keeps_the_original() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("tree");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"), "foo bar\nfoo\n").unwrap();
    std::fs::write(root.join("b.txt"), "nothing\n").unwrap();
    let journal = ReplaceJournal::open_at(dir.path().join("journal"), [7u8; 32]);
    let root_text = root.to_string_lossy().into_owned();

    let (changes, previews, result) = run(
        &root_text,
        json!({"pattern": "foo", "replacement": "baz", "dry_run": true}),
        None,
        &journal,
    );
    assert_eq!(
        std::fs::read_to_string(root.join("a.txt")).unwrap(),
        "foo bar\nfoo\n"
    );
    assert!(
        previews.contains("\"before\":\"foo bar\"") && previews.contains("\"after\":\"baz bar\""),
        "{previews}"
    );
    let change: Value = serde_json::from_str(changes.lines().next().unwrap()).unwrap();
    assert_eq!(
        (change["rel"].as_str(), change.get("etag")),
        (Some("a.txt"), None)
    );
    assert_eq!(result["dry_run"], true);
    assert_eq!(result["undo_id"], Value::Null);

    let (changes, previews, result) = run(
        &root_text,
        json!({"pattern": "foo", "replacement": "baz"}),
        None,
        &journal,
    );
    assert!(previews.is_empty());
    assert_eq!(
        std::fs::read_to_string(root.join("a.txt")).unwrap(),
        "baz bar\nbaz\n"
    );
    let change: Value = serde_json::from_str(changes.lines().next().unwrap()).unwrap();
    assert_eq!(change["replacements"], 2);
    assert!(change["etag"].is_string(), "{change}");
    assert_eq!(result["files_changed"], 1);
    assert_eq!(result["replacements"], 2);
    let id = result["undo_id"].as_str().expect("an undo id").to_string();
    let runs = journal.history().unwrap();
    assert!(runs.iter().any(|run| run.id == id));
}

#[test]
fn mcp_coding_task_replace_streams_are_masked() {
    use crate::masks::{Actor, MaskStore, NewEntry};
    let store_dir = tempfile::tempdir().unwrap();
    let store = MaskStore::open_at(store_dir.path(), [7u8; 32]).unwrap();
    let list = store.create_list(Actor::Gui, "Projekt").unwrap();
    store
        .add_entries(
            Actor::Gui,
            list,
            vec![NewEntry::literal("Geheimprojekt").with_label("Projekt")],
        )
        .unwrap();
    let set = store.active().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Geheimprojekt.md"), "Geheimprojekt: alt\n").unwrap();
    let journal = ReplaceJournal::open_at(dir.path().join(".journal"), [7u8; 32]);
    let (changes, previews, _) = run(
        &dir.path().to_string_lossy(),
        json!({"pattern": "alt", "replacement": "neu", "dry_run": true, "filters": "*.md"}),
        Some(set),
        &journal,
    );
    for stream in [&changes, &previews] {
        assert!(!stream.contains("Geheimprojekt"), "{stream}");
    }
    assert!(
        previews.contains("neu") && previews.contains('⟦'),
        "{previews}"
    );
}
