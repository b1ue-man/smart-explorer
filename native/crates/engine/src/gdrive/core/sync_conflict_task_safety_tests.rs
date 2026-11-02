use super::sync_conflict_task_fixture::{filter, Fixture, FILE, MIME};
use crate::bisync::{self, Baseline, BisyncOptions, DeletePolicy, Direction, ResolvePhase, Sig};
use crate::vfs::Backend;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn sync_conflict_task_failed_backups_preserve_files_and_baseline() {
    let f = Fixture::new(Some(b"same"), &[("a", b"same"), ("b", b"old")]);
    let mut baseline = Baseline::new();
    let old = Sig { size: 9, mtime_ms: 5, hash: 0 };
    baseline.insert(FILE.into(), (Some(old), Some(old)));
    let baseline_path = bisync::baseline_path(&f.pair);
    bisync::save_baseline(&baseline_path, &baseline).unwrap();
    f.faults.deny_read.store(true, Ordering::Release);
    let out = f.run(BisyncOptions::default());
    assert!(!out.errors.is_empty());
    assert_eq!(out.baseline, baseline);
    assert_eq!(bisync::load_baseline(&baseline_path).unwrap(), baseline);
    assert_eq!(f.mutation_count(), 0);
    assert_eq!(f.drive.named("root", FILE).len(), 2);

    let f = Fixture::new(Some(b"A"), &[("a", b"B"), ("b", b"C")]);
    let conflict = f.conflict();
    let versions = bisync::versions_dir(&f.pair);
    std::fs::create_dir_all(versions.parent().unwrap()).unwrap();
    std::fs::write(&versions, b"cannot create a directory here").unwrap();
    assert!(f.resolve(&conflict, true, None, &AtomicBool::new(false), |_| {}).is_err());
    assert_eq!(f.mutation_count(), 0);
    assert_eq!(f.local_content(), b"A");
    assert_eq!(f.drive.named("root", FILE).len(), 2);
}

#[test]
fn sync_conflict_task_changed_variant_and_early_cancel_never_authorize_cleanup() {
    let f = Fixture::new(Some(b"A"), &[("a", b"B"), ("b", b"C")]);
    let conflict = f.conflict();
    let canceled = AtomicBool::new(true);
    assert_eq!(f.resolve(&conflict, true, None, &canceled, |_| {}).unwrap_err().kind(), std::io::ErrorKind::Interrupted);
    assert_eq!(f.mutation_count(), 0);
    let error = f.resolve(&conflict, true, None, &AtomicBool::new(false), |phase| {
        if phase == ResolvePhase::BackingUp { f.drive.insert("b", FILE, "root", MIME, b"external new content"); }
    }).unwrap_err();
    assert!(error.to_string().contains("geändert"));
    assert_eq!(f.mutation_count(), 0);
    assert_eq!(f.drive.named("root", FILE).len(), 2);
    assert_eq!(f.local_content(), b"A");
}

#[test]
fn sync_conflict_task_partial_commit_retries_from_fresh_observation() {
    let f = Fixture::new(Some(b"A"), &[("a", b"B"), ("b", b"C")]);
    let conflict = f.conflict();
    let cancel = AtomicBool::new(false);
    let error = f.resolve(&conflict, true, None, &cancel, |phase| {
        if phase == ResolvePhase::Deleting { cancel.store(true, Ordering::Release); }
    }).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert_eq!(f.drive.named("root", FILE).len(), 2);
    f.assert_backed_up(b"B");
    f.assert_backed_up(b"C");
    assert!(f.resolve(&conflict, true, None, &AtomicBool::new(false), |_| {}).is_err());
    let retried = f.run(BisyncOptions::default());
    assert!(retried.errors.is_empty(), "{:?}", retried.errors);
    assert_eq!(f.remote_content(), b"A");
    assert_eq!(f.local_content(), b"A");

    let f = Fixture::new(Some(b"A"), &[("a", b"B"), ("b", b"C")]);
    let conflict = f.conflict();
    let cancel = AtomicBool::new(false);
    f.resolve(&conflict, true, None, &cancel, |phase| {
        if phase == ResolvePhase::ReadingSignatures { cancel.store(true, Ordering::Release); }
    }).unwrap();
    assert_eq!(f.remote_content(), b"A");
}

#[test]
fn sync_conflict_task_permissions_and_uncertain_trash_keep_exact_identity() {
    let f = Fixture::new(Some(b"same"), &[("a", b"same"), ("b", b"old")]);
    f.faults.deny_trash.store(true, Ordering::Release);
    let out = f.run(BisyncOptions::default());
    assert!(!out.errors.is_empty());
    assert_eq!(f.drive.named("root", FILE).len(), 2);
    assert!(out.baseline.is_empty());
    f.faults.deny_trash.store(false, Ordering::Release);
    f.faults.lose_trash_ack.store(true, Ordering::Release);
    let out = f.run(BisyncOptions::default());
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!(f.remote_content(), b"same");
    let sent = f.server.requests().iter().filter(|r| r.method == "PATCH" && r.path() == "/drive/v3/files/b").count();
    assert_eq!(sent, 2, "one denied attempt, one committed attempt; lost ACK is reconciled without replay");

    let f = Fixture::new(Some(b"A"), &[("a", b"B"), ("b", b"C")]);
    let conflict = f.conflict();
    f.faults.deny_replace.store(true, Ordering::Release);
    assert!(f.resolve(&conflict, true, None, &AtomicBool::new(false), |_| {}).is_err());
    assert_eq!(f.drive.named("root", FILE).len(), 2);
    assert_eq!(f.drive.object("a").unwrap()["md5Checksum"], format!("{:x}", md5::compute(b"B")));
    assert_eq!(f.local_content(), b"A");
}

#[test]
fn sync_conflict_task_ordinary_promotion_keeps_uniqueness_guard() {
    let f = Fixture::new(None, &[("a", b"B"), ("b", b"C")]);
    f.drive.insert("stage-id", "private-stage", "root", MIME, b"new");
    let error = f.remote.promote_staged("private-stage", FILE).unwrap_err();
    assert!(error.to_string().contains("ambiguous"));
    assert_eq!(f.mutation_count(), 0);
    assert!(f.remote.promote_staged_no_replace("private-stage", FILE).is_err());
    assert_eq!(f.mutation_count(), 0);
    f.remote.promote_staged_to_id("private-stage", FILE, Some("b")).unwrap();
    assert_eq!(f.drive.named("root", FILE).len(), 2, "ordinary publication does not itself authorize duplicate deletion");
    assert_eq!(f.drive.object("a").unwrap()["md5Checksum"], format!("{:x}", md5::compute(b"B")));
    assert_eq!(f.drive.object("b").unwrap()["md5Checksum"], format!("{:x}", md5::compute(b"new")));
}

#[test]
fn sync_conflict_task_filtered_counterparts_and_links_remain_protected() {
    let f = Fixture::new(Some(b"tiny"), &[("a", b"larger"), ("b", b"larger")]);
    let empty = bisync::empty_globset();
    let filter = bisync::WalkFilter { min_size: 5, ..filter(&empty) };
    let out = f.preview_with(BisyncOptions::default(), &filter);
    assert!(out.error.is_none(), "{:?}", out.error);
    assert!(out.actions.is_empty() && out.conflicts.is_empty());
    assert_eq!(out.duplicate_removals, 0);
    assert!(!out.omissions.is_empty());

    let f = Fixture::new(None, &[("a", b"A"), ("b", b"B")]);
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep.txt"), b"protected").unwrap();
    let link = f.directory.path().join(FILE);
    bisync::link_fixture::directory(outside.path(), &link);
    let out = f.run(BisyncOptions::default());
    bisync::link_fixture::remove_directory(&link);
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert!(out.conflicts.is_empty());
    assert!(!out.omissions.is_empty());
    assert_eq!(f.mutation_count(), 0);
    assert_eq!(std::fs::read(outside.path().join("keep.txt")).unwrap(), b"protected");
}

#[test]
fn sync_conflict_task_one_way_move_keeps_destination_after_duplicate_cleanup() {
    let f = Fixture::new(Some(b"move me"), &[("a", b"old"), ("b", b"move me")]);
    let opts = BisyncOptions { direction: Direction::AtoB, delete: DeletePolicy::Mirror,
        move_files: true, ..Default::default() };
    let out = f.run(opts);
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert!(!f.directory.path().join(FILE).exists());
    assert_eq!(f.remote_content(), b"move me");
    let again = f.run(opts);
    assert!(again.errors.is_empty(), "{:?}", again.errors);
    assert_eq!(f.remote_content(), b"move me");
}
