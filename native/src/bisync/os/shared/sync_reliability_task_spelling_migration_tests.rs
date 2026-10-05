//! Historical side relations through readonly observation, recorded retry
//! and a previously complete incremental mirror generation.
use std::collections::BTreeSet;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::fixture::{self, Identity, StateFiles, TestRemote, REMOTE_ROOT};
use crate::bisync as engine;
use crate::vfs::{Backend, BackendExtensions, Scheme, VfsMeta};

const A_REL: &str = "OldTree/Note.md";
const B_REL: &str = "oldtree/note.md";

struct ExactRemote<'a> {
    inner: &'a TestRemote,
    listings: AtomicUsize,
    deny_empty_remove: AtomicBool,
}

impl<'a> ExactRemote<'a> {
    fn new(inner: &'a TestRemote) -> Self {
        Self {
            inner,
            listings: AtomicUsize::new(0),
            deny_empty_remove: AtomicBool::new(false),
        }
    }
}

impl Backend for ExactRemote<'_> {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }
    fn state_identity(&self) -> String {
        self.inner.state_identity()
    }
    fn case_sensitive_paths(&self, _: &str) -> bool {
        true
    }
    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }
    fn list_dir(&self, path: &str) -> io::Result<Vec<VfsMeta>> {
        self.listings.fetch_add(1, Ordering::SeqCst);
        self.inner.list_dir(path)
    }
    fn stat(&self, path: &str) -> io::Result<VfsMeta> {
        self.inner.stat(path)
    }
    fn open_read(&self, path: &str) -> io::Result<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }
    fn open_write(&self, path: &str) -> io::Result<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }
    fn open_write_new(&self, path: &str) -> io::Result<Box<dyn Write + Send>> {
        self.inner.open_write_new(path)
    }
    fn rename(&self, source: &str, destination: &str) -> io::Result<()> {
        self.inner.rename(source, destination)
    }
    fn rename_no_replace(&self, source: &str, destination: &str) -> io::Result<()> {
        self.inner.rename_no_replace(source, destination)
    }
    fn remove_file(&self, path: &str) -> io::Result<()> {
        self.inner.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> io::Result<()> {
        if path == format!("{REMOTE_ROOT}/emptyhistory")
            && self.deny_empty_remove.swap(false, Ordering::SeqCst)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "historical directory removal denied once",
            ));
        }
        self.inner.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> io::Result<()> {
        self.inner.mkdir_all(path)
    }
    fn promote_staged(&self, stage: &str, destination: &str) -> io::Result<()> {
        self.inner.promote_staged(stage, destination)
    }
}

impl BackendExtensions for ExactRemote<'_> {
    fn open_read_regular(&self, path: &str, id: Option<&str>) -> io::Result<Box<dyn Read + Send>> {
        crate::vfs::open_read_regular(self.inner, path, id)
    }
    fn finish_stage(
        &self,
        path: &str,
        finish: crate::vfs::StageFinish,
    ) -> io::Result<crate::vfs::StageFinished> {
        crate::vfs::finish_stage(self.inner, path, finish)
    }
}

fn run(
    a: &dyn Backend,
    b: &dyn Backend,
    options: engine::BisyncOptions,
    db: &Path,
) -> engine::Outcome {
    let ignore = engine::empty_globset();
    let filter = engine::WalkFilter::basic(true, &ignore);
    engine::orchestration::run_with_store_path(
        engine::incremental::SyncEndpoints::new(a, REMOTE_ROOT, b, REMOTE_ROOT),
        options,
        &AtomicBool::new(false),
        &filter,
        db,
    )
}

fn complete(out: &engine::Outcome) {
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert!(out.conflicts.is_empty());
    assert!(out.omissions.is_empty(), "{:?}", out.omissions);
    assert!(out.deferred.is_empty());
    assert!(out.blocked.is_none());
    assert!(out.stopped.is_none());
    assert!(!out.canceled);
    assert_eq!(out.stats.errors, 0);
}

fn observed_preview(
    a: &dyn Backend,
    b: &dyn Backend,
    options: engine::BisyncOptions,
) -> engine::preview::Preview {
    let ignore = engine::empty_globset();
    let filter = engine::WalkFilter::basic(true, &ignore);
    engine::preview(
        a,
        REMOTE_ROOT,
        b,
        REMOTE_ROOT,
        options,
        &AtomicBool::new(false),
        &filter,
    )
}

fn seed(a: &Path, b: &Path) {
    std::fs::create_dir_all(a.join("OldTree")).unwrap();
    std::fs::create_dir_all(b.join("oldtree")).unwrap();
    std::fs::write(a.join(A_REL), b"original").unwrap();
    std::fs::write(b.join(B_REL), b"original").unwrap();
}

fn dirs(key: &engine::StateKey) -> BTreeSet<String> {
    let path = engine::baseline_file(key)
        .unwrap()
        .with_extension("dirs.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn sync_reliability_task_options_legacy_spellings_recorded_retry_keeps_literal_slots() {
    let a_dir = tempfile::tempdir().unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    seed(a_dir.path(), b_dir.path());
    std::fs::create_dir(a_dir.path().join("EmptyHistory")).unwrap();
    std::fs::create_dir(b_dir.path().join("emptyhistory")).unwrap();
    let a = TestRemote::new(a_dir.path(), "legacy-recorded-a");
    let b = TestRemote::new(b_dir.path(), "legacy-recorded-b");
    let identity = Identity::current(engine::incremental::SyncEndpoints::new(
        &a,
        REMOTE_ROOT,
        &b,
        REMOTE_ROOT,
    ));
    let _files = StateFiles::new(&[identity]);
    let db = state_dir.path().join("index.sqlite");
    let options = engine::BisyncOptions {
        compare: engine::CompareMode::Checksum,
        ..Default::default()
    };
    let initial = run(&a, &b, options, &db);
    complete(&initial);
    assert_eq!(
        initial
            .baseline
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec![A_REL]
    );
    let key = initial.state.clone().unwrap();
    let base_path = engine::baseline_file(&key).unwrap();
    let names_path = base_path.with_extension("spellings.json");
    let dirs_path = base_path.with_extension("dirs.json");
    let historical_dirs = std::fs::read(&dirs_path).unwrap();
    assert_eq!(
        dirs(&key),
        BTreeSet::from(["EMPTYHISTORY".into(), "OLDTREE".into()])
    );
    let historical_names = std::fs::read(&names_path).unwrap();
    let old_json: serde_json::Value = serde_json::from_slice(&historical_names).unwrap();
    assert_eq!(old_json.as_object().unwrap().len(), 4);
    let base_bytes = std::fs::read(&base_path).unwrap();
    let exact_a = ExactRemote::new(&a);
    let exact_b = ExactRemote::new(&b);
    let cancel = AtomicBool::new(false);
    let preview = observed_preview(&exact_a, &exact_b, options);
    assert!(preview.error.is_none(), "{:?}", preview.error);
    assert!(preview.actions.is_empty());
    assert!(preview.conflicts.is_empty());
    assert!(preview.dirs.is_empty());
    complete(&run(
        &exact_a,
        &exact_b,
        engine::BisyncOptions {
            dry_run: true,
            ..options
        },
        &db,
    ));
    assert_eq!(std::fs::read(&names_path).unwrap(), historical_names);
    assert_eq!(std::fs::read(&base_path).unwrap(), base_bytes);
    assert_eq!(std::fs::read(&dirs_path).unwrap(), historical_dirs);
    let reopened = run(&exact_a, &exact_b, options, &db);
    complete(&reopened);
    assert_eq!(reopened.state.as_ref(), Some(&key));
    assert_eq!(reopened.baseline, initial.baseline);
    assert_eq!((reopened.stats.a_to_b, reopened.stats.b_to_a), (0, 0));
    engine::state_metadata::write_bytes(&dirs_path, &historical_dirs).unwrap();
    std::fs::remove_dir(a_dir.path().join("EmptyHistory")).unwrap();
    let deletion = observed_preview(&exact_a, &exact_b, options);
    assert!(deletion.error.is_none(), "{:?}", deletion.error);
    assert!(deletion.dirs.iter().any(|action| {
        matches!(
            action,
            engine::DirAction::Remove {
                side: engine::PairSide::B,
                rel
            } if rel == "emptyhistory"
        )
    }));
    assert_eq!(std::fs::read(&dirs_path).unwrap(), historical_dirs);
    exact_b.deny_empty_remove.store(true, Ordering::SeqCst);
    let failed_remove = run(&exact_a, &exact_b, options, &db);
    assert_eq!(failed_remove.baseline, initial.baseline);
    assert_eq!(engine::load_baseline(&base_path).unwrap(), initial.baseline);
    assert!(failed_remove.stats.errors > 0 || !failed_remove.omissions.is_empty());
    assert!(!exact_b.deny_empty_remove.load(Ordering::SeqCst));
    assert!(b_dir.path().join("emptyhistory").try_exists().unwrap());
    assert_eq!(
        dirs(&key),
        BTreeSet::from(["EmptyHistory".into(), "OldTree".into()])
    );
    complete(&run(&exact_a, &exact_b, options, &db));
    assert!(!b_dir.path().join("emptyhistory").try_exists().unwrap());
    assert_eq!(dirs(&key), BTreeSet::from(["OldTree".into()]));
    std::fs::create_dir(a_dir.path().join("EmptyHistory")).unwrap();
    complete(&run(&exact_a, &exact_b, options, &db));
    assert!(std::fs::read_dir(b_dir.path())
        .unwrap()
        .any(|entry| entry.unwrap().file_name().to_str() == Some("EmptyHistory")));
    std::fs::write(a_dir.path().join(A_REL), b"winner-a").unwrap();
    std::fs::write(b_dir.path().join(B_REL), b"loser-b").unwrap();
    let conflict = run(&exact_a, &exact_b, options, &db);
    assert!(conflict.errors.is_empty());
    assert_eq!(conflict.conflicts.len(), 1);
    assert_eq!(conflict.conflicts[0].rel, A_REL);
    assert_eq!(conflict.baseline, initial.baseline);
    let paths = engine::recorded_original_paths_for_key(
        &exact_a,
        REMOTE_ROOT,
        &exact_b,
        REMOTE_ROOT,
        &key,
        A_REL,
    )
    .unwrap();
    assert_eq!((paths.rel_a.as_str(), paths.rel_b.as_str()), (A_REL, B_REL));
    let old_recorded = engine::recorded_original_paths_for_key(
        &exact_a,
        REMOTE_ROOT,
        &exact_b,
        REMOTE_ROOT,
        &key,
        B_REL,
    )
    .unwrap();
    assert_eq!(old_recorded, paths);
    b.lose_next_ack.store(true, Ordering::SeqCst);
    let failed = engine::resolve_recorded(
        &exact_a,
        REMOTE_ROOT,
        &exact_b,
        REMOTE_ROOT,
        &conflict.conflicts[0],
        true,
        None,
        &key,
        &cancel,
        |_| {},
    );
    assert_eq!(failed.unwrap_err().kind(), io::ErrorKind::ConnectionReset);
    assert_eq!(
        std::fs::read(b_dir.path().join(B_REL)).unwrap(),
        b"winner-a"
    );
    assert_eq!(engine::load_baseline(&base_path).unwrap(), initial.baseline);
    let pending = fixture::intent(&key.pair_id);
    assert_eq!(pending.destination, format!("{REMOTE_ROOT}/{B_REL}"));
    {
        let lock = engine::PairLock::acquire(&key.lock_id).unwrap();
        let blocked = engine::single_recorded::apply_one(
            engine::incremental::SyncEndpoints::new(&exact_a, REMOTE_ROOT, &exact_b, REMOTE_ROOT),
            &lock,
            &key,
            &engine::Action::DeleteA(A_REL.into()),
            (None, None),
            &Default::default(),
            options,
            &cancel,
        );
        assert_eq!(blocked.unwrap_err().kind(), io::ErrorKind::WouldBlock);
        assert_eq!(
            std::fs::read(a_dir.path().join(A_REL)).unwrap(),
            b"winner-a"
        );
        assert_eq!(
            std::fs::read(b_dir.path().join(B_REL)).unwrap(),
            b"winner-a"
        );
    }
    let promotions = b.promotions.load(Ordering::SeqCst);
    let resumed = run(&exact_a, &exact_b, options, &db);
    complete(&resumed);
    assert_eq!(b.promotions.load(Ordering::SeqCst), promotions);
    fixture::assert_publication_finished(&exact_b, &pending);
    assert_eq!(
        std::fs::read(a_dir.path().join(A_REL)).unwrap(),
        b"winner-a"
    );
    assert_eq!(
        std::fs::read(b_dir.path().join(B_REL)).unwrap(),
        b"winner-a"
    );
    let side = engine::versions::VersionSide {
        side: engine::PairSide::B,
        backend: &exact_b,
        root: REMOTE_ROOT,
    };
    let backups = engine::versions::list_versions(&key.pair_id, &[side], &cancel).unwrap();
    let backup = backups
        .iter()
        .find(|entry| entry.reason == Some(engine::versions::VersionReason::Replaced))
        .unwrap();
    assert_eq!(backup.rel, B_REL);
    assert_eq!(std::fs::read(&backup.stored_path).unwrap(), b"loser-b");
    std::fs::write(b_dir.path().join("oldtree/Added.md"), b"new child").unwrap();
    complete(&run(&exact_a, &exact_b, options, &db));
    assert_eq!(
        std::fs::read(a_dir.path().join("OldTree/Added.md")).unwrap(),
        b"new child"
    );
    let noop = run(&exact_a, &exact_b, options, &db);
    complete(&noop);
    assert_eq!(
        (noop.stats.a_to_b, noop.stats.b_to_a, noop.stats.deleted),
        (0, 0, 0)
    );
    std::fs::remove_file(a_dir.path().join(A_REL)).unwrap();
    let deleted = run(&exact_a, &exact_b, options, &db);
    complete(&deleted);
    assert_eq!(deleted.stats.deleted, 1);
    assert!(!deleted.baseline.contains_key(A_REL));
    assert!(!b_dir.path().join(B_REL).try_exists().unwrap());
    std::fs::write(a_dir.path().join("OldTree/note.md"), b"new literal file").unwrap();
    let new_file = run(&exact_a, &exact_b, options, &db);
    complete(&new_file);
    assert_eq!(new_file.stats.a_to_b, 1);
    assert!(new_file.baseline.contains_key("OldTree/note.md"));
    assert!(!new_file.baseline.contains_key(A_REL));
    assert_eq!(
        std::fs::read(b_dir.path().join(B_REL)).unwrap(),
        b"new literal file"
    );
    let final_noop = run(&exact_a, &exact_b, options, &db);
    complete(&final_noop);
    assert_eq!(final_noop.baseline, new_file.baseline);
    assert_eq!(
        (
            final_noop.stats.a_to_b,
            final_noop.stats.b_to_a,
            final_noop.stats.deleted,
        ),
        (0, 0, 0)
    );
}

#[test]
fn sync_reliability_task_options_legacy_spellings_incremental_and_invalid_maps_keep_basis() {
    let a_dir = tempfile::tempdir().unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    seed(a_dir.path(), b_dir.path());
    let a = TestRemote::new(a_dir.path(), "legacy-index-a");
    let b = TestRemote::new(b_dir.path(), "legacy-index-b");
    let identity = Identity::current(engine::incremental::SyncEndpoints::new(
        &a,
        REMOTE_ROOT,
        &b,
        REMOTE_ROOT,
    ));
    let _files = StateFiles::new(&[identity]);
    let db = state_dir.path().join("index.sqlite");
    let options = engine::BisyncOptions {
        direction: engine::Direction::AtoB,
        delete: engine::DeletePolicy::Mirror,
        compare: engine::CompareMode::Checksum,
        ..Default::default()
    };
    let initial = run(&a, &b, options, &db);
    complete(&initial);
    let key = initial.state.clone().unwrap();
    let base_path = engine::baseline_file(&key).unwrap();
    assert_eq!(dirs(&key), BTreeSet::from(["OLDTREE".into()]));
    let exact_a = ExactRemote::new(&a);
    let exact_b = ExactRemote::new(&b);
    complete(&run(&exact_a, &exact_b, options, &db));
    assert_eq!(dirs(&key), BTreeSet::from(["OldTree".into()]));
    exact_b.listings.store(0, Ordering::SeqCst);
    std::fs::write(a_dir.path().join(A_REL), b"mirror change").unwrap();
    let changed = run(&exact_a, &exact_b, options, &db);
    complete(&changed);
    assert_eq!(changed.stats.a_to_b, 1);
    assert_eq!(exact_b.listings.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read(b_dir.path().join(B_REL)).unwrap(),
        b"mirror change"
    );
    assert!(!base_path
        .with_extension("index-dirty")
        .try_exists()
        .unwrap());
    let store = engine::state_store::SyncStateStore::open_at(&db).unwrap();
    let pair = engine::replica_state::index_id(&key).unwrap();
    let record = store.load_pair(&pair).unwrap().unwrap();
    assert!(record.bootstrapped && record.target_managed);
    let names_path = base_path.with_extension("spellings.json");
    let valid_names = std::fs::read(&names_path).unwrap();
    let base_bytes = std::fs::read(&base_path).unwrap();
    let mut invalid: serde_json::Value = serde_json::from_slice(&valid_names).unwrap();
    invalid["files_b"][A_REL] = serde_json::Value::String("unrelated.txt".into());
    engine::state_metadata::write_bytes(&names_path, &serde_json::to_vec(&invalid).unwrap())
        .unwrap();
    let invalid_bytes = std::fs::read(&names_path).unwrap();
    let failed = run(&exact_a, &exact_b, options, &db);
    assert_eq!(failed.baseline, changed.baseline);
    assert_eq!(failed.stats.errors, 1);
    assert_eq!(failed.errors[0].0, "Pfad-Schreibweisen");
    assert_eq!(std::fs::read(&names_path).unwrap(), invalid_bytes);
    assert_eq!(std::fs::read(&base_path).unwrap(), base_bytes);
    assert_eq!(
        std::fs::read(b_dir.path().join(B_REL)).unwrap(),
        b"mirror change"
    );
    engine::state_metadata::write_bytes(&names_path, &valid_names).unwrap();
    exact_b.listings.store(0, Ordering::SeqCst);
    let noop = run(&exact_a, &exact_b, options, &db);
    complete(&noop);
    assert_eq!(noop.baseline, changed.baseline);
    assert_eq!(
        (noop.stats.a_to_b, noop.stats.b_to_a, noop.stats.deleted),
        (0, 0, 0)
    );
    assert_eq!(exact_b.listings.load(Ordering::SeqCst), 0);
    assert!(!base_path
        .with_extension("index-dirty")
        .try_exists()
        .unwrap());
}
