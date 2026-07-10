use super::super::incremental::SyncEndpoints;
use super::super::orchestration::run_with_store_path;
use super::super::persistence::{baseline_path, pair_id_for, versions_dir};
use super::super::snapshot::{empty_globset, WalkFilter};
use super::super::state_validation::StateBudget;
use super::super::types::{BisyncOptions, DeletePolicy, Direction};
use super::*;
use crate::vfs::LocalBackend;
use std::sync::atomic::AtomicBool;

fn temp_db() -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.push(format!(
        "se_sync_state_{}_{}.sqlite",
        std::process::id(),
        nanos
    ));
    path
}

fn pair() -> PairRecord {
    PairRecord {
        pair: "p".into(),
        root_a: "a".into(),
        root_b: "b".into(),
        mode: "mirror".into(),
        source_side: Side::A,
        source_cursor: Some("c1".into()),
        root_a_id: None,
        root_b_id: Some("root".into()),
        bootstrapped: true,
        target_managed: true,
    }
}

fn item(rel: &str) -> ItemRecord {
    ItemRecord {
        side: Side::A,
        rel: rel.into(),
        id: Some("id1".into()),
        parent_id: None,
        name: rel.rsplit('/').next().map(str::to_owned),
        sig: Some(Sig {
            size: 3,
            mtime_ms: 9,
            hash: 7,
        }),
        is_dir: false,
        deleted: false,
    }
}

#[test]
fn pair_and_items_roundtrip() {
    let path = temp_db();
    let mut store = SyncStateStore::open_at(&path).unwrap();
    store.save_pair(&pair()).unwrap();
    store.save_items("p", &[item("f.txt")]).unwrap();
    assert_eq!(
        store
            .load_pair("p")
            .unwrap()
            .unwrap()
            .source_cursor
            .as_deref(),
        Some("c1")
    );
    assert_eq!(
        store.rel_for_id("p", Side::A, "id1").unwrap().as_deref(),
        Some("f.txt")
    );
    assert_eq!(
        store.load_side("p", Side::A).unwrap()["f.txt"]
            .sig
            .unwrap()
            .hash,
        7
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn rollback_keeps_previous_item_on_failed_transaction() {
    let path = temp_db();
    let mut store = SyncStateStore::open_at(&path).unwrap();
    let mut original = item("f.txt");
    original.id = None;
    original.name = None;
    original.sig = Some(Sig {
        size: 1,
        mtime_ms: 1,
        hash: 1,
    });
    store.save_items("p", &[original.clone()]).unwrap();
    let tx = store.conn.transaction().unwrap();
    original.sig = Some(Sig {
        size: 2,
        mtime_ms: 2,
        hash: 2,
    });
    upsert_item_tx(&tx, "p", &original).unwrap();
    drop(tx);
    assert_eq!(
        store.load_side("p", Side::A).unwrap()["f.txt"]
            .sig
            .unwrap()
            .size,
        1
    );
    let _ = std::fs::remove_file(path);
}

fn populated_store(tag: &str) -> PathBuf {
    let path = temp_db().with_extension(format!("{tag}.sqlite"));
    let mut store = SyncStateStore::open_at(&path).unwrap();
    store.save_pair(&pair()).unwrap();
    store.save_items("p", &[item("f.txt")]).unwrap();
    path
}

#[test]
fn corrupt_side_signature_and_relative_path_are_rejected() {
    let side_path = populated_store("bad_side");
    rusqlite::Connection::open(&side_path)
        .unwrap()
        .execute("UPDATE pairs SET source_side = 'X' WHERE pair = 'p'", [])
        .unwrap();
    assert!(SyncStateStore::open_at(&side_path)
        .unwrap()
        .load_pair("p")
        .is_err());

    let item_side_path = populated_store("bad_item_side");
    rusqlite::Connection::open(&item_side_path)
        .unwrap()
        .execute("UPDATE items SET side = 'X' WHERE pair = 'p'", [])
        .unwrap();
    assert!(SyncStateStore::open_at(&item_side_path)
        .unwrap()
        .load_side("p", Side::A)
        .is_err());

    let signature_path = populated_store("bad_signature");
    rusqlite::Connection::open(&signature_path)
        .unwrap()
        .execute(
            "UPDATE items SET size = 'not-a-number' WHERE pair = 'p'",
            [],
        )
        .unwrap();
    assert!(SyncStateStore::open_at(&signature_path)
        .unwrap()
        .load_side("p", Side::A)
        .is_err());

    let rel_path = populated_store("bad_rel");
    rusqlite::Connection::open(&rel_path)
        .unwrap()
        .execute("UPDATE items SET rel = '../escape' WHERE pair = 'p'", [])
        .unwrap();
    assert!(SyncStateStore::open_at(&rel_path)
        .unwrap()
        .load_side("p", Side::A)
        .is_err());

    for path in [side_path, item_side_path, signature_path, rel_path] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn state_load_budget_fails_before_growing_unbounded() {
    let first = item("first.txt");
    let second = item("second.txt");
    let mut budget = StateBudget::with_limits(1, 1024);
    budget.record_item(&first).unwrap();
    assert!(budget.record_item(&second).is_err());

    let mut text_budget = StateBudget::with_limits(10, 4);
    assert!(text_budget.record_item(&first).is_err());
}

#[test]
fn corrupt_incremental_state_falls_back_to_a_safe_full_rebuild() {
    let root = temp_db().with_extension("fallback");
    let a = root.join("a");
    let b = root.join("b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(a.join("f.txt"), b"one").unwrap();
    let root_a = a.to_string_lossy().replace('\\', "/");
    let root_b = b.to_string_lossy().replace('\\', "/");
    let backend_a = LocalBackend::new(&root_a);
    let backend_b = LocalBackend::new(&root_b);
    let db = root.join("state.sqlite");
    let cancel = AtomicBool::new(false);
    let ignore = empty_globset();
    let filter = WalkFilter::basic(true, &ignore);
    let opts = BisyncOptions {
        direction: Direction::AtoB,
        delete: DeletePolicy::Mirror,
        ..Default::default()
    };
    let endpoints = SyncEndpoints::new(&backend_a, &root_a, &backend_b, &root_b);
    assert!(run_with_store_path(endpoints, opts, &cancel, &filter, &db)
        .errors
        .is_empty());

    rusqlite::Connection::open(&db)
        .unwrap()
        .execute("UPDATE pairs SET source_side = 'X'", [])
        .unwrap();
    std::fs::write(a.join("f.txt"), b"two-longer").unwrap();
    let recovered = run_with_store_path(endpoints, opts, &cancel, &filter, &db);
    assert!(recovered.errors.is_empty());
    assert_eq!(std::fs::read(b.join("f.txt")).unwrap(), b"two-longer");

    let sync_pair = pair_id_for(&backend_a, &root_a, &backend_b, &root_b);
    let _ = std::fs::remove_file(baseline_path(&sync_pair));
    let _ = std::fs::remove_dir_all(versions_dir(&sync_pair));
    let _ = std::fs::remove_dir_all(root);
}
