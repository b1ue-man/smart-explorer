use super::*;
use crate::mount::engine::EntryState;
use crate::mount::retirement_queue::RetirementQueue;
use crate::mount::{Baseline, EntryCondition};
use std::collections::HashSet;
use std::sync::{atomic::AtomicUsize, Mutex};
use std::time::Instant;

fn entry(path: &str, retirements: &Arc<RetirementQueue>) -> Arc<Entry> {
    Arc::new(Entry {
        state: Mutex::new(EntryState {
            remote_path: path.into(),
            spool_name: "identity-only-no-spool".into(),
            baseline: Baseline::Missing,
            condition: EntryCondition::Clean,
            delete_token: None,
            delete_committed: false,
            clean_since: Instant::now(),
            retired: false,
        }),
        pins: AtomicUsize::new(0),
        retirements: Arc::clone(retirements),
    })
}

fn assert_index(table: &EntryTable, expected: &HashMap<String, Arc<Entry>>) {
    assert_eq!(table.paths.len(), expected.len());
    assert_eq!(table.values().count(), expected.len());
    let mut by_parent: HashMap<&str, HashSet<*const Entry>> = HashMap::new();
    for (key, identity) in expected {
        by_parent.entry(parent_path(key)).or_default().insert(Arc::as_ptr(identity));
    }
    assert_eq!(table.parents.len(), by_parent.len());
    let mut visited = HashSet::new();
    for (parent, children) in &table.parents {
        assert!(!children.is_empty(), "empty parent buckets must be removed");
        assert_eq!(table.children(parent).count(), children.len());
        for (slot, key) in children.iter().enumerate() {
            assert!(visited.insert(key), "one key must own exactly one slot");
            assert_eq!(parent_path(key), parent.as_str());
            let indexed = table.paths.get(key).expect("every dense slot is live");
            assert_eq!(indexed.parent_slot, slot, "reverse slot for {key}");
            assert!(Arc::ptr_eq(&indexed.entry, expected.get(key).unwrap()));
        }
        let actual = table.children(parent).map(Arc::as_ptr).collect::<HashSet<_>>();
        assert_eq!(&actual, by_parent.get(parent.as_str()).unwrap(),
            "exact-parent overlay for {parent}");
    }
    assert_eq!(visited.len(), expected.len());
}

#[test]
fn mount_vault_task_entry_table_dense_slots_survive_10000_removals() {
    let retirements = Arc::new(RetirementQueue::default());
    let mut table = EntryTable::new();
    let mut expected = HashMap::new();
    for index in 0..10_000 {
        let path = format!("/wide/note-{index:05}");
        let identity = entry(&path, &retirements);
        assert!(table.insert(path.clone(), Arc::clone(&identity)).is_none());
        expected.insert(path, identity);
    }
    for path in ["/wider/keep", "/wide/nested/keep", "/root-note"] {
        let identity = entry(path, &retirements);
        table.insert(path.into(), Arc::clone(&identity));
        expected.insert(path.into(), identity);
    }
    assert_index(&table, &expected);
    assert_eq!(table.children("/wide").count(), 10_000);
    assert_eq!(table.children("/wide/nested").count(), 1);
    assert_eq!(table.children("/wid").count(), 0);

    // Coprime with 10,000: each key is removed once in non-slot order, forcing
    // swap-removal to repair the moved sibling's reverse slot repeatedly.
    for turn in 0..10_000 {
        let path = format!("/wide/note-{:05}", (turn * 7_919) % 10_000);
        let removed = table.remove(&path).unwrap();
        assert!(Arc::ptr_eq(&removed, &expected.remove(&path).unwrap()));
        assert!(!table.contains_key(&path));
        if matches!(turn, 0 | 31 | 4_999 | 9_999) {
            assert_index(&table, &expected);
        }
    }
    assert!(!table.parents.contains_key("/wide"));
    assert_eq!(table.children("/wide").count(), 0);
    assert!(table.remove("/wide/missing").is_none());
    for path in ["/wider/keep", "/wide/nested/keep", "/root-note"] {
        table.remove(path).unwrap();
    }
    assert!(table.is_empty());
    assert!(table.parents.is_empty());
}

#[test]
fn mount_vault_task_entry_table_replace_and_rename_keys_across_10000_identities() {
    let retirements = Arc::new(RetirementQueue::default());
    let mut table = EntryTable::new();
    let mut expected = HashMap::new();
    for index in 0..10_000 {
        let path = format!("/parent-{:03}/note-{index:05}", index / 100);
        expected.insert(path.clone(), entry(&path, &retirements));
    }
    table.extend(expected.iter().map(|(key, value)| (key.clone(), Arc::clone(value))));
    assert_index(&table, &expected);
    for index in (0..10_000).step_by(97) {
        let path = format!("/parent-{:03}/note-{index:05}", index / 100);
        let slot = table.paths.get(&path).unwrap().parent_slot;
        let replacement = entry(&path, &retirements);
        let previous = table.insert(path.clone(), Arc::clone(&replacement)).unwrap();
        assert!(Arc::ptr_eq(&previous, &expected.insert(path.clone(), replacement).unwrap()));
        assert_eq!(table.paths.get(&path).unwrap().parent_slot, slot);
    }
    assert_index(&table, &expected);

    // The table's rename primitive is remove + insert. Cover both an existing
    // destination and a new parent; namespace mutation owns state-path updates.
    for (source, destination) in [
        ("/parent-000/note-00000", "/parent-099/note-09999"),
        ("/parent-000/note-00001", "/renamed"),
        ("/parent-001/note-00100", "/new-parent/note"),
    ] {
        let moved = table.remove(source).unwrap();
        assert!(Arc::ptr_eq(&moved, &expected.remove(source).unwrap()));
        moved.state.lock().unwrap().remote_path = destination.into();
        let displaced = table.insert(destination.into(), Arc::clone(&moved));
        let wanted = expected.insert(destination.into(), moved);
        match (displaced, wanted) {
            (Some(actual), Some(wanted)) => assert!(Arc::ptr_eq(&actual, &wanted)),
            (None, None) => {},
            _ => panic!("replacement must return exactly the old destination identity"),
        }
        assert!(!table.contains_key(source));
        assert_index(&table, &expected);
    }
    let replacement = entry("/renamed", &retirements);
    let added = entry("/new-parent/second", &retirements);
    let additions = [
        ("/renamed".into(), replacement),
        ("/new-parent/second".into(), added),
    ];
    table.extend(additions.iter().map(|(key, value): &(String, Arc<Entry>)| {
        (key.clone(), Arc::clone(value))
    }));
    expected.extend(additions);
    assert_index(&table, &expected);
}
