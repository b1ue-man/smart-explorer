// Version stores, restore and effective retention in complete runs.
#[test]
fn sync_reliability_task_options_version_schemes_stores_restore_and_noop() {
    for scheme in VersioningScheme::ALL {
        for location in VersionsLocation::ALL {
            let pair = Pair::new();
            let globs = empty_globset();
            let filter = WalkFilter::basic(true, &globs);
            let opts = BisyncOptions { direction: Direction::Both, compare: CompareMode::Checksum,
                versions: location, versioning: Versioning { scheme, days: 30, count: 2 }, ..forward() };
            pair.put(PairSide::A, "file.txt", b"initial bytes", TIME);
            clean(&pair.run(opts, &filter));
            pair.put(PairSide::A, "file.txt", b"replacement bytes", TIME + 10_000);
            let replaced = pair.run(opts, &filter);
            clean(&replaced);
            let entries = pair.versions(&replaced);
            assert_eq!(entries.len(), 1, "{scheme:?}/{location:?}");
            assert_eq!(entries[0].store, match location {
                VersionsLocation::Auto => versions::VersionStore::SyncRoot,
                VersionsLocation::AppData => versions::VersionStore::AppData,
            });
            assert_eq!(pair.version_bytes(&entries[0]), b"initial bytes");
            pair.restore(&replaced, &entries[0], PairSide::B);
            assert_eq!(pair.bytes(PairSide::B, "file.txt"), b"initial bytes");
            assert_eq!(pair.stored(&replaced), replaced.baseline, "restore must leave comparison basis intact");
            assert!(pair.versions(&replaced).iter().any(|entry| entry.reason == Some(versions::VersionReason::Restored)
                && pair.version_bytes(entry) == b"replacement bytes"));
            let restored = pair.run(opts, &filter);
            clean(&restored);
            assert_eq!(pair.bytes(PairSide::A, "file.txt"), b"initial bytes");
            assert_eq!(pair.stored(&restored), restored.baseline);
            pair.no_op(opts, &filter);
        }
    }
}

#[test]
fn sync_reliability_task_options_retention_prunes_actual_run_versions() {
    use super::apply_guard::{capture, ExpectedFile};
    let day = versions::now_ms() / 86_400_000;
    for scheme in VersioningScheme::ALL {
        for location in VersionsLocation::ALL {
            let pair = Pair::new();
            let globs = empty_globset();
            let filter = WalkFilter::basic(true, &globs);
            let opts = BisyncOptions { compare: CompareMode::Checksum, versions: location,
                versioning: Versioning { scheme, days: 5, count: 2 }, ..forward() };
            pair.put(PairSide::A, "file.txt", b"seed", TIME);
            let first = pair.run(opts, &filter);
            clean(&first);
            let state = first.state.as_ref().unwrap();
            let mut prepared = Vec::new();
            {
                let lock = PairLock::acquire(&state.lock_id).unwrap();
                let sides = pair.sides();
                for (stamp, payload) in [((day - 400) * 86_400_000 + 3_600_000, b"expired".as_slice()),
                    ((day - 3) * 86_400_000 + 3_600_000, b"older bucket".as_slice()),
                    ((day - 3) * 86_400_000 + 7_200_000, b"newer bucket".as_slice())] {
                    pair.put(PairSide::B, "file.txt", payload, TIME);
                    let mut context = versions::VersionsContext::new(&state.pair_id,
                        state.owner.clone(), location, opts.versioning);
                    context.started_ms = stamp;
                    context.run_id = versions::new_run_id(stamp);
                    let archived = versions::RunVersions::begin(context);
                    archived.bind_lock(lock.id()).unwrap();
                    let path = pair.path(PairSide::B, "file.txt");
                    let path = path.to_str().unwrap();
                    let captured = capture(&pair.b, path, ExpectedFile::Unknown, "retention fixture").unwrap();
                    super::version_save::save(&archived, &sides[1], path, "file.txt", &captured,
                        ExpectedFile::Unknown, versions::VersionReason::Replaced, &pair.cancel).unwrap();
                    archived.finish().unwrap();
                    let entry = pair.versions(&first).into_iter().find(|entry| entry.run_id == archived.run_id()).unwrap();
                    // Historical input, prepared under the pair lock: bytes/IDs/replica
                    // come from real version_save; only their original preservation date is aged.
                    let dir = std::path::Path::new(&entry.stored_path).parent().unwrap();
                    for name in ["entry.json", "intent.json"] {
                        let record = dir.join(name);
                        let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
                        value["preserved_ms"] = serde_json::json!(stamp);
                        std::fs::write(&record, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
                    }
                    prepared.push((payload.to_vec(), entry.stored_path));
                }
            }
            pair.put(PairSide::B, "file.txt", b"seed", TIME);
            pair.put(PairSide::A, "file.txt", b"current replacement", TIME + 10_000);
            let out = pair.run(opts, &filter);
            clean(&out);
            let entries = pair.versions(&out);
            let payloads: Vec<_> = entries.iter().map(|entry| pair.version_bytes(entry)).collect();
            let expected: &[&[u8]] = match scheme {
                VersioningScheme::Days => &[b"seed", b"newer bucket", b"older bucket"],
                VersioningScheme::Count | VersioningScheme::Gfs => &[b"seed", b"newer bucket"],
                VersioningScheme::Staggered => &[b"seed", b"newer bucket", b"expired"],
            };
            assert_eq!(payloads.len(), expected.len(), "{scheme:?}/{location:?}: {payloads:?}");
            for bytes in expected { assert!(payloads.iter().any(|actual| actual.as_slice() == *bytes)); }
            for (bytes, path) in prepared {
                assert_eq!(std::path::Path::new(&path).try_exists().unwrap(),
                    expected.iter().any(|kept| *kept == bytes.as_slice()), "pruning must remove actual data: {scheme:?}");
            }
            assert_eq!(pair.bytes(PairSide::B, "file.txt"), b"current replacement");
            pair.no_op(opts, &filter);
            let retained = entries.iter().find(|entry| pair.version_bytes(entry) == b"newer bucket").unwrap();
            pair.restore(&out, retained, PairSide::B);
            assert_eq!(pair.bytes(PairSide::B, "file.txt"), b"newer bucket");
        }
    }
}
