//! One retained live journal protects every successive missing-only click.
use super::*;
use crate::transfer::{Endpoint, JobItems, PairItem, RecoveryMode, ResolvedRoot, TransferJob};
use crate::transfer::engine::{
    batches_backend_compatibility_task_tests::Limited,
    test_backend::{fwd, job, run, write},
};
use crate::types::CopyMode;
use std::{fs, sync::{Arc, atomic::Ordering}};

fn repeat(job: &mut TransferJob, roots: Vec<ResolvedRoot>) {
    job.recovery = RecoveryMode::Resume;
    job.resume = Some(roots);
}

#[test]
fn batches_task_transfer_compat_missing_only_retries_before_journal_for_copy_and_move() {
    for mode in [CopyMode::Copy, CopyMode::Move] {
        let root = tempfile::tempdir().unwrap(); let source = root.path().join("a.bin"); let target = root.path().join("target");
        write(&source, b"ours"); write(&target, b"not a directory");
        let mut candidate = job(Endpoint::Local, Endpoint::Local, &target, &[&source]); candidate.mode = mode;
        let first = run(candidate.clone()); assert!(!first.issues.is_empty()); assert!(first.roots.is_empty());
        fs::remove_file(&target).unwrap(); fs::create_dir(&target).unwrap();
        repeat(&mut candidate, first.roots);
        let next = run(candidate); assert!(next.issues.is_empty(), "{:?}", next.issues);
        assert_eq!(fs::read(target.join("a.bin")).unwrap(), b"ours"); assert_eq!(source.exists(), mode == CopyMode::Copy);
    }
}

#[test]
fn batches_task_transfer_compat_missing_only_retries_empty_roots_after_no_id_listing_failure() {
    let root = tempfile::tempdir().unwrap(); let source = root.path().join("Photos"); let target = root.path().join("target");
    write(&source.join("a.bin"), b"ours"); fs::create_dir(&target).unwrap();
    let backend = Arc::new(Limited::new("early-listing-no-id")); backend.refuse_listing.store(true, Ordering::SeqCst);
    let mut candidate = job(Endpoint::Local, Endpoint::Remote(backend.clone()), &target, &[&source]);
    let first = run(candidate.clone()); assert!(!first.issues.is_empty()); assert!(first.roots.is_empty());
    assert_eq!(backend.creates.load(Ordering::SeqCst), 0);
    repeat(&mut candidate, first.roots);
    let next = run(candidate); assert!(next.issues.is_empty(), "{:?}", next.issues);
    assert_eq!(fs::read(target.join("Photos/a.bin")).unwrap(), b"ours"); assert_eq!(backend.creates.load(Ordering::SeqCst), 1);
}

#[test]
fn batches_task_transfer_compat_missing_only_local_move_keeps_completed_pairs_and_conflicts() {
    let root = tempfile::tempdir().unwrap(); let first = root.path().join("a.bin"); let absent = root.path().join("b.bin"); let target = root.path().join("target");
    write(&first, b"aaa"); fs::create_dir(&target).unwrap();
    let mut candidate = job(Endpoint::Local, Endpoint::Local, &target, &[]); candidate.mode = CopyMode::Move;
    let mut one = PairItem::new(fwd(&first), "a.bin"); one.size = Some(3);
    let mut two = PairItem::new(fwd(&absent), "b.bin"); two.size = Some(3);
    candidate.items = JobItems::Pairs(vec![one, two]);
    let partial = run(candidate.clone()); assert!(!partial.issues.is_empty()); assert!(!first.exists());
    assert_eq!(fs::read(target.join("a.bin")).unwrap(), b"aaa");
    write(&absent, b"bbb"); write(&target.join("b.bin"), b"foreign content");
    repeat(&mut candidate, partial.roots);
    let conflict = run(candidate.clone()); assert!(!conflict.issues.is_empty());
    assert_eq!(fs::read(&absent).unwrap(), b"bbb"); assert_eq!(fs::read(target.join("b.bin")).unwrap(), b"foreign content");
    fs::remove_file(target.join("b.bin")).unwrap(); repeat(&mut candidate, conflict.roots);
    let done = run(candidate); assert!(done.issues.is_empty(), "{:?}", done.issues);
    assert!(!first.exists()); assert!(!absent.exists()); assert_eq!(fs::read(target.join("b.bin")).unwrap(), b"bbb");
    assert_eq!(fs::read_dir(&target).unwrap().count(), 2);
}

#[test]
fn batches_task_transfer_compat_missing_only_lost_directory_reply_stays_protected_with_empty_roots() {
    let root = tempfile::tempdir().unwrap(); let source = root.path().join("Photos"); let target = root.path().join("target");
    write(&source.join("a.bin"), b"ours"); fs::create_dir(&target).unwrap();
    let backend = Arc::new(Limited::new("retry-lost-mkdir")); backend.lost_directory.store(true, Ordering::SeqCst);
    let mut candidate = job(Endpoint::Local, Endpoint::Remote(backend.clone()), &target, &[&source]);
    let mut outcome = run(candidate.clone()); assert!(outcome.roots.is_empty());
    for _ in 0..2 {
        repeat(&mut candidate, outcome.roots);
        outcome = run(candidate.clone()); assert!(!outcome.issues.is_empty()); assert!(outcome.roots.is_empty());
    }
    assert_eq!(backend.creates.load(Ordering::SeqCst), 1); assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    assert!(!target.join("Photos/a.bin").exists()); assert!(source.join("a.bin").exists());
}

#[test]
fn batches_task_transfer_compat_missing_only_lost_file_reply_survives_successive_clicks() {
    let root = tempfile::tempdir().unwrap(); let source = root.path().join("a.bin"); let target = root.path().join("target");
    write(&source, b"ours"); fs::create_dir(&target).unwrap();
    let backend = Arc::new(Limited::new("retry-lost-publish")); backend.lost_publication.store(true, Ordering::SeqCst);
    let mut candidate = job(Endpoint::Local, Endpoint::Remote(backend.clone()), &target, &[&source]);
    let mut outcome = run(candidate.clone()); assert!(!outcome.issues.is_empty()); assert!(!outcome.roots.is_empty());
    for _ in 0..2 {
        repeat(&mut candidate, outcome.roots);
        outcome = run(candidate.clone()); assert!(!outcome.issues.is_empty());
    }
    assert_eq!(backend.promotes.load(Ordering::SeqCst), 1); assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    assert_eq!(fs::read(&source).unwrap(), b"ours"); assert_eq!(fs::read(target.join("a.bin")).unwrap(), b"ours");
}

#[test]
fn batches_task_transfer_compat_missing_only_acknowledged_upload_needs_no_read_or_object_id() {
    let root = tempfile::tempdir().unwrap(); let source = root.path().join("a.bin"); let target = root.path().join("target");
    write(&source, b"ours"); fs::create_dir(&target).unwrap();
    let mut limited = Limited::new("retry-write-only"); limited.read_error = Some(io::ErrorKind::PermissionDenied); let backend = Arc::new(limited);
    let mut candidate = job(Endpoint::Local, Endpoint::Remote(backend.clone()), &target, &[&source]);
    let first = run(candidate.clone()); assert!(first.issues.is_empty(), "{:?}", first.issues);
    fs::remove_file(target.join("a.bin")).unwrap(); repeat(&mut candidate, first.roots);
    let done = run(candidate); assert!(done.issues.is_empty(), "{:?}", done.issues);
    assert_eq!(backend.promotes.load(Ordering::SeqCst), 2); assert_eq!(fs::read(target.join("a.bin")).unwrap(), b"ours");
}

#[test]
fn batches_task_transfer_compat_missing_only_keeps_stored_recovery_and_journal_loss_strict() {
    let root = tempfile::tempdir().unwrap(); let source = root.path().join("a.bin"); let target = root.path().join("target");
    write(&source, b"ours"); fs::create_dir(&target).unwrap();
    let backend = Arc::new(Limited::new("retry-journal-loss"));
    let mut candidate = job(Endpoint::Local, Endpoint::Remote(backend.clone()), &target, &[&source]);
    let first = run(candidate.clone()); assert!(first.issues.is_empty());
    candidate.recovery = RecoveryMode::Resume;
    assert!(!run(candidate.clone()).issues.is_empty(), "stored recovery still needs target identity");
    let files = crate::support_dirs::app_data_file("transfer-jobs").join(format!("{}.files", candidate.id));
    fs::remove_file(files.join("index.json")).unwrap(); repeat(&mut candidate, first.roots);
    assert!(!run(candidate).issues.is_empty(), "missing index never becomes an empty live journal");
    assert_eq!(backend.promotes.load(Ordering::SeqCst), 1);
}

#[test]
fn batches_task_transfer_compat_missing_only_never_discards_unknown_move_quarantine() {
    use crate::copy::PublicationObserver;
    use crate::transfer::engine::{Engine, FolderRegister, job_store, queue::FileWork, roots::RootPlan, side_flow, file_recovery::LocalObserver};
    let root = tempfile::tempdir().unwrap(); let source = root.path().join("a.bin"); let target = root.path().join("target");
    let quarantine = root.path().join(".a.bin.smart-explorer-0123456789abcdef.move");
    let stage = target.join(".a.bin.smart-explorer-0123456789abcdef.part"); let destination = target.join("a.bin");
    write(&source, b"ours"); fs::create_dir(&target).unwrap();
    let mut candidate = job(Endpoint::Local, Endpoint::Local, &target, &[&source]); candidate.mode = CopyMode::Move;
    let cancel = AtomicBool::new(false); let report = |_| {};
    {
        let view = JobView::of(&candidate); let folders = FolderRegister::new(view.target, view.target_dir, side_flow(view.target, view.target_dir));
        let mut engine = Engine::new(view, view.kind(), folders, &cancel, &report, &RootPlan::default());
        engine.job_record = job_store::begin(&view, view.target_dir, &cancel).unwrap(); engine.job_record.as_ref().unwrap().prepare_apply().unwrap();
        let metadata = fs::metadata(&source).unwrap(); let work = FileWork::new(fwd(&source), "a.bin".into(), 4, crate::local_access::system_time_ms(metadata.modified().unwrap()));
        let observer = LocalObserver::new(&engine, &work);
        observer.before_source_quarantine(&source, &quarantine).unwrap(); fs::rename(&source, &quarantine).unwrap();
        fs::copy(&quarantine, &stage).unwrap(); observer.before_publish(&stage, &destination).unwrap();
        fs::rename(&stage, &destination).unwrap(); fs::rename(&destination, target.join("saved-original")).unwrap(); write(&destination, b"ours");
    }
    repeat(&mut candidate, vec![ResolvedRoot { source: fwd(&source), rel: "a.bin".into() }]);
    let result = run(candidate); assert!(!result.issues.is_empty());
    assert_eq!(fs::read(&quarantine).unwrap(), b"ours"); assert_eq!(fs::read(&destination).unwrap(), b"ours"); assert!(!source.exists());
}
