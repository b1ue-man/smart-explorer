// Deletion policies, verified moves and confirmation limits in complete runs.
#[test]
fn sync_reliability_task_options_delete_policies_move_and_restorable_deletes() {
    for delete in [
        DeletePolicy::Propagate,
        DeletePolicy::Mirror,
        DeletePolicy::NoDelete,
    ] {
        let pair = Pair::new();
        let globs = empty_globset();
        let filter = WalkFilter::basic(true, &globs);
        let opts = BisyncOptions {
            delete,
            compare: CompareMode::Checksum,
            ..forward()
        };
        pair.put(PairSide::A, "anchor", b"anchor", TIME);
        pair.put(PairSide::A, "gone", b"recover deleted bytes", TIME);
        let seeded = pair.run(opts, &filter);
        clean(&seeded);
        std::fs::remove_file(pair.path(PairSide::A, "gone")).unwrap();
        pair.put(PairSide::B, "orphan", b"never on source", TIME);
        // A newly added target-only file needs the explicit full target scan.
        // Incremental mirror intentionally checks its confirmed managed index.
        let out = pair.run_settings(
            opts,
            &filter,
            RunSettings {
                depth: ScanDepth::Full,
                ..Default::default()
            },
        );
        clean(&out);
        let deleted = match delete {
            DeletePolicy::Mirror => 2,
            DeletePolicy::Propagate => 1,
            DeletePolicy::NoDelete => 0,
        };
        assert_eq!(out.stats.deleted, deleted);
        assert_eq!(
            pair.path(PairSide::B, "gone").exists(),
            delete == DeletePolicy::NoDelete
        );
        assert_eq!(
            pair.path(PairSide::B, "orphan").exists(),
            delete != DeletePolicy::Mirror
        );
        pair.no_op(opts, &filter);
        if delete != DeletePolicy::NoDelete {
            let version = pair
                .versions(&out)
                .into_iter()
                .find(|entry| entry.rel == "gone")
                .unwrap();
            assert_eq!(version.reason, Some(versions::VersionReason::Deleted));
            assert_eq!(pair.version_bytes(&version), b"recover deleted bytes");
            pair.restore(&out, &version, PairSide::B);
            assert_eq!(pair.bytes(PairSide::B, "gone"), b"recover deleted bytes");
        }
    }
    for direction in [Direction::AtoB, Direction::BtoA] {
        let pair = Pair::new();
        let source = if direction == Direction::AtoB {
            PairSide::A
        } else {
            PairSide::B
        };
        let globs = empty_globset();
        let filter = WalkFilter::basic(true, &globs);
        let opts = BisyncOptions {
            direction,
            move_files: true,
            compare: CompareMode::Checksum,
            ..forward()
        };
        pair.put(source, "file.txt", b"move complete content", TIME);
        let out = pair.run(opts, &filter);
        clean(&out);
        assert!(!pair.path(source, "file.txt").exists());
        assert_eq!(
            pair.bytes(source.other(), "file.txt"),
            b"move complete content"
        );
        assert_eq!(pair.stored(&out), out.baseline);
        pair.no_op(opts, &filter);
    }
}

#[test]
fn sync_reliability_task_options_delete_limits_require_matching_confirmation() {
    for percentage in [false, true] {
        let pair = Pair::new();
        let globs = empty_globset();
        let filter = WalkFilter::basic(true, &globs);
        let opts = BisyncOptions {
            max_delete: if percentage { 0 } else { 1 },
            max_delete_pct: if percentage { 50 } else { 0 },
            max_delete_min: 2,
            ..forward()
        };
        for name in ["anchor", "remove-1", "remove-2"] {
            pair.put(PairSide::A, name, name.as_bytes(), TIME);
        }
        let first = pair.run(opts, &filter);
        clean(&first);
        for name in ["remove-1", "remove-2"] {
            std::fs::remove_file(pair.path(PairSide::A, name)).unwrap();
        }
        let preview = pair.preview(opts, &filter);
        let blocked = pair.run(opts, &filter);
        assert!(blocked.errors.is_empty());
        assert_eq!(preview.blocked, blocked.blocked);
        let block = blocked.blocked.unwrap();
        assert_eq!(
            block.code(),
            if percentage {
                "mass_delete"
            } else {
                "delete_limit"
            }
        );
        assert_eq!(pair.stored(&first), first.baseline);
        for name in ["remove-1", "remove-2"] {
            assert_eq!(pair.bytes(PairSide::B, name), name.as_bytes());
        }
        let out = pair.run_settings(
            opts,
            &filter,
            RunSettings {
                confirmed: vec![block.confirmation()],
                ..Default::default()
            },
        );
        clean(&out);
        assert_eq!(out.stats.deleted, 2);
        assert_eq!(pair.versions(&out).len(), 2);
        for name in ["remove-1", "remove-2"] {
            assert!(!pair.path(PairSide::B, name).exists());
        }
        pair.no_op(opts, &filter);
    }
}
