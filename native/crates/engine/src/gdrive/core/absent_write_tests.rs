//! G3-08b task tests against the loopback Drive: a new file in an existing
//! folder takes one write request (`open_write_absent`), and a name another
//! client takes at the same moment is never shared: the own object goes to
//! the trash by its exact ID and the writer reports `AlreadyExists`.
use super::api::FOLDER_MIME;
use super::task_drive::{count, drive_server, multipart, FakeDrive};
use super::task_http::{Request, Server};
use crate::vfs::Backend;
use std::io::{self, Write};

fn write_absent(backend: &dyn Backend, path: &str, content: &[u8]) -> io::Result<()> {
    let mut writer = crate::vfs::open_write_absent(backend, path, content.len() as u64)?
        .ok_or_else(|| io::Error::other("Drive must offer the absent writer"))?;
    writer.write_all(content)?;
    writer.flush()
}

/// The existing folder "Ziel" (ID `target`) the new files go into. A
/// remembered folder hint is checked by its exact ID before use
/// (resolution.rs `valid_cached_id`, since 1bee4b6a), so it must exist.
fn with_target(drive: &FakeDrive) {
    drive.insert("target", "Ziel", "root", FOLDER_MIME, b"");
}

fn writes(requests: &[Request]) -> Vec<&Request> {
    requests
        .iter()
        .filter(|request| request.method != "GET")
        .collect()
}

#[test]
fn speed_task_drive_absent_file_takes_one_write_request() {
    let (drive, server) = drive_server();
    with_target(&drive);
    let backend = server.backend();
    backend
        .remember_path("Ziel", "target", Some(FOLDER_MIME))
        .unwrap();
    write_absent(&backend, "Ziel/neu.txt", b"abc").unwrap();
    let requests = server.requests();
    let writes = writes(&requests);
    // One create under the final name; no stage, no rename, no trash.
    assert_eq!(writes.len(), 1, "one write request per new file");
    assert_eq!(writes[0].method, "POST");
    assert_eq!(writes[0].path(), "/upload/drive/v3/files");
    let (metadata, media) = multipart(writes[0]);
    assert_eq!(metadata["name"], "neu.txt");
    assert_eq!(media, b"abc");
    let named = drive.named("target", "neu.txt");
    assert_eq!(named.len(), 1);
    assert_eq!(named[0]["id"], metadata["id"]);
    // The name was listed to prove it leads to the new ID alone.
    assert!(count(&requests, "GET", "/drive/v3/files") >= 1);
    let id = metadata["id"].as_str().unwrap();
    let cached = backend.cached_id("Ziel/neu.txt").unwrap();
    assert_eq!(cached.as_deref(), Some(id));
    assert!(backend.pending_upload_ids_guard().unwrap().is_empty());

    // A writer dropped before its flush creates nothing.
    let writer = crate::vfs::open_write_absent(&backend, "Ziel/nie.txt", 3)
        .unwrap()
        .unwrap();
    drop(writer);
    assert!(drive.named("target", "nie.txt").is_empty());
    assert_eq!(writes_of(&server), 1, "an unflushed writer sends no create");
}

fn writes_of(server: &Server) -> usize {
    writes(&server.requests()).len()
}

#[test]
fn speed_task_drive_absent_taken_before_create_is_never_shared() {
    // The caller's "absent" was stale: the name already exists. The create
    // still happens (no probe), the proof sees two objects and withdraws ours.
    let (drive, server) = drive_server();
    with_target(&drive);
    drive.insert("old-id", "alt.txt", "target", "text/plain", b"old");
    let backend = server.backend();
    backend
        .remember_path("Ziel", "target", Some(FOLDER_MIME))
        .unwrap();
    let error = write_absent(&backend, "Ziel/alt.txt", b"new").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    let named = drive.named("target", "alt.txt");
    assert_eq!(named.len(), 1);
    assert_eq!(named[0]["id"], "old-id");
    assert_eq!(drive.bytes("old-id"), b"old");
}
