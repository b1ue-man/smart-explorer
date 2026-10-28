//! M15 (2026-10-28) in the engine: a large move between two different block
//! devices asks for range threads like a copy; a move on one device (one
//! rename), or between sides of unknown devices, asks for none.
use super::extras_tests::Registered;
use super::*;
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
