//! M15 (2026-10-28) in the engine: a large move between two different block
//! devices asks for range threads like a copy; a move on one device (one
//! rename), or between sides of unknown devices, asks for none.
use super::extras_tests::{not_asked, Registered};
use super::plan::extras_wanted;
use super::*;
use crate::transfer::segments::MIN_SEGMENT;
use crate::types::CopyMode;

const MIB: u64 = 1024 * 1024;

#[test]
fn speed_task_local_move_crosses_volumes_only_between_two_known_devices() {
    assert!(crosses_volumes(Some((1, 2))));
    assert!(!crosses_volumes(Some((3, 3))), "one device: a rename");
    assert!(!crosses_volumes(None), "a side without block device");
    // A move across volumes plans like a copy of the same size.
    let (flash, untested) = (|| Drive::Flash, || Memory::Untested);
    let planned = plan::plan(64 * MIB, false, 0, false, flash, untested);
    assert_eq!(planned, Plan::Ranges(7));
}

/// The engine hands a move the threads its device pair allows; the bytes
/// arrive whole either way (the runner renames on its one file system, the
/// copy module's own tests cover the staged path across volumes).
#[test]
fn speed_task_local_move_engine_grants_threads_only_across_volumes() {
    use super::super::test_backend::{job, run, write};
    use crate::transfer::Endpoint;
    let root = tempfile::tempdir().expect("temp dir");
    let content: Vec<u8> = (0..17 * MIB).map(|i| (i % 253) as u8).collect();
    let same = (u64::MAX - 51, u64::MAX - 51);
    for (name, key) in [("same", same), ("cross", (1, 2))] {
        let source = root.path().join(format!("src-{name}/big.bin"));
        write(&source, &content);
        let dest = root.path().join(name);
        std::fs::create_dir_all(&dest).expect("dest");
        // A flash drive: a copy of this size would get every free permit.
        let registered = Registered::on(&dest, Some(false), Some(key));
        let mut moving = job(Endpoint::Local, Endpoint::Local, &dest, &[&source]);
        moving.mode = CopyMode::Move;
        let finished = run(moving);
        assert!(finished.issues.is_empty(), "{:?}", finished.issues);
        assert!(std::fs::read(dest.join("big.bin")).expect("moved") == content);
        assert!(!source.exists(), "{name}: moved");
        let seen = registered.seen();
        assert_eq!(seen.len(), 1, "{name}");
        if name == "same" {
            assert_eq!(seen, [1], "a rename needs no thread");
        } else {
            // One extra per part beyond the first (17 MiB: one), when the
            // flow had it free.
            assert!((1..=2).contains(&seen[0]), "{seen:?}");
        }
    }
}

/// Pinned, deliberately changed by M15 (2026-10-28): a move that stays one
/// rename on its volume gets no extras; a move to another volume is a durable
/// copy whose stage is synced whole before publication, so it asks like a copy.
#[test]
fn speed_task_local_no_extras_for_small_files_or_moves() {
    assert_eq!(extras_wanted(SPLIT_FROM - 1, false, 0), 0);
    assert_eq!(extras_wanted(SPLIT_FROM, false, 0), 1);
    assert_eq!(extras_wanted(SPLIT_FROM + MIN_SEGMENT - 1, false, 0), 1);
    assert_eq!(extras_wanted(SPLIT_FROM + MIN_SEGMENT, false, 0), 2);
    assert_eq!(extras_wanted(64 * MIB, true, 0), 0, "a move on one volume");
    assert_eq!(
        extras_wanted(64 * MIB, false, 0),
        7,
        "a move across volumes"
    );
    // Neither a small file nor a renaming move asks the drive or the memory.
    for (size, renames) in [(SPLIT_FROM - 1, false), (512 * MIB, true)] {
        let planned = plan::plan(size, renames, 0, true, not_asked, not_asked);
        assert_eq!(planned, Plan::Single, "{size} {renames}");
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
