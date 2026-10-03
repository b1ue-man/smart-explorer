use super::super::incremental::SyncEndpoints;
use super::super::incremental_changes::{action_plan_for, apply_trees};
use super::super::orchestration::run_with_store_path;
use super::super::persistence::versions_dir;
use super::super::replica_state::index_id;
use super::super::snapshot::{empty_globset, WalkFilter};
use super::super::state_store::{Side, SyncStateStore};
use super::super::types::{Action, BisyncOptions, DeletePolicy, Direction};
use super::*;
use crate::vfs::{Backend, ChangeKind, LocalBackend, VfsChange};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

#[path = "incremental_safety_fixture.rs"]
mod fixture;
use fixture::{active_item, change, feed, record, resolved, temp_path};

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
    let root = tempfile::tempdir().unwrap();
    let source_root = root.path().join("source");
    let target_root = root.path().join("target");
    std::fs::create_dir_all(&source_root).unwrap();
    std::fs::create_dir_all(&target_root).unwrap();
    std::fs::write(source_root.join("a.txt"), b"from-a").unwrap();
    std::fs::write(source_root.join("b.txt"), b"from-b-longer").unwrap();
    std::fs::write(target_root.join("a.txt"), b"from-a").unwrap();
    std::fs::write(target_root.join("b.txt"), b"from-b-longer").unwrap();
    let source_root = source_root.to_string_lossy().replace('\\', "/");
    let target_root = target_root.to_string_lossy().replace('\\', "/");
    let source = feed(&source_root, Vec::new());
    let target = LocalBackend::new(&target_root);
    let endpoints = SyncEndpoints::new(&source, &source_root, &target, &target_root);
    let db = root.path().join("state.sqlite");
    let cancel = AtomicBool::new(false);
    let include = empty_globset();
    let filter = WalkFilter::basic(true, &include);
    let opts = BisyncOptions {
        direction: Direction::AtoB,
        delete: DeletePolicy::Mirror,
        reversible: false,
        ..Default::default()
    };
    // Create the authoritative owner/replica baseline, history and complete
    // index through the actual recorded engine before producing the swap.
    let before = run_with_store_path(endpoints, opts, &cancel, &filter, &db);
    assert!(before.errors.is_empty(), "{:?}", before.errors);
    assert!(before.blocked.is_none() && before.stopped.is_none() && before.deferred.is_empty());
    let key = before.state.unwrap();
    assert!(!key.is_legacy());
    let index = index_id(&key).unwrap();
    let initial = SyncStateStore::open_at(&db)
        .unwrap()
        .load_side(&index, Side::A)
        .unwrap();
    assert_eq!(initial["a.txt"].id.as_deref(), Some("id-a"));
    assert_eq!(initial["b.txt"].id.as_deref(), Some("id-b"));
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
    *source.batch.lock().unwrap() = crate::vfs::VfsChangeBatch {
        changes: vec![
            VfsChange {
                kind: ChangeKind::Upsert,
                rel: Some("b.txt".into()),
                id: Some("id-a".into()),
                parent_id: Some("feed-root".into()),
                name: Some("b.txt".into()),
                meta: Some(source.stat(&format!("{source_root}/b.txt")).unwrap()),
            },
            VfsChange {
                kind: ChangeKind::Upsert,
                rel: Some("a.txt".into()),
                id: Some("id-b".into()),
                parent_id: Some("feed-root".into()),
                name: Some("a.txt".into()),
                meta: Some(source.stat(&format!("{source_root}/a.txt")).unwrap()),
            },
        ],
        new_cursor: Some("cursor-2".into()),
        reset: false,
    };
    let outcome = run_with_store_path(endpoints, opts, &cancel, &filter, &db);
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    assert!(outcome.blocked.is_none() && outcome.stopped.is_none() && outcome.deferred.is_empty());
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
        .load_side(&index, Side::A)
        .unwrap();
    assert_eq!(state["a.txt"].id.as_deref(), Some("id-b"));
    assert_eq!(state["b.txt"].id.as_deref(), Some("id-a"));
    let basis = super::super::baseline_file(&key).unwrap();
    let pair_dir = basis.parent().unwrap();
    assert!(pair_dir.ends_with(&key.pair_id));
    let _ = std::fs::remove_dir_all(pair_dir);
    let _ = std::fs::remove_dir_all(versions_dir(&key.pair_id));
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
