//! Upload authorization and crash ownership are separate contracts.
use super::super::file_evidence::{observe, readback_unavailable};
use super::super::{
    batches_backend_compatibility_task_tests::Limited,
    side_flow,
    test_backend::{fwd, job, run, write},
    Engine, FolderRegister,
};
use super::*;
use crate::transfer::{Endpoint, RecoveryMode};
use std::{fs, sync::Arc};

fn fixture<'a>(
    candidate: &'a crate::transfer::TransferJob,
    _directory: &Path,
    cancel: &'a AtomicBool,
    report: &'a (dyn Fn(crate::transfer::TransferMsg) + Sync),
) -> Engine<'a> {
    let view = JobView::of(candidate);
    let record = super::super::job_store::begin(&view, view.target_dir, cancel).unwrap();
    let folders = FolderRegister::new(
        view.target,
        view.target_dir,
        side_flow(view.target, view.target_dir),
    );
    let mut engine = Engine::new(
        view,
        view.kind(),
        folders,
        cancel,
        report,
        &super::super::roots::RootPlan::default(),
    );
    engine.job_record = record;
    engine
}

#[test]
fn batches_task_transfer_compat_upload_only_target_completes_without_claiming_recovery() {
    for (kind, collision) in [
        (io::ErrorKind::PermissionDenied, false),
        (io::ErrorKind::PermissionDenied, true),
        (io::ErrorKind::Unsupported, false),
        (io::ErrorKind::Unsupported, true),
    ] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source/a.bin");
        let target = root.path().join("target");
        write(&source, b"payload");
        fs::create_dir(&target).unwrap();
        if collision {
            write(&target.join("a.bin"), b"foreign");
        }
        let mut limited = Limited::new("upload-only");
        limited.read_error = Some(kind);
        let backend = Arc::new(limited);
        let candidate = job(
            Endpoint::Local,
            Endpoint::Remote(backend.clone()),
            &target,
            &[&source],
        );
        let work = FileWork::new(fwd(&source), "a.bin".into(), 7, 0);
        let cancel = AtomicBool::new(false);
        let report = |_| {};
        let engine = fixture(&candidate, root.path(), &cancel, &report);
        super::super::worker::run_file(
            &engine,
            work.clone(),
            &mut [0; super::super::ops::COPY_BUFFER],
        );
        assert_eq!(engine.issues.total(), 0, "{:?}", engine.issues.shown().0);
        assert_eq!(
            fs::read(target.join(if collision { "a (2).bin" } else { "a.bin" })).unwrap(),
            b"payload"
        );
        if collision {
            assert_eq!(fs::read(target.join("a.bin")).unwrap(), b"foreign");
        }
        let receipt = engine
            .job_record
            .as_ref()
            .unwrap()
            .read_file(&work)
            .unwrap()
            .unwrap();
        assert!(receipt.publication_authorized);
        assert!(receipt.proof.is_none());
        assert!(!receipt.confirmed);
        assert!(receipt.acknowledged);
        // The target can be neither hashed on its device nor read back: the
        // in-doubt check names that instead of claiming the file.
        let error = doubt::ownership(&receipt, engine.view.source, engine.view.target, &cancel)
            .expect_err("an unreadable acknowledged upload is not an ownership proof");
        assert!(readback_unavailable(engine.view.target, &error), "{error}");
        assert!(error
            .to_string()
            .contains("weder geprüft noch zurückgelesen"));
    }
}

#[test]
fn batches_task_transfer_compat_upload_only_lost_answer_never_republishes_or_removes_source() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.bin");
    let target = root.path().join("target");
    write(&source, b"payload");
    fs::create_dir(&target).unwrap();
    let mut limited = Limited::new("upload-only-lost");
    limited.read_error = Some(io::ErrorKind::PermissionDenied);
    limited.lost_publication.store(true, Ordering::SeqCst);
    let backend = Arc::new(limited);
    let mut candidate = job(
        Endpoint::Local,
        Endpoint::Remote(backend.clone()),
        &target,
        &[&source],
    );
    let first = run(candidate.clone());
    assert!(!first.issues.is_empty());
    assert_eq!(backend.promotes.load(Ordering::SeqCst), 1);
    candidate.recovery = RecoveryMode::Resume;
    let retry = run(candidate);
    assert!(!retry.issues.is_empty());
    assert_eq!(backend.promotes.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    assert_eq!(fs::read(target.join("a.bin")).unwrap(), b"payload");
    assert_eq!(fs::read(&source).unwrap(), b"payload");
}

#[test]
fn batches_task_transfer_compat_refused_readback_does_not_publish_missing_directory_or_resized_stage(
) {
    // Without any read-back, the pre-publication check of kind and size
    // still refuses a stage that vanished, became a folder or was resized.
    for mutation in [1, 2, 3] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("a.bin");
        let target = root.path().join("target");
        write(&source, b"payload");
        fs::create_dir(&target).unwrap();
        let mut limited = Limited::new("changed-upload-stage");
        limited.read_error = Some(io::ErrorKind::PermissionDenied);
        limited.mutate_stage.store(mutation, Ordering::SeqCst);
        let backend = Arc::new(limited);
        let result = run(job(
            Endpoint::Local,
            Endpoint::Remote(backend),
            &target,
            &[&source],
        ));
        assert!(!result.issues.is_empty());
        assert!(!target.join("a.bin").exists());
        assert_eq!(fs::read(&source).unwrap(), b"payload");
    }
}

#[test]
fn batches_task_transfer_compat_readback_io_failure_is_not_permission_fallback() {
    // The live upload reads nothing back, so a failing target read cannot
    // affect it. A retry keeps an unchanged target by metadata; once its
    // time changed, the in-doubt read-back fallback reports a real I/O
    // failure, and only a refused read keeps the "transfer missing" size rule.
    for (kind, refused) in [
        (io::ErrorKind::TimedOut, false),
        (io::ErrorKind::InvalidData, false),
        (io::ErrorKind::UnexpectedEof, false),
        (io::ErrorKind::PermissionDenied, true),
    ] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("a.bin");
        let target = root.path().join("target");
        write(&source, b"payload");
        fs::create_dir(&target).unwrap();
        let mut limited = Limited::new("readback-real-failure");
        limited.read_error = Some(kind);
        let backend = Arc::new(limited);
        let mut candidate = job(
            Endpoint::Local,
            Endpoint::Remote(backend.clone()),
            &target,
            &[&source],
        );
        let first = run(candidate.clone());
        assert!(first.issues.is_empty(), "{:?}", first.issues);
        candidate.recovery = RecoveryMode::Resume;
        candidate.resume = Some(first.roots);
        let unchanged = run(candidate.clone());
        assert!(unchanged.issues.is_empty(), "{:?}", unchanged.issues);
        fs::File::options()
            .write(true)
            .open(target.join("a.bin"))
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000))
            .unwrap();
        candidate.resume = Some(unchanged.roots);
        let retry = run(candidate);
        assert_eq!(
            retry.issues.is_empty(),
            refused,
            "{kind:?}: {:?}",
            retry.issues
        );
        assert_eq!(backend.promotes.load(Ordering::SeqCst), 1);
        assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
        assert_eq!(fs::read(target.join("a.bin")).unwrap(), b"payload");
        assert_eq!(fs::read(&source).unwrap(), b"payload");
    }
}

#[test]
fn batches_task_transfer_compat_optional_remote_readback_never_hides_local_permission_error() {
    let cancel = AtomicBool::new(true);
    let error = io::Error::from(io::ErrorKind::PermissionDenied);
    assert!(!readback_unavailable(Side::Local, &error));
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("a.bin");
    write(&path, b"alpha");
    assert_eq!(
        observe(Side::Local, &fwd(&path), &cancel)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
}
