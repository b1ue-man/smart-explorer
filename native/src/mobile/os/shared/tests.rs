//! Facade methods against temporary folders, on a runtime that is not
//! installed globally (no host values, no app trash volumes).
use super::config::HostSettings;
use super::dispatch::dispatch;
use super::edits_store::{self, EditRecord};
use super::error::ApiError;
use super::runtime::Runtime;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct Fixture {
    _dir: tempfile::TempDir,
    rt: Runtime,
    work: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("temp dir");
    let files = dir.path().join("files");
    let cache = dir.path().join("cache");
    let work = dir.path().join("work");
    for path in [&files, &cache, &work] {
        std::fs::create_dir_all(path).expect("create fixture dir");
    }
    let config = json!({ "filesDir": files, "cacheDir": cache, "startDaemon": false });
    let settings = HostSettings::parse(&config.to_string()).expect("settings");
    Fixture {
        _dir: dir,
        rt: Runtime::detached(settings),
        work,
    }
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn call(rt: &Runtime, method: &str, args: Value) -> Result<Value, ApiError> {
    dispatch(rt, method, &args)
}

fn kind(result: Result<Value, ApiError>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(error) => error.kind,
    }
}

fn names(listing: &Value) -> Vec<String> {
    listing["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|entry| entry["name"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// Waits for a task to finish and returns its snapshot.
fn wait_task(rt: &Runtime, id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let snapshot = rt
            .hub()
            .with(|state| state.tasks.get(id).map(|record| record.snapshot()))
            .expect("task exists");
        let state = snapshot["state"].as_str().unwrap_or_default().to_string();
        if matches!(state.as_str(), "done" | "failed" | "canceled") {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "task {id} did not finish: {snapshot}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn android_task_local_fs_methods_create_rename_check_and_list() {
    let fx = fixture();
    let root = text(&fx.work);
    let folder = call(
        &fx.rt,
        "fs.mkdir",
        json!({ "parent": root, "name": "Fotos" }),
    )
    .expect("mkdir");
    assert_eq!(folder["isDir"], true);
    assert_eq!(folder["location"], format!("{root}/Fotos"));
    assert_eq!(
        kind(call(
            &fx.rt,
            "fs.mkdir",
            json!({ "parent": root, "name": "Fotos" })
        )),
        "exists"
    );

    let file = call(
        &fx.rt,
        "fs.newFile",
        json!({ "parent": root, "name": "notiz.txt" }),
    )
    .expect("new file");
    assert_eq!(
        (file["isDir"].clone(), file["size"].clone()),
        (json!(false), json!(0))
    );
    let renamed = call(
        &fx.rt,
        "fs.rename",
        json!({ "location": format!("{root}/notiz.txt"), "newName": "bericht.txt" }),
    )
    .expect("rename");
    assert_eq!(renamed["name"], "bericht.txt");
    assert_eq!(
        kind(call(
            &fx.rt,
            "fs.rename",
            json!({ "location": format!("{root}/bericht.txt"), "newName": "Fotos" })
        )),
        "exists"
    );

    let taken = call(
        &fx.rt,
        "fs.checkName",
        json!({ "parent": root, "name": "Fotos" }),
    )
    .expect("check");
    assert_eq!(
        (taken["exists"].clone(), taken["problem"].clone()),
        (json!(true), Value::Null)
    );
    let invalid = call(
        &fx.rt,
        "fs.checkName",
        json!({ "parent": root, "name": "a/b" }),
    )
    .expect("check");
    assert_eq!(invalid["invalid"], true);
    let windows = call(
        &fx.rt,
        "fs.checkName",
        json!({ "parent": root, "name": "nul.txt" }),
    )
    .expect("check");
    assert!(windows["problem"].is_string());

    std::fs::write(fx.work.join("gross.bin"), vec![0u8; 100]).expect("write");
    std::fs::write(fx.work.join(".versteckt"), b"x").expect("write");
    let sort = json!({ "key": "size", "desc": true, "dirsFirst": true });
    let listing = call(
        &fx.rt,
        "fs.list",
        json!({ "location": root, "showHidden": false, "sort": sort }),
    )
    .expect("list");
    assert_eq!(names(&listing), vec!["Fotos", "gross.bin", "bericht.txt"]);
    assert_eq!(listing["totalBytes"], 100);
    assert_eq!(listing["backend"], "local");
    assert_eq!(listing["readOnly"], false);
    let with_hidden = call(
        &fx.rt,
        "fs.list",
        json!({ "location": root, "showHidden": true, "sort": sort }),
    )
    .expect("list");
    assert!(names(&with_hidden).contains(&".versteckt".to_string()));
    let filtered = call(
        &fx.rt,
        "fs.list",
        json!({ "location": root, "showHidden": false, "sort": sort,
                "filter": { "text": "gross", "mode": "substring", "files": true, "dirs": true } }),
    )
    .expect("filtered list");
    assert_eq!(names(&filtered), vec!["gross.bin"]);

    let stat = call(
        &fx.rt,
        "fs.stat",
        json!({ "location": format!("{root}/Fotos") }),
    )
    .expect("stat");
    assert_eq!(stat["isDir"], true);
    let conflicts = call(
        &fx.rt,
        "fs.conflicts",
        json!({ "sources": ["/elsewhere/bericht.txt", "/elsewhere/neu.txt"], "targetDir": root }),
    )
    .expect("conflicts");
    assert_eq!(
        conflicts,
        json!({ "names": ["bericht.txt"], "choosable": true })
    );
}

#[test]
fn android_task_app_internal_locations_are_rejected() {
    let fx = fixture();
    let root = text(&fx.work);
    let zip = "zip:///sdcard/archiv.zip!/";
    assert_eq!(
        kind(call(
            &fx.rt,
            "loc.toggleFavorite",
            json!({ "location": zip })
        )),
        "invalid"
    );
    assert_eq!(
        kind(call(
            &fx.rt,
            "loc.isFavorite",
            json!({ "location": "trash://" })
        )),
        "invalid"
    );
    assert_eq!(
        kind(call(
            &fx.rt,
            "fs.mkdir",
            json!({ "parent": zip, "name": "a" })
        )),
        "permission"
    );
    assert_eq!(
        kind(call(&fx.rt, "fs.stat", json!({ "location": "trash://" }))),
        "invalid"
    );
    assert_eq!(
        kind(call(
            &fx.rt,
            "fs.transfer",
            json!({ "sources": ["trash://"], "targetDir": root })
        )),
        "invalid"
    );
    assert_eq!(
        kind(call(
            &fx.rt,
            "fs.transfer",
            json!({ "sources": [format!("{root}/a")], "targetDir": zip })
        )),
        "permission"
    );
    // Moving to a remote is refused before any connection is opened.
    assert_eq!(
        kind(call(
            &fx.rt,
            "fs.transfer",
            json!({ "sources": [format!("{root}/a")], "targetDir": "sftp://u@h:22/x", "mode": "move" })
        )),
        "unsupported"
    );
    // Without app trash volumes a local delete must be confirmed as permanent.
    std::fs::write(fx.work.join("weg.txt"), b"x").expect("write");
    assert_eq!(
        kind(call(
            &fx.rt,
            "fs.delete",
            json!({ "locations": [format!("{root}/weg.txt")], "permanent": false })
        )),
        "unsupported"
    );
    assert!(fx.work.join("weg.txt").exists());
}

#[test]
fn android_task_local_copy_keeps_both_names_and_delete_is_permanent() {
    let fx = fixture();
    let source = fx.work.join("quelle");
    let target = fx.work.join("ziel");
    std::fs::create_dir_all(&source).expect("mkdir");
    std::fs::create_dir_all(&target).expect("mkdir");
    std::fs::write(source.join("a.txt"), b"neu").expect("write");
    std::fs::write(target.join("a.txt"), b"alt").expect("write");
    let started = call(
        &fx.rt,
        "fs.transfer",
        json!({ "sources": [text(&source.join("a.txt"))], "targetDir": text(&target),
                "mode": "copy", "conflict": "keepBoth" }),
    )
    .expect("transfer");
    let task = wait_task(&fx.rt, started["taskId"].as_str().expect("task id"));
    assert_eq!(task["state"], "done", "{task}");
    assert_eq!(std::fs::read(target.join("a.txt")).expect("read"), b"alt");
    assert_eq!(std::fs::read_dir(&target).expect("list").count(), 2);

    let started = call(
        &fx.rt,
        "fs.delete",
        json!({ "locations": [text(&source)], "permanent": true }),
    )
    .expect("delete");
    let task = wait_task(&fx.rt, started["taskId"].as_str().expect("task id"));
    assert_eq!(task["state"], "done", "{task}");
    assert!(!source.exists());
}

#[test]
fn android_task_scan_view_returns_a_windowed_tree_and_revisions() {
    let fx = fixture();
    std::fs::create_dir_all(fx.work.join("b")).expect("mkdir");
    std::fs::write(fx.work.join("a.txt"), b"a").expect("write");
    std::fs::write(fx.work.join("b/c.jpg"), b"c").expect("write");
    let started = call(
        &fx.rt,
        "scan.start",
        json!({ "location": text(&fx.work), "showHidden": false }),
    )
    .expect("scan");
    let id = started["taskId"].as_str().expect("task id").to_string();
    let task = wait_task(&fx.rt, &id);
    assert_eq!(task["state"], "done", "{task}");
    let view = call(
        &fx.rt,
        "scan.view",
        json!({ "taskId": id, "offset": 0, "limit": 500, "collapsed": [] }),
    )
    .expect("view");
    assert_eq!(names(&view), vec!["b", "c.jpg", "a.txt"]);
    assert_eq!(view["entries"][1]["depth"], 1);
    assert_eq!(view["entries"][0]["hasChildren"], true);
    assert_eq!(view["matches"], 3);
    let again = call(
        &fx.rt,
        "scan.view",
        json!({ "taskId": id, "offset": 0, "limit": 500, "collapsed": [],
                "sinceRevision": view["revision"] }),
    )
    .expect("view");
    assert_eq!(again["unchanged"], true);
    assert!(again["entries"].as_array().is_some_and(Vec::is_empty));
    let folded = call(
        &fx.rt,
        "scan.view",
        json!({ "taskId": id, "offset": 0, "limit": 500,
                "collapsed": [text(&fx.work.join("b"))] }),
    )
    .expect("view");
    assert_eq!(names(&folded), vec!["b", "a.txt"]);
    assert_eq!(folded["entries"][0]["expanded"], false);
}

#[test]
fn android_task_edit_register_round_trip_and_change_event() {
    let fx = fixture();
    let dir = edits_store::open_root(&fx.rt).join("e1");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let local = dir.join("text.txt");
    std::fs::write(&local, b"eins").expect("write");
    let mut record = EditRecord {
        edit_id: "e1".into(),
        name: "text.txt".into(),
        location: "sftp://u@h:22/home/text.txt".into(),
        local_path: text(&local),
        remote_mtime_ms: 1_000,
        ..EditRecord::default()
    };
    record.rebase_local();
    edits_store::update(&fx.rt, |records| records.push(record.clone()));
    assert_eq!(edits_store::load(&fx.rt), vec![record.clone()]);
    assert_eq!(record.modified(), Some(false));

    std::fs::write(&local, b"eins und zwei").expect("write");
    let listed = call(&fx.rt, "fs.edits", json!({})).expect("edits");
    assert_eq!(listed[0]["modified"], true);
    let (events, _) = fx
        .rt
        .hub()
        .with(|state| state.take_ready(Instant::now(), 256));
    assert!(events.iter().any(|event| event["type"] == "edits"));
    let _ = call(&fx.rt, "fs.edits", json!({})).expect("edits");
    let (events, _) = fx
        .rt
        .hub()
        .with(|state| state.take_ready(Instant::now(), 256));
    assert!(
        !events.iter().any(|event| event["type"] == "edits"),
        "announced once"
    );

    call(&fx.rt, "fs.discardEdit", json!({ "editId": "e1" })).expect("discard");
    assert!(edits_store::load(&fx.rt).is_empty());
    assert!(!dir.exists());
}

#[test]
fn android_task_bad_arguments_and_unknown_ids() {
    let fx = fixture();
    assert_eq!(kind(call(&fx.rt, "fs.list", json!({}))), "invalid");
    let validation = call(
        &fx.rt,
        "scan.validate",
        json!({ "filter": { "text": "(", "mode": "regex" } }),
    )
    .expect("validate");
    assert!(validation["error"].is_string());
    assert_eq!(
        kind(call(&fx.rt, "task.get", json!({ "id": "x" }))),
        "not_found"
    );
    assert_eq!(
        kind(call(&fx.rt, "scan.view", json!({ "taskId": "x" }))),
        "not_found"
    );
    assert_eq!(
        kind(call(&fx.rt, "fs.discardEdit", json!({ "editId": "../x" }))),
        "ok"
    );
    assert_eq!(
        super::transfer::common_parent(&["/a/b/c".into(), "/a/b/d/e".into()]),
        "/a/b"
    );
}
