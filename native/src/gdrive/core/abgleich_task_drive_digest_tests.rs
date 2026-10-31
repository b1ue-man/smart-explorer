//! `content_digest` over Drive: Google's `sha256Checksum` of the stored
//! content comes back from one `files.get`, the content is not downloaded
//! (`alt=media`), and files without a stored content answer `Ok(None)`.
use super::api::FOLDER_MIME;
use super::content_digest::from_file;
use super::task_drive::{drive_server, sha256_hex_of};
use crate::vfs::content_digest;
use serde_json::json;
use std::sync::atomic::AtomicBool;

#[test]
fn abgleich_task_drive_digest_is_the_checksum_google_reports_not_a_download() {
    let (drive, server) = drive_server();
    drive.insert("dir", "Dir", "root", FOLDER_MIME, b"");
    drive.insert("f1", "a.bin", "dir", "application/octet-stream", b"abcde");
    let backend = server.backend();
    backend
        .remember_path("Dir", "dir", Some(FOLDER_MIME))
        .unwrap();
    backend
        .remember_path("Dir/a.bin", "f1", Some("application/octet-stream"))
        .unwrap();
    let found = content_digest(&backend, "Dir/a.bin", &AtomicBool::new(false))
        .unwrap()
        .expect("Drive stores a SHA-256");
    assert_eq!(found.sha256, sha256_hex_of(b"abcde"));
    assert_eq!(found.size, 5);
    let media = server
        .requests()
        .iter()
        .filter(|request| request.query("alt").as_deref() == Some("media"))
        .count();
    assert_eq!(media, 0, "the content was not downloaded");
    let asked = server
        .requests()
        .iter()
        .filter(|request| request.path() == "/drive/v3/files/f1")
        .filter_map(|request| request.query("fields"))
        .collect::<Vec<_>>();
    assert_eq!(asked.len(), 1);
    assert!(asked[0].contains("sha256Checksum"), "{asked:?}");
}

#[test]
fn abgleich_task_drive_digest_answers_by_kind_of_object() {
    let file = |extra: serde_json::Value| {
        let mut json =
            json!({"id": "x", "mimeType": "application/pdf", "trashed": false, "size": "7"});
        for (key, value) in extra.as_object().unwrap() {
            json[key] = value.clone();
        }
        json
    };
    let hex = sha256_hex_of(b"1234567");
    let found = from_file(
        &file(json!({"sha256Checksum": hex.to_ascii_uppercase()})),
        "x",
    )
    .unwrap()
    .unwrap();
    assert_eq!((found.sha256.as_str(), found.size), (hex.as_str(), 7));
    assert_eq!(
        from_file(&file(json!({})), "x").unwrap(),
        None,
        "no checksum yet"
    );
    assert_eq!(
        from_file(&file(json!({"sha256Checksum": "abc"})), "x").unwrap(),
        None,
        "no digest from a malformed value"
    );
    assert_eq!(
        from_file(
            &file(
                json!({"mimeType": "application/vnd.google-apps.document", "sha256Checksum": hex})
            ),
            "x"
        )
        .unwrap(),
        None,
        "a Docs Editors file has no stored content"
    );
    assert_eq!(
        from_file(&file(json!({"mimeType": FOLDER_MIME})), "x")
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(
        from_file(&file(json!({"trashed": true})), "x")
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotFound
    );
    assert_eq!(
        from_file(&file(json!({})), "y").unwrap_err().kind(),
        std::io::ErrorKind::InvalidData,
        "another object than asked for"
    );
}
