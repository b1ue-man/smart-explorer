//! The parallel copy pass of the one-way mirror against remote-like fakes:
//! same results as a serial run, links stay protected omissions, a failed
//! copy blocks the delete pass, destination folders are listed instead of
//! probed per file, and equal paths on two remotes stay two locations.
use super::*;
use crate::bisync::link_fixture;
use crate::bisync::test_remote::{FakeRemote, REMOTE_ROOT};
use crate::vfs::{BackendHandle, LocalBackend};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn forward(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn mirror(
    source: BackendHandle,
    source_root: &str,
    destination: BackendHandle,
    destination_root: &str,
    delete_extra: bool,
    dry_run: bool,
) -> SyncResult {
    let (tx, rx) = crossbeam_channel::unbounded();
    let _handle = start_sync(
        source,
        source_root.to_string(),
        destination,
        destination_root.to_string(),
        SyncOptions {
            delete_extra,
            dry_run,
        },
        tx,
    );
    loop {
        match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(SyncMsg::Done(result)) => return result,
            Ok(SyncMsg::Progress(_)) => {}
            Err(error) => panic!("sync did not finish: {error}"),
        }
    }
}

/// Relative path → content of every file below `root`.
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                visit(root, &path, out);
            } else if meta.is_file() {
                let rel = forward(path.strip_prefix(root).unwrap());
                out.insert(rel, std::fs::read(path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

fn write(root: &Path, rel: &str, content: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// 3 folders × 2 subfolders × 4 files plus root files, sizes from empty to
/// beyond one copy block.
fn populate(root: &Path) {
    write(root, "empty.bin", b"");
    write(root, "big.bin", &[7u8].repeat(600 * 1024));
    for top in 0..3 {
        for sub in 0..2 {
            for file in 0..4 {
                let content = format!("{top}/{sub}/{file}").repeat(file * 50 + 1);
                write(
                    root,
                    &format!("top{top}/sub{sub}/file{file}.txt"),
                    content.as_bytes(),
                );
            }
        }
    }
}

fn same_stats(left: &SyncStats, right: &SyncStats) {
    assert_eq!(left.copied, right.copied);
    assert_eq!(left.skipped, right.skipped);
    assert_eq!(left.deleted, right.deleted);
    assert_eq!(left.bytes, right.bytes);
    assert_eq!(left.errors, right.errors);
}

#[test]
fn transfer_engine_task_sync_parallel_mirror_matches_serial_result() {
    let source = tempfile::tempdir().unwrap();
    let serial_target = tempfile::tempdir().unwrap();
    let parallel_target = tempfile::tempdir().unwrap();
    populate(source.path());
    // One connection limited to one transfer (the old serial pass) and one
    // left to its flow; each read takes a moment so overlap is visible.
    let serial = Arc::new(
        FakeRemote::new(source.path(), "serial")
            .with_ceiling(1)
            .with_delay(Duration::from_millis(20)),
    );
    let parallel =
        Arc::new(FakeRemote::new(source.path(), "parallel").with_delay(Duration::from_millis(20)));
    let run = |remote: &Arc<FakeRemote>, target: &Path| {
        let target_backend: BackendHandle = Arc::new(LocalBackend::new(&forward(target)));
        let remote_handle: BackendHandle = remote.clone();
        mirror(
            remote_handle,
            REMOTE_ROOT,
            target_backend,
            &forward(target),
            false,
            false,
        )
    };

    let first_serial = run(&serial, serial_target.path());
    let first_parallel = run(&parallel, parallel_target.path());
    assert!(first_serial.errors.is_empty(), "{:?}", first_serial.errors);
    assert!(
        first_parallel.errors.is_empty(),
        "{:?}",
        first_parallel.errors
    );
    assert_eq!(first_serial.stats.copied, 26);
    same_stats(&first_serial.stats, &first_parallel.stats);
    assert_eq!(tree(source.path()), tree(serial_target.path()));
    assert_eq!(tree(serial_target.path()), tree(parallel_target.path()));
    assert_eq!(serial.calls.peak_reads.load(Ordering::SeqCst), 1);
    assert!(parallel.calls.peak_reads.load(Ordering::SeqCst) >= 2);

    // Updates: a changed size, a new file in a new folder, the rest skipped.
    write(source.path(), "top1/sub0/file2.txt", b"changed");
    write(source.path(), "fresh/new.txt", b"new");
    let second_serial = run(&serial, serial_target.path());
    let second_parallel = run(&parallel, parallel_target.path());
    assert!(
        second_serial.errors.is_empty(),
        "{:?}",
        second_serial.errors
    );
    assert!(
        second_parallel.errors.is_empty(),
        "{:?}",
        second_parallel.errors
    );
    assert_eq!(second_serial.stats.copied, 2);
    assert_eq!(second_serial.stats.skipped, 25);
    same_stats(&second_serial.stats, &second_parallel.stats);
    assert_eq!(tree(source.path()), tree(parallel_target.path()));
    assert_eq!(tree(serial_target.path()), tree(parallel_target.path()));
}

#[test]
fn transfer_engine_task_sync_parallel_links_stay_protected_omissions() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("private.txt"), b"outside").unwrap();
    link_fixture::directory(outside.path(), &a.path().join("node_modules"));
    std::fs::create_dir(b.path().join("node_modules")).unwrap();
    std::fs::write(b.path().join("node_modules/keep.txt"), b"keep destination").unwrap();
    std::fs::create_dir(b.path().join("extra_parent")).unwrap();
    link_fixture::directory(outside.path(), &b.path().join("extra_parent/target_link"));
    std::fs::write(
        b.path().join("extra_parent/extra.txt"),
        b"delete extra sibling",
    )
    .unwrap();
    std::fs::create_dir(a.path().join("target_link")).unwrap();
    std::fs::write(a.path().join("target_link/not-outside.txt"), b"protected").unwrap();
    link_fixture::directory(outside.path(), &b.path().join("target_link"));
    for index in 0..12 {
        write(
            a.path(),
            &format!("ordinary/file{index}.txt"),
            b"ordinary sync",
        );
    }
    // Two remotes: both flows regulate, copies and listings run in parallel.
    let source =
        Arc::new(FakeRemote::new(a.path(), "links-a").with_delay(Duration::from_millis(5)));
    let destination = Arc::new(FakeRemote::new(b.path(), "links-b"));
    let run = |dry_run: bool| {
        let source_handle: BackendHandle = source.clone();
        let destination_handle: BackendHandle = destination.clone();
        mirror(
            source_handle,
            REMOTE_ROOT,
            destination_handle,
            REMOTE_ROOT,
            true,
            dry_run,
        )
    };

    let dry = run(true);
    assert!(dry.errors.is_empty(), "{:?}", dry.errors);
    assert_eq!(
        dry.omissions.reported_paths().collect::<Vec<_>>(),
        ["extra_parent/target_link", "node_modules", "target_link"]
    );
    assert!(!b.path().join("ordinary").exists());
    let result = run(false);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.stats.copied, 12);
    assert_eq!(
        result.omissions.reported_paths().collect::<Vec<_>>(),
        ["extra_parent/target_link", "node_modules", "target_link"]
    );
    assert_eq!(
        std::fs::read(b.path().join("node_modules/keep.txt")).unwrap(),
        b"keep destination"
    );
    assert!(!b.path().join("node_modules/private.txt").exists());
    assert!(!b.path().join("extra_parent/extra.txt").exists());
    assert!(b.path().join("extra_parent").exists());
    assert!(!outside.path().join("not-outside.txt").exists());
    assert_eq!(
        std::fs::read(outside.path().join("private.txt")).unwrap(),
        b"outside"
    );
    assert_eq!(
        tree(&a.path().join("ordinary")),
        tree(&b.path().join("ordinary"))
    );
    link_fixture::remove_directory(&a.path().join("node_modules"));
    link_fixture::remove_directory(&b.path().join("extra_parent/target_link"));
    link_fixture::remove_directory(&b.path().join("target_link"));
}

#[test]
fn transfer_engine_task_sync_copy_error_blocks_mirror_deletion() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    for index in 0..10 {
        write(a.path(), &format!("f{index}.txt"), b"content");
    }
    write(b.path(), "orphan.txt", b"keep me");
    write(b.path(), "orphan-dir/inner.txt", b"keep me too");
    let source: BackendHandle = Arc::new(LocalBackend::new(&forward(a.path())));
    let destination: BackendHandle =
        Arc::new(FakeRemote::new(b.path(), "failing").failing_writes_to("f3.txt"));

    let result = mirror(
        source,
        &forward(a.path()),
        destination,
        REMOTE_ROOT,
        true,
        false,
    );

    assert_eq!(result.stats.copied, 9);
    assert_eq!(result.stats.deleted, 0);
    assert!(result.stats.errors >= 2);
    assert!(result
        .errors
        .iter()
        .any(|(path, _)| path.ends_with("f3.txt")));
    assert!(result.errors.iter().any(|(_, message)| {
        message == "mirror deletion skipped because the copy/source pass reported errors"
    }));
    assert!(b.path().join("orphan.txt").exists());
    assert!(b.path().join("orphan-dir/inner.txt").exists());
    assert!(!b.path().join("f3.txt").exists());
    assert!(b.path().join("f9.txt").exists());
}

#[test]
fn transfer_engine_task_sync_lists_destination_once_instead_of_stat_per_file() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let mut files = Vec::new();
    for (folder, count) in [("", 20), ("sub/", 10), ("sub/deeper/", 5)] {
        for index in 0..count {
            let rel = format!("{folder}file{index}.txt");
            write(a.path(), &rel, rel.as_bytes());
            files.push(rel);
        }
    }
    let destination = Arc::new(FakeRemote::new(b.path(), "counting"));
    let run = || {
        let source: BackendHandle = Arc::new(LocalBackend::new(&forward(a.path())));
        let destination_handle: BackendHandle = destination.clone();
        mirror(
            source,
            &forward(a.path()),
            destination_handle,
            REMOTE_ROOT,
            false,
            false,
        )
    };

    let first = run();
    assert!(first.errors.is_empty(), "{:?}", first.errors);
    assert_eq!(first.stats.copied, 35);
    let calls = &destination.calls;
    // New folders are created once each and never listed; the root is
    // listed once. Before, every file cost a `stat` and a `mkdir_all`.
    assert_eq!(calls.mkdir_all.load(Ordering::SeqCst), 0);
    assert_eq!(calls.create_dir.load(Ordering::SeqCst), 2);
    assert_eq!(calls.list.load(Ordering::SeqCst), 1);
    for rel in &files {
        // Only the check right before publishing remains.
        assert_eq!(calls.stats_of(&format!("{REMOTE_ROOT}/{rel}")), 1, "{rel}");
    }

    calls.stat.store(0, Ordering::SeqCst);
    calls.list.store(0, Ordering::SeqCst);
    calls
        .stat_paths
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    let second = run();
    assert!(second.errors.is_empty(), "{:?}", second.errors);
    assert_eq!(second.stats.copied, 0);
    assert_eq!(second.stats.skipped, 35);
    // One listing per folder, no `stat` per file: only the root check.
    assert_eq!(calls.list.load(Ordering::SeqCst), 3);
    assert_eq!(calls.stat.load(Ordering::SeqCst), 1);
    assert_eq!(calls.mkdir_all.load(Ordering::SeqCst), 0);
}

#[test]
fn transfer_engine_task_sync_same_relative_paths_on_two_remotes_stay_separate() {
    let alpha_dir = tempfile::tempdir().unwrap();
    let beta_dir = tempfile::tempdir().unwrap();
    write(alpha_dir.path(), "shared.txt", b"from alpha, longer");
    write(alpha_dir.path(), "only-alpha/x.txt", b"x");
    write(beta_dir.path(), "shared.txt", b"beta");
    write(beta_dir.path(), "only-beta.txt", b"stays");
    let alpha = Arc::new(FakeRemote::new(alpha_dir.path(), "alpha"));
    let beta = Arc::new(FakeRemote::new(beta_dir.path(), "beta"));
    assert_ne!(
        crate::transfer::flow_for(&*alpha, REMOTE_ROOT).key(),
        crate::transfer::flow_for(&*beta, REMOTE_ROOT).key()
    );
    let alpha_handle: BackendHandle = alpha.clone();
    let beta_handle: BackendHandle = beta.clone();

    // `/data` on alpha and `/data` on beta: two locations, not one.
    let result = mirror(
        alpha_handle,
        REMOTE_ROOT,
        beta_handle,
        REMOTE_ROOT,
        false,
        false,
    );

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.stats.copied, 2);
    assert_eq!(
        std::fs::read(beta_dir.path().join("shared.txt")).unwrap(),
        b"from alpha, longer"
    );
    assert_eq!(
        std::fs::read(beta_dir.path().join("only-alpha/x.txt")).unwrap(),
        b"x"
    );
    assert!(beta_dir.path().join("only-beta.txt").exists());
    assert!(!alpha_dir.path().join("only-beta.txt").exists());
    assert_eq!(
        std::fs::read(alpha_dir.path().join("shared.txt")).unwrap(),
        b"from alpha, longer"
    );
}

#[test]
fn transfer_engine_task_sync_cancel_ends_parallel_pass_without_deletion() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    for index in 0..40 {
        write(a.path(), &format!("dir{}/f{index}.txt", index % 4), b"data");
    }
    write(b.path(), "orphan.txt", b"stays after cancel");
    // The first read cancels the run through its own handle (waiting until
    // the handle is known, so the test never races the worker).
    let slot: Arc<std::sync::Mutex<Option<Arc<AtomicBool>>>> =
        Arc::new(std::sync::Mutex::new(None));
    let hook_slot = slot.clone();
    let source = FakeRemote::new(a.path(), "cancel")
        .with_delay(Duration::from_millis(10))
        .with_hook(Arc::new(move |operation: &str, _path: &str| {
            if operation != "read" {
                return;
            }
            for _ in 0..1_000 {
                if let Some(flag) = hook_slot.lock().unwrap().as_ref() {
                    flag.store(true, Ordering::SeqCst);
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }));
    let source: BackendHandle = Arc::new(source);
    let destination: BackendHandle = Arc::new(LocalBackend::new(&forward(b.path())));
    let (tx, rx) = crossbeam_channel::unbounded();
    let handle = start_sync(
        source,
        REMOTE_ROOT.to_string(),
        destination,
        forward(b.path()),
        SyncOptions {
            delete_extra: true,
            dry_run: false,
        },
        tx,
    );
    *slot.lock().unwrap() = Some(handle.cancel.clone());
    let result = loop {
        match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(SyncMsg::Done(result)) => break result,
            Ok(SyncMsg::Progress(_)) => {}
            Err(error) => panic!("canceled sync did not finish: {error}"),
        }
    };
    assert!(handle.cancel.load(Ordering::SeqCst));
    assert_eq!(result.stats.deleted, 0);
    assert!(result.stats.copied < 40);
    assert_eq!(
        std::fs::read(b.path().join("orphan.txt")).unwrap(),
        b"stays after cancel"
    );
}
