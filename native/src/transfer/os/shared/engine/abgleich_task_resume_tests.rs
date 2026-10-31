//! A resumed job under the file rule (review G12): a file whose receipt gave
//! no result (the target is no longer the file this job published) is decided
//! by the rule like any other file, not by the job's policy keeping both.
use super::super::queue::FileWork;
use super::super::test_backend::{fwd, job, write};
use super::super::{file_recovery, side_flow, Engine, FolderRegister};
use super::*;
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

#[path = "file_recovery_fixture.rs"]
mod fixtures;
use fixtures::{apply, file, fixture, names};

fn stamp(path: &Path, seconds: u64) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
        .unwrap();
}

#[test]
fn abgleich_task_a_resumed_file_without_a_receipt_result_follows_the_rule() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.jpg");
    let target = root.path().join("target");
    write(&source, b"alpha");
    stamp(&source, 2_000_000_000);
    fs::create_dir(&target).unwrap();
    let mut job = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
    job.conflict = Conflict::Update;
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    {
        // The first run inserts the file and writes its receipt.
        let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
        apply(&engine, file(&source, "a.jpg"));
        assert_eq!(engine.issues.total(), 0, "{:?}", engine.issues.shown().0);
    }
    assert_eq!(fs::read(target.join("a.jpg")).unwrap(), b"alpha");
    // Someone saves another file of the same size under that name, older
    // than the source: the receipt no longer proves it is this job's own.
    write(&target.join("a.jpg"), b"other");
    stamp(&target.join("a.jpg"), 1_000_000_000);
    job.recovery = crate::transfer::RecoveryMode::Resume;
    let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
    apply(&engine, file(&source, "a.jpg"));
    assert_eq!(engine.issues.total(), 0, "{:?}", engine.issues.shown().0);
    assert_eq!(
        names(&target),
        ["a.jpg"],
        "the newer source replaced the older target; the policy did not number a copy"
    );
    assert_eq!(fs::read(target.join("a.jpg")).unwrap(), b"alpha");
    assert_eq!(fs::read(&source).unwrap(), b"alpha");
}
