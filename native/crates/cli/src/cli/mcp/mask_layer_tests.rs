//! The mask layer of `se mcp` (plan 2026-11-28, M4): results masked once,
//! arguments restored, raw job output masked across appends, whole-file
//! reads, se's own data guarded, the `se` tool's refusals and the entries
//! of the `masks` tool.
use super::{escaped, finish, mask_text, mask_value, Escape, MaskLayer};
use crate::cli::mcp::args::Args;
use crate::cli::mcp::job_view::{job_value, Offsets};
use crate::cli::mcp::jobs::{JobEnd, JobKind, JobRegistry, JobStatus};
use crate::cli::mcp::mask_layer_guard::{cli_refusal, guard_local, Reach};
use crate::cli::mcp::response;
use crate::masks::{Actor, MaskSet, MaskStore, NewEntry};
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{json, Value};
use std::sync::Arc;

const SECRET: &str = "Geheimprojekt";

/// A store in a temporary folder with one enabled list masking [`SECRET`].
fn layer() -> (tempfile::TempDir, Arc<MaskLayer>) {
    let dir = tempfile::tempdir().unwrap();
    let store = MaskStore::open_at(dir.path(), [7u8; 32]).unwrap();
    let list = store.create_list(Actor::Gui, "Projekt").unwrap();
    store
        .add_entries(
            Actor::Gui,
            list,
            vec![NewEntry::literal(SECRET).with_label("Projekt")],
        )
        .unwrap();
    (dir, Arc::new(MaskLayer::with_store(store)))
}

/// A layer whose store has no list: nothing is masked.
fn empty() -> (tempfile::TempDir, Arc<MaskLayer>) {
    let dir = tempfile::tempdir().unwrap();
    let store = MaskStore::open_at(dir.path(), [7u8; 32]).unwrap();
    (dir, Arc::new(MaskLayer::with_store(store)))
}

fn text_of(result: &CallToolResult) -> String {
    match &result.content[0] {
        ContentBlock::Text(text) => text.text.clone(),
        _ => panic!("no text block"),
    }
}

#[test]
fn mcp_masks_task_results_are_masked_exactly_once() {
    let (_dir, layer) = layer();
    let set = layer.set().unwrap();
    let placeholder = mask_text(&set, SECRET);
    assert!(placeholder.starts_with("⟦Projekt-"), "{placeholder}");
    let result = finish(
        &set,
        response::ok(json!({
            "path": format!("/work/{SECRET}/main.rs"),
            "entries": [{"name": SECRET, "size": 3}],
            "note": "a literal ⟦ stays readable",
        })),
    );
    let structured = result.structured_content.clone().unwrap();
    let text = text_of(&result);
    assert!(!text.contains(SECRET), "{text}");
    // The text block repeats the masked structured result.
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), structured);
    assert!(structured["path"].as_str().unwrap().contains(&placeholder));
    assert_eq!(structured["entries"][0]["name"], json!(placeholder));
    assert_eq!(structured["entries"][0]["size"], 3);
    assert_eq!(structured["note"], "a literal ⟦⟦ stays readable");
    assert!(result.meta.is_none());
    // A result the tool masked itself passes unchanged (no second escape).
    let mut own = json!({ "content": format!("{placeholder} ⟦⟦") });
    let kept = own.clone();
    let passed = finish(&set, response::masked(own.take()));
    assert_eq!(passed.structured_content, Some(kept));
    assert!(passed.meta.is_none());
    // Errors are masked as well.
    let failed = finish(&set, response::error(format!("{SECRET} is missing")));
    assert!(!text_of(&failed).contains(SECRET));
    assert_eq!(failed.is_error, Some(true));
    // A picture cannot be masked: withheld while a list is active.
    let picture = CallToolResult::success(vec![ContentBlock::image("AAAA", "image/png")]);
    let refused = finish(&set, picture);
    assert_eq!(refused.is_error, Some(true));
    assert!(refused
        .content
        .iter()
        .all(|block| matches!(block, ContentBlock::Text(_))));
}

#[test]
fn mcp_masks_task_without_lists_every_answer_stays_byte_identical() {
    let (_dir, layer) = empty();
    let set = layer.set().unwrap();
    assert!(set.is_empty());
    let original = response::ok(json!({"path": format!("/w/{SECRET}"), "x": "⟦"}));
    let before = serde_json::to_string(&original).unwrap();
    let after = serde_json::to_string(&finish(&set, original)).unwrap();
    assert_eq!(before, after);
    let mut value = json!({"name": "⟦a⟧"});
    mask_value(&set, &mut value);
    assert_eq!(value["name"], "⟦a⟧");
    // Arguments pass as they are, a literal `⟦⟦` included.
    let arguments = json!({"content": "⟦⟦ and ⟦Projekt-99⟧"});
    let (_, restored) = layer
        .prepare("write", arguments.as_object().cloned())
        .unwrap();
    assert_eq!(Value::Object(restored.unwrap()), arguments);
}

#[test]
fn mcp_masks_task_arguments_are_restored_and_unknown_placeholders_refused() {
    let (_dir, layer) = layer();
    let set = layer.set().unwrap();
    let placeholder = mask_text(&set, SECRET);
    let arguments = json!({
        "target": format!("src/{placeholder}.rs"),
        "content": format!("name = \"{placeholder}\" ⟦⟦"),
        "edits": [{"old_text": placeholder, "new_text": "x"}],
        "limit": 3,
    });
    let (_, restored) = layer
        .prepare("write", arguments.as_object().cloned())
        .unwrap();
    let restored = Value::Object(restored.unwrap());
    assert_eq!(restored["target"], format!("src/{SECRET}.rs"));
    assert_eq!(restored["content"], format!("name = \"{SECRET}\" ⟦"));
    assert_eq!(restored["edits"][0]["old_text"], SECRET);
    assert_eq!(restored["limit"], 3);
    let unknown = json!({"content": "⟦Projekt-999⟧"});
    let error = layer
        .prepare("write", unknown.as_object().cloned())
        .err()
        .unwrap();
    assert!(error.contains("`content`"), "{error}");
    let refused = super::refusal(error);
    assert_eq!(refused.is_error, Some(true));
}

#[test]
fn mcp_masks_task_patterns_restore_placeholders_as_literal_values() {
    assert_eq!(escaped("a.b(c)?", Escape::Regex), r"a\.b\(c\)\?");
    assert_eq!(escaped(r"C:\x", Escape::Extended), r"C:\\x");
    assert_eq!(escaped("$1 (x?:y)", Escape::Format), r"\$1 \(x\?\:y\)");
    assert_eq!(escaped("plain", Escape::None), "plain");
    let (_dir, layer) = layer();
    let arguments = json!({"pattern": "⟦⟦|x", "mode": "regex", "folders": ["a.b"]});
    let (_, restored) = layer
        .prepare("grep", arguments.as_object().cloned())
        .unwrap();
    let restored = Value::Object(restored.unwrap());
    assert_eq!(restored["pattern"], "⟦|x");
    assert_eq!(restored["folders"][0], "a.b");
    // The rest of a pattern keeps its meaning; the restored value matches
    // itself.
    let set = layer.set().unwrap();
    let placeholder = mask_text(&set, SECRET);
    let arguments = json!({"pattern": format!("{placeholder}.*"), "mode": "regex"});
    let (_, restored) = layer
        .prepare("grep", arguments.as_object().cloned())
        .unwrap();
    assert_eq!(
        Value::Object(restored.unwrap())["pattern"],
        format!("{SECRET}.*")
    );
}

#[test]
fn mcp_masks_task_raw_job_output_is_masked_across_appends() {
    let (dir, layer) = layer();
    let registry = JobRegistry::with_masks(dir.path(), layer.clone());
    let job = registry
        .create(
            JobKind::Shell,
            format!("shell cat {SECRET}.txt"),
            &["stdout", "stderr", "hits"],
        )
        .unwrap();
    let (head, tail) = SECRET.split_at(5);
    job.append("stdout", format!("vor {head}").as_bytes())
        .unwrap();
    job.append("stdout", format!("{tail} nach\n").as_bytes())
        .unwrap();
    // Search hits are masked by their tool and kept as written.
    job.append("hits", b"{\"path\":\"a\"}\n").unwrap();
    job.finish(JobEnd {
        status: JobStatus::Completed,
        result: json!({"exit_code": 0, "cwd": format!("/w/{SECRET}")}),
        error: None,
    });
    let value = job_value(&job, &Offsets::default(), 8_192).unwrap();
    let text = value.to_string();
    assert!(!text.contains(SECRET), "{text}");
    let stdout = value["stdout"]["text"].as_str().unwrap();
    assert!(stdout.starts_with("vor ⟦Projekt-"), "{stdout}");
    assert!(stdout.ends_with("⟧ nach\n"), "{stdout}");
    assert_eq!(value["hits"]["items"][0]["path"], "a");
    assert!(value["summary"].as_str().unwrap().contains("⟦Projekt-"));
    assert_eq!(value["kind"], "shell");
}

#[test]
fn mcp_masks_task_reads_mask_the_whole_file_and_keep_line_numbers() {
    let (_dir, layer) = layer();
    let set = layer.set().unwrap();
    let file = format!("eins\nzwei {SECRET}\ndrei\n");
    let masked = set.mask_bytes(file.as_bytes());
    let empty = serde_json::Map::new();
    let by_lines = json!({"line": 2, "lines": 1});
    let lines = Args::new(by_lines.as_object());
    let request = Some(crate::cli::mcp::line_read::request(&lines).unwrap());
    let page = super::super::mask_layer_read::page(
        &set,
        &masked,
        &format!("/w/{SECRET}.txt"),
        Some("e1".into()),
        false,
        request,
        false,
        &lines,
    )
    .unwrap();
    let mut page = page;
    assert!(response::take_masked_mark(&mut page));
    let value = page.structured_content.unwrap();
    assert!(!value.to_string().contains(SECRET), "{value}");
    let content = value["content"].as_str().unwrap();
    assert!(content.starts_with("     2\tzwei ⟦Projekt-"), "{content}");
    assert_eq!(value["next_line"], 3);
    assert_eq!(value["masked"], true);
    // Byte pages count the masked text.
    let bytes = Args::new(Some(&empty));
    let page = super::super::mask_layer_read::page(
        &set, &masked, "/w/a.txt", None, false, None, false, &bytes,
    )
    .unwrap();
    let value = page.structured_content.unwrap();
    assert_eq!(value["size"], masked.len());
    assert_eq!(
        value["content"].as_str().unwrap().as_bytes(),
        masked.as_slice()
    );
    // base64 writes are restored in their bytes.
    let restored = super::super::mask_layer_read::restore_bytes(&set, masked.clone()).unwrap();
    assert_eq!(restored, file.as_bytes());
}

#[test]
fn mcp_masks_task_own_data_is_guarded_inside_and_as_a_subtree() {
    let root = tempfile::tempdir().unwrap();
    let guarded = root.path().join("app-data");
    std::fs::create_dir_all(guarded.join("masks")).unwrap();
    let places = [guarded.clone()];
    let inside = guarded.join("masks").join("lists.sealed");
    assert!(guard_local(&places, &inside, Reach::Entry).is_err());
    // A file about to be written below it, in another case.
    let upper = root.path().join("APP-DATA").join("new.txt");
    assert!(guard_local(&places, &upper, Reach::Entry).is_err());
    // A folder that contains it: refused for tree operations only.
    assert!(guard_local(&places, root.path(), Reach::Entry).is_ok());
    let touched = guard_local(&places, root.path(), Reach::Tree).unwrap_err();
    assert!(!touched.within);
    let elsewhere = root.path().join("project").join("main.rs");
    assert!(guard_local(&places, &elsewhere, Reach::Tree).is_ok());
    #[cfg(unix)]
    {
        // A link into the guarded place is followed.
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&guarded, &link).unwrap();
        assert!(guard_local(&places, &link.join("masks"), Reach::Entry).is_err());
    }
}

#[test]
fn mcp_masks_task_se_tool_refuses_mask_changes_and_plugin_removal() {
    let words = |items: &[&str]| {
        items
            .iter()
            .map(|item| item.to_string())
            .collect::<Vec<_>>()
    };
    let (_dir, active) = layer();
    let (_empty_dir, inactive) = empty();
    let active = active.set().unwrap();
    let inactive: Arc<MaskSet> = inactive.set().unwrap();
    for set in [&active, &inactive] {
        assert!(cli_refusal(set, &words(&["masks", "unlock", "1"])).is_some());
        assert!(cli_refusal(set, &words(&["masks", "list", "--json"])).is_some());
        assert!(cli_refusal(set, &words(&["elevated", "--pipe", "x"])).is_some());
        assert!(cli_refusal(set, &words(&["masks", "--help"])).is_none());
        assert!(cli_refusal(set, &words(&["sync", "list", "--json"])).is_none());
    }
    // Removing the plugin only while a list is active, as before otherwise.
    assert!(cli_refusal(&active, &words(&["plugin", "uninstall", "claude"])).is_some());
    assert!(cli_refusal(&inactive, &words(&["plugin", "uninstall", "claude"])).is_none());
}

#[test]
fn mcp_masks_task_masks_tool_entries_are_typed() {
    use crate::masks::{EntryKind, Tolerance};
    let entries = super::super::masks_tool::parse_entries(Some(&json!([
        {"value": "Schweinsbraten", "kind": "fuzzy", "tolerance": "hoch", "label": "Essen"},
        {"value": "\\d{4}-\\d{4}", "kind": "regex"},
        {"value": "ACME", "case_sensitive": true, "whole_word": true}
    ])))
    .unwrap();
    assert_eq!(entries[0].kind, EntryKind::Fuzzy);
    assert_eq!(entries[0].tolerance, Tolerance::High);
    assert_eq!(entries[0].label.as_deref(), Some("Essen"));
    assert_eq!(entries[1].kind, EntryKind::Regex);
    assert!(entries[2].case_sensitive && entries[2].whole_word);
    let parse = |value: Value| super::super::masks_tool::parse_entries(Some(&value));
    assert!(parse(json!([])).is_err());
    assert!(parse(json!([{"value": ""}])).is_err());
    assert!(parse(json!([{"value": "x", "kind": "magic"}])).is_err());
    assert!(parse(json!([{"value": "x", "tolerance": "high"}])).is_err());
    assert!(parse(json!([{"value": "x", "kind": "regex", "whole_word": true}])).is_err());
    // A fuzzy entry without a level gets the middle one.
    let fuzzy = parse(json!([{"value": "Braten", "kind": "fuzzy"}])).unwrap();
    assert_eq!(fuzzy[0].tolerance, Tolerance::Medium);
}
