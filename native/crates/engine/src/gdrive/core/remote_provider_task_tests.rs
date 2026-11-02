//! RV1 provider cases for the single remote task suite. No OAuth or live
//! service credentials are needed; requests use the existing Drive fixture.
use super::api::FOLDER_MIME;
use super::task_drive::{drive_server, FakeDrive};
use super::task_http::{Answer, Server};
use super::GDriveBackend;
use crate::vfs::{self, Backend, BackendExtensions, ChangeKind, StageDurability, StageFinish};
use serde_json::json;
use std::io::{Read, Write};
use std::sync::Arc;

#[test]
fn rv1_remote_provider_task_drive_identity_survives_token_rotation() {
    let mut backend = GDriveBackend::test_backend("http://127.0.0.1:9/drive/v3");
    backend.root = "a%3Ab".into();
    let stable = backend.state_identity();
    let former = vfs::previous_state_identities(&backend).unwrap();
    backend.tokens_guard().unwrap().refresh_token = "rotated-token".into();
    assert_eq!(backend.state_identity(), stable);
    let current = vfs::previous_state_identities(&backend).unwrap();
    assert_ne!(current, former);
    assert_eq!(current, vec![backend.legacy_state_identity().unwrap()]);
    let mut other = backend.clone();
    other.drive_account_key = Arc::from("another-account");
    assert_ne!(other.state_identity(), stable);
    other = backend.clone();
    other.root = " tail ".into();
    assert!(vfs::previous_state_identities(&other).unwrap().is_empty());
}

#[test]
fn rv1_remote_provider_task_drive_literal_names_and_omissions_keep_exact_ids() {
    let drive = Arc::new(FakeDrive::default());
    for (id, name) in [
        ("percent", "%3A"),
        ("reserved", "CON"),
        ("space", " title "),
    ] {
        drive.insert(id, name, "root", "application/octet-stream", b"ok");
    }
    drive.insert(
        "native",
        "Notes",
        "root",
        "application/vnd.google-apps.document",
        b"",
    );
    drive.insert(
        "shortcut",
        "Link",
        "root",
        "application/vnd.google-apps.shortcut",
        b"",
    );
    for id in ["folder-one", "folder-two"] {
        drive.insert(id, "Ambiguous", "root", FOLDER_MIME, b"");
    }
    drive.insert(
        "bad",
        "not/a-component",
        "root",
        "application/octet-stream",
        b"keep",
    );
    let server = {
        let drive = Arc::clone(&drive);
        Server::start(move |request| drive.answer(request))
    };
    let backend = server.backend();
    let listing = backend.list_dir_tolerant("").unwrap();
    for name in ["%3A", "CON", " title "] {
        let entry = listing
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap();
        let path = vfs::sync_child_path(&backend, "", name).unwrap();
        let mut content = String::new();
        let mut reader = backend
            .open_read_regular(&path, entry.id.as_deref())
            .unwrap();
        reader.read_to_string(&mut content).unwrap();
        assert_eq!(content, "ok");
    }
    assert_eq!(vfs::sync_child_path(&backend, "", "%3A").unwrap(), "/%253A");
    assert!(
        listing
            .entries
            .iter()
            .find(|entry| entry.name == "Notes")
            .unwrap()
            .special
    );
    assert!(
        listing
            .entries
            .iter()
            .find(|entry| entry.name == "Link")
            .unwrap()
            .is_symlink
    );
    assert_eq!(
        listing
            .entries
            .iter()
            .filter(|item| item.is_dir
                && item
                    .id
                    .as_deref()
                    .is_some_and(|id| id.starts_with("folder-")))
            .count(),
        2
    );
    assert!(listing
        .omitted
        .iter()
        .any(|item| item.rel == "not/a-component"));
    assert!(backend.open_read_regular("Notes", Some("native")).is_err());
    assert!(backend.open_read_regular("Link", Some("shortcut")).is_err());
    assert_eq!(drive.named("root", "Ambiguous").len(), 2);
}

#[test]
fn rv1_remote_provider_task_drive_time_survives_selected_id_replacement() {
    let (drive, server) = drive_server();
    drive.insert(
        "original",
        "dest.bin",
        "root",
        "application/octet-stream",
        b"old",
    );
    let backend = server.backend();
    let stage = "dest.bin.se-copy-0123456789abcdef";
    let time = 1_704_110_400_123;
    let mut writer = backend.open_write_copy_stage_timed(stage, 3, time).unwrap();
    writer.write_all(b"new").unwrap();
    writer.flush().unwrap();
    drop(writer);
    let finished = backend
        .finish_stage(
            stage,
            StageFinish {
                mtime_ms: Some(time),
                mode: None,
                durability: StageDurability::Now,
            },
        )
        .unwrap();
    assert!(finished.mtime_applied);
    assert!(!finished.durable);
    backend
        .promote_staged_to_id(stage, "dest.bin", Some("original"))
        .unwrap();
    let live = drive.named("root", "dest.bin");
    assert_eq!(live.len(), 1);
    assert_eq!(live[0]["id"], "original");
    assert_eq!(
        live[0]["md5Checksum"],
        format!("{:x}", md5::compute(b"new"))
    );
    assert_eq!(backend.stat("dest.bin").unwrap().mtime_ms, time);
}

#[test]
fn rv1_remote_provider_task_drive_feed_requires_complete_pages_and_keeps_removed_ids() {
    let server = Server::start(|request| match request.query("pageToken").as_deref() {
        Some("first") => Answer::json(json!({"nextPageToken": "second", "changes": [{
            "fileId": "outside-id", "removed": false, "file": {
                "id": "outside-id", "name": "%3A", "parents": ["outside-root"],
                "mimeType": "application/octet-stream", "size": "2",
                "modifiedTime": "2024-01-01T00:00:00.123Z", "trashed": false
            }
        }]})),
        Some("second") => Answer::json(json!({"newStartPageToken": "final", "changes": [{
            "fileId": "removed-id", "removed": true
        }]})),
        _ => Answer::status(400, json!({"error": {"message": "unexpected cursor"}})),
    });
    let batch = server
        .backend()
        .changes_since("selected-root", "first")
        .unwrap();
    assert_eq!(batch.new_cursor.as_deref(), Some("final"));
    assert!(!batch.reset);
    assert_eq!(batch.changes[0].parent_id.as_deref(), Some("outside-root"));
    assert_eq!(batch.changes[0].name.as_deref(), Some("%3A"));
    assert!(batch.changes[0].rel.is_none());
    assert_eq!(batch.changes[1].kind, ChangeKind::Remove);
    assert_eq!(batch.changes[1].id.as_deref(), Some("removed-id"));
    assert!(batch.changes[1].meta.is_none());
    for response in [
        json!({"nextPageToken": "first", "changes": []}),
        json!({"changes": []}),
        json!({"newStartPageToken": "final", "changes": [{"fileId": "id", "removed": "true"}]}),
    ] {
        let server = Server::start(move |_| Answer::json(response.clone()));
        assert_eq!(
            server
                .backend()
                .changes_since("selected-root", "first")
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidData
        );
    }
}
