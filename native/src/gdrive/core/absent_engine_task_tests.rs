//! G3-08b through the transfer engine against the loopback Drive: a new
//! top-level file whose name the job's own listing showed absent is created
//! in one write request under its final name; a name another client takes
//! between that listing and the write is never shared: the engine's own
//! object goes to the trash by its ID, the file is tried again at once on
//! the staged path (without using up its one retry) and lands numbered.
use super::api::FOLDER_MIME;
use super::task_drive::{count, multipart, FakeDrive};
use super::task_http::{Request, Server};
use super::GDriveBackend;
use crate::transfer::{
    Endpoint, FolderConflict, FolderConflictPolicy, JobItems, JobView, Layout, RecoveryMode,
    TransferIssue, TransferJob, TransferMsg, TransferProgress,
};
use crate::types::{Conflict, CopyMode};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

struct Finished {
    progress: TransferProgress,
    errors: Vec<String>,
    issues: Vec<TransferIssue>,
}

type Loopback = (Arc<FakeDrive>, Server, GDriveBackend);

/// A loopback Drive with the folder "Ziel"; `after` sees every request once
/// the Drive answered it.
fn drive_with_target(after: impl Fn(&FakeDrive, &Request) + Send + Sync + 'static) -> Loopback {
    let drive = Arc::new(FakeDrive::default());
    drive.insert("target", "Ziel", "root", FOLDER_MIME, b"");
    let server = {
        let drive = Arc::clone(&drive);
        Server::start(move |request| {
            let answer = drive.answer(request);
            after(drive.as_ref(), request);
            answer
        })
    };
    let backend = server.backend();
    backend
        .remember_path("Ziel", "target", Some(FOLDER_MIME))
        .unwrap();
    (drive, server, backend)
}

/// Uploads `source` into the Drive folder "Ziel" ("keep both") through the
/// engine, on this thread, to its terminal message.
fn upload(backend: GDriveBackend, source: &Path) -> Finished {
    let job = TransferJob {
        id: crate::transfer::new_job_id(),
        recoverable: false,
        recovery: RecoveryMode::New,
        folders: FolderConflictPolicy::new(FolderConflict::KeepBoth),
        source: Endpoint::Local,
        target: Endpoint::Remote(Arc::new(backend)),
        target_dir: "Ziel".to_string(),
        items: JobItems::Roots {
            paths: vec![source.to_string_lossy().replace('\\', "/")],
            base: None,
        },
        layout: Layout::Tree,
        filter: None,
        conflict: Conflict::Rename,
        mode: CopyMode::Copy,
        source_label: "Quelle".to_string(),
        target_label: "Drive".to_string(),
        resume: None,
    };
    let messages = Mutex::new(Vec::new());
    let cancel = AtomicBool::new(false);
    crate::transfer::run_view_here(
        JobView::of(&job),
        &|message| messages.lock().unwrap().push(message),
        &cancel,
    );
    let mut finished = None;
    for message in messages.into_inner().unwrap() {
        if let TransferMsg::Done {
            progress,
            errors,
            issues,
            ..
        } = message
        {
            assert!(finished.is_none(), "exactly one terminal message");
            finished = Some(Finished {
                progress,
                errors,
                issues,
            });
        }
    }
    finished.expect("the job ended with its terminal message")
}

fn absent_creates<'r>(requests: &'r [Request], name: &str) -> Vec<&'r Request> {
    requests
        .iter()
        .filter(|request| {
            request.method == "POST"
                && request.path() == "/upload/drive/v3/files"
                && request.query("uploadType").as_deref() == Some("multipart")
                && multipart(request).0["name"] == name
        })
        .collect()
}

#[test]
fn speed_task_drive_absent_engine_upload_takes_one_write_request() {
    let local = tempfile::tempdir().unwrap();
    let source = local.path().join("neu.txt");
    std::fs::write(&source, b"abc").unwrap();
    let (drive, server, backend) = drive_with_target(|_, _| {});
    let finished = upload(backend, &source);
    assert!(finished.errors.is_empty(), "{:?}", finished.errors);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 1);
    let requests = server.requests();
    let writes: Vec<&Request> = requests
        .iter()
        .filter(|request| request.method != "GET")
        .collect();
    // The listing showed the name absent: one create under the final name,
    // no stage, no rename, no trash.
    let paths: Vec<&str> = writes.iter().map(|write| write.path()).collect();
    assert_eq!(writes.len(), 1, "{paths:?}");
    assert_eq!(absent_creates(&requests, "neu.txt").len(), 1);
    let named = drive.named("target", "neu.txt");
    assert_eq!(named.len(), 1);
    assert_eq!(drive.bytes(named[0]["id"].as_str().unwrap()), b"abc");
    assert_eq!(drive.children("target").len(), 1, "nothing else was left");
}

#[test]
fn speed_task_drive_absent_engine_name_race_lands_numbered_and_never_shares() {
    let local = tempfile::tempdir().unwrap();
    let source = local.path().join("race.txt");
    std::fs::write(&source, b"ours").unwrap();
    // Another client creates the same name right after the engine's create.
    let (drive, server, backend) = drive_with_target(|drive, request| {
        if request.method == "POST"
            && request.query("uploadType").as_deref() == Some("multipart")
            && multipart(request).0["name"] == "race.txt"
        {
            drive.insert("zz-foreign", "race.txt", "target", "text/plain", b"theirs");
        }
    });
    let finished = upload(backend, &source);
    assert!(finished.errors.is_empty(), "{:?}", finished.errors);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 1);
    let requests = server.requests();
    // Exactly one one-request attempt; the second attempt took the stage.
    let creates = absent_creates(&requests, "race.txt");
    assert_eq!(creates.len(), 1);
    let own = multipart(creates[0]).0["id"].as_str().unwrap().to_string();
    assert_eq!(drive.object(&own).unwrap()["trashed"], true);
    assert_eq!(
        count(&requests, "PATCH", &format!("/drive/v3/files/{own}")),
        1,
        "exactly the own first object is trashed"
    );
    // The foreign file is untouched and alone under its name.
    let named = drive.named("target", "race.txt");
    assert_eq!(named.len(), 1);
    assert_eq!(named[0]["id"], "zz-foreign");
    assert_eq!(drive.bytes("zz-foreign"), b"theirs");
    // The file landed once, numbered, byte-exact; no stage is left.
    let numbered = drive.named("target", "race (2).txt");
    assert_eq!(numbered.len(), 1);
    assert_eq!(drive.bytes(numbered[0]["id"].as_str().unwrap()), b"ours");
    let names: Vec<String> = drive
        .children("target")
        .iter()
        .map(|child| child["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(
        names.iter().all(|name| !name.contains(".se-upload-")),
        "{names:?}"
    );
}
