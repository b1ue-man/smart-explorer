//! C05 share-private versions through a saved locator: restore, restart and the
//! quiet no-op run, against the suite-owned direct and room providers. Moved here
//! from bisync/os/shared/sync_reliability_task_versions_tests.rs: it names the
//! connect fixture and the endpoint resolution, which lie above the transfer layer
//! (plan docs/plaene/2026-11-01-idealzustand, D13).
use crate::bisync::checkpoint_journal::Journal;
use crate::bisync::replica_state::pair_dir;
use crate::bisync::sync_reliability_task_fixture::{clean, forward};
use crate::bisync::{
    baseline_file, baseline_path, empty_globset, forget_job_state, forget_pair_state, pair_id_for,
    pair_key_policy, run_with, versions, versions_dir, BisyncOptions, CompareMode, Direction,
    PairLock, PairSide, RunRequest, RunSettings, ScanDepth, StateOwner, VersionsLocation,
    WalkFilter,
};

struct OwnedShareVersionState {
    pair: String,
    owner: String,
}

impl OwnedShareVersionState {
    fn new(pair: String) -> Self {
        for path in [pair_dir(&pair), baseline_path(&pair), versions_dir(&pair)] {
            assert!(
                !path.try_exists().unwrap(),
                "share fixture state already exists: {path:?}"
            );
        }
        Self {
            owner: format!("c05_share_versions_{pair}"),
            pair,
        }
    }
}

impl Drop for OwnedShareVersionState {
    fn drop(&mut self) {
        let versions = std::fs::remove_dir_all(versions_dir(&self.pair));
        for result in [
            forget_job_state(&self.owner),
            forget_pair_state(&self.pair),
            versions,
        ] {
            if let Err(error) = result {
                if error.kind() != std::io::ErrorKind::NotFound {
                    if std::thread::panicking() {
                        eprintln!("owned share version cleanup {}: {error}", self.pair);
                    } else {
                        panic!("owned share version cleanup {}: {error}", self.pair);
                    }
                }
            }
        }
    }
}

#[test]
fn sync_reliability_task_options_share_private_versions_restore_restart_and_noop() {
    use crate::connect::sync_reliability_task_provider_fixture as fixture;
    use crate::vfs::{self, Backend, CachingBackend, LocalBackend};
    use std::sync::atomic::AtomicBool;
    let providers = fixture::providers();
    for name in ["direct", "room"] {
        let provider = providers
            .iter()
            .find(|provider| provider.name == name)
            .unwrap_or_else(|| panic!("C05 missing actual {name} fixture"));
        let local = tempfile::tempdir().unwrap();
        let root_a = local.path().to_str().unwrap().replace('\\', "/");
        let a = LocalBackend::new(&root_a);
        let child = format!("c05-versions-{}", versions::new_run_id(versions::now_ms()));
        let locator = format!(
            "{}/{}",
            provider.endpoint.trim_end_matches(['/', '\\']),
            child
        );
        let (raw_b, root_b) = provider.open(&child);
        let identity = raw_b.state_identity();
        let b = CachingBackend::new(raw_b.clone());
        for backend in [&*raw_b, &b as &dyn Backend] {
            assert_eq!(
                vfs::version_archive_policy(backend),
                vfs::VersionArchivePolicy::AppPrivate
            );
            assert_eq!(backend.state_identity(), identity);
        }
        let owned = OwnedShareVersionState::new(pair_id_for(&a, &root_a, &b, &root_b));
        let opts = BisyncOptions {
            direction: Direction::Both,
            compare: CompareMode::Checksum,
            versions: VersionsLocation::Auto,
            ..forward()
        };
        let globs = empty_globset();
        let filter = WalkFilter::basic(true, &globs);
        let cancel = AtomicBool::new(false);
        let run = |backend: &dyn Backend, root: &str| {
            let mut request = RunRequest::new(&a, &root_a, backend, root, opts, &filter, &cancel);
            request.settings = RunSettings {
                depth: ScanDepth::Full,
                ..RunSettings::for_job(&owned.owner)
            };
            let out = run_with(request);
            clean(&out);
            assert_eq!(out.stats.errors, 0);
            assert!(out.omissions.is_empty());
            let key = out.state.as_ref().unwrap();
            assert_eq!(key.pair_id, owned.pair);
            assert_eq!(key.owner, StateOwner::Job(owned.owner.clone()));
            let (_, records, _) =
                Journal::load(key, pair_key_policy(&a, &root_a, backend, root)).unwrap();
            assert_eq!(records.baseline, out.baseline);
            out
        };
        let file = local.path().join("file.txt");
        std::fs::write(&file, b"share initial bytes").unwrap();
        let first = run(&b, &root_b);
        assert_eq!(first.stats.a_to_b, 1);
        assert_eq!(
            fixture::read(&b, &root_b, "file.txt"),
            b"share initial bytes"
        );
        std::fs::write(&file, b"share replacement with different bytes").unwrap();
        let replaced = run(&b, &root_b);
        assert_eq!(replaced.state, first.state);
        assert_eq!(
            fixture::read(&b, &root_b, "file.txt"),
            b"share replacement with different bytes"
        );
        let key = replaced.state.as_ref().unwrap();
        let baseline = std::fs::read(baseline_file(key).unwrap()).unwrap();
        {
            let sides = [
                versions::VersionSide {
                    side: PairSide::A,
                    backend: &a,
                    root: &root_a,
                },
                versions::VersionSide {
                    side: PairSide::B,
                    backend: &b,
                    root: &root_b,
                },
            ];
            let entries = versions::list_versions(&key.pair_id, &sides, &cancel).unwrap();
            let archived = entries
                .iter()
                .find(|entry| {
                    entry.side == Some(PairSide::B)
                        && entry.rel == "file.txt"
                        && entry.reason == Some(versions::VersionReason::Replaced)
                })
                .unwrap();
            assert_eq!(archived.store, versions::VersionStore::AppData);
            assert_eq!(archived.job_id.as_deref(), Some(owned.owner.as_str()));
            assert_eq!(
                std::fs::read(&archived.stored_path).unwrap(),
                b"share initial bytes"
            );
            let error = b
                .stat(&fixture::path(&b, &root_b, ".se-versions"))
                .unwrap_err();
            assert!(
                error.to_string().contains("Pfad ist nicht freigegeben"),
                "{error}"
            );
            assert!(vfs::sync_stat(&b, &root_b).unwrap().is_dir);
            let lock = PairLock::acquire(&key.lock_id).unwrap();
            let mut foreign = archived.clone();
            foreign.job_id = Some(format!("foreign_{}", owned.owner));
            assert_eq!(
                versions::restore_version(&lock, &key.pair_id, &foreign, &sides[1], &cancel)
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::InvalidData
            );
            assert_eq!(
                fixture::read(&b, &root_b, "file.txt"),
                b"share replacement with different bytes"
            );
            assert_eq!(
                std::fs::read(baseline_file(key).unwrap()).unwrap(),
                baseline
            );
            versions::restore_version(&lock, &key.pair_id, archived, &sides[1], &cancel).unwrap();
            assert_eq!(
                fixture::read(&b, &root_b, "file.txt"),
                b"share initial bytes"
            );
            assert_eq!(
                std::fs::read(baseline_file(key).unwrap()).unwrap(),
                baseline
            );
            assert!(versions::list_versions(&key.pair_id, &sides, &cancel)
                .unwrap()
                .iter()
                .any(|entry| {
                    entry.reason == Some(versions::VersionReason::Restored)
                        && entry.job_id.as_deref() == Some(owned.owner.as_str())
                        && std::fs::read(&entry.stored_path).unwrap()
                            == b"share replacement with different bytes"
                }));
        }
        drop(b);
        drop(raw_b);
        let (reopened, reopened_root) = crate::connect::resolve_endpoint(&locator).unwrap();
        assert_eq!(reopened_root, root_b);
        assert_eq!(reopened.state_identity(), identity);
        assert_eq!(
            vfs::version_archive_policy(&*reopened),
            vfs::VersionArchivePolicy::AppPrivate
        );
        let restored = run(&*reopened, &reopened_root);
        assert_eq!(restored.state, replaced.state);
        assert_eq!(std::fs::read(&file).unwrap(), b"share initial bytes");
        let baseline_after_restore = std::fs::read(baseline_file(key).unwrap()).unwrap();
        let quiet = run(&*reopened, &reopened_root);
        assert_eq!(quiet.state, restored.state);
        assert_eq!(quiet.baseline, restored.baseline);
        assert_eq!(
            std::fs::read(baseline_file(key).unwrap()).unwrap(),
            baseline_after_restore
        );
        assert_eq!(
            fixture::read(&*reopened, &reopened_root, "file.txt"),
            b"share initial bytes"
        );
        assert_eq!(
            (
                quiet.stats.a_to_b,
                quiet.stats.b_to_a,
                quiet.stats.deleted,
                quiet.stats.bytes
            ),
            (0, 0, 0, 0)
        );
        println!("C05 {name} normal saved locator/private version/restore/reopen/owner/baseline/noop confirmed");
    }
}
