// Atomic verified transfers, scan depths and confirmed incremental mirror state.
#[test]
fn sync_reliability_task_options_atomic_verify_and_run_depths() {
    for atomic in [true, false] {
        let pair = Pair::new();
        let globs = empty_globset();
        let filter = WalkFilter::basic(true, &globs);
        let opts = BisyncOptions {
            atomic,
            verify: true,
            delete: DeletePolicy::Mirror,
            compare: CompareMode::Checksum,
            ..forward()
        };
        pair.put(
            PairSide::A,
            "file.txt",
            b"initial depth seed",
            TIME - 10_000,
        );
        clean(&pair.run_settings(opts, &filter, RunSettings::default()));
        pair.put(
            PairSide::B,
            "late-target-only.txt",
            b"verify on full scan",
            TIME,
        );
        for (index, depth) in [
            ScanDepth::Incremental,
            ScanDepth::VerifySources,
            ScanDepth::Full,
        ]
        .into_iter()
        .enumerate()
        {
            let payload = vec![b'0' + index as u8; 4_096 + index];
            pair.put(
                PairSide::A,
                "file.txt",
                &payload,
                TIME + index as i64 * 10_000,
            );
            let settings = RunSettings {
                depth,
                ..Default::default()
            };
            pair.b.listings.store(0, Ordering::SeqCst);
            let changed = pair.run_settings(opts, &filter, settings.clone());
            clean(&changed);
            assert_eq!(pair.bytes(PairSide::B, "file.txt"), payload);
            assert_eq!(pair.stored(&changed), changed.baseline);
            assert_eq!(
                pair.b.listings.load(Ordering::SeqCst) > 0,
                depth == ScanDepth::Full
            );
            assert_eq!(
                pair.path(PairSide::B, "late-target-only.txt").exists(),
                depth != ScanDepth::Full
            );
            if depth == ScanDepth::Full {
                assert!(pair
                    .versions(&changed)
                    .iter()
                    .any(|entry| entry.rel == "late-target-only.txt"
                        && pair.version_bytes(entry) == b"verify on full scan"));
            }
            let noop = pair.run_settings(opts, &filter, settings);
            clean(&noop);
            assert_eq!(
                noop.stats.a_to_b + noop.stats.b_to_a + noop.stats.deleted,
                0
            );
        }
    }
}

#[test]
fn sync_reliability_task_options_mirror_uses_confirmed_incremental_index() {
    for direction in [Direction::AtoB, Direction::BtoA] {
        let pair = Pair::new();
        let globs = empty_globset();
        let filter = WalkFilter::basic(true, &globs);
        let source = if direction == Direction::AtoB {
            PairSide::A
        } else {
            PairSide::B
        };
        let target = if source == PairSide::A {
            &pair.b
        } else {
            &pair.a
        };
        let opts = BisyncOptions {
            direction,
            delete: DeletePolicy::Mirror,
            compare: CompareMode::Checksum,
            ..forward()
        };
        pair.put(source, "file.txt", b"seed", TIME);
        pair.put(source, "unchanged/note.txt", b"confirmed nested bytes", TIME);
        let first = pair.run(opts, &filter);
        clean(&first);
        assert!(pair.index_complete(&first));
        target.listings.store(0, Ordering::SeqCst);
        pair.put(source, "file.txt", b"incrementally changed", TIME + 10_000);
        let changed = pair.run(opts, &filter);
        clean(&changed);
        assert_eq!(
            target.listings.load(Ordering::SeqCst),
            0,
            "trusted mirror checks touched target files"
        );
        assert_eq!(
            pair.bytes(source.other(), "file.txt"),
            b"incrementally changed"
        );
        assert_eq!(pair.stored(&changed), changed.baseline);
        assert_eq!(
            changed.baseline["unchanged/note.txt"],
            first.baseline["unchanged/note.txt"]
        );
        assert_eq!(
            pair.bytes(source.other(), "unchanged/note.txt"),
            b"confirmed nested bytes"
        );
        assert!(pair.index_complete(&changed));
        pair.no_op(opts, &filter);
    }
}
