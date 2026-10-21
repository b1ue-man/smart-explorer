//! The extra permits of a large local copy and what the engine hands the
//! copy: rules, never waiting, giving back, the measured start of a copy
//! around the page cache on a drive that is not known to be flash, and the
//! request that arrives.
use super::plan::{extras_wanted, plan, unbuffered, PROBE_FROM, UNBUFFERED_EXTRAS};
use super::*;
use crate::transfer::flow;
use crate::transfer::segments::MIN_SEGMENT;
use std::cell::Cell;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Per test: the folder whose copies are recorded and the drive kind they
/// are told to have. Engine workers run on other threads, so the table is
/// global and keyed by the (unique) folder.
struct Entry {
    prefixes: Vec<PathBuf>,
    rotation: Option<bool>,
    /// The device pair the folder's copies are told to run on (`None`: a
    /// side without block device).
    key: Option<DeviceKey>,
    /// Whether the folder's copies count as going around the page cache
    /// (what the memory of the host would say otherwise).
    unbuffered: bool,
    seen: Vec<usize>,
    probed: Vec<bool>,
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
            entry.probed.push(request.probe.is_some());
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

/// Whether a test makes the copies into a registered folder go around the
/// page cache.
pub(super) fn forced_unbuffered(target: &Path) -> Option<bool> {
    let entries = entries();
    let entry = entries
        .iter()
        .find(|entry| entry.prefixes.iter().any(|p| target.starts_with(p)))?;
    Some(entry.unbuffered)
}

/// The device pair a test forces for a copy into a registered folder.
pub(super) fn forced_key(target: &Path) -> Option<Option<DeviceKey>> {
    let entries = entries();
    let entry = entries
        .iter()
        .find(|entry| entry.prefixes.iter().any(|p| target.starts_with(p)))?;
    Some(entry.key)
}

struct Registered(PathBuf);

impl Registered {
    fn new(folder: &Path, rotation: Option<bool>) -> Self {
        Self::on(folder, rotation, None)
    }

    fn on(folder: &Path, rotation: Option<bool>, key: Option<DeviceKey>) -> Self {
        let mut prefixes = vec![folder.to_path_buf()];
        prefixes.extend(std::fs::canonicalize(folder));
        entries().push(Entry {
            prefixes,
            rotation,
            key,
            unbuffered: false,
            seen: Vec::new(),
            probed: Vec::new(),
        });
        Self(folder.to_path_buf())
    }

    /// Makes the folder's copies count as going around the page cache.
    fn unbuffered(self) -> Self {
        for entry in entries().iter_mut() {
            if entry.prefixes.contains(&self.0) {
                entry.unbuffered = true;
            }
        }
        self
    }

    fn probed(&self) -> Vec<bool> {
        let entries = entries();
        entries
            .iter()
            .find(|entry| entry.prefixes.contains(&self.0))
            .map(|entry| entry.probed.clone())
            .unwrap_or_default()
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
    assert_eq!(extras_wanted(size, false, 0), 7);
    assert_eq!(extras_wanted(size, false, 1), 0, "work is waiting");
    // Nothing else waiting: the drive is asked only then.
    let asked = Cell::new(false);
    let flash = || {
        asked.set(true);
        Drive::Flash
    };
    assert_eq!(
        plan(size, false, 3, false, flash, || Memory::Untested),
        Plan::Single
    );
    assert!(!asked.get(), "the drive is not probed while work waits");
    assert_eq!(
        plan(size, false, 0, false, flash, || Memory::Untested),
        Plan::Ranges(7)
    );
    assert!(asked.get());
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
    assert_eq!(extras_wanted(SPLIT_FROM - 1, false, 0), 0);
    assert_eq!(extras_wanted(SPLIT_FROM, false, 0), 1);
    assert_eq!(extras_wanted(SPLIT_FROM + MIN_SEGMENT - 1, false, 0), 1);
    assert_eq!(extras_wanted(SPLIT_FROM + MIN_SEGMENT, false, 0), 2);
    assert_eq!(extras_wanted(64 * MIB, true, 0), 0, "a move");
    // Neither a small file nor a move asks the drive or the memory.
    for (size, is_move) in [(SPLIT_FROM - 1, false), (512 * MIB, true)] {
        let planned = plan(
            size,
            is_move,
            0,
            true,
            || unreachable!("the drive is not asked"),
            || unreachable!("the memory is not asked"),
        );
        assert_eq!(planned, Plan::Single, "{size} {is_move}");
    }
    // What the system says of the drive under the target.
    let folder = tempfile::tempdir().expect("temp dir");
    let target = folder.path().join("big.bin");
    for (rotation, drive) in [
        (Some(true), Drive::Spinning),
        (None, Drive::Unknown),
        (Some(false), Drive::Flash),
    ] {
        let _registered = Registered::new(folder.path(), rotation);
        assert_eq!(drive_of(&target), drive, "{rotation:?}");
    }
}

#[test]
fn speed_task_local_unbuffered_rule_is_a_quarter_of_the_available_memory() {
    let memory = Some(16 * 1024 * MIB);
    assert!(unbuffered(4 * 1024 * MIB, memory), "exactly a quarter");
    assert!(!unbuffered(4 * 1024 * MIB - 1, memory), "one byte below");
    assert!(!unbuffered(100 * 1024 * MIB, None), "unknown memory: never");
}

#[test]
fn speed_task_local_probe_is_planned_only_for_a_direct_copy_on_a_drive_not_known_flash() {
    let size = 256 * MIB;
    // A drive known to be flash needs no measurement and no memory. A buffered
    // copy gets every thread its parts allow; a copy around the page cache
    // runs 4 threads in all (three beyond its own), however large the file.
    let planned = plan(
        size,
        false,
        0,
        false,
        || Drive::Flash,
        || unreachable!("not asked"),
    );
    assert_eq!(planned, Plan::Ranges(31));
    let planned = plan(
        size,
        false,
        0,
        true,
        || Drive::Flash,
        || unreachable!("not asked"),
    );
    assert_eq!(planned, Plan::Ranges(UNBUFFERED_EXTRAS));
    // Spinning or unknown: a buffered copy stays on one thread whatever the
    // memory says (its writes are queued deep by the kernel anyway, and the
    // probe's syncs cost seconds); a direct copy without a verdict measures,
    // with the same permits.
    for drive in [Drive::Spinning, Drive::Unknown] {
        for memory in [
            Memory::Untested,
            Memory::Probing,
            Memory::Hurts,
            Memory::Unavailable,
            Memory::Helps,
        ] {
            let planned = plan(size, false, 0, false, || drive, || memory);
            assert_eq!(planned, Plan::Single, "buffered {drive:?} {memory:?}");
        }
        let table = [
            (Memory::Untested, Plan::Probe(3)),
            (Memory::Probing, Plan::Single),
            (Memory::Hurts, Plan::Single),
            (Memory::Unavailable, Plan::Single),
            (Memory::Helps, Plan::Ranges(3)),
        ];
        for (memory, expected) in table {
            let planned = plan(size, false, 0, true, || drive, || memory);
            assert_eq!(planned, expected, "{drive:?} {memory:?}");
        }
    }
    // Only a file the probe fits into is measured (two sections of 32 MiB at
    // most half of it); a smaller one stays on one thread, a known verdict
    // still applies to it.
    let spinning = || Drive::Spinning;
    let spinning_untested = |size| plan(size, false, 0, true, spinning, || Memory::Untested);
    assert_eq!(spinning_untested(PROBE_FROM - 1), Plan::Single);
    assert_eq!(spinning_untested(PROBE_FROM), Plan::Probe(3));
    assert_eq!(
        plan(PROBE_FROM - 1, false, 0, true, spinning, || Memory::Helps),
        Plan::Ranges(3)
    );
    // Work waiting, a move and a file below two parts never measure.
    assert_eq!(
        plan(size, false, 1, true, spinning, || Memory::Untested),
        Plan::Single
    );
    assert_eq!(
        plan(size, true, 0, true, spinning, || Memory::Untested),
        Plan::Single
    );
    assert_eq!(
        plan(SPLIT_FROM - 1, false, 0, true, spinning, || {
            Memory::Untested
        }),
        Plan::Single
    );
}

#[test]
fn speed_task_local_probe_verdict_is_remembered_per_device_pair() {
    let verdicts = Verdicts::new();
    let (one, other) = ((1, 2), (2, 1));
    assert_eq!(verdicts.memory(one), Memory::Untested);
    let ticket = verdicts.claim(one).expect("first claim");
    assert_eq!(verdicts.memory(one), Memory::Probing);
    assert!(verdicts.claim(one).is_none(), "one measurement at a time");
    // Copying the other way round is another pair.
    let reverse = verdicts.claim(other).expect("other pair");
    assert_eq!(verdicts.memory(other), Memory::Probing);
    ticket.record(true);
    assert_eq!(verdicts.memory(one), Memory::Helps);
    drop(ticket);
    assert_eq!(verdicts.memory(one), Memory::Helps, "the verdict stays");
    assert!(verdicts.claim(one).is_none(), "and is not measured again");
    // A ticket that ends without a verdict (canceled, failed, no second
    // thread) gives the claim back.
    drop(reverse);
    assert_eq!(verdicts.memory(other), Memory::Untested);
    let again = verdicts.claim(other).expect("claim given back");
    // The first verdict is fixed for the process.
    again.record(false);
    again.record(true);
    assert_eq!(verdicts.memory(other), Memory::Hurts);
    drop(again);
    assert_eq!(verdicts.memory(other), Memory::Hurts);
    assert_eq!(verdicts.memory(one), Memory::Helps, "pairs do not mix");
    // What is remembered decides the next plan of the pair.
    let planned = |key| {
        plan(
            256 * MIB,
            false,
            0,
            true,
            || Drive::Spinning,
            || verdicts.memory(key),
        )
    };
    assert_eq!(planned(one), Plan::Ranges(3));
    assert_eq!(planned(other), Plan::Single);
    assert_eq!(planned((3, 3)), Plan::Probe(3));
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

/// Small files never ask for extra threads, not even on flash with nothing
/// else queued: every copy gets exactly its own permit's thread.
#[test]
fn speed_task_local_every_small_file_runs_on_one_thread() {
    use super::super::test_backend::{job, run, write};
    use crate::transfer::Endpoint;
    let root = tempfile::tempdir().expect("temp dir");
    let sources: Vec<PathBuf> = (0..40)
        .map(|index| {
            let path = root.path().join(format!("src/small-{index}.bin"));
            write(&path, &vec![index as u8; 1024]);
            path
        })
        .collect();
    let dest = root.path().join("flash");
    std::fs::create_dir_all(&dest).expect("dest");
    let registered = Registered::new(&dest, Some(false));
    let refs: Vec<&Path> = sources.iter().map(PathBuf::as_path).collect();
    let finished = run(job(Endpoint::Local, Endpoint::Local, &dest, &refs));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(registered.seen(), vec![1; 40]);
    for index in 0..40 {
        let copy = std::fs::read(dest.join(format!("small-{index}.bin"))).expect("copy");
        assert_eq!(copy, vec![index as u8; 1024]);
    }
}

/// A probe-sized file on a drive that says "rotating", both sides on block
/// devices, copied around the page cache: the first copy of the pair is
/// measured when the flow had a permit to spare. Whether a verdict comes out
/// depends on the host (the copy decides again whether it can go direct, else
/// it copies plainly and leaves the pair untested); a verdict is kept and not
/// measured again; the bytes are the same and counted once.
#[test]
fn speed_task_local_engine_measures_a_direct_copy_on_a_drive_that_says_rotating() {
    use super::super::test_backend::{job, run, write};
    use crate::transfer::Endpoint;
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("src/big.bin");
    let content: Vec<u8> = (0..PROBE_FROM)
        .map(|i| (i.wrapping_mul(7) ^ (i >> 12)) as u8)
        .collect();
    write(&source, &content);
    // A device pair no other test or real file system has.
    let key = (u64::MAX - 41, u64::MAX - 42);
    let mut first = None;
    for name in ["first", "second"] {
        let dest = root.path().join(name);
        std::fs::create_dir_all(&dest).expect("dest");
        let registered = Registered::on(&dest, Some(true), Some(key)).unbuffered();
        let finished = run(job(Endpoint::Local, Endpoint::Local, &dest, &[&source]));
        assert!(finished.issues.is_empty(), "{:?}", finished.issues);
        assert!(std::fs::read(dest.join("big.bin")).expect("copy") == content);
        assert_eq!(finished.progress.bytes_done, content.len() as u64);
        let (seen, probed) = (registered.seen(), registered.probed());
        assert_eq!((seen.len(), probed.len()), (1, 1), "{name}");
        let memory = VERDICTS.memory(key);
        if name == "first" {
            // With a permit to spare the copy was asked to measure; otherwise
            // it ran on one thread and left the pair untested.
            assert_eq!(probed[0], seen[0] > 1, "{seen:?}");
            if !probed[0] {
                assert_eq!(memory, Memory::Untested);
            }
            first = Some((probed[0], memory));
        } else if let Some((true, Memory::Helps | Memory::Hurts)) = first {
            assert_eq!(probed, [false], "a verdict is not measured again");
            match memory {
                // Ranges again when the flow has permits to spare.
                Memory::Helps => assert!(seen[0] >= 1, "{seen:?}"),
                _ => assert_eq!(seen, [1], "does not help: one thread"),
            }
            assert_eq!(Some(memory), first.map(|(_, memory)| memory), "fixed");
        }
    }
}
