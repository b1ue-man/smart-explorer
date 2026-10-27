//! The live path reads nothing back; a resumed or retried job decides by
//! metadata, in doubt by content digest (2026-10-27).
use super::super::{
    batches_backend_compatibility_task_tests::Limited,
    file_evidence::with_check_meter,
    ops::{Outcome, COPY_BUFFER},
    side_flow,
    test_backend::{fwd, job, run, write, Fake},
    worker::run_file,
    Engine, FolderRegister,
};
use super::doubt::Ownership;
use super::*;
use crate::copy::PublicationObserver;
use crate::transfer::{Endpoint, RecoveryMode, TransferJob};
use crate::types::{Conflict, CopyMode};
use std::{fs, sync::Arc};

fn open<'a>(
    candidate: &'a TransferJob,
    cancel: &'a AtomicBool,
    report: &'a (dyn Fn(crate::transfer::TransferMsg) + Sync),
) -> io::Result<Engine<'a>> {
    let view = JobView::of(candidate);
    let record = super::super::job_store::begin(&view, view.target_dir, cancel)?;
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
    Ok(engine)
}

fn work(path: &Path, rel: &str) -> FileWork {
    let meta = fs::metadata(path).unwrap();
    FileWork::new(
        fwd(path),
        rel.into(),
        meta.len(),
        crate::local_access::system_time_ms(meta.modified().unwrap()),
    )
}

fn apply(engine: &Engine<'_>, file: &FileWork) {
    run_file(engine, file.clone(), &mut [0; COPY_BUFFER]);
}

/// Gives a file a time no copy in these tests has.
fn age(path: &Path) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000))
        .unwrap();
}

fn names(path: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The receipt a remote publication leaves: no content proof, acknowledged
/// after the live transfer (a crash before the answer leaves it unacknowledged).
fn plant(engine: &Engine<'_>, file: &FileWork, destination: &Path, acknowledged: bool) {
    let record = engine.job_record.as_ref().unwrap();
    record.prepare_apply().unwrap();
    record
        .start_file(
            file,
            snapshot(engine.view.source, &file.source),
            None,
            fwd(destination),
            None,
            PreviousTarget::Absent,
        )
        .unwrap();
    let mut receipt = record.read_file(file).unwrap().unwrap();
    receipt.acknowledged = acknowledged;
    record.write_file(&receipt).unwrap();
}

#[test]
fn screen_metadata_runtime_task_live_copy_records_no_proof_and_reads_nothing_back() {
    for upload in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("a.bin");
        let target = root.path().join("target");
        write(&source, b"payload");
        fs::create_dir(&target).unwrap();
        let mut fake = Fake::new("live-no-readback");
        fake.identity_proofs = true;
        let (fake, handle) = fake.handle();
        let (from, to) = if upload {
            (Endpoint::Local, Endpoint::Remote(handle))
        } else {
            (Endpoint::Remote(handle), Endpoint::Local)
        };
        let candidate = job(from, to, &target, &[&source]);
        let cancel = AtomicBool::new(false);
        let report = |_| {};
        let engine = open(&candidate, &cancel, &report).unwrap();
        let file = work(&source, "a.bin");
        apply(&engine, &file);
        assert_eq!(engine.issues.total(), 0, "{:?}", engine.issues.shown().0);
        assert_eq!(fs::read(target.join("a.bin")).unwrap(), b"payload");
        let receipt = engine
            .job_record
            .as_ref()
            .unwrap()
            .read_file(&file)
            .unwrap()
            .unwrap();
        // Evidence by metadata only: no digest anywhere in the receipt.
        for proof in [&receipt.proof, &receipt.staged, &receipt.published] {
            assert!(proof.as_ref().is_none_or(|proof| proof.sha256.is_empty()));
        }
        assert!(receipt.published.is_some());
        assert!(receipt.acknowledged);
        // Only the transfer itself read the remote side: the source of a
        // download once, the target of an upload never.
        let opened = fake.counters.opened.lock().unwrap().clone();
        assert_eq!(opened, if upload { vec![] } else { vec![fwd(&source)] });
        assert_eq!(engine.stats.check_meter().load(Ordering::Acquire), 0);
    }
}

#[test]
fn screen_metadata_runtime_task_resumed_job_with_equal_digest_skips_the_file() {
    for acknowledged in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("a.jpg");
        let target = root.path().join("target");
        write(&source, b"alpha");
        write(&target.join("a.jpg"), b"alpha");
        let mut candidate = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
        let cancel = AtomicBool::new(false);
        let report = |_| {};
        let file = work(&source, "a.jpg");
        {
            let engine = open(&candidate, &cancel, &report).unwrap();
            plant(&engine, &file, &target.join("a.jpg"), acknowledged);
        }
        candidate.recovery = RecoveryMode::Resume;
        let engine = open(&candidate, &cancel, &report).unwrap();
        assert_eq!(
            file_recovery_resume(&engine, &file),
            Some(Outcome::Skipped),
            "acknowledged: {acknowledged}"
        );
        apply(&engine, &file);
        assert_eq!(engine.issues.total(), 0, "{:?}", engine.issues.shown().0);
        assert_eq!(names(&target), ["a.jpg"]);
        assert_eq!(fs::read(target.join("a.jpg")).unwrap(), b"alpha");
    }
}

fn file_recovery_resume(engine: &Engine<'_>, file: &FileWork) -> Option<Outcome> {
    match resume(engine, file) {
        Ok(outcome) => outcome,
        Err(failure) => panic!("resume failed: {}", failure.error),
    }
}

#[test]
fn screen_metadata_runtime_task_resumed_job_with_different_digest_treats_target_as_foreign() {
    for conflict in [Conflict::Rename, Conflict::Overwrite, Conflict::Skip] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("a.jpg");
        let target = root.path().join("target");
        write(&source, b"alpha");
        // Same size, other content: only the digest tells them apart.
        write(&target.join("a.jpg"), b"omega");
        let mut candidate = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
        candidate.conflict = conflict;
        let cancel = AtomicBool::new(false);
        let report = |_| {};
        let file = work(&source, "a.jpg");
        {
            let engine = open(&candidate, &cancel, &report).unwrap();
            plant(&engine, &file, &target.join("a.jpg"), true);
        }
        candidate.recovery = RecoveryMode::Resume;
        let engine = open(&candidate, &cancel, &report).unwrap();
        let decided = {
            let record = engine.job_record.as_ref().unwrap();
            let receipt = record.read_file(&file).unwrap().unwrap();
            doubt::ownership(&receipt, engine.view.source, engine.view.target, &cancel).unwrap()
        };
        assert_eq!(decided, Ownership::Foreign);
        // Resolved without asking, as a fresh run of this policy would.
        apply(&engine, &file);
        assert_eq!(engine.issues.total(), 0, "{:?}", engine.issues.shown().0);
        match conflict {
            Conflict::Rename => {
                assert_eq!(names(&target), ["a (2).jpg", "a.jpg"]);
                assert_eq!(fs::read(target.join("a.jpg")).unwrap(), b"omega");
                assert_eq!(fs::read(target.join("a (2).jpg")).unwrap(), b"alpha");
            }
            Conflict::Overwrite => {
                assert_eq!(names(&target), ["a.jpg"]);
                assert_eq!(fs::read(target.join("a.jpg")).unwrap(), b"alpha");
            }
            Conflict::Skip => {
                assert_eq!(names(&target), ["a.jpg"]);
                assert_eq!(fs::read(target.join("a.jpg")).unwrap(), b"omega");
            }
        }
        assert_eq!(fs::read(&source).unwrap(), b"alpha");
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    /// Untouched since the rename: its metadata identity decides.
    Unchanged,
    /// Another object with the same bytes: its digest decides.
    EqualBytes,
    /// The same object rewritten with other bytes and another time.
    Rewritten,
}

#[test]
fn screen_metadata_runtime_task_resumed_move_deletes_source_only_after_equal_target() {
    for case in [Target::Unchanged, Target::EqualBytes, Target::Rewritten] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("a.jpg");
        let target = root.path().join("target");
        write(&source, b"alpha");
        fs::create_dir(&target).unwrap();
        let mut candidate = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
        candidate.mode = CopyMode::Move;
        let cancel = AtomicBool::new(false);
        let report = |_| {};
        let file = work(&source, "a.jpg");
        let quarantine = root
            .path()
            .join(".a.jpg.smart-explorer-0123456789abcdef.move");
        let stage = target.join(".a.jpg.smart-explorer-fedcba9876543210.part");
        let destination = target.join("a.jpg");
        {
            let engine = open(&candidate, &cancel, &report).unwrap();
            engine.job_record.as_ref().unwrap().prepare_apply().unwrap();
            let observer = LocalObserver::new(&engine, &file);
            observer
                .before_source_quarantine(&source, &quarantine)
                .unwrap();
            fs::rename(&source, &quarantine).unwrap();
            fs::copy(&quarantine, &stage).unwrap();
            observer.before_publish(&stage, &destination).unwrap();
            fs::rename(&stage, &destination).unwrap();
            // Crash before the publication answer and the source cleanup.
        }
        match case {
            Target::Unchanged => {}
            Target::EqualBytes => {
                fs::rename(&destination, target.join("retained")).unwrap();
                write(&destination, b"alpha");
            }
            Target::Rewritten => {
                fs::write(&destination, b"omega").unwrap();
                age(&destination);
            }
        }
        candidate.recovery = RecoveryMode::Resume;
        open(&candidate, &cancel, &report).unwrap();
        assert!(!quarantine.exists(), "{case:?}");
        if case == Target::Rewritten {
            // Not this job's content: the move is undone, nothing is lost.
            assert_eq!(fs::read(&source).unwrap(), b"alpha");
            assert_eq!(fs::read(&destination).unwrap(), b"omega");
        } else {
            assert!(!source.exists(), "{case:?}");
            assert_eq!(fs::read(&destination).unwrap(), b"alpha");
        }
    }
}

#[test]
fn screen_metadata_runtime_task_target_without_digest_falls_back_to_read_back() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.bin");
    let target = root.path().join("target");
    write(&source, b"payload");
    fs::create_dir(&target).unwrap();
    // An SFTP target without an agent: its device cannot hash.
    let candidate = job(
        Endpoint::Local,
        Endpoint::Remote(Arc::new(Limited::new("no-device-digest"))),
        &target,
        &[&source],
    );
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    let file = work(&source, "a.bin");
    {
        let engine = open(&candidate, &cancel, &report).unwrap();
        apply(&engine, &file);
        assert_eq!(engine.issues.total(), 0, "{:?}", engine.issues.shown().0);
        assert_eq!(engine.stats.check_meter().load(Ordering::Acquire), 0);
        let receipt = engine
            .job_record
            .as_ref()
            .unwrap()
            .read_file(&file)
            .unwrap()
            .unwrap();
        // Only the target is read back; the local source hashes on its disk.
        let meter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let decided = with_check_meter(meter.clone(), || {
            doubt::ownership(&receipt, engine.view.source, engine.view.target, &cancel)
        })
        .unwrap();
        assert_eq!(decided, Ownership::Own);
        assert_eq!(meter.load(Ordering::Acquire), b"payload".len() as u64);
    }
    // "Transfer missing" keeps the file as this job's own: nothing is sent
    // again and no numbered copy appears.
    let target = root.path().join("retried");
    fs::create_dir(&target).unwrap();
    let backend = Arc::new(Limited::new("no-device-digest-retry"));
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
    let retry = run(candidate);
    assert!(retry.issues.is_empty(), "{:?}", retry.issues);
    assert_eq!(backend.promotes.load(Ordering::SeqCst), 1);
    assert_eq!(names(&target), ["a.bin"]);
}

#[path = "file_recovery_metadata_tests.rs"]
mod metadata;
