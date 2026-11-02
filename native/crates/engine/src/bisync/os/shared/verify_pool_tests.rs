//! The planner's content check runs on adaptive worker pools (`verify_pool`).
use super::{pool, run_side, Job, Pace};
use crate::bisync::{self, Action, BisyncOptions, Sig, WalkFilter};
use crate::vfs::LocalBackend;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

/// Limits `Pace` chooses for a stage whose throughput at `n` workers is `f(n)`.
fn paced(start: usize, most: usize, windows: usize, f: impl Fn(usize) -> f64) -> Vec<usize> {
    let mut pace = Pace::new(start, most);
    let mut limit = start;
    let mut seen = vec![limit];
    for _ in 0..windows {
        limit = pace.next(f(limit));
        seen.push(limit);
    }
    seen
}

/// A stage that scales up to six workers: the count doubles first ([n] =>
/// [2n]), falls back once more brings nothing and then stays around six.
#[test]
fn ideal_task_pace_doubles_while_throughput_rises_and_settles_at_the_best_count() {
    let seen = paced(4, 32, 40, |n| 100.0 * n.min(6) as f64);
    assert_eq!(&seen[..3], &[4, 8, 16], "{seen:?}");
    assert!(
        seen.iter().all(|limit| (1..=32).contains(limit)),
        "{seen:?}"
    );
    assert!(
        seen[12..].iter().all(|limit| (5..=8).contains(limit)),
        "{seen:?}"
    );
}

/// A seeking disk: more than two readers cost throughput. The count halves
/// back ([2n] => [n/2]) and settles around two.
#[test]
fn ideal_task_pace_halves_when_more_workers_bring_less() {
    let disk = |n: usize| match n {
        1 => 80.0,
        2 => 100.0,
        n => (100.0 - 15.0 * (n as f64 - 2.0)).max(5.0),
    };
    let seen = paced(4, 32, 40, disk);
    assert_eq!(&seen[..3], &[4, 8, 4], "{seen:?}");
    assert!(
        seen[8..].iter().all(|limit| (1..=3).contains(limit)),
        "{seen:?}"
    );
}

/// Every item is worked on once, several at a time, and a canceled stage
/// takes no further item.
#[test]
fn ideal_task_pool_works_every_item_once_in_parallel() {
    let items: Vec<usize> = (0..64).collect();
    let running = AtomicUsize::new(0);
    let most = AtomicUsize::new(0);
    let cancel = AtomicBool::new(false);
    let mut done = pool(
        &items,
        |_| 1,
        &cancel,
        |item| {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            most.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(20));
            running.fetch_sub(1, Ordering::SeqCst);
            *item
        },
    );
    done.sort_unstable();
    assert_eq!(done, items);
    assert!(most.load(Ordering::SeqCst) >= 2, "one worker at a time");

    let canceled = AtomicBool::new(true);
    let none = pool(&items, |_| 1, &canceled, |item| *item);
    assert!(none.is_empty(), "{none:?}");
}

fn write(path: &std::path::Path, data: &[u8], mtime: SystemTime) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("folder");
    std::fs::write(path, data).expect("write");
    std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(mtime))
        .expect("time");
}

fn data(seed: usize, size: usize) -> Vec<u8> {
    (0..size)
        .map(|index| ((index * 31 + seed * 7) % 251) as u8)
        .collect()
}

/// A local side: every file gets its MD5 from the check, small files and
/// files larger than one read-ahead chunk alike.
#[test]
fn ideal_task_local_side_checks_every_file() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = directory.path().to_string_lossy().replace('\\', "/");
    let now = SystemTime::now();
    let sizes = [0, 1, 4096, 1 << 20, (1 << 20) + 1, 3 * (1 << 20) + 17];
    let mut jobs = Vec::new();
    let mut expected = Vec::new();
    for (key, size) in sizes.iter().enumerate() {
        let path = format!("{root}/d{}/f{key}.bin", key % 2);
        let content = data(key, *size);
        write(std::path::Path::new(&path), &content, now);
        expected.push(super::super::snapshot::md5_hex_to_u64(&format!(
            "{:x}",
            md5::compute(&content)
        )));
        jobs.push(Job {
            key,
            path,
            planned: Sig {
                size: *size as u64,
                mtime_ms: 0,
                hash: 0,
            },
        });
    }
    let backend = LocalBackend::new(&root);
    let cancel = AtomicBool::new(false);
    let check =
        |path: &str, _planned: Sig| super::super::snapshot_hash::hash_file(&backend, path, &cancel);
    let mut results = run_side(&backend, jobs, &check, &cancel);
    results.sort_by_key(|(key, _)| *key);
    assert_eq!(results.len(), sizes.len());
    for (key, result) in results {
        assert_eq!(result.expect("checked"), expected[key], "file {key}");
    }
}

/// The case of the report (2026-11-01): two copies of one tree whose times
/// differ and no stored state. Every file pair is read on both sides at the
/// same time; equal files need nothing, the one that differs at equal size
/// is a conflict.
#[test]
fn ideal_task_first_compare_of_copies_with_other_times_decides_by_content() {
    let a = tempfile::tempdir().expect("a");
    let b = tempfile::tempdir().expect("b");
    let root_a = a.path().to_string_lossy().replace('\\', "/");
    let root_b = b.path().to_string_lossy().replace('\\', "/");
    let old = SystemTime::now() - Duration::from_secs(86_400);
    let new = old + Duration::from_secs(4_000);
    for index in 0..48 {
        let size = if index % 12 == 0 { 3 << 20 } else { 64 << 10 };
        let content = data(index, size);
        let rel = format!("d{}/f{index:02}.bin", index % 4);
        write(&a.path().join(&rel), &content, old);
        write(&b.path().join(&rel), &content, new);
    }
    let size = 128 << 10;
    write(&a.path().join("differs.bin"), &data(1, size), old);
    write(&b.path().join("differs.bin"), &data(2, size), new);

    let ignore = bisync::empty_globset();
    let preview = bisync::preview(
        &LocalBackend::new(&root_a),
        &root_a,
        &LocalBackend::new(&root_b),
        &root_b,
        BisyncOptions::default(),
        &AtomicBool::new(false),
        &WalkFilter::basic(true, &ignore),
    );
    assert_eq!(preview.error, None);
    assert_eq!((preview.a_files, preview.b_files), (49, 49));
    let mut reported: Vec<&str> = preview
        .conflicts
        .iter()
        .map(|conflict| conflict.rel.as_str())
        .chain(preview.actions.iter().map(|action| match action {
            Action::CopyAtoB(rel)
            | Action::CopyBtoA(rel)
            | Action::FinalizeMoveAtoB(rel)
            | Action::FinalizeMoveBtoA(rel)
            | Action::DeleteA(rel)
            | Action::DeleteB(rel)
            | Action::KeepBothAtoB(rel)
            | Action::KeepBothBtoA(rel) => rel.as_str(),
        }))
        .collect();
    reported.sort_unstable();
    reported.dedup();
    assert_eq!(reported, ["differs.bin"], "{:?}", preview.actions);
    assert!(preview.omissions.is_empty(), "{:?}", preview.omissions);
}
