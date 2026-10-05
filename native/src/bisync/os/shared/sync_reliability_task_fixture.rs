//! Owned pair, confirmed checkpoint and version-store fixtures for complete runs.
use super::sync_reliability_task_backend_fixture::Probe;
use super::*;
use crate::vfs::Backend;
use std::collections::BTreeSet;
use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub(super) const TIME: i64 = 1_750_000_000_000;

pub(super) struct Pair {
    _dir: tempfile::TempDir,
    pub(super) roots: [String; 2],
    pub(super) a: Probe,
    pub(super) b: Probe,
    pub(super) db: PathBuf,
    pub(super) cancel: Arc<AtomicBool>,
    owned: Mutex<BTreeSet<PathBuf>>,
}
impl Pair {
    pub(super) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let roots = [dir.path().join("a"), dir.path().join("b")];
        for root in &roots {
            std::fs::create_dir(root).unwrap();
        }
        let roots = roots.map(|p| p.to_str().unwrap().replace('\\', "/"));
        let db = dir.path().join("index.sqlite");
        let pair = Self {
            a: Probe::new(roots[0].clone()),
            b: Probe::new(roots[1].clone()),
            roots,
            db,
            _dir: dir,
            cancel: Arc::new(AtomicBool::new(false)),
            owned: Mutex::new(BTreeSet::new()),
        };
        for id in [
            pair_id_for(&pair.a, &pair.roots[0], &pair.b, &pair.roots[1]),
            pair_id_for(&pair.b, &pair.roots[1], &pair.a, &pair.roots[0]),
        ] {
            pair.track(&id);
        }
        for path in pair.owned.lock().unwrap().iter() {
            assert!(
                !path.try_exists().unwrap(),
                "fixture state already exists: {path:?}"
            );
        }
        pair
    }
    fn track(&self, id: &str) {
        let mut owned = self.owned.lock().unwrap();
        owned.insert(super::replica_state::pair_dir(id));
        owned.insert(versions_dir(id));
        for extension in [
            "sebl",
            "journal",
            "dirs.json",
            "spellings.json",
            "index-dirty",
        ] {
            owned.insert(baseline_path(id).with_extension(extension));
        }
    }
    pub(super) fn path(&self, side: PairSide, rel: &str) -> PathBuf {
        PathBuf::from(&self.roots[if side == PairSide::A { 0 } else { 1 }]).join(rel)
    }
    pub(super) fn put(&self, side: PairSide, rel: &str, bytes: &[u8], time: i64) {
        let path = self.path(side, rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        let backend = if side == PairSide::A {
            &self.a.inner
        } else {
            &self.b.inner
        };
        assert!(
            crate::vfs::finish_stage(
                backend,
                path.to_str().unwrap(),
                crate::vfs::StageFinish {
                    mtime_ms: Some(time),
                    ..Default::default()
                }
            )
            .unwrap()
            .mtime_applied
        );
    }
    pub(super) fn bytes(&self, side: PairSide, rel: &str) -> Vec<u8> {
        std::fs::read(self.path(side, rel)).unwrap()
    }
    pub(super) fn run(&self, opts: BisyncOptions, filter: &WalkFilter<'_>) -> Outcome {
        let out = super::orchestration::run_with_store_path(
            super::incremental::SyncEndpoints::new(
                &self.a,
                &self.roots[0],
                &self.b,
                &self.roots[1],
            ),
            opts,
            &self.cancel,
            filter,
            &self.db,
        );
        if let Some(state) = &out.state {
            self.track(&state.pair_id);
        }
        out
    }
    pub(super) fn run_settings(
        &self,
        opts: BisyncOptions,
        filter: &WalkFilter<'_>,
        settings: RunSettings,
    ) -> Outcome {
        let mut request = RunRequest::new(
            &self.a,
            &self.roots[0],
            &self.b,
            &self.roots[1],
            opts,
            filter,
            &self.cancel,
        );
        request.settings = settings;
        let out = run_with(request);
        if let Some(state) = &out.state {
            self.track(&state.pair_id);
        }
        out
    }
    pub(super) fn preview(&self, opts: BisyncOptions, filter: &WalkFilter<'_>) -> Preview {
        preview(
            &self.a,
            &self.roots[0],
            &self.b,
            &self.roots[1],
            opts,
            &self.cancel,
            filter,
        )
    }
    pub(super) fn preview_settings(
        &self,
        opts: BisyncOptions,
        filter: &WalkFilter<'_>,
        settings: RunSettings,
    ) -> Preview {
        preview_with(
            &self.a,
            &self.roots[0],
            &self.b,
            &self.roots[1],
            opts,
            &self.cancel,
            filter,
            settings,
        )
    }
    pub(super) fn stored(&self, out: &Outcome) -> Baseline {
        let keys = pair_key_policy(&self.a, &self.roots[0], &self.b, &self.roots[1]);
        let (_, records, _) =
            super::checkpoint_journal::Journal::load(out.state.as_ref().unwrap(), keys).unwrap();
        records.baseline
    }
    pub(super) fn sides(&self) -> [versions::VersionSide<'_>; 2] {
        [
            versions::VersionSide {
                side: PairSide::A,
                backend: &self.a,
                root: &self.roots[0],
            },
            versions::VersionSide {
                side: PairSide::B,
                backend: &self.b,
                root: &self.roots[1],
            },
        ]
    }
    pub(super) fn versions(&self, out: &Outcome) -> Vec<versions::VersionEntry> {
        versions::list_versions(
            &out.state.as_ref().unwrap().pair_id,
            &self.sides(),
            &self.cancel,
        )
        .unwrap()
    }
    pub(super) fn version_bytes(&self, entry: &versions::VersionEntry) -> Vec<u8> {
        let mut bytes = Vec::new();
        if entry.store == versions::VersionStore::AppData {
            return std::fs::read(&entry.stored_path).unwrap();
        }
        let backend = if entry.side == Some(PairSide::A) {
            &self.a
        } else {
            &self.b
        };
        backend
            .open_read(&entry.stored_path)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    }
    pub(super) fn restore(&self, out: &Outcome, entry: &versions::VersionEntry, side: PairSide) {
        let state = out.state.as_ref().unwrap();
        let lock = PairLock::acquire(&state.lock_id).unwrap();
        let sides = self.sides();
        versions::restore_version(
            &lock,
            &state.pair_id,
            entry,
            &sides[if side == PairSide::A { 0 } else { 1 }],
            &self.cancel,
        )
        .unwrap();
    }
    pub(super) fn index_complete(&self, out: &Outcome) -> bool {
        let state = out.state.as_ref().unwrap();
        if super::state_metadata::index_dirty_path(state)
            .unwrap()
            .try_exists()
            .unwrap()
        {
            return false;
        }
        let id = super::replica_state::index_id(state).unwrap();
        let store = super::state_store::SyncStateStore::open_at(&self.db).unwrap();
        if !store
            .load_pair(&id)
            .unwrap()
            .is_some_and(|record| record.bootstrapped && record.target_managed)
        {
            return false;
        }
        let keys = pair_key_policy(&self.a, &self.roots[0], &self.b, &self.roots[1]);
        let confirmed: Baseline = self
            .stored(out)
            .into_iter()
            .map(|(rel, entry)| (keys.key(&rel).into_owned(), entry))
            .collect();
        let (a, b) = store.load_pair_items(&id).unwrap();
        let mut cached = Baseline::new();
        for (side, items) in [(PairSide::A, a), (PairSide::B, b)] {
            for item in items.values().filter(|item| !item.is_dir && !item.deleted) {
                let entry = cached.entry(keys.key(&item.rel).into_owned()).or_default();
                match side {
                    PairSide::A => entry.0 = item.sig,
                    PairSide::B => entry.1 = item.sig,
                }
            }
        }
        cached == confirmed
    }
    pub(super) fn no_op(&self, opts: BisyncOptions, filter: &WalkFilter<'_>) -> Outcome {
        let promotions =
            self.a.promotions.load(Ordering::SeqCst) + self.b.promotions.load(Ordering::SeqCst);
        let out = self.run(opts, filter);
        clean(&out);
        assert_eq!(out.stats.a_to_b + out.stats.b_to_a + out.stats.deleted, 0);
        assert_eq!(
            self.a.promotions.load(Ordering::SeqCst) + self.b.promotions.load(Ordering::SeqCst),
            promotions
        );
        assert_eq!(self.stored(&out), out.baseline);
        out
    }
}
impl Drop for Pair {
    fn drop(&mut self) {
        for path in self.owned.get_mut().unwrap().iter() {
            let result = if path.is_dir() {
                std::fs::remove_dir_all(path)
            } else {
                std::fs::remove_file(path)
            };
            if let Err(error) = result {
                assert!(
                    error.kind() == io::ErrorKind::NotFound || std::thread::panicking(),
                    "fixture cleanup {path:?}: {error}"
                );
            }
        }
    }
}

pub(super) fn clean(out: &Outcome) {
    assert!(out.errors.is_empty() && out.conflicts.is_empty() && out.blocked.is_none()
        && out.stopped.is_none() && out.deferred.is_empty() && !out.busy && !out.canceled,
        "errors={:?}, conflicts={}, blocked={:?}, stopped={:?}, deferred={:?}, busy={}, canceled={}",
        out.errors, out.conflicts.len(), out.blocked, out.stopped, out.deferred, out.busy, out.canceled);
}
pub(super) fn forward() -> BisyncOptions {
    BisyncOptions {
        direction: Direction::AtoB,
        max_transfers: 1,
        ..Default::default()
    }
}
