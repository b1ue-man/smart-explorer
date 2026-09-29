//! Transfer-engine task tests of the Drive backend (plan block C), part 2:
//! parallel folder creation, exclusive folders, sized stages, resumed
//! downloads, server-side copies, the write-rate hint and the flow key.
use super::api::FOLDER_MIME;
use super::task_drive::{count, drive_server, multipart, FakeDrive};
use super::task_http::{Answer, Server};
use super::GDriveBackend;
use crate::vfs::Backend;
use serde_json::json;
use std::io::{self, Read, Write};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

#[test]
fn transfer_engine_task_drive_folders_are_created_in_parallel_without_duplicates() {
    // Eight transfers need the same new folder: exactly one create.
    let drive = Arc::new(FakeDrive::default());
    let server = {
        let drive = Arc::clone(&drive);
        Server::start(move |request| {
            if request.method == "POST" {
                thread::sleep(Duration::from_millis(20));
            }
            drive.answer(request)
        })
    };
    let backend = server.backend();
    backend.listed_guard().unwrap().insert(String::new());
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let backend = backend.clone();
            thread::spawn(move || {
                backend.mkdir_all("Neu").unwrap();
                backend.cached_id("Neu").unwrap().unwrap()
            })
        })
        .collect();
    let ids: Vec<String> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert!(ids.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(count(&server.requests(), "POST", "/drive/v3/files"), 1);
    assert_eq!(drive.named("root", "Neu").len(), 1);

    // Two different folders: both creates are in flight at once (a
    // process-wide create lock would keep the second one waiting here).
    let drive = Arc::new(FakeDrive::default());
    let gate = Arc::new((Mutex::new(0usize), Condvar::new()));
    let server = {
        let (drive, gate) = (Arc::clone(&drive), Arc::clone(&gate));
        Server::start(move |request| {
            if request.method == "POST" {
                let (arrived, changed) = &*gate;
                let mut arrived = arrived.lock().unwrap();
                *arrived += 1;
                changed.notify_all();
                let (_arrived, wait) = changed
                    .wait_timeout_while(arrived, Duration::from_secs(5), |arrived| *arrived < 2)
                    .unwrap();
                if wait.timed_out() {
                    return Answer::status(
                        400,
                        json!({"error": {"message": "creates were serialized"}}),
                    );
                }
            }
            drive.answer(request)
        })
    };
    let backend = server.backend();
    backend.listed_guard().unwrap().insert(String::new());
    let workers = ["A", "B"].map(|name| {
        let backend = backend.clone();
        thread::spawn(move || backend.mkdir_all(name))
    });
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
    assert_eq!(drive.named("root", "A").len(), 1);
    assert_eq!(drive.named("root", "B").len(), 1);
}

#[test]
fn transfer_engine_task_drive_create_dir_new_is_exclusive() {
    let drive = Arc::new(FakeDrive::default());
    drive.insert("taken-id", "Belegt", "root", "text/plain", b"x");
    let server = {
        let drive = Arc::clone(&drive);
        Server::start(move |request| {
            let answer = drive.answer(request);
            // Another client creates "Doppelt" at the same moment.
            if request.method == "POST"
                && request.body.windows(9).any(|part| part == b"\"Doppelt\"")
            {
                drive.insert("zz-foreign", "Doppelt", "root", FOLDER_MIME, b"");
            }
            answer
        })
    };
    let backend = server.backend();
    let taken = |path: &str| backend.create_dir_new(path).unwrap_err().kind();
    assert_eq!(taken("Belegt"), io::ErrorKind::AlreadyExists);
    backend.create_dir_new("Frei").unwrap();
    assert_eq!(taken("Frei"), io::ErrorKind::AlreadyExists);
    assert_eq!(drive.named("root", "Frei").len(), 1);
    // A concurrent creator wins: our own empty folder goes back to the trash.
    assert_eq!(taken("Doppelt"), io::ErrorKind::AlreadyExists);
    let remaining = drive.named("root", "Doppelt");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0]["id"], "zz-foreign");
    assert_eq!(count(&server.requests(), "POST", "/drive/v3/files"), 2);
}

#[test]
fn transfer_engine_task_drive_sized_stages_are_created_once_and_discarded_by_exact_id() {
    let (drive, server) = drive_server();
    let backend = server.backend();
    backend
        .remember_path("Ziel", "target", Some(FOLDER_MIME))
        .unwrap();
    let stage = "Ziel/a.txt.se-copy-0123456789abcdef";
    let mut writer = backend.open_write_copy_stage_sized(stage, 3).unwrap();
    writer.write_all(b"abc").unwrap();
    writer.flush().unwrap();
    drop(writer);
    // One write request with a binary type and one check that the stage name
    // is unique; no probe before it (the caller picked a random name).
    let requests = server.requests();
    let calls: Vec<(&str, &str)> = requests
        .iter()
        .map(|request| (request.method.as_str(), request.path()))
        .collect();
    assert_eq!(
        calls,
        [
            ("GET", "/drive/v3/files/generateIds"),
            ("POST", "/upload/drive/v3/files"),
            ("GET", "/drive/v3/files"),
        ]
    );
    let (metadata, media) = multipart(&requests[1]);
    assert_eq!(media, b"abc");
    assert_eq!(metadata["mimeType"], "application/octet-stream");
    let stage_id = metadata["id"].as_str().unwrap().to_string();

    // Only an own stage is discarded, and by its exact ID.
    assert_eq!(
        backend
            .discard_copy_stage("Ziel/fremd.se-copy-1")
            .unwrap_err()
            .kind(),
        io::ErrorKind::Unsupported
    );
    backend.discard_copy_stage(stage).unwrap();
    assert_eq!(drive.object(&stage_id).unwrap()["trashed"], true);

    // A published stage is never discarded.
    let stage = "Ziel/b.txt.se-copy-1";
    let mut writer = backend.open_write_copy_stage_sized(stage, 2).unwrap();
    writer.write_all(b"ok").unwrap();
    writer.flush().unwrap();
    backend.promote_copy_stage(stage, "Ziel/b.txt").unwrap();
    assert_eq!(
        backend.discard_copy_stage(stage).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    assert_eq!(drive.named("target", "b.txt").len(), 1);

    // Exactly the announced size: short or long content fails.
    let mut short = backend
        .open_write_copy_stage_sized("Ziel/c.se-copy-2", 5)
        .unwrap();
    short.write_all(b"abc").unwrap();
    assert_eq!(
        short.flush().unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    let mut long = backend
        .open_write_copy_stage_sized("Ziel/d.se-copy-3", 1)
        .unwrap();
    assert_eq!(
        long.write(b"xy").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn transfer_engine_task_drive_downloads_resume_by_range_and_reuse_listed_types() {
    let content: &'static [u8] = b"0123456789";
    let server = Server::start(move |request| {
        let (status, body) = match request.header("range") {
            // a.bin honours the range, b.bin answers with the whole file.
            Some(range) if request.path().ends_with("/a-id") => {
                let start: usize = range
                    .trim_start_matches("bytes=")
                    .trim_end_matches('-')
                    .parse()
                    .unwrap();
                (206, content[start..].to_vec())
            }
            _ => (200, content.to_vec()),
        };
        let mut answer = Answer::status(status, json!(null));
        if status == 206 {
            answer = answer.header("Content-Range", &format!("bytes {}-9/10", 10 - body.len()));
        }
        answer.body = body;
        answer
    });
    let backend = server.backend();
    for (path, id) in [("a.bin", "a-id"), ("b.bin", "b-id")] {
        backend
            .remember_path(path, id, Some("application/octet-stream"))
            .unwrap();
    }
    let read = |path: &str, id: &str, offset: u64| {
        let mut bytes = Vec::new();
        backend
            .open_read_at(path, Some(id), offset)
            .unwrap()
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    };
    assert_eq!(read("a.bin", "a-id", 3), b"3456789");
    assert_eq!(read("b.bin", "b-id", 4), b"456789");
    assert_eq!(read("a.bin", "a-id", 0), b"0123456789");
    // The types came from the listing cache: only the three media requests.
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].header("range"), Some("bytes=3-"));
    assert!(requests
        .iter()
        .all(|request| request.query("alt").as_deref() == Some("media")));
    // An exported Google document has no stable bytes to resume.
    backend
        .remember_path(
            "Notiz",
            "doc-id",
            Some("application/vnd.google-apps.document"),
        )
        .unwrap();
    assert!(backend
        .open_read_at("Notiz", Some("doc-id"), 5)
        .unwrap()
        .is_none());
}

#[test]
fn transfer_engine_task_drive_copies_inside_drive_on_the_server() {
    let (drive, server) = drive_server();
    drive.insert(
        "src-id",
        "Quelle.bin",
        "root",
        "application/octet-stream",
        b"inhalt",
    );
    let backend = server.backend();
    backend
        .remember_path("Quelle.bin", "src-id", Some("application/octet-stream"))
        .unwrap();
    backend
        .remember_path(
            "Notiz",
            "doc-id",
            Some("application/vnd.google-apps.document"),
        )
        .unwrap();
    backend
        .remember_path("Ziel", "target", Some(FOLDER_MIME))
        .unwrap();
    let stage = "Ziel/Quelle.bin.se-copy-01";
    assert_eq!(
        backend
            .server_copy_to_stage(
                "Quelle.bin",
                stage,
                6,
                &std::sync::atomic::AtomicBool::new(false)
            )
            .unwrap(),
        Some(6)
    );
    let requests = server.requests();
    let copy = requests
        .iter()
        .find(|request| request.method == "POST")
        .unwrap();
    assert_eq!(copy.path(), "/drive/v3/files/src-id/copy");
    let metadata: serde_json::Value = serde_json::from_slice(&copy.body).unwrap();
    assert_eq!(metadata["name"], "Quelle.bin.se-copy-01");
    assert_eq!(metadata["parents"], json!(["target"]));
    let id = metadata["id"].as_str().unwrap();
    assert_eq!(
        drive.object(id).unwrap()["md5Checksum"],
        format!("{:x}", md5::compute(b"inhalt"))
    );
    // No byte crossed the client: no media download, no upload.
    assert_eq!(count(&requests, "POST", "/upload/drive/v3/files"), 0);
    assert!(requests
        .iter()
        .all(|request| request.query("alt").is_none()));
    // The copy is an own stage and publishes like any other.
    backend
        .promote_copy_stage(stage, "Ziel/Quelle.bin")
        .unwrap();
    assert_eq!(drive.named("target", "Quelle.bin").len(), 1);
    // Google documents keep streaming as an export.
    assert_eq!(
        backend
            .server_copy_to_stage(
                "Notiz",
                "Ziel/Notiz.se-copy-02",
                0,
                &std::sync::atomic::AtomicBool::new(false)
            )
            .unwrap(),
        None
    );
}

#[test]
fn transfer_engine_task_drive_reports_its_write_rate_and_one_flow_per_account() {
    let backend = GDriveBackend::test_backend("http://127.0.0.1:9/drive/v3");
    assert_eq!(
        backend.transfer_hint().as_deref(),
        Some("Google Drive nimmt höchstens etwa 3 neue Dateien pro Sekunde an")
    );
    // All clones and connections of one account share one flow and one
    // namespace, whatever their start folder.
    let other = GDriveBackend::test_backend("http://127.0.0.1:9/other/drive/v3");
    assert_eq!(backend.flow_key("/a"), backend.clone().flow_key("/b"));
    assert_eq!(backend.flow_key("/a"), other.flow_key("/a"));
    assert!(backend.flow_key("/").starts_with("gdrive:"));
    assert_eq!(backend.namespace_identity(), other.namespace_identity());
    assert_ne!(backend.namespace_identity(), backend.state_identity());
}
