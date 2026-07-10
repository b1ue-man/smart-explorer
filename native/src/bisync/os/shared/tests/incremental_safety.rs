use super::super::incremental::{try_incremental_mirror, SyncEndpoints};
use super::super::incremental_changes::{action_plan_for, apply_trees};
use super::super::persistence::{baseline_path, pair_id_for, versions_dir};
use super::super::snapshot::{empty_globset, WalkFilter};
use super::super::state_store::{ItemRecord, PairRecord, Side, SyncStateStore};
use super::super::types::{Action, BisyncOptions, DeletePolicy, Direction, Sig};
use super::*;
use crate::vfs::{
    Backend, ChangeKind, LocalBackend, Scheme, VfsChange, VfsChangeBatch, VfsMeta, VfsResult,
};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

struct FeedBackend {
    inner: LocalBackend,
    batch: Mutex<VfsChangeBatch>,
    calls: AtomicUsize,
}

impl Backend for FeedBackend {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }

    fn root_display(&self) -> String {
        self.inner.root_display()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.inner.list_dir(path)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.inner.stat(path)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }

    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename(source, destination)
    }

    fn rename_no_replace(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename_no_replace(source, destination)
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner.promote_staged(staged, destination)
    }

    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }

    fn is_local(&self) -> bool {
        true
    }

    fn supports_changes(&self) -> bool {
        true
    }

    fn changes_since(&self, _root: &str, _cursor: &str) -> VfsResult<VfsChangeBatch> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.batch.lock().unwrap().clone())
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }
}

fn temp_path(tag: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.push(format!("bisync_{tag}_{}_{}", std::process::id(), nanos));
    path
}

fn record() -> PairRecord {
    PairRecord {
        pair: "pair".into(),
        root_a: "a".into(),
        root_b: "b".into(),
        mode: "mirror".into(),
        source_side: Side::A,
        source_cursor: Some("cursor-1".into()),
        root_a_id: None,
        root_b_id: None,
        bootstrapped: true,
        target_managed: true,
    }
}

fn feed(root: &str, changes: Vec<VfsChange>) -> FeedBackend {
    FeedBackend {
        inner: LocalBackend::new(root),
        batch: Mutex::new(VfsChangeBatch {
            changes,
            new_cursor: Some("cursor-2".into()),
            reset: false,
        }),
        calls: AtomicUsize::new(0),
    }
}

fn change(kind: ChangeKind, rel: &str) -> VfsChange {
    VfsChange {
        kind: kind.clone(),
        rel: Some(rel.into()),
        id: None,
        parent_id: None,
        name: rel.rsplit('/').next().map(str::to_owned),
        meta: (kind == ChangeKind::Upsert).then(|| VfsMeta {
            name: rel.rsplit('/').next().unwrap().into(),
            size: 4,
            mtime_ms: 10,
            ..Default::default()
        }),
    }
}

fn active_item(rel: &str) -> ItemRecord {
    ItemRecord {
        side: Side::A,
        rel: rel.into(),
        id: None,
        parent_id: None,
        name: rel.rsplit('/').next().map(str::to_owned),
        sig: Some(Sig {
            size: 4,
            mtime_ms: 10,
            hash: 0,
        }),
        is_dir: false,
        deleted: false,
    }
}

fn resolved(rel: &str, old_rel: Option<&str>) -> ResolvedChange {
    ResolvedChange {
        rel: rel.into(),
        old_rel: old_rel.map(str::to_owned),
        kind: ChangeKind::Upsert,
        id: None,
        parent_id: None,
        name: rel.rsplit('/').next().map(str::to_owned),
        source_sig: Some(Sig {
            size: 1,
            mtime_ms: 1,
            hash: 0,
        }),
        managed: true,
        old_managed: old_rel.is_some(),
    }
}

#[test]
fn rename_swap_copies_both_final_paths_without_deleting_them() {
    let changes = vec![
        resolved("b.txt", Some("a.txt")),
        resolved("a.txt", Some("b.txt")),
    ];
    let plan = action_plan_for(Side::A, &changes);
    assert_eq!(
        plan.upserts,
        vec![
            Action::CopyAtoB("b.txt".into()),
            Action::CopyAtoB("a.txt".into())
        ]
    );
    assert!(plan.deletes.is_empty());

    let source_items = BTreeMap::from([
        ("a.txt".into(), active_item("a.txt")),
        ("b.txt".into(), active_item("b.txt")),
    ]);
    let (planned_source, _) = apply_trees(Side::A, &source_items, &source_items, &changes).unwrap();
    assert!(planned_source.contains_key("a.txt"));
    assert!(planned_source.contains_key("b.txt"));

    let simple = action_plan_for(Side::A, &[resolved("new.txt", Some("old.txt"))]);
    assert_eq!(simple.upserts, vec![Action::CopyAtoB("new.txt".into())]);
    assert_eq!(simple.deletes, vec![Action::DeleteB("old.txt".into())]);
}

#[test]
fn rename_swap_applies_and_persists_both_final_paths() {
    let root = temp_path("rename_swap");
    let source_root = root.join("source");
    let target_root = root.join("target");
    std::fs::create_dir_all(&source_root).unwrap();
    std::fs::create_dir_all(&target_root).unwrap();
    std::fs::write(source_root.join("a.txt"), b"from-a").unwrap();
    std::fs::write(source_root.join("b.txt"), b"from-b-longer").unwrap();
    std::fs::write(target_root.join("a.txt"), b"from-a").unwrap();
    std::fs::write(target_root.join("b.txt"), b"from-b-longer").unwrap();
    let source_root = source_root.to_string_lossy().replace('\\', "/");
    let target_root = target_root.to_string_lossy().replace('\\', "/");
    let initial_source = LocalBackend::new(&source_root);
    let target = LocalBackend::new(&target_root);
    let source_a = sig_from_meta(
        &initial_source
            .stat(&format!("{source_root}/a.txt"))
            .unwrap(),
    )
    .unwrap();
    let source_b = sig_from_meta(
        &initial_source
            .stat(&format!("{source_root}/b.txt"))
            .unwrap(),
    )
    .unwrap();
    let target_a = sig_from_meta(&target.stat(&format!("{target_root}/a.txt")).unwrap()).unwrap();
    let target_b = sig_from_meta(&target.stat(&format!("{target_root}/b.txt")).unwrap()).unwrap();

    std::fs::rename(
        format!("{source_root}/a.txt"),
        format!("{source_root}/swap.tmp"),
    )
    .unwrap();
    std::fs::rename(
        format!("{source_root}/b.txt"),
        format!("{source_root}/a.txt"),
    )
    .unwrap();
    std::fs::rename(
        format!("{source_root}/swap.tmp"),
        format!("{source_root}/b.txt"),
    )
    .unwrap();
    let final_source = LocalBackend::new(&source_root);
    let source = feed(
        &source_root,
        vec![
            VfsChange {
                kind: ChangeKind::Upsert,
                rel: Some("b.txt".into()),
                id: Some("id-a".into()),
                parent_id: None,
                name: Some("b.txt".into()),
                meta: Some(final_source.stat(&format!("{source_root}/b.txt")).unwrap()),
            },
            VfsChange {
                kind: ChangeKind::Upsert,
                rel: Some("a.txt".into()),
                id: Some("id-b".into()),
                parent_id: None,
                name: Some("a.txt".into()),
                meta: Some(final_source.stat(&format!("{source_root}/a.txt")).unwrap()),
            },
        ],
    );
    let pair_id = pair_id_for(&source, &source_root, &target, &target_root);
    let db = root.join("state.sqlite");
    let mut store = SyncStateStore::open_at(&db).unwrap();
    store
        .save_pair(&PairRecord {
            pair: pair_id.clone(),
            root_a: source_root.clone(),
            root_b: target_root.clone(),
            mode: "mirror".into(),
            source_side: Side::A,
            source_cursor: Some("cursor-1".into()),
            root_a_id: None,
            root_b_id: None,
            bootstrapped: true,
            target_managed: true,
        })
        .unwrap();
    store
        .save_items(
            &pair_id,
            &[
                stored_item(Side::A, "a.txt", Some("id-a"), source_a),
                stored_item(Side::A, "b.txt", Some("id-b"), source_b),
                stored_item(Side::B, "a.txt", None, target_a),
                stored_item(Side::B, "b.txt", None, target_b),
            ],
        )
        .unwrap();
    drop(store);

    let cancel = AtomicBool::new(false);
    let include = empty_globset();
    let filter = WalkFilter::basic(true, &include);
    let opts = BisyncOptions {
        direction: Direction::AtoB,
        delete: DeletePolicy::Mirror,
        reversible: false,
        ..Default::default()
    };
    let outcome = try_incremental_mirror(
        SyncEndpoints::new(&source, &source_root, &target, &target_root),
        opts,
        &cancel,
        &filter,
        Some(&db),
    )
    .unwrap();
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    assert_eq!(
        std::fs::read(format!("{target_root}/a.txt")).unwrap(),
        b"from-b-longer"
    );
    assert_eq!(
        std::fs::read(format!("{target_root}/b.txt")).unwrap(),
        b"from-a"
    );
    let state = SyncStateStore::open_at(&db)
        .unwrap()
        .load_side(&pair_id, Side::A)
        .unwrap();
    assert_eq!(state["a.txt"].id.as_deref(), Some("id-b"));
    assert_eq!(state["b.txt"].id.as_deref(), Some("id-a"));

    let _ = std::fs::remove_file(baseline_path(&pair_id));
    let _ = std::fs::remove_dir_all(versions_dir(&pair_id));
    let _ = std::fs::remove_dir_all(root);
}

fn stored_item(side: Side, rel: &str, id: Option<&str>, sig: Sig) -> ItemRecord {
    ItemRecord {
        side,
        rel: rel.into(),
        id: id.map(str::to_owned),
        parent_id: None,
        name: rel.rsplit('/').next().map(str::to_owned),
        sig: Some(sig),
        is_dir: false,
        deleted: false,
    }
}

#[test]
fn ignored_remove_feed_never_becomes_a_delete_action() {
    let db = temp_path("ignored_feed.sqlite");
    let store = SyncStateStore::open_at(&db).unwrap();
    let source = feed("/", vec![change(ChangeKind::Remove, "ignored.txt")]);
    let ignore = globset::GlobSetBuilder::new()
        .add(globset::Glob::new("ignored.txt").unwrap())
        .build()
        .unwrap();
    let filter = WalkFilter::basic(true, &ignore);
    let items = BTreeMap::from([("ignored.txt".into(), active_item("ignored.txt"))]);
    let cancel = AtomicBool::new(false);
    let ChangeCollection::Ready { changes, .. } = changes_from_backend(
        &store,
        &record(),
        &source,
        "/",
        Side::A,
        &items,
        &filter,
        &cancel,
    ) else {
        panic!("ignored feed should be consumed without a rebuild");
    };
    assert!(!changes[0].managed);
    let plan = action_plan_for(Side::A, &changes);
    assert!(plan.upserts.is_empty() && plan.deletes.is_empty());
    let _ = std::fs::remove_file(db);
}

#[test]
fn canceled_and_over_budget_feeds_fail_closed() {
    let db = temp_path("bounded_feed.sqlite");
    let store = SyncStateStore::open_at(&db).unwrap();
    let source = feed(
        "/",
        vec![
            change(ChangeKind::Upsert, "one.txt"),
            change(ChangeKind::Upsert, "two.txt"),
        ],
    );
    let include = empty_globset();
    let filter = WalkFilter::basic(true, &include);
    let items = BTreeMap::new();
    let canceled = AtomicBool::new(true);
    assert!(matches!(
        changes_from_backend(
            &store,
            &record(),
            &source,
            "/",
            Side::A,
            &items,
            &filter,
            &canceled,
        ),
        ChangeCollection::Canceled
    ));
    assert_eq!(source.calls.load(Ordering::Relaxed), 0);

    let running = AtomicBool::new(false);
    assert!(matches!(
        changes_from_backend_with_limits(
            &store,
            &record(),
            &source,
            "/",
            Side::A,
            &items,
            &filter,
            &running,
            CollectionLimits::new(1, 4096, 8),
        ),
        ChangeCollection::Rebuild
    ));

    let local = LocalBackend::new("/");
    assert!(matches!(
        changes_from_source_walk(
            &local,
            "/",
            &local,
            BisyncOptions::default(),
            &filter,
            &items,
            &canceled,
        ),
        ChangeCollection::Canceled
    ));
    let _ = std::fs::remove_file(db);
}
