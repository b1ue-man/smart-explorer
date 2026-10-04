//! Sync modes through preview and complete runs; option families share this test namespace.
use super::sync_reliability_task_fixture::{clean, forward, Pair, TIME};
use super::*;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

#[test]
fn sync_reliability_task_options_directions_preview_changes_and_noop() {
    for direction in [Direction::AtoB, Direction::BtoA, Direction::Both] {
        let pair = Pair::new();
        pair.put(PairSide::A, "a.txt", b"A initial", TIME);
        pair.put(PairSide::B, "b.txt", b"B initial", TIME);
        let opts = BisyncOptions { direction, compare: CompareMode::Checksum, ..forward() };
        let globs = empty_globset();
        let filter = WalkFilter::basic(true, &globs);
        let planned = pair.preview(opts, &filter);
        assert!(planned.error.is_none() && planned.conflicts.is_empty() && planned.blocked.is_none());
        let expected = match direction { Direction::Both => 2, Direction::AtoB | Direction::BtoA => 1 };
        assert_eq!(planned.actions.len(), expected);
        assert!(!pair.path(PairSide::A, "b.txt").exists());
        assert!(!pair.path(PairSide::B, "a.txt").exists());
        let dry = pair.run(BisyncOptions { dry_run: true, ..opts }, &filter);
        clean(&dry);
        for side in [PairSide::A, PairSide::B] {
            assert!(!pair.path(side, REPLICA_MARKER_NAME).exists());
        }
        assert!(!pair.path(PairSide::A, "b.txt").exists());
        assert!(!pair.path(PairSide::B, "a.txt").exists());
        let first = pair.run(opts, &filter);
        clean(&first);
        assert_eq!(first.stats.a_to_b + first.stats.b_to_a, expected as u64);
        assert_eq!(pair.stored(&first), first.baseline);
        if direction != Direction::BtoA {
            assert_eq!(pair.bytes(PairSide::B, "a.txt"), b"A initial");
            pair.put(PairSide::A, "a.txt", b"A changed completely", TIME + 10_000);
        }
        if direction != Direction::AtoB {
            assert_eq!(pair.bytes(PairSide::A, "b.txt"), b"B initial");
            pair.put(PairSide::B, "b.txt", b"B changed completely", TIME + 20_000);
        }
        let changed = pair.run(opts, &filter);
        clean(&changed);
        assert_eq!(changed.stats.a_to_b + changed.stats.b_to_a, expected as u64);
        if direction != Direction::BtoA { assert_eq!(pair.bytes(PairSide::B, "a.txt"), b"A changed completely"); }
        if direction != Direction::AtoB { assert_eq!(pair.bytes(PairSide::A, "b.txt"), b"B changed completely"); }
        assert_eq!(pair.versions(&changed).len(), expected);
        pair.no_op(opts, &filter);
    }
}

#[test]
fn sync_reliability_task_options_all_conflicts_preserve_losing_bytes() {
    for conflict in ConflictMode::ALL {
        let pair = Pair::new();
        let globs = empty_globset();
        let filter = WalkFilter::basic(true, &globs);
        let opts = BisyncOptions { direction: Direction::Both, conflict,
            compare: CompareMode::Checksum, ..forward() };
        pair.put(PairSide::A, "file.txt", b"original", TIME);
        let seeded = pair.run(opts, &filter);
        clean(&seeded);
        pair.put(PairSide::A, "file.txt", b"alpha-newer-long", TIME + 20_000);
        pair.put(PairSide::B, "file.txt", b"bravo", TIME + 10_000);
        let preview = pair.preview(opts, &filter);
        assert!(preview.error.is_none() && preview.blocked.is_none());
        let out = pair.run(opts, &filter);
        let keep_a = match conflict {
            ConflictMode::FileLevel | ConflictMode::NewerWins | ConflictMode::LargerWins
            | ConflictMode::SourceWins | ConflictMode::KeepBoth => true,
            ConflictMode::OlderWins | ConflictMode::SmallerWins | ConflictMode::DestWins => false,
        };
        if conflict == ConflictMode::FileLevel {
            assert_eq!(preview.conflicts.len(), 1);
            assert_eq!(out.conflicts.len(), 1);
            assert!(out.errors.is_empty());
            assert_eq!(pair.stored(&out), seeded.baseline);
            assert_eq!(pair.bytes(PairSide::A, "file.txt"), b"alpha-newer-long");
            assert_eq!(pair.bytes(PairSide::B, "file.txt"), b"bravo");
            resolve_recorded(&pair.a, &pair.roots[0], &pair.b, &pair.roots[1],
                &out.conflicts[0], true, None, out.state.as_ref().unwrap(), &pair.cancel, |_| {}).unwrap();
        } else {
            assert!(preview.conflicts.is_empty());
            clean(&out);
        }
        let winner: &[u8] = if keep_a { b"alpha-newer-long" } else { b"bravo" };
        let loser: &[u8] = if keep_a { b"bravo" } else { b"alpha-newer-long" };
        assert_eq!(pair.bytes(PairSide::A, "file.txt"), winner, "{conflict:?}");
        assert_eq!(pair.bytes(PairSide::B, "file.txt"), winner, "{conflict:?}");
        let settled = pair.run(opts, &filter);
        clean(&settled);
        assert!(pair.versions(&settled).iter().any(|entry| pair.version_bytes(entry) == loser), "{conflict:?}");
        if conflict == ConflictMode::KeepBoth {
            for side in [PairSide::A, PairSide::B] {
                let sibling = std::fs::read_dir(pair.path(side, "")).unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| path.file_name().unwrap().to_str().unwrap().contains("Konflikt")).unwrap();
                assert_eq!(std::fs::read(sibling).unwrap(), loser);
            }
        }
        pair.no_op(opts, &filter);
    }
}

#[test]
fn sync_reliability_task_options_compare_classes_and_modify_window() {
    for (compare, delta, window, copies) in [
        (CompareMode::MtimeSize, 1_000, 0, true),
        (CompareMode::MtimeSize, 1_000, 2_000, false),
        (CompareMode::MtimeSize, 3_000, 2_000, true),
        (CompareMode::SizeOnly, 20_000, 0, false),
        (CompareMode::Checksum, 0, 0, true),
    ] {
        let pair = Pair::new();
        let globs = empty_globset();
        let filter = WalkFilter::basic(true, &globs);
        let opts = BisyncOptions { compare, modify_window_ms: window, ..forward() };
        pair.put(PairSide::A, "file.txt", b"before", TIME);
        let first = pair.run(opts, &filter);
        clean(&first);
        pair.put(PairSide::A, "file.txt", b"after!", TIME + delta);
        let preview = pair.preview(opts, &filter);
        assert!(preview.error.is_none());
        assert_eq!(preview.actions.len(), usize::from(copies));
        let out = pair.run(opts, &filter);
        clean(&out);
        assert_eq!(out.stats.a_to_b, u64::from(copies));
        assert_eq!(pair.bytes(PairSide::B, "file.txt"), if copies { b"after!" } else { b"before" });
        assert_eq!(pair.stored(&out), out.baseline);
        pair.no_op(opts, &filter);
    }
}

include!("sync_reliability_task_filter_tests.rs");
include!("sync_reliability_task_delete_tests.rs");
include!("sync_reliability_task_versions_tests.rs");
include!("sync_reliability_task_scan_tests.rs");
