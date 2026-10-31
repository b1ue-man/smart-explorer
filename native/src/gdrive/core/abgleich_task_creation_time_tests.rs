//! A file written in one step (`open_write_fresh_timed`,
//! `open_write_absent_timed`) has the source's time from its creation on: the
//! create request carries `modifiedTime` (the engine opens the writer with
//! it), also for files in folders the transfer created itself.
use super::absent_engine_task_tests::{drive_with_target, upload, upload_with};
use super::task_drive::{drive_server, job_backend, write_fresh};
use crate::vfs::BackendExtensions;
use serde_json::Value;
use std::io::Write;
use std::time::{Duration, SystemTime};

/// 2026-03-04 05:06:07 UTC.
const SOURCE_SECONDS: u64 = 1_772_600_767;
const SOURCE_TIME: &str = "2026-03-04T05:06:07.000Z";
const DEFAULT_TIME: &str = "2026-10-02T12:34:56.789Z";

fn modified(object: &Value) -> &str {
    object["modifiedTime"].as_str().unwrap()
}

fn set_source_time(path: &std::path::Path) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(SOURCE_SECONDS))
        .unwrap();
}

#[test]
fn abgleich_task_drive_fresh_writer_creates_with_the_given_time() {
    let (drive, server) = drive_server();
    let backend = job_backend(&drive, &server);
    let mut timed = backend
        .open_write_fresh_timed("Job/mit.txt", 3, SOURCE_SECONDS as i64 * 1000)
        .unwrap()
        .expect("Drive offers the timed one-step writer");
    timed.write_all(b"abc").unwrap();
    timed.flush().unwrap();
    write_fresh(&backend, "Job/ohne.txt", b"abc").unwrap();
    let with = drive.named("job-folder", "mit.txt");
    let without = drive.named("job-folder", "ohne.txt");
    assert_eq!(modified(&with[0]), SOURCE_TIME);
    assert_eq!(
        modified(&without[0]),
        DEFAULT_TIME,
        "untimed writer: the provider's own time"
    );
    let creates = server
        .requests()
        .iter()
        .filter(|request| request.method != "GET")
        .count();
    assert_eq!(creates, 2, "the time cost no request of its own");
}

#[test]
fn abgleich_task_drive_absent_writer_creates_with_the_given_time() {
    let (drive, server, backend) = drive_with_target(|_, _| {});
    let mut writer = crate::vfs::open_write_absent_timed(
        &backend,
        "Ziel/neu.txt",
        3,
        SOURCE_SECONDS as i64 * 1000,
    )
    .unwrap()
    .expect("Drive offers the one-step writer");
    writer.write_all(b"abc").unwrap();
    writer.flush().unwrap();
    let named = drive.named("target", "neu.txt");
    assert_eq!(named.len(), 1);
    assert_eq!(modified(&named[0]), SOURCE_TIME);
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|r| r.method != "GET")
            .count(),
        1
    );
}

#[test]
fn abgleich_task_drive_engine_upload_keeps_the_source_time_in_a_new_folder() {
    let local = tempfile::tempdir().unwrap();
    let folder = local.path().join("ordner");
    std::fs::create_dir(&folder).unwrap();
    let file = folder.join("a.txt");
    std::fs::write(&file, b"abc").unwrap();
    set_source_time(&file);
    let (drive, _server, backend) = drive_with_target(|_, _| {});
    // A selected folder kept beside an existing one needs a reservation
    // Drive cannot make (`gdrive/core/folder_reservation.rs`, the engine's
    // `roots.rs` refuses the job); merging, the default folder policy,
    // creates the folder as the transfer goes.
    let finished = upload_with(backend, &folder, crate::transfer::FolderConflict::Merge);
    assert!(finished.errors.is_empty(), "{:?}", finished.errors);
    assert_eq!(finished.progress.files_done, 1);
    let created = drive.named("target", "ordner");
    assert_eq!(created.len(), 1, "the engine created the folder");
    let id = created[0]["id"].as_str().unwrap();
    let inside = drive.named(id, "a.txt");
    assert_eq!(inside.len(), 1);
    assert_eq!(modified(&inside[0]), SOURCE_TIME);
}

#[test]
fn abgleich_task_drive_engine_upload_keeps_the_source_time_in_one_request() {
    let local = tempfile::tempdir().unwrap();
    let file = local.path().join("neu.txt");
    std::fs::write(&file, b"abc").unwrap();
    set_source_time(&file);
    let (drive, server, backend) = drive_with_target(|_, _| {});
    let finished = upload(backend, &file);
    assert!(finished.errors.is_empty(), "{:?}", finished.errors);
    let named = drive.named("target", "neu.txt");
    assert_eq!(named.len(), 1);
    assert_eq!(modified(&named[0]), SOURCE_TIME);
    let writes = server
        .requests()
        .iter()
        .filter(|request| request.method != "GET")
        .count();
    assert_eq!(writes, 1, "created under its final name in one request");
}
