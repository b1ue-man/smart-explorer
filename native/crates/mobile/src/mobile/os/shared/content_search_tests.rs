//! `search.content.*` (api.md §4.14) through the dispatcher of a runtime
//! that is not installed globally, over a temporary local folder: query
//! fields, an invalid pattern at once, polling until the end, the hits in
//! UTF-16 offsets, skipped files, the backlog bound and the cancel.
use super::super::config::HostSettings;
use super::super::dispatch::dispatch;
use super::*;
use crate::agent_proto::TextMode;
use std::path::Path;
use std::time::Instant;

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

fn task_state(rt: &Runtime, id: &str) -> String {
    rt.hub()
        .with(|state| state.tasks.get(id).map(|record| record.snapshot()))
        .and_then(|snapshot| snapshot["state"].as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Polls `id` from offset 0 on until the search is done; all events.
fn poll_all(fx: &Fixture, id: &str) -> (Vec<Value>, Value) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut since = 0;
    let mut events = Vec::new();
    loop {
        let answer = dispatch(
            &fx.rt,
            "search.content.poll",
            &json!({ "taskId": id, "since": since }),
        )
        .expect("poll");
        events.extend(answer["events"].as_array().cloned().unwrap_or_default());
        since = answer["next"].as_u64().expect("next");
        if answer["done"] == true {
            return (events, answer);
        }
        assert!(Instant::now() < deadline, "search did not end: {answer}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn content_search_task_bridge_query_fields() {
    let camel = parse_query(&json!({
        "pattern": "a\\tb", "mode": "extended", "matchCase": true, "wholeWord": true,
        "filters": "*.rs !*.log", "folderInclude": "^src(/|$)", "folderExclude": " ",
        "subfolders": false, "hidden": true, "contextLines": 2, "maxHits": 9, "filesOnly": true,
    }))
    .expect("camelCase query");
    assert_eq!(camel.mode, TextMode::Extended);
    assert!(camel.match_case && camel.whole_word && camel.hidden && camel.files_only);
    assert!(!camel.subfolders);
    assert_eq!(camel.filters, "*.rs !*.log");
    assert_eq!(camel.folder_include.as_deref(), Some("^src(/|$)"));
    assert_eq!(camel.folder_exclude, None, "a blank field is no expression");
    assert_eq!((camel.context_lines, camel.max_hits), (2, 9));
    // The core's own field names work as well.
    let snake = parse_query(&json!({
        "pattern": "x", "mode": "regex", "match_case": true, "dot_matches_newline": true,
        "folder_exclude": "^tests$", "context_lines": 1, "max_hits": 3, "files_only": true,
    }))
    .expect("snake_case query");
    assert_eq!(snake.mode, TextMode::Regex);
    assert!(snake.match_case && snake.dot_matches_newline && snake.files_only);
    assert!(snake.subfolders, "subfolders default on as in Notepad++");
    assert_eq!(snake.folder_exclude.as_deref(), Some("^tests$"));
    assert_eq!((snake.context_lines, snake.max_hits), (1, 3));
}

#[test]
fn content_search_task_bridge_invalid_input_fails_at_once() {
    let fx = fixture();
    let location = fx.dir.path().to_string_lossy().into_owned();
    let start = |args: Value| dispatch(&fx.rt, "search.content.start", &args).map_err(|e| e.kind);
    assert_eq!(
        start(json!({ "location": location, "pattern": "(", "mode": "regex" })),
        Err("invalid")
    );
    assert_eq!(
        start(json!({ "location": location, "pattern": "" })),
        Err("invalid")
    );
    assert_eq!(
        start(json!({ "location": location, "pattern": "x", "mode": "fuzzy" })),
        Err("invalid")
    );
    assert_eq!(
        start(json!({ "location": location, "pattern": "x", "folderInclude": "(" })),
        Err("invalid")
    );
    assert_eq!(start(json!({ "pattern": "x" })), Err("invalid"));
    assert_eq!(
        start(json!({ "location": "trash://", "pattern": "x" })),
        Err("invalid")
    );
    assert_eq!(
        dispatch(&fx.rt, "search.content.poll", &json!({ "taskId": "fehlt" })).map_err(|e| e.kind),
        Err("not_found")
    );
}

#[test]
fn content_search_task_bridge_finds_and_polls_until_done() {
    let fx = fixture();
    let root = fx.dir.path().join("baum");
    write(&root, "a.txt", "Hallo Welt\nGröße hallo\n".as_bytes());
    write(&root, "sub/b.rs", b"fn hallo() {}\n");
    write(&root, "skip/c.txt", b"hallo\n");
    write(&root, ".versteckt/d.txt", b"hallo\n");
    write(&root, "bild.dat", b"\0\0\0hallo");
    write(&root, "leer.txt", b"nichts\n");
    let started = dispatch(
        &fx.rt,
        "search.content.start",
        &json!({ "location": root.to_string_lossy(), "pattern": "hallo", "folderExclude": "^skip$" }),
    )
    .expect("start");
    let id = started["taskId"].as_str().expect("taskId").to_string();
    let (events, end) = poll_all(&fx, &id);

    let hits: Vec<&Value> = events.iter().filter(|e| e["type"] == "hits").collect();
    let mut files: Vec<&str> = hits.iter().filter_map(|e| e["rel"].as_str()).collect();
    files.sort();
    files.dedup();
    assert_eq!(files, vec!["a.txt", "sub/b.rs"], "{events:?}");
    let a = hits.iter().find(|e| e["rel"] == "a.txt").expect("a.txt");
    assert_eq!(a["location"], root.join("a.txt").to_string_lossy().as_ref());
    let lines = a["lines"].as_array().expect("lines");
    assert_eq!(lines.len(), 2);
    assert_eq!(
        (lines[0]["line"].as_u64(), lines[0]["ranges"].clone()),
        (Some(1), json!([[0, 5]]))
    );
    // "Größe " is 6 UTF-16 units (8 bytes).
    assert_eq!(lines[1]["text"], "Größe hallo");
    assert_eq!(lines[1]["ranges"], json!([[6, 11]]));
    assert_eq!(lines[1]["context"], false);
    let skipped: Vec<&Value> = events.iter().filter(|e| e["type"] == "skipped").collect();
    assert!(
        skipped
            .iter()
            .any(|e| e["rel"] == "bild.dat" && e["reason"] == "binary"),
        "{skipped:?}"
    );
    assert!(!events.iter().any(|e| e["rel"]
        .as_str()
        .is_some_and(|rel| rel.starts_with("skip/") || rel.starts_with(".versteckt/"))));

    let summary = &end["summary"];
    assert_eq!(summary["filesMatched"], 2);
    assert_eq!(summary["hits"], 3);
    assert!(summary["filesSearched"].as_u64().is_some_and(|n| n >= 3));
    assert_eq!(end["canceled"], false);
    assert!(end["error"].is_null());
    let deadline = Instant::now() + Duration::from_secs(10);
    while task_state(&fx.rt, &id) != "done" {
        assert!(Instant::now() < deadline, "task did not end");
        std::thread::sleep(Duration::from_millis(10));
    }
    // A finished search stays readable; fetched events are released.
    let again = dispatch(
        &fx.rt,
        "search.content.poll",
        &json!({ "taskId": id, "since": end["next"] }),
    )
    .expect("poll again");
    assert_eq!(again["events"], json!([]));
    assert_eq!(again["done"], true);
}

#[test]
fn content_search_task_bridge_backlog_waits_and_cancel_ends() {
    let fx = fixture();
    let root = fx.dir.path().join("viele");
    // More files with a hit than the backlog and the channel hold.
    for index in 0..(MAX_BACKLOG + CHANNEL_SLOTS + 400) {
        write(
            &root,
            &format!("d{}/f{index}.txt", index % 16),
            b"treffer\n",
        );
    }
    let started = dispatch(
        &fx.rt,
        "search.content.start",
        &json!({ "location": root.to_string_lossy(), "pattern": "treffer" }),
    )
    .expect("start");
    let id = started["taskId"].as_str().expect("taskId").to_string();
    let session = fx.rt.inner.content.get(&id).expect("session");
    let deadline = Instant::now() + Duration::from_secs(30);
    while lock(&session.data).events.len() < MAX_BACKLOG {
        assert!(Instant::now() < deadline, "backlog did not fill");
        std::thread::sleep(Duration::from_millis(10));
    }
    // Unfetched, the search waits: the backlog stays at its bound.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(lock(&session.data).events.len(), MAX_BACKLOG);
    assert_eq!(task_state(&fx.rt, &id), "running");
    let first = session.poll(0);
    assert_eq!(
        first["events"].as_array().map(Vec::len),
        Some(MAX_POLL_EVENTS)
    );
    assert_eq!(first["next"], MAX_POLL_EVENTS);
    assert_eq!(first["done"], false);

    dispatch(&fx.rt, "search.content.cancel", &json!({ "taskId": id })).expect("cancel");
    let deadline = Instant::now() + Duration::from_secs(20);
    while task_state(&fx.rt, &id) != "canceled" {
        assert!(Instant::now() < deadline, "cancel did not end the search");
        std::thread::sleep(Duration::from_millis(10));
    }
    let (_, end) = poll_all(&fx, &id);
    assert_eq!(end["canceled"], true);
    assert!(end["summary"].is_null());
}

#[test]
fn content_search_task_bridge_poll_releases_and_registry_keeps_running() {
    let base = Loc::parse("/storage/emulated/0").expect("location");
    let session = SearchSession::new(base);
    let cancel = AtomicBool::new(false);
    for index in 0..5 {
        assert!(session.push(json!({ "n": index }), &cancel));
    }
    let answer = session.poll(0);
    assert_eq!(answer["events"].as_array().map(Vec::len), Some(5));
    assert_eq!(answer["next"], 5);
    let answer = session.poll(3);
    assert_eq!(answer["events"], json!([{ "n": 3 }, { "n": 4 }]));
    assert_eq!(
        lock(&session.data).first,
        3,
        "events before `since` are released"
    );
    assert_eq!(answer["done"], false);
    session.finish(&Ok(ContentSummary::default()));
    // The answer that hands out the last events of an ended search is done.
    let last = session.poll(4);
    assert_eq!(last["events"], json!([{ "n": 4 }]));
    assert_eq!(last["done"], true);
    assert_eq!(session.poll(5)["events"], json!([]));

    let registry = ContentSearches::default();
    let running = SearchSession::new(Loc::parse("/a").expect("location"));
    registry.insert("run".into(), running);
    for index in 0..MAX_SESSIONS + 2 {
        let done = SearchSession::new(Loc::parse("/b").expect("location"));
        done.finish(&Err(ApiError::canceled()));
        registry.insert(format!("done{index}"), done);
    }
    assert!(
        registry.get("run").is_ok(),
        "a running search is never dropped"
    );
    assert!(registry.get("done0").is_err());
    assert!(registry.get(&format!("done{}", MAX_SESSIONS + 1)).is_ok());
}
