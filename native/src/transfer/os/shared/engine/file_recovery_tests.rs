//! Isolated publication/reopen oracles for the final remote task suite.
use super::super::queue::FileWork;
use super::super::test_backend::{fwd, job, write, Fake};
use super::super::{file_recovery, side_flow, Engine, FolderRegister};
use super::*;
use crate::copy::PublicationObserver;
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

#[path = "file_recovery_fixture.rs"]
mod fixtures;
use fixtures::{apply, file, fixture, names, remote};

#[test]
fn batches_task_p14_file_reopen_keeps_the_actual_numbered_destination() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.jpg");
    let target = root.path().join("target");
    write(&source, b"own bytes");
    write(&target.join("a.jpg"), b"foreign");
    let mut job = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
    let work = file(&source, "a.jpg");
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    {
        let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
        apply(&engine, work.clone());
        assert_eq!(engine.issues.total(), 0);
    }
    job.recovery = crate::transfer::RecoveryMode::Resume;
    let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
    apply(&engine, work);
    assert_eq!(engine.issues.total(), 0);
    assert_eq!(names(&target), ["a (2).jpg", "a.jpg"]);
    assert_eq!(fs::read(target.join("a.jpg")).unwrap(), b"foreign");
    assert_eq!(fs::read(target.join("a (2).jpg")).unwrap(), b"own bytes");
    assert_eq!(
        engine.job_record.as_ref().unwrap().saved_file_roots()[0].rel,
        "a (2).jpg"
    );
}

#[test]
fn batches_task_p14_partial_file_move_reopens_after_the_source_was_removed() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("a.jpg");
    let second = root.path().join("b.jpg");
    let target = root.path().join("target");
    write(&first, b"alpha");
    write(&second, b"beta");
    fs::create_dir(&target).unwrap();
    let mut job = job(
        Endpoint::Local,
        Endpoint::Local,
        &target,
        &[&first, &second],
    );
    job.mode = CopyMode::Move;
    let first_work = file(&first, "a.jpg");
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    {
        let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
        apply(&engine, first_work);
        assert_eq!(engine.issues.total(), 0);
    }
    assert!(!first.exists());
    job.recovery = crate::transfer::RecoveryMode::Resume;
    let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
    let run = engine.job_record.as_ref().unwrap();
    assert_eq!(run.completed_roots().len(), 1);
    let plan = super::super::roots::plan(
        &engine.view,
        &engine.folders,
        &cancel,
        run.completed_roots(),
        run.saved_file_roots(),
        false,
    )
    .unwrap();
    assert_eq!(plan.walk_roots.len(), 1);
    assert_eq!(plan.walk_roots[0].path, fwd(&second));
    apply(&engine, file(&second, "b.jpg"));
    assert_eq!(engine.issues.total(), 0);
    assert_eq!(names(&target), ["a.jpg", "b.jpg"]);
    assert!(!second.exists());
}

#[test]
fn batches_task_p14_reopen_refuses_same_sized_target_mutation_and_equal_foreign_objects() {
    for replace_object in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("a.jpg");
        let target = root.path().join("target");
        write(&source, b"alpha");
        fs::create_dir(&target).unwrap();
        let mut job = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
        let cancel = AtomicBool::new(false);
        let report = |_| {};
        {
            let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
            apply(&engine, file(&source, "a.jpg"));
            assert_eq!(engine.issues.total(), 0);
        }
        if replace_object {
            // Retain the owned object so its ID cannot be recycled.
            fs::rename(target.join("a.jpg"), target.join("retained-owned-object")).unwrap();
            write(&target.join("a.jpg"), b"alpha");
        } else {
            write(&target.join("a.jpg"), b"other");
        }
        job.recovery = crate::transfer::RecoveryMode::Resume;
        let reopened = fixture(&job, root.path(), &cancel, &report);
        assert!(
            reopened.is_err(),
            "foreign or modified target cannot become our success"
        );
        assert!(!target.join("a (2).jpg").exists());
        assert_eq!(fs::read(&source).unwrap(), b"alpha");
    }
}

#[test]
fn batches_task_p14_all_endpoint_pairs_share_the_same_publication_receipt() {
    for (remote_source, remote_target) in [(false, true), (true, false), (true, true)] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("a.jpg");
        let target = root.path().join("target");
        write(&source, b"alpha");
        write(&target.join("a.jpg"), b"foreign");
        let source_endpoint = if remote_source {
            remote("receipt-source", true)
        } else {
            Endpoint::Local
        };
        let target_endpoint = if remote_target {
            remote("receipt-target", true)
        } else {
            Endpoint::Local
        };
        let mut job = job(source_endpoint, target_endpoint, &target, &[&source]);
        let work = file(&source, "a.jpg");
        let cancel = AtomicBool::new(false);
        let report = |_| {};
        {
            let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
            apply(&engine, work.clone());
            assert_eq!(
                engine.issues.total(),
                0,
                "{remote_source}/{remote_target}: {:?}",
                engine.issues.shown().0
            );
        }
        job.recovery = crate::transfer::RecoveryMode::Resume;
        let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
        apply(&engine, work);
        assert_eq!(engine.issues.total(), 0);
        assert_eq!(names(&target), ["a (2).jpg", "a.jpg"]);
        assert_eq!(fs::read(target.join("a.jpg")).unwrap(), b"foreign");
    }
}

#[test]
fn batches_task_p14_identityless_remote_receipt_does_not_adopt_equal_content() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.jpg");
    let target = root.path().join("target");
    write(&source, b"alpha");
    fs::create_dir(&target).unwrap();
    let mut job = job(
        Endpoint::Local,
        remote("unproven-target", false),
        &target,
        &[&source],
    );
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
    apply(&engine, file(&source, "a.jpg"));
    assert_eq!(engine.issues.total(), 0);
    let record = engine.job_record.as_ref().unwrap();
    let error = file_recovery::verify_saved(
        &engine.view,
        &super::super::lock(&record.data),
        &record.path,
        None,
        &cancel,
    )
    .unwrap_err();
    assert!(error.to_string().contains("ohne stabile Objektidentität"));
    assert_eq!(names(&target), ["a.jpg"]);
}

#[test]
fn batches_task_p14_crash_before_publication_restores_the_bound_move_quarantine() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.jpg");
    let target = root.path().join("target");
    write(&source, b"alpha");
    fs::create_dir(&target).unwrap();
    let mut job = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
    job.mode = CopyMode::Move;
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    let work = file(&source, "a.jpg");
    let quarantine = root
        .path()
        .join(".a.jpg.smart-explorer-0123456789abcdef.move");
    {
        let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
        let observer = file_recovery::LocalObserver::new(&engine, &work);
        observer
            .before_source_quarantine(&source, &quarantine)
            .unwrap();
        fs::rename(&source, &quarantine).unwrap();
    }
    job.recovery = crate::transfer::RecoveryMode::Resume;
    let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
    assert_eq!(fs::read(&source).unwrap(), b"alpha");
    assert!(!quarantine.exists());
    apply(&engine, work);
    assert_eq!(engine.issues.total(), 0);
    assert_eq!(fs::read(target.join("a.jpg")).unwrap(), b"alpha");
    assert!(!source.exists());
}

#[test]
fn batches_task_p14_crash_after_staged_move_publication_keeps_target_and_cleans_only_own_source() {
    for replace_target in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("a.jpg");
        let target = root.path().join("target");
        write(&source, b"alpha");
        fs::create_dir(&target).unwrap();
        let mut job = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
        job.mode = CopyMode::Move;
        let cancel = AtomicBool::new(false);
        let report = |_| {};
        let work = file(&source, "a.jpg");
        let quarantine = root
            .path()
            .join(".a.jpg.smart-explorer-0123456789abcdef.move");
        let stage = target.join(".a.jpg.smart-explorer-fedcba9876543210.part");
        let destination = target.join("a.jpg");
        {
            let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
            let observer = file_recovery::LocalObserver::new(&engine, &work);
            observer
                .before_source_quarantine(&source, &quarantine)
                .unwrap();
            fs::rename(&source, &quarantine).unwrap();
            fs::copy(&quarantine, &stage).unwrap();
            observer.before_publish(&stage, &destination).unwrap();
            fs::rename(&stage, &destination).unwrap();
            // Simulate a crash before after_publish and quarantine removal.
        }
        if replace_target {
            fs::rename(&destination, target.join("retained-own-target")).unwrap();
            write(&destination, b"alpha");
        }
        job.recovery = crate::transfer::RecoveryMode::Resume;
        let reopened = fixture(&job, root.path(), &cancel, &report);
        if replace_target {
            assert!(reopened.is_err());
            assert_eq!(
                fs::read(&quarantine).unwrap(),
                b"alpha",
                "foreign replacement cannot authorize source cleanup"
            );
        } else {
            let engine = reopened.unwrap();
            assert_eq!(
                engine.job_record.as_ref().unwrap().completed_roots().len(),
                1
            );
            assert!(!quarantine.exists());
            assert!(!source.exists());
            assert_eq!(names(&target), ["a.jpg"]);
            assert_eq!(fs::read(&destination).unwrap(), b"alpha");
        }
    }
}

#[test]
fn batches_task_p14_packet_success_is_bound_and_lost_answers_remain_unconfirmed() {
    for lost_answer in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("a.jpg");
        let second = root.path().join("b.jpg");
        let target = root.path().join("target");
        write(&first, b"alpha");
        write(&second, b"beta");
        fs::create_dir(&target).unwrap();
        let mut fake = Fake::new("receipt-packet");
        fake.identity_proofs = true;
        fake.batch = Some(crate::vfs::BatchLimits {
            max_files: 4,
            max_bytes: 1024,
        });
        fake.faults.packet_answer_lost = lost_answer;
        let mut job = job(
            Endpoint::Local,
            Endpoint::Remote(fake.handle().1),
            &target,
            &[&first, &second],
        );
        let files = vec![file(&first, "a.jpg"), file(&second, "b.jpg")];
        let cancel = AtomicBool::new(false);
        let report = |_| {};
        {
            let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
            let backend = engine.view.target.backend().unwrap();
            super::super::batch_put::upload(
                &engine,
                backend,
                files.clone(),
                &mut [0; super::super::ops::COPY_BUFFER],
            );
            assert_eq!(engine.issues.total() == 0, !lost_answer);
        }
        job.recovery = crate::transfer::RecoveryMode::Resume;
        let reopened = fixture(&job, root.path(), &cancel, &report);
        if lost_answer {
            assert!(reopened.is_err());
        } else {
            let engine = reopened.unwrap();
            for file in files {
                apply(&engine, file);
            }
            assert_eq!(engine.issues.total(), 0);
        }
        assert_eq!(names(&target), ["a.jpg", "b.jpg"]);
    }
}

#[test]
fn batches_task_p14_missing_publication_receipt_is_an_incomplete_journal() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.jpg");
    let target = root.path().join("target");
    write(&source, b"alpha");
    fs::create_dir(&target).unwrap();
    let job = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    {
        let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
        apply(&engine, file(&source, "a.jpg"));
        assert_eq!(engine.issues.total(), 0);
    }
    let receipt = fs::read_dir(root.path().join("job.files"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.file_name().unwrap() != "index.json")
        .unwrap();
    fs::remove_file(receipt).unwrap();
    let error = match fixture(&job, root.path(), &cancel, &report) {
        Ok(_) => panic!("missing receipt must not look unstarted"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("Dateijournal ist unvollständig"));
    assert_eq!(names(&target), ["a.jpg"]);
}

#[test]
fn batches_task_p14_equal_paths_on_distinct_remote_connections_do_not_share_receipts() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.jpg");
    let target = root.path().join("target");
    write(&source, b"alpha");
    fs::create_dir(&target).unwrap();
    let mut job = job(
        remote("first-account", true),
        Endpoint::Local,
        &target,
        &[&source],
    );
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    {
        let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
        apply(&engine, file(&source, "a.jpg"));
        assert_eq!(engine.issues.total(), 0);
    }
    job.source = remote("second-account", true);
    assert!(fixture(&job, root.path(), &cancel, &report).is_err());
    assert_eq!(names(&target), ["a.jpg"]);
}

#[test]
fn batches_task_p14_explicit_pair_move_retains_completed_results_during_reopen() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("a.jpg");
    let second = root.path().join("b.jpg");
    let target = root.path().join("target");
    write(&first, b"alpha");
    write(&second, b"beta");
    write(&target.join("a.jpg"), b"foreign");
    let mut job = job(
        Endpoint::Local,
        Endpoint::Local,
        &target,
        &[&first, &second],
    );
    job.mode = CopyMode::Move;
    let pairs = [(&first, "a.jpg"), (&second, "b.jpg")].into_iter().map(|(path, rel)| {
        serde_json::from_value(serde_json::json!({"source": fwd(path), "rel": rel, "size": fs::metadata(path).unwrap().len(), "mtime_ms": 0, "id": null})).unwrap()
    }).collect();
    job.items = JobItems::Pairs(pairs);
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    let mut first_work = file(&first, "a.jpg");
    first_work.receipt_key = Some(format!("{}\0a.jpg", fwd(&first)));
    {
        let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
        apply(&engine, first_work);
        assert_eq!(engine.issues.total(), 0);
    }
    job.recovery = crate::transfer::RecoveryMode::Resume;
    let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
    let run = engine.job_record.as_ref().unwrap();
    let plan = super::super::roots::plan(
        &engine.view,
        &engine.folders,
        &cancel,
        run.completed_roots(),
        run.saved_file_roots(),
        false,
    )
    .unwrap();
    assert_eq!(plan.pairs.len(), 1);
    assert_eq!(plan.pairs[0].0.source, fwd(&second));
    assert!(plan
        .roots
        .iter()
        .any(|entry| entry.source == "a.jpg" && entry.planned == "a (2).jpg"));
    apply(&engine, plan.pairs[0].0.clone());
    assert_eq!(engine.issues.total(), 0);
    assert_eq!(names(&target), ["a (2).jpg", "a.jpg", "b.jpg"]);
    assert_eq!(fs::read(target.join("a.jpg")).unwrap(), b"foreign");
}

#[path = "file_recovery_fix_tests.rs"]
mod corrections;
