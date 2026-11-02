//! Metadata evidence (2026-10-27): an unchanged acknowledged target is kept
//! without reading it, local copies and moves record no digest, and a
//! remote stage left by a crash is removed exactly.
use super::*;

#[test]
fn screen_metadata_runtime_task_unchanged_target_is_kept_without_reading_it() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.bin");
    let target = root.path().join("target");
    write(&source, b"payload");
    fs::create_dir(&target).unwrap();
    // Every read of this target fails: only an attempt to read shows.
    let mut limited = Limited::new("metadata-shortcut");
    limited.read_error = Some(io::ErrorKind::TimedOut);
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
    assert_eq!(backend.promotes.load(Ordering::SeqCst), 1);
    // Changed metadata puts the file in doubt: now it is read, and fails.
    age(&target.join("a.bin"));
    candidate.resume = Some(unchanged.roots);
    let changed = run(candidate);
    assert!(!changed.issues.is_empty());
    assert_eq!(backend.promotes.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read(target.join("a.bin")).unwrap(), b"payload");
}

#[test]
fn screen_metadata_runtime_task_local_copy_and_move_record_no_digest() {
    for mode in [CopyMode::Copy, CopyMode::Move] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("a.jpg");
        let target = root.path().join("target");
        write(&source, b"alpha");
        write(&target.join("a.jpg"), b"foreign");
        let mut candidate = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
        candidate.mode = mode;
        let cancel = AtomicBool::new(false);
        let report = |_| {};
        let file = work(&source, "a.jpg");
        {
            let engine = open(&candidate, &cancel, &report).unwrap();
            engine.job_record.as_ref().unwrap().prepare_apply().unwrap();
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
            let records = [&receipt.proof, &receipt.quarantine, &receipt.published];
            for proof in records.into_iter().flatten() {
                assert!(proof.sha256.is_empty(), "{mode:?}: {proof:?}");
            }
            let evidence = receipt.proof.as_ref().unwrap();
            assert!(
                evidence.object.is_some(),
                "the stage's object, renamed into place"
            );
            assert!(receipt.published.is_some());
            assert_eq!(receipt.quarantine.is_some(), mode == CopyMode::Move);
        }
        assert_eq!(names(&target), ["a (2).jpg", "a.jpg"]);
        assert_eq!(source.exists(), mode == CopyMode::Copy);
        if mode == CopyMode::Copy {
            // Resumed: the unchanged numbered target is this job's own.
            candidate.recovery = RecoveryMode::Resume;
            let engine = open(&candidate, &cancel, &report).unwrap();
            assert_eq!(file_recovery_resume(&engine, &file), Some(Outcome::Skipped));
            assert_eq!(engine.stats.check_meter().load(Ordering::Acquire), 0);
            assert_eq!(names(&target), ["a (2).jpg", "a.jpg"]);
        }
    }
}

#[test]
fn screen_metadata_runtime_task_crashed_remote_stage_is_removed_on_resume() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.bin");
    let target = root.path().join("target");
    write(&source, b"payload");
    fs::create_dir(&target).unwrap();
    let mut fake = Fake::new("crashed-remote-stage");
    fake.identity_proofs = true;
    let (_fake, handle) = fake.handle();
    let mut candidate = job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &target,
        &[&source],
    );
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    let file = work(&source, "a.bin");
    let destination = fwd(&target.join("a.bin"));
    let stage = super::super::super::publish::stage_name(&destination);
    {
        let engine = open(&candidate, &cancel, &report).unwrap();
        engine.job_record.as_ref().unwrap().prepare_apply().unwrap();
        fs::write(&stage, b"payload").unwrap();
        before(&engine, &file, &stage, &destination).unwrap();
        // Crash before the stage is published.
    }
    candidate.recovery = RecoveryMode::Resume;
    let engine = open(&candidate, &cancel, &report).unwrap();
    assert!(!std::path::Path::new(&stage).exists(), "stage removed");
    apply(&engine, &file);
    assert_eq!(engine.issues.total(), 0, "{:?}", engine.issues.shown().0);
    assert_eq!(names(&target), ["a.bin"]);
    assert_eq!(fs::read(target.join("a.bin")).unwrap(), b"payload");
}
