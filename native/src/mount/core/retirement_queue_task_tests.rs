use super::*;
use crate::mount::engine::{lock, write_lock, EntryState};
use crate::mount::entry_lifecycle::EntryPin;
use crate::mount::open_handle::OpenHandleKind;
use crate::mount::optimization_fixture::{FixtureDirectory, OptimizationBackend};
use crate::mount::{
    Baseline, EntryCondition, MountCachePolicy, MountEngine, MountId, MountMode,
    MountRuntimeConfig, OpenDisposition, OpenFileOptions,
};
use std::collections::HashSet;
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

fn entry(queue: &Arc<RetirementQueue>) -> Arc<Entry> {
    Arc::new(Entry {
        state: Mutex::new(EntryState {
            // Deliberately identical paths: candidates are object identities,
            // so replacement objects must not collapse into one path ticket.
            remote_path: "/same-path".into(),
            spool_name: "identity-only-no-spool".into(),
            baseline: Baseline::Missing,
            condition: EntryCondition::Clean,
            delete_token: None,
            delete_committed: false,
            clean_since: Instant::now(),
            retired: false,
        }),
        pins: AtomicUsize::new(0),
        retirements: Arc::clone(queue),
    })
}

fn identities(batch: impl Iterator<Item = Weak<Entry>>) -> HashSet<usize> {
    let mut result = HashSet::new();
    for ticket in batch {
        let identity = ticket.upgrade().expect("fixture still owns the identity");
        assert!(result.insert(Arc::as_ptr(&identity) as usize), "duplicate ticket");
    }
    result
}

#[test]
fn mount_vault_task_retirement_batches_deduplicate_10000_identities() {
    let queue = Arc::new(RetirementQueue::default());
    let entries = (0..10_000).map(|_| entry(&queue)).collect::<Vec<_>>();
    let expected = entries.iter().map(|entry| Arc::as_ptr(entry) as usize)
        .collect::<HashSet<_>>();
    for entry in &entries {
        for _ in 0..4 { queue.enqueue(entry); }
        assert_eq!(Arc::strong_count(entry), 1, "queue never owns file contents");
        assert_eq!(Arc::weak_count(entry), 1, "dedup retains one weak ticket");
    }
    let first_batch = queue.take();
    assert!(queue.is_empty(), "taking releases all old dedup keys immediately");

    let replacement = entry(&queue);
    queue.enqueue(&entries[0]);
    queue.enqueue(&entries[0]);
    queue.enqueue(&replacement);
    assert_eq!(identities(first_batch), expected);
    assert!(!queue.is_empty(), "new work is separate from the consumed batch");
    assert_eq!(identities(queue.take()), HashSet::from([
        Arc::as_ptr(&entries[0]) as usize, Arc::as_ptr(&replacement) as usize,
    ]));
    assert!(queue.is_empty());
    assert_eq!(queue.take().count(), 0);
}

#[test]
fn mount_vault_task_retirement_tickets_are_weak_and_last_pin_driven() {
    let queue = Arc::new(RetirementQueue::default());
    let identity = entry(&queue);
    let first = EntryPin::new(Arc::clone(&identity));
    let last = first.clone();
    identity.schedule_retirement();
    assert!(queue.is_empty());
    assert_eq!(identity.pins.load(Ordering::Acquire), 2);
    drop(first);
    assert!(queue.is_empty(), "another operation still pins this entry");
    drop(last);
    assert_eq!(identity.pins.load(Ordering::Acquire), 0);
    let old_batch = queue.take();
    let repinned = EntryPin::new(Arc::clone(&identity));
    drop(repinned);
    let expected = HashSet::from([Arc::as_ptr(&identity) as usize]);
    assert_eq!(identities(old_batch), expected);
    assert_eq!(identities(queue.take()), expected, "last pin can reschedule taken work");

    queue.enqueue(&identity);
    assert_eq!(Arc::strong_count(&identity), 1);
    drop(identity);
    let mut dead_batch = queue.take();
    assert!(dead_batch.next().unwrap().upgrade().is_none());
    assert!(dead_batch.next().is_none());
    assert!(queue.is_empty());
}

fn engine(directory: &FixtureDirectory, backend: &Arc<OptimizationBackend>, mib: u32) -> MountEngine {
    let config = MountRuntimeConfig::new(
        MountId::parse("vault-retirement-task").unwrap(), MountMode::ReadWrite,
    ).with_cache_policy(MountCachePolicy::new(mib).unwrap());
    let engine = MountEngine::open_host_cache(config, backend.clone(), directory.path()).unwrap();
    engine.prepare_host_remote().unwrap();
    engine
}

#[test]
fn mount_vault_task_retirement_last_pin_reuses_clean_spool_without_double_disposal() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/note", b"pinned bytes");
    let engine = engine(&directory, &backend, 500);
    let acquisition = engine.materialize_at("\\note", OpenDisposition::OpenExisting).unwrap();
    let identity = Arc::clone(&acquisition);
    let last = acquisition.clone();
    drop(acquisition);
    engine.retirements.enqueue(&identity); // An old ticket may meet a new pin.
    engine.maintain_cache().unwrap();
    assert!(!lock(&identity.state).unwrap().retired);
    assert!(lock(&engine.entries).unwrap().contains_key("/note"));
    assert!(engine.retirements.is_empty());

    drop(last);
    assert!(!engine.retirements.is_empty());
    engine.maintain_cache().unwrap();
    let old_spool = {
        let state = lock(&identity.state).unwrap();
        assert!(state.retired);
        state.spool_name.clone()
    };
    assert!(lock(&engine.entries).unwrap().is_empty());
    let reopened = engine.materialize_at("\\note", OpenDisposition::OpenExisting).unwrap();
    assert!(!Arc::ptr_eq(&identity, &reopened));
    assert_eq!(lock(&reopened.state).unwrap().spool_name, old_spool);
    engine.retirements.enqueue(&identity);
    engine.maintain_cache().unwrap();
    let mut bytes = Vec::new();
    engine.spool.open_file(&old_spool, false).unwrap().read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"pinned bytes", "late old identity cannot dispose adopted spool");
    assert!(!lock(&reopened.state).unwrap().retired);
    assert_eq!(backend.read_count(), 1, "the same clean spool was adopted");
    drop(reopened);
    engine.maintain_cache().unwrap();
    assert!(lock(&engine.entries).unwrap().is_empty());
}

#[test]
fn mount_vault_task_retirement_preserves_dirty_and_recovery_referenced_spools() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/note", b"old");
    let engine = engine(&directory, &backend, 0);
    let handle = engine.open_file("\\note", OpenFileOptions {
        writable: true, disposition: OpenDisposition::OpenExisting,
    }).unwrap();
    engine.write(handle, 0, b"new").unwrap();
    let identity = match engine.handle(handle).unwrap().kind {
        OpenHandleKind::Materialized(pin) => pin.release(),
        _ => panic!("written file must be materialized"),
    };
    engine.close(handle).unwrap();
    engine.maintain_cache().unwrap();
    let (spool, dirty) = {
        let state = lock(&identity.state).unwrap();
        assert!(!state.retired);
        assert_ne!(state.condition, EntryCondition::Clean);
        (state.spool_name.clone(), state.condition.clone())
    };
    assert!(engine.spool.is_recovery_referenced(&spool).unwrap());
    assert!(lock(&engine.entries).unwrap().contains_key("/note"));

    // Model the explicit durability guard: in-memory Clean is not authority
    // to delete a spool whose already-written recovery record still exists.
    {
        let _namespace = write_lock(&engine.namespace).unwrap();
        lock(&identity.state).unwrap().condition = EntryCondition::Clean;
        identity.schedule_retirement();
    }
    engine.maintain_cache().unwrap();
    assert!(!lock(&identity.state).unwrap().retired);
    assert!(lock(&engine.entries).unwrap().get("/note")
        .is_some_and(|current| Arc::ptr_eq(current, &identity)));
    let mut bytes = Vec::new();
    engine.spool.open_file(&spool, false).unwrap().read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"new");
    assert_eq!(backend.bytes("/note"), b"old");
    let _namespace = write_lock(&engine.namespace).unwrap();
    lock(&identity.state).unwrap().condition = dirty;
}
