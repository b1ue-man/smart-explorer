use super::{duplicates, names, gui_task_http::{step, Fixture, Reply, Request}};
use crate::vfs::{Backend, VfsMeta};
use serde_json::{json, Value};
use std::io::{Read, Write};

const TITLE: &str = "Entwicklung einer HMI-basierten I/O-Diagnoseoberfläche – Universelle Lösung auf Basis einer funktionalen Gruppenstruktur";
const FILES: &str = "/drive/v3/files";
const ITEM: &str = "/drive/v3/files/item-id";
const GENERATE: &str = "/drive/v3/files/generateIds";
const UPLOAD: &str = "/upload/drive/v3/files";

fn object(name: &str, id: &str, folder: bool) -> Value {
    json!({"id": id, "name": name, "parents": ["root"], "trashed": false,
        "mimeType": if folder { super::api::FOLDER_MIME } else { "application/octet-stream" },
        "size": "3", "md5Checksum": "900150983cd24fb0d6963f7d28e17f72",
        "modifiedTime": "2026-09-15T11:41:38Z"})
}

fn assert_query(request: &Request, parent: &str, name: &str) {
    let quote = |value: &str| value.replace('\\', "\\\\").replace('\'', "\\'");
    let query = format!("'{}' in parents and name = '{}' and trashed = false", quote(parent), quote(name));
    assert!(request.target.contains(&format!("q={}", super::core::cloud_urlenc(&query))), "{:?}", request);
}

#[test]
fn gui_design_task_drive_names_are_safe_reversible_and_collision_free() {
    let titles = [TITLE, "I%2FO", "I/O", "I\\O", ".", "..", "a\0b", "a\nb", "NUL.txt",
        "CON", "LPT¹", "f.", " f ", "a:b?c*d|e<f>", "a [drive-id abc]", "100%", "Projekte – älter"];
    let mut unique = std::collections::HashSet::new();
    for title in titles {
        let name = names::encode(title);
        crate::vfs::validate_child_name(&name).unwrap();
        assert_eq!(names::decode(&name).unwrap(), title);
        assert!(unique.insert(name));
    }
    assert_eq!(names::encode(TITLE), TITLE.replace('/', "%2F"));
    assert!(names::decode("../escape").is_err());
    assert!(names::decode("%xx").is_err());
    let entries = ["abcdefgh1", "abcdefgh2", "abcdefgh3"].into_iter().enumerate().map(|(i, id)| VfsMeta {
        name: "I/O".into(), id: Some(id.into()), mtime_ms: i as i64, ..Default::default()
    }).collect();
    let listed = duplicates::disambiguate(entries);
    assert_eq!(listed[0].name, "I%2FO");
    assert_eq!(listed[1].name, "I%2FO [drive-id abcdefgh2]");
    assert_eq!(listed[2].name, "I%2FO [drive-id abcdefgh1]");
}

#[test]
fn gui_design_task_drive_recursive_scan_stat_read_and_trash_use_exact_ids() {
    let child = object("Messung\\2026.txt", "item-id", false);
    let fixture = Fixture::new(vec![
        step("GET", FILES, Reply::Json(json!({"files": [object(TITLE, "folder-id", true)]}))),
        step("GET", FILES, Reply::Json(json!({"files": [child.clone()]}))),
        step("GET", ITEM, Reply::Json(child)),
        step("GET", ITEM, Reply::Bytes("abc".into())),
        step("PATCH", ITEM, Reply::Json(json!({"id": "item-id", "trashed": true}))),
    ]);
    let backend = fixture.backend();
    let (tx, rx) = crossbeam_channel::unbounded();
    let handle = crate::rscan::start_scan_backend(std::sync::Arc::new(backend.clone()), "/".into(), None, tx);
    let mut entries = Vec::new();
    loop {
        use crate::scanner::ScanMessage;
        match rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap() {
            ScanMessage::Entries(batch) => entries.extend(batch),
            ScanMessage::Error(error) => panic!("{error}"),
            ScanMessage::FailedPaths(paths) => assert!(paths.is_empty(), "{paths:?}"),
            ScanMessage::Done(progress) => { assert_eq!(progress.errors, 0); break; }
            ScanMessage::Progress(_) => {}
        }
    }
    handle.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    let path = format!("/{}/Messung%5C2026.txt", names::encode(TITLE));
    assert!(entries.iter().any(|entry| entry.path.as_ref() == path && entry.id.as_deref() == Some("item-id")));
    assert_eq!(backend.stat(&path).unwrap().name, "Messung%5C2026.txt");
    let mut bytes = Vec::new();
    backend.open_read(&path).unwrap().read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"abc");
    backend.remove_file(&path).unwrap();
    let requests = fixture.finish();
    assert!(requests[1].target.contains(&super::core::cloud_urlenc("'folder-id' in parents")));
    assert_eq!(serde_json::from_slice::<Value>(&requests[4].body).unwrap(), json!({"trashed": true}));
}

#[test]
fn gui_design_task_drive_uncached_paged_markers_and_reloaded_cache() {
    let raw = "I/O\\Gerät's Daten";
    let first = object(raw, "abcdefgh1", false);
    let second = object(raw, "abcdefgh2", false);
    let fixture = Fixture::new(vec![
        step("GET", FILES, Reply::Json(json!({"files": [first], "nextPageToken": "next"}))),
        step("GET", FILES, Reply::Json(json!({"files": [second.clone()]}))),
        step("GET", "/drive/v3/files/abcdefgh2", Reply::Json(second)),
        step("GET", FILES, Reply::Json(json!({"files": [object("literal [drive-id abc]", "literal-id", false)]}))),
    ]);
    let backend = fixture.backend();
    let path = format!("{} [drive-id abcdefgh2]", names::encode(raw));
    assert_eq!(backend.resolve(&path).unwrap(), "abcdefgh2");
    backend.untrusted_guard().unwrap().insert(path.clone());
    assert_eq!(backend.resolve(&path).unwrap(), "abcdefgh2");
    assert_eq!(backend.resolve(&names::encode("literal [drive-id abc]")).unwrap(), "literal-id");
    assert!(backend.state_identity().starts_with("gdrive:path-v2:"));
    let requests = fixture.finish();
    assert_query(&requests[0], "root", raw);
    assert!(requests[1].target.contains("pageToken=next"));
    assert_query(&requests[3], "root", "literal [drive-id abc]");
}

#[test]
fn gui_design_task_drive_repeated_or_incomplete_pages_fail_closed() {
    for incomplete in [false, true] {
        let body = json!({"files": [], "nextPageToken": "repeat", "incompleteSearch": incomplete});
        let mut steps = vec![step("GET", FILES, Reply::Json(body.clone()))];
        if !incomplete { steps.push(step("GET", FILES, Reply::Json(body))); }
        let fixture = Fixture::new(steps);
        assert!(fixture.backend().find_child("root", "I%2FO").is_err());
        fixture.finish();
    }
}

#[test]
fn gui_design_task_drive_folder_and_file_creation_decode_metadata_once() {
    let fixture = Fixture::new(vec![
        step("GET", GENERATE, Reply::Json(json!({"ids": ["folder-id"]}))),
        step("POST", FILES, Reply::Json(json!({"id": "folder-id"}))),
        step("GET", FILES, Reply::Json(json!({"files": []}))),
        step("GET", GENERATE, Reply::Json(json!({"ids": ["item-id"]}))),
        step("POST", UPLOAD, Reply::Session),
        step("PUT", "/session", Reply::Json(json!({"id": "item-id"}))),
    ]);
    let backend = fixture.backend();
    backend.listed_guard().unwrap().insert(String::new());
    backend.mkdir_all("I%2FO").unwrap();
    let mut writer = backend.open_write("I%2FO/Daten%252F.csv").unwrap();
    writer.write_all(b"abc").unwrap();
    writer.flush().unwrap();
    let requests = fixture.finish();
    assert_eq!(serde_json::from_slice::<Value>(&requests[1].body).unwrap()["name"], "I/O");
    assert_query(&requests[2], "folder-id", "Daten%2F.csv");
    let metadata: Value = serde_json::from_slice(&requests[4].body).unwrap();
    assert_eq!(metadata["name"], "Daten%2F.csv");
    assert_eq!(metadata["parents"], json!(["folder-id"]));
}

#[test]
fn gui_design_task_drive_content_replacement_never_renames_an_alias() {
    let fixture = Fixture::new(vec![
        step("PATCH", "/upload/drive/v3/files/item-id", Reply::Session),
        step("PUT", "/session", Reply::Json(json!({"id": "item-id"}))),
    ]);
    let backend = fixture.backend();
    let path = "I%2FO [drive-id item-id]";
    backend.remember_path(path, "item-id", None).unwrap();
    let mut writer = backend.open_write(path).unwrap();
    writer.write_all(b"abc").unwrap();
    writer.flush().unwrap();
    let requests = fixture.finish();
    assert_eq!(serde_json::from_slice::<Value>(&requests[0].body).unwrap(), json!({}));
}

#[test]
fn gui_design_task_drive_rename_and_copy_promotion_preserve_original_titles() {
    let fixture = Fixture::new(vec![
        step("GET", FILES, Reply::Json(json!({"files": [object("I/O", "item-id", false)]}))),
        step("GET", FILES, Reply::Json(json!({"files": []}))),
        step("PATCH", ITEM, Reply::Json(json!({"id": "item-id"}))),
        step("GET", FILES, Reply::Json(json!({"files": [object("I/O neu", "item-id", false)]}))),
    ]);
    let backend = fixture.backend();
    backend.rename("I%2FO", "I%2FO neu").unwrap();
    let requests = fixture.finish();
    assert_query(&requests[0], "root", "I/O");
    assert_eq!(serde_json::from_slice::<Value>(&requests[2].body).unwrap()["name"], "I/O neu");

    let stage = object("I/O.stage", "item-id", false);
    let fixture = Fixture::new(vec![
        step("GET", FILES, Reply::Json(json!({"files": []}))),
        step("GET", GENERATE, Reply::Json(json!({"ids": ["item-id"]}))),
        step("POST", UPLOAD, Reply::Session),
        step("PUT", "/session", Reply::Json(json!({"id": "item-id"}))),
        step("GET", ITEM, Reply::Json(stage.clone())),
        step("GET", FILES, Reply::Json(json!({"files": [stage.clone()]}))),
        step("GET", FILES, Reply::Json(json!({"files": [stage]}))),
        step("GET", FILES, Reply::Json(json!({"files": []}))),
        step("PATCH", ITEM, Reply::Json(json!({"id": "item-id"}))),
        step("GET", FILES, Reply::Json(json!({"files": [object("I/O.txt", "item-id", false)]}))),
    ]);
    let backend = fixture.backend();
    let mut writer = backend.open_write_copy_stage("I%2FO.stage").unwrap();
    writer.write_all(b"abc").unwrap();
    writer.flush().unwrap();
    backend.promote_copy_stage("I%2FO.stage", "I%2FO.txt").unwrap();
    let requests = fixture.finish();
    assert_eq!(serde_json::from_slice::<Value>(&requests[2].body).unwrap()["name"], "I/O.stage");
    assert_eq!(serde_json::from_slice::<Value>(&requests[8].body).unwrap()["name"], "I/O.txt");
}
