//! The extra permits of a large local copy and what the engine hands the
//! copy: rules, never waiting, giving back, and the request that arrives.
use super::*;
use crate::transfer::flow;
use std::cell::Cell;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Per test: the folder whose copies are recorded and the drive kind they
/// are told to have. Engine workers run on other threads, so the table is
/// global and keyed by the (unique) folder.
struct Entry {
    prefixes: Vec<PathBuf>,
    rotation: Option<bool>,
    seen: Vec<usize>,
}

static ENTRIES: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

fn entries() -> std::sync::MutexGuard<'static, Vec<Entry>> {
    ENTRIES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Called with every request the engine hands to the copy.
pub(super) fn record(request: &LocalRequest<'_>) {
    for entry in entries().iter_mut() {
        if entry.prefixes.iter().any(|p| request.target.starts_with(p)) {
            entry.seen.push(request.parallel);
        }
    }
}

/// The drive kind a test forces for `target`, if it registered its folder.
pub(super) fn forced_rotation(target: &Path) -> Option<Option<bool>> {
    let entries = entries();
    let entry = entries
        .iter()
        .find(|entry| entry.prefixes.iter().any(|p| target.starts_with(p)))?;
    Some(entry.rotation)
}

struct Registered(PathBuf);

impl Registered {
    fn new(folder: &Path, rotation: Option<bool>) -> Self {
        let mut prefixes = vec![folder.to_path_buf()];
        prefixes.extend(std::fs::canonicalize(folder));
        entries().push(Entry {
            prefixes,
            rotation,
            seen: Vec::new(),
        });
        Self(folder.to_path_buf())
    }

    fn seen(&self) -> Vec<usize> {
        let entries = entries();
        entries
            .iter()
            .find(|entry| entry.prefixes.contains(&self.0))
            .map(|entry| entry.seen.clone())
            .unwrap_or_default()
    }
}

impl Drop for Registered {
    fn drop(&mut self) {
        entries().retain(|entry| !entry.prefixes.contains(&self.0));
    }
}

const MIB: u64 = 1024 * 1024;

fn completed() -> Result<LocalOutcome, LocalFailure> {
    Ok(LocalOutcome::Completed {
        bytes: 1,
        target: PathBuf::from("t"),
    })
}

fn in_flight(flow: &Arc<Flow>) -> usize {
    flow.snapshot().in_flight
}

#[test]
fn speed_task_local_extra_permits_only_when_the_queue_is_empty() {
    let size = 64 * MIB;
    // One thread per shortest part, the first is the operation's own.
    assert_eq!(extras_wanted(size, false, 0, || true), 7);
    assert_eq!(extras_wanted(size, false, 1, || true), 0, "work is waiting");
    // Nothing else waiting: the drive is asked only then.
    let asked = Cell::new(false);
    assert_eq!(
        extras_wanted(size, false, 3, || {
            asked.set(true);
            true
        }),
        0
    );
    assert!(!asked.get(), "the drive is not probed while work waits");
}

#[test]
fn speed_task_local_extra_permits_are_released_after_the_copy() {
    let one = flow(super::super::test_backend::unique("local-extra"), Some(8));
    let main = one.try_acquire().expect("the operation's own permit");
    for result in [
        completed(),
        Ok(LocalOutcome::Skipped),
        Ok(LocalOutcome::Canceled),
        Err(LocalFailure::Target(std::io::Error::other("disk full"))),
    ] {
        let extras = take_permits(&one, None, 4);
        assert_eq!(extras.count(), 1, "the flow starts at a limit of two");
        assert_eq!(in_flight(&one), 2);
        let accounts = one.snapshot().accounts;
        extras.finish(&result);
        assert_eq!(in_flight(&one), 1, "{result:?} gives the permit back");
        // Extras count permits only: no time is booked for them.
        assert_eq!(one.snapshot().accounts, accounts);
    }
    main.finish(OpOutcome::Done);
    assert_eq!(in_flight(&one), 0);
}

#[test]
fn speed_task_local_extra_permits_never_wait() {
    let one = flow(super::super::test_backend::unique("local-first"), Some(8));
    let other = flow(super::super::test_backend::unique("local-second"), Some(8));
    let held: Vec<_> = (0..2).filter_map(|_| other.try_acquire()).collect();
    assert_eq!(held.len(), 2, "the second flow is full");
    let main = one.try_acquire().expect("own permit");
    let began = Instant::now();
    // The first flow has a permit, the second none: the first goes back.
    let extras = take_permits(&one, Some(&other), 3);
    assert!(
        began.elapsed() < Duration::from_millis(500),
        "returned at once"
    );
    assert_eq!(extras.count(), 0);
    assert_eq!(
        in_flight(&one),
        1,
        "only the operation's own permit is held"
    );
    // The same flow on both sides is one flow: one permit per extra.
    let extras = take_permits(&one, Some(&one), 3);
    assert_eq!(extras.count(), 1);
    assert_eq!(in_flight(&one), 2);
    extras.finish(&Ok(LocalOutcome::Skipped));
    // Two flows with room: one permit on each per extra.
    for permit in held {
        permit.abandon();
    }
    let extras = take_permits(&one, Some(&other), 5);
    assert_eq!(extras.count(), 1);
    assert_eq!((in_flight(&one), in_flight(&other)), (2, 1));
    extras.finish(&completed());
    assert_eq!((in_flight(&one), in_flight(&other)), (1, 0));
    main.finish(OpOutcome::Done);
}

#[test]
fn speed_task_local_no_extras_for_small_files_or_moves() {
    assert_eq!(extras_wanted(SPLIT_FROM - 1, false, 0, || true), 0);
    assert_eq!(extras_wanted(SPLIT_FROM, false, 0, || true), 1);
    assert_eq!(
        extras_wanted(SPLIT_FROM + MIN_SEGMENT - 1, false, 0, || true),
        1
    );
    assert_eq!(
        extras_wanted(SPLIT_FROM + MIN_SEGMENT, false, 0, || true),
        2
    );
    assert_eq!(extras_wanted(64 * MIB, true, 0, || true), 0, "a move");
    assert_eq!(extras_wanted(64 * MIB, false, 0, || false), 0, "no flash");
    // Only a drive known to be flash counts; the rest is not parallel.
    let folder = tempfile::tempdir().expect("temp dir");
    let target = folder.path().join("big.bin");
    for (rotation, flash) in [(Some(true), false), (None, false), (Some(false), true)] {
        let _registered = Registered::new(folder.path(), rotation);
        assert_eq!(drive_is_flash(&target), flash, "{rotation:?}");
    }
}

#[test]
fn speed_task_local_engine_passes_the_granted_threads_to_the_copy() {
    use super::super::test_backend::{job, run, write};
    use crate::transfer::Endpoint;
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("src/big.bin");
    let content: Vec<u8> = (0..17 * MIB).map(|i| (i % 251) as u8).collect();
    write(&source, &content);
    // A drive that spins or is unknown: one thread, whatever is free.
    for (name, rotation) in [("spinning", Some(true)), ("unknown", None)] {
        let dest = root.path().join(name);
        std::fs::create_dir_all(&dest).expect("dest");
        let registered = Registered::new(&dest, rotation);
        let finished = run(job(Endpoint::Local, Endpoint::Local, &dest, &[&source]));
        assert!(finished.issues.is_empty(), "{:?}", finished.issues);
        assert_eq!(registered.seen(), [1], "{name}");
        assert!(std::fs::read(dest.join("big.bin")).expect("copy") == content);
    }
    // A flash drive: the permits that were free became threads (at most one
    // per part beyond the first), and the bytes are the same.
    let dest = root.path().join("flash");
    std::fs::create_dir_all(&dest).expect("dest");
    let registered = Registered::new(&dest, Some(false));
    let finished = run(job(Endpoint::Local, Endpoint::Local, &dest, &[&source]));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    let seen = registered.seen();
    assert_eq!(seen.len(), 1);
    assert!((1..=2).contains(&seen[0]), "{seen:?}");
    assert!(std::fs::read(dest.join("big.bin")).expect("copy") == content);
    assert_eq!(finished.progress.bytes_done, content.len() as u64);
    assert_eq!(std::fs::read(&source).expect("source").len(), content.len());
}
