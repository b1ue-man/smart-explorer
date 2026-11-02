use super::{
    duplicates,
    gui_task_fixture::{assert_query, object},
    gui_task_http::{step, Fixture, Reply},
    names,
};
use crate::vfs::{Backend, VfsMeta};
use serde_json::{json, Value};
use std::io::{Read, Write};
use self::scan_task_tests::{child as drive_object, identity, listing, root, ROOT_ID};

#[path = "scan_task_tests.rs"]
mod scan_task_tests;
#[path = "paging_boundary_task_tests.rs"]
mod paging_boundary_task_tests;

const TITLE: &str = "Entwicklung einer HMI-basierten I/O-Diagnoseoberfläche – Universelle Lösung auf Basis einer funktionalen Gruppenstruktur";
const FILES: &str = "/drive/v3/files";
const ITEM: &str = "/drive/v3/files/item-id";
const GENERATE: &str = "/drive/v3/files/generateIds";
const UPLOAD: &str = "/upload/drive/v3/files";

#[test]
fn gui_design_task_drive_names_are_safe_reversible_and_collision_free() {
    let titles = [
        TITLE,
        "I%2FO",
        "I/O",
        "I\\O",
        ".",
        "..",
        "a\0b",
        "a\nb",
        "NUL.txt",
        "CON",
        "LPT¹",
        "f.",
        " f ",
        "a:b?c*d|e<f>",
        "a [drive-id abc]",
        "100%",
        "Projekte – älter",
    ];
    let mut unique = std::collections::HashSet::new();
    for title in titles {
        let name = names::encode(title);
        crate::vfs::validate_child_name(&name).unwrap();
        assert_eq!(names::decode(&name).unwrap(), title);
        assert!(unique.insert(name));
    }
    assert_eq!(names::encode(TITLE), TITLE.replace('/', "%2F"));
    assert!(names::decode("../escape").is_err());
    assert_eq!(names::decode("%xx").unwrap(), "%xx");
    let entries = ["abcdefgh1", "abcdefgh2", "abcdefgh3"]
        .into_iter()
        .enumerate()
        .map(|(i, id)| VfsMeta {
            name: "I/O".into(),
            id: Some(id.into()),
            mtime_ms: i as i64,
            ..Default::default()
        })
        .collect();
    let listed = duplicates::disambiguate(entries);
    assert_eq!(listed[0].name, "I%2FO");
    assert_eq!(listed[1].name, "I%2FO [drive-id abcdefgh2]");
    assert_eq!(listed[2].name, "I%2FO [drive-id abcdefgh1]");
}

#[test]
fn gui_design_task_drive_recursive_scan_stat_read_and_trash_use_exact_ids() {
    const FOLDER: &str = "/drive/v3/files/folder-id";
    let folder = drive_object(TITLE, "folder-id", ROOT_ID, true);
    let child = drive_object("Messung\\2026.txt", "item-id", "folder-id", false);
    let mut steps = vec![
        root(),
        listing(ROOT_ID, None, None, Reply::Json(json!({"files": [folder.clone()]}))),
        identity(FOLDER, &folder),
        listing("folder-id", None, None, Reply::Json(json!({"files": [child.clone()]}))),
        identity(FOLDER, &folder),
    ];
    // stat resolves both path segments, then independently resolves its parent.
    steps.extend((0..3).map(|_| identity(FOLDER, &folder)));
    steps.push(identity(ITEM, &child));
    // open_read retains the exact file ID and its cached binary MIME type.
    steps.extend((0..2).map(|_| identity(FOLDER, &folder)));
    steps.push(step("GET", ITEM, Reply::Bytes("abc".into())).query("alt", Some("media".into())));
    // Trash freshly validates the parent and exact named object before PATCH.
    steps.extend((0..3).map(|_| identity(FOLDER, &folder)));
    steps.push(step("GET", ITEM, Reply::Json(child)).query("fields",
        Some("id,name,mimeType,size,md5Checksum,modifiedTime,parents,trashed".into())));
    steps.push(step("PATCH", ITEM, Reply::Json(json!({"id": "item-id", "trashed": true})))
        .query("fields", Some("id,trashed".into())));
    let fixture = Fixture::new(steps);
    let backend = fixture.backend();
    let (tx, rx) = crossbeam_channel::unbounded();
    let handle = crate::rscan::start_scan_backend(
        std::sync::Arc::new(backend.clone()),
        "/".into(),
        None,
        None,
        tx,
    );
    let mut entries = Vec::new();
    loop {
        use crate::scanner::ScanMessage;
        match rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap() {
            ScanMessage::Entries(batch) => entries.extend(batch),
            ScanMessage::Error(error) => panic!("{error}"),
            ScanMessage::FailedPaths(paths) => assert!(paths.is_empty(), "{paths:?}"),
            ScanMessage::Done(progress) => {
                assert_eq!(progress.errors, 0);
                break;
            }
            ScanMessage::Progress(_) | ScanMessage::DeniedRoots(_) => {}
        }
    }
    handle
        .cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let path = format!("/{}/Messung%5C2026.txt", names::encode(TITLE));
    assert!(entries
        .iter()
        .any(|entry| entry.path.as_ref() == path && entry.id.as_deref() == Some("item-id")));
    assert_eq!(backend.stat(&path).unwrap().name, "Messung%5C2026.txt");
    let mut bytes = Vec::new();
    backend
        .open_read(&path)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, b"abc");
    backend.remove_file(&path).unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 17);
    assert!(requests[3]
        .target
        .contains(&super::core::cloud_urlenc("'folder-id' in parents")));
    assert_eq!(
        serde_json::from_slice::<Value>(&requests.last().unwrap().body).unwrap(),
        json!({"trashed": true})
    );

    // Google-native documents have the same title mapping, but use export.
    let mut document = drive_object(TITLE, "item-id", ROOT_ID, false);
    document["mimeType"] = json!("application/vnd.google-apps.document");
    let fixture = Fixture::new(vec![
        root(),
        listing(ROOT_ID, None, None, Reply::Json(json!({"files": [document]}))),
        step(
            "GET",
            "/drive/v3/files/item-id/export",
            Reply::Bytes("office-data".into()),
        ),
    ]);
    let backend = fixture.backend();
    let listed = backend.list_dir("/").unwrap();
    let path = format!("/{}", listed[0].name);
    assert_eq!(
        crate::connect::gdrive_endpoint(&path),
        format!("gdrive://{path}")
    );
    assert_eq!(
        backend.download_name(&path, &listed[0].name),
        format!("{}.docx", names::encode(TITLE))
    );
    let mut bytes = Vec::new();
    backend
        .open_read(&path)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, b"office-data");
    let requests = fixture.finish();
    assert_eq!(requests.len(), 3);
    assert!(requests[2].target.contains(
        "mimeType=application%2Fvnd.openxmlformats-officedocument.wordprocessingml.document"
    ));
}

#[test]
fn gui_design_task_drive_uncached_paged_markers_and_reloaded_cache() {
    paging_boundary_task_tests::uncached_paged_markers_and_reloaded_cache();
}

#[test]
fn gui_design_task_drive_repeated_or_incomplete_pages_fail_closed() {
    paging_boundary_task_tests::repeated_or_incomplete_pages_fail_closed();
}

#[test]
fn gui_design_task_drive_folder_and_file_creation_decode_metadata_once() {
    let folder = drive_object("I/O", "folder-id", ROOT_ID, true);
    let mut ids: Vec<String> = (0..8).map(|index| format!("unused-id-{index}")).collect();
    ids.extend(["item-id".to_string(), "folder-id".to_string()]);
    let fixture = Fixture::new(vec![
        root(),
        listing(ROOT_ID, None, Some("I/O"), Reply::Json(json!({"files": []}))),
        step("GET", GENERATE, Reply::Json(json!({"ids": ids})))
            .query("count", Some("10".into()))
            .query("space", Some("drive".into()))
            .query("type", Some("files".into())),
        step("POST", FILES, Reply::Json(json!({"id": "folder-id"}))),
        identity("/drive/v3/files/folder-id", &folder),
        listing("folder-id", None, Some("Daten%2F.csv"), Reply::Json(json!({"files": []}))),
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
    assert_eq!(requests.len(), 8);
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[3].body).unwrap()["name"],
        "I/O"
    );
    let folder_metadata: Value = serde_json::from_slice(&requests[3].body).unwrap();
    assert_eq!(folder_metadata["parents"], json!([ROOT_ID]));
    assert_eq!(folder_metadata["id"], "folder-id");
    assert_query(&requests[5], "folder-id", "Daten%2F.csv");
    let metadata: Value = serde_json::from_slice(&requests[6].body).unwrap();
    assert_eq!(metadata["name"], "Daten%2F.csv");
    assert_eq!(metadata["parents"], json!(["folder-id"]));
    assert_eq!(metadata["id"], "item-id");
}

#[test]
fn gui_design_task_drive_content_replacement_never_renames_an_alias() {
    let fixture = Fixture::new(vec![
        root(),
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
    assert_eq!(requests.len(), 3);
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[1].body).unwrap(),
        json!({})
    );
}

#[test]
fn gui_design_task_drive_rename_and_copy_promotion_preserve_original_titles() {
    let fixture = Fixture::new(vec![
        root(),
        listing(ROOT_ID, None, Some("I/O"), Reply::Json(json!({
            "files": [drive_object("I/O", "item-id", ROOT_ID, false)]
        }))),
        listing(ROOT_ID, None, Some("I/O neu"), Reply::Json(json!({"files": []}))),
        step("PATCH", ITEM, Reply::Json(json!({"id": "item-id"}))),
        step(
            "GET",
            ITEM,
            Reply::Json(drive_object("I/O neu", "item-id", ROOT_ID, false)),
        ),
        listing(ROOT_ID, None, Some("I/O neu"), Reply::Json(json!({
            "files": [drive_object("I/O neu", "item-id", ROOT_ID, false)]
        }))),
    ]);
    let backend = fixture.backend();
    backend.rename("I%2FO", "I%2FO neu").unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 6);
    assert_query(&requests[1], ROOT_ID, "I/O");
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[3].body).unwrap()["name"],
        "I/O neu"
    );

    let stage = drive_object("I/O.stage", "item-id", ROOT_ID, false);
    let mut ids: Vec<String> = (0..9).map(|index| format!("unused-id-{index}")).collect();
    ids.push("item-id".to_string());
    let fixture = Fixture::new(vec![
        root(),
        listing(ROOT_ID, None, Some("I/O.stage"), Reply::Json(json!({"files": []}))),
        step("GET", GENERATE, Reply::Json(json!({"ids": ids})))
            .query("count", Some("10".into())),
        step("POST", UPLOAD, Reply::Session),
        step("PUT", "/session", Reply::Json(json!({"id": "item-id"}))),
        step("GET", ITEM, Reply::Json(stage.clone())),
        listing(ROOT_ID, None, Some("I/O.stage"), Reply::Json(json!({"files": [stage.clone()]}))),
        listing(ROOT_ID, None, Some("I/O.stage"), Reply::Json(json!({"files": [stage]}))),
        listing(ROOT_ID, None, Some("I/O.txt"), Reply::Json(json!({"files": []}))),
        step("PATCH", ITEM, Reply::Json(json!({"id": "item-id"}))),
        step(
            "GET",
            ITEM,
            Reply::Json(drive_object("I/O.txt", "item-id", ROOT_ID, false)),
        ),
        listing(ROOT_ID, None, Some("I/O.txt"), Reply::Json(json!({
            "files": [drive_object("I/O.txt", "item-id", ROOT_ID, false)]
        }))),
    ]);
    let backend = fixture.backend();
    let mut writer = backend.open_write_copy_stage("I%2FO.stage").unwrap();
    writer.write_all(b"abc").unwrap();
    writer.flush().unwrap();
    backend
        .promote_copy_stage("I%2FO.stage", "I%2FO.txt")
        .unwrap();
    let requests = fixture.finish();
    assert_eq!(requests.len(), 12);
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[3].body).unwrap()["name"],
        "I/O.stage"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[9].body).unwrap()["name"],
        "I/O.txt"
    );
}

#[test]
fn gui_design_task_drive_optional_empty_pages_agree_at_read_and_mutation_boundaries() {
    paging_boundary_task_tests::optional_empty_pages_agree_at_read_and_mutation_boundaries();
}

#[test]
fn gui_design_task_drive_empty_intermediate_pages_continue_and_empty_children_scan() {
    scan_task_tests::empty_intermediate_pages_continue_and_empty_children_scan();
}

#[test]
fn gui_design_task_drive_paginated_scan_preserves_literal_file_names_and_ids() {
    scan_task_tests::paginated_scan_preserves_literal_file_names_and_ids();
}

#[test]
fn gui_design_task_drive_paginated_scan_reports_provider_and_schema_failures() {
    scan_task_tests::paginated_scan_reports_provider_and_schema_failures();
}
