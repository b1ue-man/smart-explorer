//! More local throughput cases (2026-10-28, M3/M15), next to
//! `speed_task_local_tests.rs` and printed the same way: the GUI paste of
//! small and medium files with local packets switched off (the comparison
//! for M3), the second paste of the same tree into the same folder ("keep
//! both": every file numbered, each with its own intent), and the durable
//! replacement of one large file, which copies with range threads (M15).
//!
//! All are ignored by default: `cargo test --release -- --ignored
//! speed_task_local_ --nocapture --test-threads=1` prints one
//! `BENCH <case> files_per_s=<n> mib_per_s=<n>` line per case.
use super::super::super::test_backend::{job, run, write};
use super::tests::PACKETS_OFF;
use crate::transfer::{Endpoint, FolderConflict, FolderConflictPolicy};
use crate::types::Conflict;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Instant;

/// `files` files of `bytes` each in `folders` folders below `root/tree`.
fn fill(root: &Path, files: usize, folders: usize, bytes: usize) -> PathBuf {
    let tree = root.join("tree");
    let block = vec![0x5Au8; bytes];
    for index in 0..files {
        let folder = tree.join(format!("d{:02}", index % folders.max(1)));
        write(&folder.join(format!("f{index:05}.bin")), &block);
    }
    tree
}

fn print(case: &str, files: usize, bytes: u64, seconds: f64) {
    let seconds = seconds.max(1e-9);
    println!(
        "BENCH {case} files_per_s={:.0} mib_per_s={:.1}",
        files as f64 / seconds,
        bytes as f64 / seconds / (1024.0 * 1024.0)
    );
}

/// Packets back on when a case ends, also when it fails.
struct PacketsOff;

impl PacketsOff {
    fn new() -> Self {
        PACKETS_OFF.store(true, Ordering::Release);
        PacketsOff
    }
}

impl Drop for PacketsOff {
    fn drop(&mut self) {
        PACKETS_OFF.store(false, Ordering::Release);
    }
}

/// The GUI paste (crash log, keep both) of a fresh tree, timed.
fn paste(name: &str, files: usize, folders: usize, bytes: usize) {
    let root = tempfile::tempdir().expect("temp dir");
    let tree = fill(root.path(), files, folders, bytes);
    let target = root.path().join("target");
    std::fs::create_dir(&target).expect("target");
    let started = Instant::now();
    let finished = run(job(Endpoint::Local, Endpoint::Local, &target, &[&tree]));
    let seconds = started.elapsed().as_secs_f64();
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, files as u64);
    print(name, files, (files * bytes) as u64, seconds);
}

#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture"]
fn speed_task_local_small_gui_without_packets() {
    let _off = PacketsOff::new();
    paste("local_5000x4k_gui_without_packets", 5000, 50, 4096);
}

#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture"]
fn speed_task_local_medium_gui_without_packets() {
    let _off = PacketsOff::new();
    paste("local_1000x1m_gui_without_packets", 1000, 10, 1024 * 1024);
}

/// The same tree pasted twice into one folder whose folder policy merges:
/// the second paste numbers every file ("keep both").
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture"]
fn speed_task_local_small_gui_second_paste_numbers_every_file() {
    let (files, bytes) = (5000, 4096);
    let root = tempfile::tempdir().expect("temp dir");
    let tree = fill(root.path(), files, 50, bytes);
    let target = root.path().join("target");
    std::fs::create_dir(&target).expect("target");
    let paste = || {
        let mut again = job(Endpoint::Local, Endpoint::Local, &target, &[&tree]);
        again.folders = FolderConflictPolicy::new(FolderConflict::Merge);
        run(again)
    };
    let first = paste();
    assert!(first.issues.is_empty(), "{:?}", first.issues);
    let started = Instant::now();
    let second = paste();
    let seconds = started.elapsed().as_secs_f64();
    assert!(second.issues.is_empty(), "{:?}", second.issues);
    assert_eq!(second.progress.files_done, files as u64);
    assert!(target.join("tree/d00/f00000 (2).bin").exists(), "numbered");
    print(
        "local_5000x4k_gui_second_paste",
        files,
        (files * bytes) as u64,
        seconds,
    );
}

/// One 2 GiB file replacing an older one: a durable copy (synced stage,
/// write-through publication) with the range threads of M15.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture"]
fn speed_task_local_large_overwrite() {
    let bytes: u64 = 2 * 1024 * 1024 * 1024;
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("big.bin");
    let mut file = std::fs::File::create(&source).expect("fixture");
    let block = vec![0x5Au8; 8 * 1024 * 1024];
    for _ in 0..bytes / block.len() as u64 {
        std::io::Write::write_all(&mut file, &block).expect("fixture bytes");
    }
    drop(file);
    let target = root.path().join("target");
    write(&target.join("big.bin"), b"older");
    let mut replace = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
    replace.conflict = Conflict::Overwrite;
    let started = Instant::now();
    let finished = run(replace);
    let seconds = started.elapsed().as_secs_f64();
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(
        std::fs::metadata(target.join("big.bin"))
            .expect("copy")
            .len(),
        bytes
    );
    print("local_1x2g_gui_overwrite", 1, bytes, seconds);
}
