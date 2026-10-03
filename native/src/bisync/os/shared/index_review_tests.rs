use super::checkpoint_review_tests::Fixture;
use super::state_store::{ItemRecord, PairRecord, Side, SyncStateStore};
use super::*;
use std::collections::BTreeMap;

fn pair(id: &str, cursor: &str) -> PairRecord {
    PairRecord {
        pair: id.into(),
        root_a: "a".into(),
        root_b: "b".into(),
        mode: "mirror".into(),
        source_side: Side::A,
        source_cursor: Some(cursor.into()),
        root_a_id: None,
        root_b_id: None,
        bootstrapped: true,
        target_managed: true,
    }
}

fn baseline(rel: &str) -> Baseline {
    let sig = Sig {
        size: 5,
        mtime_ms: 10,
        hash: 9,
    };
    Baseline::from([(rel.into(), (Some(sig), Some(sig)))])
}

#[test]
fn review_task_index_bootstrap_rolls_back_rows_and_cursor_together() {
    let fixture = Fixture::new();
    let mut store = SyncStateStore::open_at(fixture.root.join("index.sqlite")).unwrap();
    let ids = BTreeMap::new();
    let first = pair("pair", "before");
    store
        .bootstrap(&first, &baseline("old"), &ids, &ids)
        .unwrap();
    let mut invalid = pair("pair", "after");
    invalid.mode.clear();
    assert!(store
        .bootstrap(&invalid, &baseline("new"), &ids, &ids)
        .is_err());
    assert_eq!(
        store
            .load_pair("pair")
            .unwrap()
            .unwrap()
            .source_cursor
            .as_deref(),
        Some("before")
    );
    assert!(store.load_baseline("pair").unwrap().contains_key("old"));
    assert!(!store.load_baseline("pair").unwrap().contains_key("new"));
    store
        .bootstrap(&pair("pair", "after"), &baseline("new"), &ids, &ids)
        .unwrap();
    assert_eq!(
        store
            .load_pair("pair")
            .unwrap()
            .unwrap()
            .source_cursor
            .as_deref(),
        Some("after")
    );
    assert_eq!(store.load_baseline("pair").unwrap(), baseline("new"));
}

#[test]
fn review_task_index_tombstones_remove_rows_and_owner_cleanup_stays_scoped() {
    let fixture = Fixture::new();
    let mut store = SyncStateStore::open_at(fixture.root.join("index.sqlite")).unwrap();
    let ids = BTreeMap::new();
    for id in [
        "pair:job-first:r1",
        "pair:job-first:r2",
        "pair:job-other:r1",
    ] {
        store
            .bootstrap(&pair(id, "cursor"), &baseline("file"), &ids, &ids)
            .unwrap();
    }
    let mut item: ItemRecord =
        store.load_side("pair:job-first:r1", Side::A).unwrap()["file"].clone();
    item.deleted = true;
    store.save_items("pair:job-first:r1", &[item]).unwrap();
    assert!(store
        .load_side("pair:job-first:r1", Side::A)
        .unwrap()
        .is_empty());
    store.forget_owner("job-first").unwrap();
    for id in ["pair:job-first:r1", "pair:job-first:r2"] {
        assert!(store.load_pair(id).unwrap().is_none());
        assert!(store.load_baseline(id).unwrap().is_empty());
    }
    assert!(store.load_pair("pair:job-other:r1").unwrap().is_some());
    assert_eq!(
        store.load_baseline("pair:job-other:r1").unwrap(),
        baseline("file")
    );
}
