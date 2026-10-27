//! End-to-end scan fixtures with explicit root discovery and identity probes.
use super::super::core::cloud_urlenc;
use super::super::gui_task_http::Step;
use super::{duplicates, names, object, step, Fixture, Reply, FILES, TITLE};
use crate::types::{FileEntry, ScanProgress};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

const ROOT: &str = "/drive/v3/files/root";
pub(super) const ROOT_ID: &str = "scan-root-id";
const EMPTY: &str = "/drive/v3/files/empty-id";

pub(super) fn root() -> Step {
    step("GET", ROOT, Reply::Json(json!({"id": ROOT_ID})))
        .query("fields", Some("id".into()))
}

pub(super) fn child(name: &str, id: &str, parent: &str, folder: bool) -> Value {
    let mut value = object(name, id, folder);
    value["parents"] = json!([parent]);
    value
}

pub(super) fn listing(parent: &str, token: Option<&str>, name: Option<&str>, reply: Reply) -> Step {
    let mut query = format!("'{parent}' in parents");
    if let Some(name) = name {
        let name = name.replace('\\', "\\\\").replace('\'', "\\'");
        query.push_str(&format!(" and name = '{name}'"));
    }
    query.push_str(" and trashed = false");
    step("GET", FILES, reply)
        .query("q", Some(cloud_urlenc(&query)))
        .query("pageToken", token.map(cloud_urlenc))
}

pub(super) fn identity(path: &'static str, object: &Value) -> Step {
    step("GET", path, Reply::Json(object.clone()))
        .query("fields", Some(super::super::identity::OBJECT_FIELDS.into()))
}

struct ScanOutcome {
    entries: Vec<FileEntry>,
    failed: Vec<(String, String)>,
    errors: Vec<String>,
    progress: ScanProgress,
    truncated: bool,
    cancelled: bool,
}

fn scan(fixture: &Fixture) -> ScanOutcome {
    let (tx, rx) = crossbeam_channel::unbounded();
    let handle = crate::rscan::start_scan_backend(
        Arc::new(fixture.backend()), "/".into(), None, None, tx,
    );
    let mut entries = Vec::new();
    let mut failed = Vec::new();
    let mut errors = Vec::new();
    let progress = loop {
        use crate::scanner::ScanMessage;
        match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            ScanMessage::Entries(batch) => entries.extend(batch),
            ScanMessage::FailedPaths(paths) => failed.extend(paths),
            ScanMessage::Error(error) => errors.push(error),
            ScanMessage::DeniedRoots(roots) => {
                let warning = roots.storage_warning();
                assert!(warning.is_none(), "denied-root ledger failed: {warning:?}");
                let paths = roots.roots().expect("read lossless denied-root ledger");
                assert!(paths.is_empty(), "scan omitted roots: {paths:?}");
            }
            ScanMessage::Done(progress) => break progress,
            ScanMessage::Progress(_) => {}
        }
    };
    let cancelled = handle.cancel.load(Ordering::Relaxed);
    handle.cancel.store(true, Ordering::Relaxed);
    ScanOutcome {
        entries, failed, errors, progress, cancelled,
        truncated: handle.truncated.load(Ordering::Relaxed),
    }
}

fn assert_complete(outcome: &ScanOutcome) {
    assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    assert_eq!(outcome.progress.errors, 0);
    assert_eq!(outcome.progress.permission_denied, 0);
    assert!(!outcome.truncated);
    assert!(!outcome.cancelled);
}

pub(super) fn empty_intermediate_pages_continue_and_empty_children_scan() {
    let folder = child(TITLE, "empty-id", ROOT_ID, true);
    let fixture = Fixture::new(vec![
        root(),
        listing(ROOT_ID, None, None, Reply::Json(json!({"nextPageToken": "second"}))),
        listing(ROOT_ID, Some("second"), None,
            Reply::Json(json!({"files": null, "nextPageToken": "third"}))),
        listing(ROOT_ID, Some("third"), None,
            Reply::Json(json!({"files": [], "nextPageToken": "fourth"}))),
        listing(ROOT_ID, Some("fourth"), None,
            Reply::Json(json!({"files": [folder.clone()]}))),
        identity(EMPTY, &folder),
        listing("empty-id", None, None, Reply::Json(json!({"files": null}))),
        identity(EMPTY, &folder),
    ]);
    let outcome = scan(&fixture);
    assert_complete(&outcome);
    assert_eq!(outcome.progress.scanned, 1);
    assert_eq!(outcome.entries.len(), 2);
    let folder = outcome.entries.iter()
        .find(|entry| entry.id.as_deref() == Some("empty-id")).unwrap();
    let encoded = names::encode(TITLE);
    assert_eq!(folder.path.as_ref(), format!("/{encoded}"));
    assert_eq!(folder.name.as_ref(), encoded);
    assert_eq!(names::decode(&folder.name).unwrap(), TITLE);
    assert!(folder.is_dir);
    assert_eq!(fixture.finish().len(), 8);

    for resolve in [false, true] {
        let fixture = Fixture::new(vec![
            root(),
            listing(ROOT_ID, None, Some("I/O"),
                Reply::Json(json!({"files": null, "nextPageToken": "next"}))),
            listing(ROOT_ID, Some("next"), Some("I/O"), Reply::Json(json!({
                "files": [child("I/O", "item-id", ROOT_ID, false)]
            }))),
        ]);
        let backend = fixture.backend();
        if resolve {
            assert_eq!(backend.find_child("root", "I%2FO").unwrap().as_deref(), Some("item-id"));
        } else {
            let objects = backend.named_objects("root", "I/O").unwrap();
            assert_eq!(objects.len(), 1);
            assert_eq!(objects[0].id, "item-id");
        }
        assert_eq!(fixture.finish().len(), 3);
    }
}

pub(super) fn paginated_scan_preserves_literal_file_names_and_ids() {
    let cases = [
        ("alpha-id", "I/O", "I%2FO".to_string()),
        ("beta-id", "I/O", "I%2FO [drive-id beta-id]".to_string()),
        ("literal-id", "I%2FO", "I%252FO".to_string()),
        ("marker-id", "I/O [drive-id beta-id]", "I%2FO %5Bdrive-id beta-id]".to_string()),
        ("unicode-id", TITLE, names::encode(TITLE)),
        ("backslash-id", "Messung\\2026.txt", "Messung%5C2026.txt".to_string()),
    ];
    let files: Vec<_> = cases.iter()
        .map(|(id, title, _)| child(title, id, ROOT_ID, false)).collect();
    let fixture = Fixture::new(vec![
        root(),
        listing(ROOT_ID, None, None,
            Reply::Json(json!({"files": &files[..1], "nextPageToken": "empty"}))),
        listing(ROOT_ID, Some("empty"), None,
            Reply::Json(json!({"files": null, "nextPageToken": "last"}))),
        listing(ROOT_ID, Some("last"), None,
            Reply::Json(json!({"files": &files[1..]}))),
    ]);
    let outcome = scan(&fixture);
    assert_complete(&outcome);
    assert_eq!(outcome.progress.scanned, cases.len() as u64);
    assert_eq!(outcome.entries.len(), cases.len() + 1);
    for (id, title, name) in cases {
        let entries: Vec<_> = outcome.entries.iter()
            .filter(|entry| entry.id.as_deref() == Some(id)).collect();
        assert_eq!(entries.len(), 1, "lost or duplicated Drive ID {id}");
        let entry = entries[0];
        assert_eq!(entry.name.as_ref(), name);
        assert_eq!(entry.path.as_ref(), format!("/{name}"));
        assert_eq!(entry.parent.as_ref(), "/");
        assert!(!entry.is_dir);
        let plain = if id == "beta-id" {
            let (plain, prefix) = duplicates::parse_marker(&entry.name).unwrap();
            assert_eq!(prefix, id);
            plain
        } else {
            &entry.name
        };
        assert_eq!(names::decode(plain).unwrap(), title);
    }
    assert_eq!(fixture.finish().len(), 4);
}

fn denied() -> Reply {
    Reply::HttpError(403, json!({"error": {
        "code": 403, "message": "fixture permission denied",
        "errors": [{"reason": "insufficientFilePermissions"}]
    }}))
}

pub(super) fn paginated_scan_reports_provider_and_schema_failures() {
    for (root_fault, reply, detail) in [
        (true, denied(), "HTTP 403"),
        (false, denied(), "HTTP 403"),
        (false, Reply::Json(json!({"files": false, "nextPageToken": "unused"})),
            "files is not an array"),
    ] {
        let steps = if root_fault {
            vec![step("GET", ROOT, reply).query("fields", Some("id".into()))]
        } else {
            vec![
                root(),
                listing(ROOT_ID, None, None, Reply::Json(json!({
                    "files": [child("Unconfirmed.txt", "partial-id", ROOT_ID, false)],
                    "nextPageToken": "next"
                }))),
                listing(ROOT_ID, Some("next"), None, reply),
            ]
        };
        let fixture = Fixture::new(steps);
        let outcome = scan(&fixture);
        assert_eq!(outcome.progress.errors, 1);
        assert_eq!(outcome.progress.scanned, 0);
        assert_eq!(outcome.entries.len(), 1);
        assert_eq!(outcome.entries[0].path.as_ref(), "/");
        assert!(outcome.entries.iter().all(|entry| entry.id.is_none()));
        assert_eq!(outcome.failed.len(), 1);
        assert_eq!(outcome.failed[0].0, "/");
        assert!(outcome.failed[0].1.contains(detail), "{:?}", outcome.failed);
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert!(!outcome.truncated);
        assert!(!outcome.cancelled);
        assert_eq!(fixture.finish().len(), if root_fault { 1 } else { 3 });
    }
}
