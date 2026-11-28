//! `search.content.replace.*` (api.md §4.14) through the dispatcher of a
//! runtime that is not installed globally, over a temporary local folder:
//! the request fields (UTF-16 ranges of the hits become byte ranges), the
//! mistakes that fail at once, the dry run that writes nothing, the run of
//! chosen hits that skips a file changed since the search, and the run of
//! every match with its journal entry, `history` and `undo`.
use super::super::config::HostSettings;
use super::super::dispatch::dispatch;
use super::super::runtime::Runtime;
use super::content_search_replace::{parse_hit, parse_request};
use crate::agent_proto::ReplaceSelection;
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

struct Fixture {
    dir: tempfile::TempDir,
    rt: Runtime,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("temp dir");
    let files = dir.path().join("files");
    let cache = dir.path().join("cache");
    for path in [&files, &cache] {
        std::fs::create_dir_all(path).expect("fixture dir");
    }
    let config = json!({ "filesDir": files, "cacheDir": cache, "startDaemon": false });
    let settings = HostSettings::parse(&config.to_string()).expect("settings");
    Fixture {
        dir,
        rt: Runtime::detached(settings),
    }
}

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent");
    }
    std::fs::write(path, bytes).expect("write");
}

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).expect("read")
}

/// Polls `method` for `id` from offset 0 on until it is done; all events.
fn poll_all(fx: &Fixture, method: &str, id: &str) -> (Vec<Value>, Value) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut since = 0;
    let mut events = Vec::new();
    loop {
        let answer =
            dispatch(&fx.rt, method, &json!({ "taskId": id, "since": since })).expect("poll");
        events.extend(answer["events"].as_array().cloned().unwrap_or_default());
        since = answer["next"].as_u64().expect("next");
        if answer["done"] == true {
            return (events, answer);
        }
        assert!(Instant::now() < deadline, "did not end: {answer}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn replace(fx: &Fixture, args: Value) -> (Vec<Value>, Value) {
    let started = dispatch(&fx.rt, "search.content.replace.start", &args).expect("start");
    let id = started["taskId"].as_str().expect("taskId").to_string();
    poll_all(fx, "search.content.replace.poll", &id)
}

#[test]
fn replace_ui_task_bridge_request_fields_and_mistakes_fail_at_once() {
    // "Größe " is 6 UTF-16 units but 8 bytes: the hit's ranges become byte ranges.
    let hit = parse_hit(&json!({
        "rel": "a/b.txt", "line": 2, "text": "Größe hallo", "ranges": [[6, 11]],
        "size": 30, "mtimeMs": 1_700,
    }))
    .expect("hit");
    assert_eq!((hit.rel.as_str(), hit.line), ("a/b.txt", 2));
    assert_eq!(hit.ranges, vec![(8, 13)]);
    assert_eq!((hit.size, hit.mtime_ms), (30, 1_700));
    // A cut line names every match of that line.
    let cut = parse_hit(
        &json!({ "rel": "c.txt", "line": 1, "text": "…x…", "ranges": [[1, 2]], "truncated": true }),
    )
    .expect("cut hit");
    assert!(cut.ranges.is_empty());

    let request = parse_request(&json!({
        "pattern": "hallo", "replacement": "", "respectIgnoreFiles": true, "dryRun": true,
        "selection": "hits", "hits": [{ "rel": "a.txt", "line": 1, "text": "hallo", "ranges": [[0, 5]] }],
    }))
    .expect("request");
    assert!(request.dry_run && request.query.respect_ignore_files);
    assert_eq!(request.replacement, "", "an empty replacement deletes");
    assert!(matches!(&request.selection, ReplaceSelection::Hits(hits) if hits.len() == 1));
    let all =
        parse_request(&json!({ "pattern": "x", "replacement": "y", "respect_ignore_files": true }))
            .expect("snake_case request");
    assert_eq!(all.selection, ReplaceSelection::All);
    assert!(all.query.respect_ignore_files && !all.dry_run);
    let files = parse_request(
        &json!({ "pattern": "x", "replacement": "y", "selection": "files", "files": ["a.txt"] }),
    )
    .expect("files request");
    assert_eq!(
        files.selection,
        ReplaceSelection::Files(vec!["a.txt".into()])
    );

    let fx = fixture();
    let location = fx.dir.path().to_string_lossy().into_owned();
    let start = |args: Value| {
        dispatch(&fx.rt, "search.content.replace.start", &args).map_err(|error| error.kind)
    };
    for args in [
        json!({ "location": location, "pattern": "x" }),
        json!({ "location": location, "pattern": "", "replacement": "y" }),
        json!({ "location": location, "pattern": "(", "mode": "regex", "replacement": "y" }),
        json!({ "location": location, "pattern": "x", "replacement": "y", "selection": "hits", "hits": [] }),
        json!({ "location": location, "pattern": "x", "replacement": "y", "selection": "irgendwas" }),
        json!({ "location": location, "pattern": "x", "replacement": "y", "selection": "hits",
                "hits": [{ "rel": "../aus.txt", "line": 1 }] }),
        json!({ "location": location, "pattern": "x", "replacement": "y", "selection": "hits",
                "hits": [{ "rel": "/abs.txt", "line": 1 }] }),
        json!({ "location": location, "pattern": "x", "replacement": "y", "selection": "hits",
                "hits": [{ "rel": "a.txt", "line": 0 }] }),
        json!({ "location": "trash://", "pattern": "x", "replacement": "y" }),
        json!({ "pattern": "x", "replacement": "y" }),
    ] {
        assert_eq!(start(args.clone()), Err("invalid"), "{args}");
    }
    assert_eq!(
        dispatch(
            &fx.rt,
            "search.content.replace.poll",
            &json!({ "taskId": "fehlt" })
        )
        .map_err(|error| error.kind),
        Err("not_found")
    );
}

#[test]
fn replace_ui_task_bridge_dry_run_counts_and_writes_nothing() {
    let fx = fixture();
    let root = fx.dir.path().join("baum");
    write(&root, "a.txt", b"foo eins\nfoo foo\n");
    write(&root, "sub/b.txt", b"kein Treffer\n");
    write(&root, "bild.dat", b"\0\0foo");
    let (events, end) = replace(
        &fx,
        json!({ "location": root.to_string_lossy(), "pattern": "foo", "replacement": "bar", "dryRun": true }),
    );
    let summary = &end["summary"];
    assert_eq!(summary["replacements"], 3, "{end}");
    assert_eq!(summary["filesChanged"], 1);
    assert!(summary["undoId"].is_null(), "a dry run keeps no backup");
    let file = events
        .iter()
        .find(|e| e["type"] == "file")
        .expect("file event");
    assert_eq!(file["rel"], "a.txt");
    assert_eq!(file["replacements"], 3);
    assert_eq!(
        file["location"],
        root.join("a.txt").to_string_lossy().as_ref()
    );
    assert!(events
        .iter()
        .any(|e| e["type"] == "skipped" && e["rel"] == "bild.dat" && e["reason"].is_string()));
    assert_eq!(
        read(&root, "a.txt"),
        "foo eins\nfoo foo\n",
        "nothing written"
    );
}

#[test]
fn replace_ui_task_bridge_chosen_hits_skip_a_file_changed_since_the_search() {
    let fx = fixture();
    let root = fx.dir.path().join("wahl");
    write(&root, "a.txt", b"foo eins\nfoo zwei\n");
    write(&root, "b.txt", b"foo drei\n");
    let started = dispatch(
        &fx.rt,
        "search.content.start",
        &json!({ "location": root.to_string_lossy(), "pattern": "foo" }),
    )
    .expect("search");
    let id = started["taskId"].as_str().expect("taskId").to_string();
    let (found, _) = poll_all(&fx, "search.content.poll", &id);
    // The hits as the poll delivered them: only line 2 of a.txt, and b.txt.
    let mut hits = Vec::new();
    for event in found.iter().filter(|e| e["type"] == "hits") {
        for line in event["lines"].as_array().expect("lines") {
            if event["rel"] == "a.txt" && line["line"] != 2 {
                continue;
            }
            hits.push(json!({
                "rel": event["rel"], "line": line["line"], "text": line["text"],
                "ranges": line["ranges"], "truncated": line["truncated"],
                "size": event["size"], "mtimeMs": event["mtimeMs"],
            }));
        }
    }
    assert_eq!(hits.len(), 2, "{found:?}");
    // b.txt changes after the search: it is skipped, a.txt is replaced.
    write(&root, "b.txt", b"foo drei, jetzt laenger\n");
    let (events, end) = replace(
        &fx,
        json!({ "location": root.to_string_lossy(), "pattern": "foo", "replacement": "bar",
                "selection": "hits", "hits": hits }),
    );
    assert_eq!(read(&root, "a.txt"), "foo eins\nbar zwei\n", "{events:?}");
    assert_eq!(read(&root, "b.txt"), "foo drei, jetzt laenger\n");
    assert!(events
        .iter()
        .any(|e| e["type"] == "skipped" && e["rel"] == "b.txt" && e["reasonKey"] == "changed"));
    assert_eq!(end["summary"]["replacements"], 1);
}

#[test]
fn replace_ui_task_bridge_run_keeps_a_journal_entry_that_undo_restores() {
    let fx = fixture();
    let root = fx.dir.path().join("lauf");
    write(&root, "a.txt", b"Hallo Welt\n");
    write(&root, "sub/b.md", b"hallo hallo\n");
    let (_, end) = replace(
        &fx,
        json!({ "location": root.to_string_lossy(), "pattern": "hallo", "replacement": "Servus" }),
    );
    let summary = &end["summary"];
    assert_eq!(summary["replacements"], 3, "{end}");
    assert_eq!(summary["filesChanged"], 2);
    assert_eq!(read(&root, "a.txt"), "Servus Welt\n");
    assert_eq!(read(&root, "sub/b.md"), "Servus Servus\n");
    let undo_id = summary["undoId"].as_str().expect("undo id").to_string();
    let history = dispatch(&fx.rt, "search.content.replace.history", &json!({})).expect("history");
    assert!(
        history["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .any(|entry| entry["undoId"] == undo_id.as_str()),
        "{history}"
    );
    dispatch(
        &fx.rt,
        "search.content.replace.undo",
        &json!({ "undoId": undo_id }),
    )
    .expect("undo");
    assert_eq!(read(&root, "a.txt"), "Hallo Welt\n");
    assert_eq!(read(&root, "sub/b.md"), "hallo hallo\n");
    assert_eq!(
        dispatch(&fx.rt, "search.content.replace.undo", &json!({})).map_err(|error| error.kind),
        Err("invalid")
    );
}
