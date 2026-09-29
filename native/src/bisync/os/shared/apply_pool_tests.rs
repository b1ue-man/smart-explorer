//! Two-way/one-way apply over the flows: concurrent where the old fixed
//! `min(parallelism)` was serial, `max_transfers` still an upper bound, new
//! folders created once, related paths in one group (deletions first),
//! failures never completed, overload waited out, a panic ends the run.
use super::super::apply::{apply_planned_with_results, ApplyReport};
use super::super::apply_groups::action_groups;
use super::super::test_remote::{FakeRemote, REMOTE_ROOT};
use super::super::*;
use crate::vfs::{Backend, LocalBackend};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn forward(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn write(root: &Path, rel: &str, content: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// Walks both sides, plans A → B and applies it.
fn apply_a_to_b(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    max_transfers: usize,
    versions: &Path,
) -> (ApplyReport, Vec<(String, String)>) {
    let cancel = AtomicBool::new(false);
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let tree_a = walk_files(a, root_a, &cancel, &filter, HashMode::None, None).unwrap();
    let tree_b = walk_files(b, root_b, &cancel, &filter, HashMode::None, None).unwrap();
    let opts = BisyncOptions {
        direction: Direction::AtoB,
        max_transfers,
        ..Default::default()
    };
    let (actions, conflicts, _) = plan(&tree_a, &tree_b, &Baseline::new(), opts);
    assert!(conflicts.is_empty());
    let mut errors = Vec::new();
    let report = apply_planned_with_results(
        &actions,
        &tree_a,
        &tree_b,
        a,
        root_a,
        b,
        root_b,
        opts,
        versions,
        &mut errors,
        &cancel,
    );
    (report, errors)
}

fn populate(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for index in 0..16 {
        files.push(format!("file{index}.txt"));
    }
    for index in 0..4 {
        files.push(format!("new/nested{index}.txt"));
    }
    for rel in &files {
        write(root, rel, rel.as_bytes());
    }
    files
}

#[test]
fn transfer_engine_task_bisync_apply_runs_actions_concurrently_over_flows() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let versions = tempfile::tempdir().unwrap();
    let files = populate(a.path());
    // `parallelism() == 1`: the old apply ran this pair strictly serially.
    let source = FakeRemote::new(a.path(), "apply-parallel").with_delay(Duration::from_millis(30));
    let target = LocalBackend::new(&forward(b.path()));

    let (report, errors) = apply_a_to_b(
        &source,
        REMOTE_ROOT,
        &target,
        &forward(b.path()),
        0,
        versions.path(),
    );

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(report.stats.a_to_b, 20);
    assert_eq!(report.completed.len(), 20);
    assert!(source.calls.peak_reads.load(Ordering::SeqCst) >= 2);
    for rel in &files {
        assert_eq!(std::fs::read(b.path().join(rel)).unwrap(), rel.as_bytes());
    }
}

#[test]
fn transfer_engine_task_bisync_apply_keeps_max_transfers_as_upper_bound() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let versions = tempfile::tempdir().unwrap();
    populate(a.path());
    let source = FakeRemote::new(a.path(), "apply-capped").with_delay(Duration::from_millis(10));
    let target = LocalBackend::new(&forward(b.path()));

    let (report, errors) = apply_a_to_b(
        &source,
        REMOTE_ROOT,
        &target,
        &forward(b.path()),
        1,
        versions.path(),
    );

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(report.stats.a_to_b, 20);
    assert_eq!(source.calls.peak_reads.load(Ordering::SeqCst), 1);
}

#[test]
fn transfer_engine_task_bisync_new_folder_created_once_for_concurrent_copies() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let versions = tempfile::tempdir().unwrap();
    for index in 0..12 {
        write(a.path(), &format!("new/deeper/f{index}.txt"), b"payload");
    }
    let source = LocalBackend::new(&forward(a.path()));
    // A naive client: two concurrent creations of one folder collide.
    let target = FakeRemote::new(b.path(), "apply-racy").with_racy_folders();

    let (report, errors) = apply_a_to_b(
        &source,
        &forward(a.path()),
        &target,
        REMOTE_ROOT,
        0,
        versions.path(),
    );

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(report.stats.a_to_b, 12);
    // `new` and `new/deeper`, once each, through the folder register.
    assert_eq!(target.calls.create_dir.load(Ordering::SeqCst), 2);
    for index in 0..12 {
        assert!(b.path().join(format!("new/deeper/f{index}.txt")).exists());
    }
}

#[test]
fn transfer_engine_task_bisync_failed_action_is_never_completed() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let versions = tempfile::tempdir().unwrap();
    for index in 1..=5 {
        write(a.path(), &format!("x{index}.txt"), b"content");
    }
    let source = LocalBackend::new(&forward(a.path()));
    let target = FakeRemote::new(b.path(), "apply-failing").failing_writes_to("x3.txt");

    let (report, errors) = apply_a_to_b(
        &source,
        &forward(a.path()),
        &target,
        REMOTE_ROOT,
        0,
        versions.path(),
    );

    assert_eq!(report.stats.errors, 1);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].0.contains("x3.txt"), "{errors:?}");
    assert_eq!(report.completed.len(), 4);
    assert!(!report
        .completed
        .contains(&Action::CopyAtoB("x3.txt".to_string())));
    assert!(!b.path().join("x3.txt").exists());
}

#[test]
fn transfer_engine_task_bisync_case_variants_run_in_plan_order() {
    let actions = [
        Action::DeleteB("Foo.txt".to_string()),
        Action::CopyAtoB("bar.txt".to_string()),
        Action::CopyAtoB("foo.txt".to_string()),
        Action::CopyAtoB("dir/FOO.txt".to_string()),
    ];
    assert_eq!(action_groups(&actions), vec![vec![0, 2], vec![1], vec![3]]);
}

#[test]
fn transfer_engine_task_bisync_related_paths_share_a_group_deletions_first() {
    let actions = [
        Action::CopyAtoB("dir/file.txt".to_string()),
        Action::DeleteB("dir".to_string()),
        Action::CopyAtoB("other.txt".to_string()),
        Action::CopyAtoB("Foo.txt".to_string()),
        Action::DeleteB("foo.txt".to_string()),
        Action::CopyAtoB("DIR/second.txt".to_string()),
        Action::CopyAtoB("dir-sibling/file.txt".to_string()),
    ];
    // A file `dir` replaced by a folder: its deletion runs first, then both
    // copies below it; `dir-sibling` is no descendant of `dir`.
    assert_eq!(
        action_groups(&actions),
        vec![vec![1, 0, 5], vec![2], vec![4, 3], vec![6]]
    );
}

#[test]
fn transfer_engine_task_bisync_overload_is_waited_out_before_publication() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let versions = tempfile::tempdir().unwrap();
    for index in 0..6 {
        write(a.path(), &format!("f{index}.txt"), b"payload");
    }
    let source = LocalBackend::new(&forward(a.path()));
    // Four stage uploads are refused as overload before they succeed.
    let target = FakeRemote::new(b.path(), "apply-busy").with_overload("write", 4);

    let (report, errors) = apply_a_to_b(
        &source,
        &forward(a.path()),
        &target,
        REMOTE_ROOT,
        0,
        versions.path(),
    );

    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(report.stats.a_to_b, 6);
    assert_eq!(report.completed.len(), 6);
    assert_eq!(target.calls.congested.load(Ordering::SeqCst), 4);
    for index in 0..6 {
        assert_eq!(
            std::fs::read(b.path().join(format!("f{index}.txt"))).unwrap(),
            b"payload"
        );
    }
}

#[test]
fn transfer_engine_task_bisync_panicking_action_ends_the_run() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let versions = tempfile::tempdir().unwrap();
    for index in 0..4 {
        write(a.path(), &format!("f{index}.txt"), b"payload");
    }
    write(a.path(), "boom.txt", b"payload");
    let source = FakeRemote::new(a.path(), "apply-panic").with_hook(Arc::new(
        |operation: &str, path: &str| {
            if operation == "read" && path.ends_with("boom.txt") {
                panic!("injected action panic");
            }
        },
    ));
    let target = LocalBackend::new(&forward(b.path()));

    let (report, errors) = apply_a_to_b(
        &source,
        REMOTE_ROOT,
        &target,
        &forward(b.path()),
        0,
        versions.path(),
    );

    assert!(errors
        .iter()
        .any(|(_, message)| message.contains("stopped unexpectedly")));
    assert!(!report
        .completed
        .contains(&Action::CopyAtoB("boom.txt".to_string())));
    assert!(!b.path().join("boom.txt").exists());
}
