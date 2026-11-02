//! Durable copies with range threads (M15, 2026-10-28): a move to another
//! volume and a replacement get the threads the engine granted, the stage is
//! synced and published whole, and the bytes arrive exactly. "Another volume"
//! is the rename's EXDEV answer, given here without renaming
//! (`rename_reply`), so the staged path runs on any test machine; where the
//! runner has a second file system (`/dev/shm` on Linux) a real one runs too.
use super::*;
use std::cell::{Cell, RefCell};

thread_local! {
    static SEEN: RefCell<Vec<(usize, bool)>> = const { RefCell::new(Vec::new()) };
    static CROSS_DEVICE: Cell<bool> = const { Cell::new(false) };
}

/// Called by the copy with the threads its stage may use and whether it is
/// durable.
pub(super) fn saw(threads: usize, durable: bool) {
    SEEN.with(|seen| seen.borrow_mut().push((threads, durable)));
}

/// The same-volume rename of a move, or EXDEV while a test says the target
/// is on another volume (nothing is renamed then).
pub(super) fn rename_reply(rename: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    if CROSS_DEVICE.with(Cell::get) {
        return Err(io::Error::new(
            io::ErrorKind::CrossesDevices,
            "another volume (test)",
        ));
    }
    rename()
}

/// Every rename of this thread answers EXDEV while the guard lives.
struct AnotherVolume;

impl AnotherVolume {
    fn new() -> Self {
        CROSS_DEVICE.with(|flag| flag.set(true));
        AnotherVolume
    }
}

impl Drop for AnotherVolume {
    fn drop(&mut self) {
        CROSS_DEVICE.with(|flag| flag.set(false));
    }
}

const MIB: usize = 1024 * 1024;

/// 40 MiB plus a tail that is no multiple of a range border: every thread
/// gets a range of its own and the last one a ragged end.
fn content() -> Vec<u8> {
    (0..40 * MIB + 4093)
        .map(|index| (index.wrapping_mul(31) ^ (index >> 13)) as u8)
        .collect()
}

fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn run(
    source: &Path,
    target: &Path,
    conflict: Conflict,
    mode: CopyMode,
    parallel: usize,
) -> (LocalOutcome, u64, Vec<(usize, bool)>) {
    SEEN.with(|seen| seen.borrow_mut().clear());
    let cancel = AtomicBool::new(false);
    let request = LocalRequest {
        source,
        target,
        conflict,
        mode,
        cancel: &cancel,
        parallel,
        probe: None,
    };
    let mut streamed = 0u64;
    let mut progress = |bytes: u64| streamed += bytes;
    let outcome = transfer_local(&request, &mut progress).expect("transfer");
    (outcome, streamed, SEEN.with(|seen| seen.borrow().clone()))
}

#[test]
fn speed_task_local_durable_move_to_another_volume_uses_range_threads_byte_exact() {
    let root = tempfile::tempdir().unwrap();
    let (from, to) = (root.path().join("from"), root.path().join("to"));
    std::fs::create_dir(&from).unwrap();
    std::fs::create_dir(&to).unwrap();
    let source = from.join("big.bin");
    let target = to.join("big.bin");
    let bytes = content();
    std::fs::write(&source, &bytes).unwrap();
    let (outcome, streamed, seen) = {
        let _volume = AnotherVolume::new();
        run(&source, &target, Conflict::Rename, CopyMode::Move, 4)
    };
    let LocalOutcome::Completed {
        bytes: done,
        target: at,
    } = &outcome
    else {
        panic!("{outcome:?}");
    };
    assert_eq!((*done, at), (bytes.len() as u64, &target));
    // The staged path ran (the rename answered EXDEV), durable, with every
    // granted thread; the bytes streamed once.
    assert_eq!(seen, [(4, true)]);
    assert_eq!(streamed, bytes.len() as u64);
    assert!(std::fs::read(&target).unwrap() == bytes, "byte-exact");
    // The source left only after the published copy was durable; no stage,
    // quarantine or alias is left on either side.
    assert!(names(&from).is_empty(), "{:?}", names(&from));
    assert_eq!(names(&to), ["big.bin"]);
}

#[test]
fn speed_task_local_durable_replacement_uses_range_threads_byte_exact() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("new.bin");
    let target = root.path().join("old.bin");
    let bytes = content();
    std::fs::write(&source, &bytes).unwrap();
    std::fs::write(&target, b"the older file").unwrap();
    let (outcome, _, seen) = run(&source, &target, Conflict::Overwrite, CopyMode::Copy, 3);
    assert!(
        matches!(outcome, LocalOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert_eq!(seen, [(3, true)]);
    assert!(std::fs::read(&target).unwrap() == bytes, "byte-exact");
    assert!(std::fs::read(&source).unwrap() == bytes, "source untouched");
    assert_eq!(names(root.path()), ["new.bin", "old.bin"]);
}

/// A new copy keeps what it had: the threads it was granted, not durable.
#[test]
fn speed_task_local_durable_new_copy_keeps_its_threads() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.bin");
    std::fs::write(&source, b"small").unwrap();
    let (_, _, seen) = run(
        &source,
        &root.path().join("b.bin"),
        Conflict::Rename,
        CopyMode::Copy,
        2,
    );
    assert_eq!(seen, [(2, false)]);
}

/// With a real second file system on the runner: the same move, renamed by
/// nobody, byte-exact (prints why it did not run otherwise).
#[cfg(target_os = "linux")]
#[test]
fn speed_task_local_durable_move_across_real_file_systems_is_byte_exact() {
    use std::os::unix::fs::MetadataExt;
    let root = tempfile::tempdir().unwrap();
    let shm = Path::new("/dev/shm");
    let (Ok(here), Ok(there)) = (std::fs::metadata(root.path()), std::fs::metadata(shm)) else {
        println!("skipped: no /dev/shm");
        return;
    };
    if here.dev() == there.dev() {
        println!("skipped: /dev/shm is the same file system");
        return;
    }
    let other = tempfile::tempdir_in(shm).unwrap();
    let source = root.path().join("big.bin");
    let target = other.path().join("big.bin");
    let bytes = content();
    std::fs::write(&source, &bytes).unwrap();
    let (outcome, _, seen) = run(&source, &target, Conflict::Rename, CopyMode::Move, 4);
    assert!(
        matches!(outcome, LocalOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert_eq!(seen, [(4, true)]);
    assert!(std::fs::read(&target).unwrap() == bytes, "byte-exact");
    assert!(!source.exists(), "moved");
    assert_eq!(names(other.path()), ["big.bin"]);
}
