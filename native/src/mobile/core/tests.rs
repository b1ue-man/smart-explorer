//! Pure parts of the facade: envelopes, error kinds, task snapshots and
//! their bundling, locations, filter/sort arguments and scan tree windows.
use super::args::{parse_filter, sort_arg, SortSpec};
use super::entry::{kind_of, mime_of, problem_of};
use super::error::{envelope, ApiError};
use super::events::HubState;
use super::location::{is_app_internal, Loc, LocKind};
use super::scanview::{tree_rows, visible_rows, window};
use super::tasks::{TaskState, EMIT_INTERVAL};
use crate::types::{FileEntry, FilterDef, SortDir, SortKey, TextMode};
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).expect("envelope is JSON")
}

#[test]
fn android_task_envelopes_carry_ok_values_and_error_kinds() {
    assert_eq!(
        parsed(&envelope(Ok(json!({ "a": 1 })))),
        json!({ "ok": { "a": 1 } })
    );
    let error = ApiError::invalid("Name ist leer");
    assert_eq!(
        parsed(&envelope(Err(error))),
        json!({ "err": { "kind": "invalid", "message": "Name ist leer" } })
    );
}

#[test]
fn android_task_io_errors_map_to_protocol_kinds() {
    use std::io::{Error, ErrorKind};
    let cases = [
        (ErrorKind::NotFound, "not_found"),
        (ErrorKind::PermissionDenied, "permission"),
        (ErrorKind::AlreadyExists, "exists"),
        (ErrorKind::InvalidInput, "invalid"),
        (ErrorKind::Unsupported, "unsupported"),
        (ErrorKind::TimedOut, "network"),
        (ErrorKind::Interrupted, "canceled"),
        (ErrorKind::Other, "internal"),
    ];
    for (kind, expected) in cases {
        assert_eq!(
            ApiError::from(Error::new(kind, "x")).kind,
            expected,
            "{kind:?}"
        );
    }
    // A custom (German) message is kept as it is.
    let custom = ApiError::from(Error::new(
        ErrorKind::PermissionDenied,
        "ZIP ist schreibgeschützt",
    ));
    assert_eq!(custom.message, "ZIP ist schreibgeschützt");
    assert_eq!(
        ApiError::connection("Authentifizierung fehlgeschlagen").kind,
        "auth"
    );
    assert_eq!(
        ApiError::connection("Host nicht erreichbar").kind,
        "network"
    );
}

#[test]
fn android_task_task_snapshots_are_bundled_per_task_and_terminal_is_immediate() {
    let mut hub = HubState::new(1);
    let start = Instant::now();
    let id = hub.tasks.create(
        "transfer",
        "Kopieren".into(),
        Arc::new(AtomicBool::new(false)),
        0,
    );
    // Creation is delivered right away.
    let (events, _) = hub.take_ready(start, 256);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["task"]["state"], "queued");

    // Progress within the interval is held back and reported as due later.
    if let Some(record) = hub.tasks.get_mut(&id) {
        record.set_running();
        record.progress((10, 100, 1, 10), start);
    }
    let (events, next) = hub.take_ready(start, 256);
    assert!(events.is_empty());
    assert_eq!(next, Some(start + EMIT_INTERVAL));

    // Several updates collapse into one snapshot once due.
    if let Some(record) = hub.tasks.get_mut(&id) {
        record.progress((50, 100, 5, 10), start);
    }
    let (events, _) = hub.take_ready(start + EMIT_INTERVAL, 256);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["task"]["doneBytes"], 50);

    // The terminal snapshot is not held back by the interval.
    if let Some(record) = hub.tasks.get_mut(&id) {
        record.finish(TaskState::Done, None, Some(json!({ "files": 10 })), 5);
    }
    let (events, next) = hub.take_ready(start + EMIT_INTERVAL, 256);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["task"]["state"], "done");
    assert_eq!(events[0]["task"]["result"]["files"], 10);
    assert_eq!(next, None);
}

#[test]
fn android_task_event_polls_are_capped_and_keep_order() {
    let mut hub = HubState::new(1);
    for index in 0..300 {
        hub.push_event(json!({ "type": "volumes", "n": index }));
    }
    let now = Instant::now();
    let (first, next) = hub.take_ready(now, 256);
    assert_eq!(first.len(), 256);
    assert_eq!(first[0]["n"], 0);
    assert_eq!(next, Some(now));
    let (rest, _) = hub.take_ready(now, 256);
    assert_eq!(rest.len(), 44);
    assert_eq!(rest[43]["n"], 299);
}

#[test]
fn android_task_cancel_and_clear_touch_only_matching_tasks() {
    let mut hub = HubState::new(1);
    let transfer_flag = Arc::new(AtomicBool::new(false));
    let scan_flag = Arc::new(AtomicBool::new(false));
    let transfer = hub
        .tasks
        .create("transfer", "a".into(), transfer_flag.clone(), 0);
    let scan = hub.tasks.create("scan", "b".into(), scan_flag.clone(), 0);
    hub.tasks.cancel_all(Some("transfer"));
    assert!(transfer_flag.load(std::sync::atomic::Ordering::Acquire));
    assert!(!scan_flag.load(std::sync::atomic::Ordering::Acquire));
    if let Some(record) = hub.tasks.get_mut(&transfer) {
        record.finish(TaskState::Canceled, None, None, 1);
    }
    let _ = hub.take_ready(Instant::now(), 256);
    hub.tasks.clear_finished();
    assert!(hub.tasks.get(&transfer).is_none());
    assert!(hub.tasks.get(&scan).is_some());
    assert!(!hub.tasks.cancel("unbekannt"));
}

#[test]
fn android_task_locations_parse_into_connection_and_path() {
    let local = Loc::parse("/storage/emulated/0/DCIM/").expect("local");
    assert_eq!(
        (local.kind, local.path.as_str()),
        (LocKind::Local, "/storage/emulated/0/DCIM")
    );
    assert_eq!(local.child("a.jpg"), "/storage/emulated/0/DCIM/a.jpg");
    assert_eq!(local.parent().as_deref(), Some("/storage/emulated/0"));

    let sftp = Loc::parse("sftp://user@host:22/home/user").expect("sftp");
    assert_eq!(sftp.prefix, "sftp://user@host:22");
    assert_eq!(sftp.child("x"), "sftp://user@host:22/home/user/x");
    assert_eq!(Loc::parse("sftp://user@host:22").expect("root").path, "/");

    let drive = Loc::parse("gdrive:///Ordner").expect("drive");
    assert_eq!(drive.location(), "gdrive:///Ordner");
    assert_eq!(
        Loc::parse("gdrive://").expect("drive root").location(),
        "gdrive:///"
    );

    let zip = Loc::parse("zip:///sdcard/a.zip!/inner/b").expect("zip");
    assert_eq!(zip.zip_archive(), Some("/sdcard/a.zip"));
    assert_eq!(zip.path, "/inner/b");
    let zip_root = Loc::parse("zip:///sdcard/a.zip!/").expect("zip root");
    assert_eq!(zip_root.parent().as_deref(), Some("/sdcard"));

    assert!(Loc::parse("/a/../b").is_err());
    assert!(Loc::parse("relative/path").is_err());
    assert!(Loc::parse("smb://server/share").is_err());
}

#[test]
fn android_task_app_internal_locations_are_recognized() {
    assert!(is_app_internal("zip:///sdcard/a.zip!/"));
    assert!(is_app_internal("trash://"));
    assert!(!is_app_internal("/storage/emulated/0"));
    assert!(!is_app_internal("share://direct/abc/"));
    assert_eq!(Loc::parse("trash://").expect("trash").kind, LocKind::Trash);
    assert!(Loc::parse("zip:///a.zip!/x")
        .expect("zip")
        .is_app_internal());
}

#[test]
fn android_task_favorite_keys_follow_the_desktop_format() {
    assert_eq!(
        Loc::parse("/storage/emulated/0/DCIM/")
            .expect("local")
            .favorite_key(),
        "/storage/emulated/0/DCIM"
    );
    assert_eq!(
        Loc::parse("sftp://u@h:22/home/")
            .expect("sftp")
            .favorite_key(),
        crate::connect::location_key(Some("sftp://u@h:22"), "/home")
    );
}

#[test]
fn android_task_filter_and_sort_arguments_map_to_the_desktop_model() {
    let filter = parse_filter(
        &json!({
            "text": "urlaub", "mode": "glob", "extensions": "jpg; *.heic",
            "sizeMin": 1024, "sizeMax": null, "mtimeMinMs": 5, "files": true,
            "dirs": false, "hidden": false, "problemOnly": true
        }),
        true,
    )
    .expect("filter");
    assert_eq!(filter.text_mode, TextMode::Glob);
    assert_eq!(
        filter.extensions,
        vec!["heic".to_string(), "jpg".to_string()]
    );
    assert_eq!((filter.size.min, filter.size.max), (Some(1024), None));
    assert_eq!(filter.mtime.min, Some(5));
    assert!(!filter.include_dirs && filter.include_files);
    assert!(
        filter.include_hidden,
        "the view option shows hidden entries"
    );
    assert!(filter.problem_names_only);
    assert!(parse_filter(&json!({ "mode": "fuzzy" }), false).is_err());

    let sort = sort_arg(
        &json!({ "sort": { "key": "type", "desc": true, "dirsFirst": false } }),
        "sort",
    )
    .expect("sort");
    assert_eq!(
        sort,
        SortSpec {
            key: SortKey::Ext,
            dir: SortDir::Desc,
            dirs_first: false
        }
    );
    assert_eq!(
        sort_arg(&json!({}), "sort").expect("default"),
        SortSpec::default()
    );
}

#[test]
fn android_task_entry_types_mime_and_problem_names() {
    assert_eq!(kind_of("jpg", false), "image");
    assert_eq!(kind_of("apk", false), "apk");
    assert_eq!(kind_of("", true), "dir");
    assert_eq!(mime_of("Bericht.PDF"), "application/pdf");
    assert_eq!(mime_of("ohne"), "application/octet-stream");
    assert!(problem_of("nul.txt").is_some());
    assert!(problem_of("normal.txt").is_none());
}

fn entry(path: &str, is_dir: bool, depth: u32, size: u64) -> FileEntry {
    let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
    FileEntry {
        path: Arc::from(path),
        parent: Arc::from(if parent.is_empty() { "/" } else { parent }),
        name: Arc::from(name),
        ext: Arc::from(super::entry::extension(name, is_dir).as_str()),
        size,
        mtime_ms: 0,
        btime_ms: 0,
        is_dir,
        is_symlink: false,
        hidden: false,
        system: false,
        depth,
        id: None,
    }
}

#[test]
fn android_task_scan_tree_folds_collapsed_folders_and_windows() {
    let entries = vec![
        entry("/r", true, 0, 0),
        entry("/r/b", true, 1, 0),
        entry("/r/a.txt", false, 1, 5),
        entry("/r/b/c.txt", false, 2, 7),
        entry("/r/b/d.jpg", false, 2, 9),
    ];
    let filter = FilterDef::new();
    let tree = tree_rows(&entries, "/r", &filter, SortSpec::default());
    let names: Vec<&str> = tree
        .rows
        .iter()
        .map(|&(index, _)| entries[index].name.as_ref())
        .collect();
    assert_eq!(names, vec!["b", "c.txt", "d.jpg", "a.txt"]);
    assert_eq!(tree.has_children, vec![true, false, false, false]);

    let open = visible_rows(&tree, &entries, |_| false);
    assert_eq!(open.len(), 4);
    let folded = visible_rows(&tree, &entries, |entry| entry.path.as_ref() == "/r/b");
    assert_eq!(folded, vec![0, 3]);
    assert_eq!(window(&open, 1, 2), &[1, 2]);
    assert!(window(&open, 9, 2).is_empty());

    let mut jpg_only = FilterDef::new();
    jpg_only.extensions = vec!["jpg".into()];
    let filtered = tree_rows(&entries, "/r", &jpg_only, SortSpec::default());
    let names: Vec<&str> = filtered
        .rows
        .iter()
        .map(|&(index, _)| entries[index].name.as_ref())
        .collect();
    assert_eq!(names, vec!["b", "d.jpg"], "a match keeps its folder row");
}
