//! The case of `mount_vault_task_daemon_mutation_fences_persistent_and_waiter_authority`
//! in `vault_cache_task_tests.rs` where the mutation hits another path: it
//! keeps the active shared flight and is no whole-cache epoch.
use super::*;

pub(super) fn mount_vault_task_daemon_unrelated_mutation_preserves_active_shared_flight() {
    for retained in [true, false] {
        let inner = Arc::new(ListingBackend::default());
        inner.entries("/one", &["kept"]);
        inner.entries("/two", &["removed"]);
        let cache = cache(&inner, retained);
        let release = inner.gate("/one");
        let leader = load(&cache, "/one");
        release.wait();
        let waiter = load(&cache, "/one");
        wait_for_owners(&cache, "/one", 2);
        let generation = cache.cache.lock().unwrap().generation;
        cache.remove_file("/two/removed").unwrap();
        assert_eq!(
            cache.cache.lock().unwrap().generation,
            generation,
            "ordinary unrelated mutation is not a whole-cache epoch"
        );
        release.release();
        let first = leader.finish().unwrap();
        let second = waiter.finish().unwrap();
        assert_eq!(first.entries[0].name, "kept");
        assert!(Arc::ptr_eq(&first.entries, &second.entries));
        assert!(Arc::ptr_eq(&first.index, &second.index));
        assert_eq!(
            inner.count("/one"),
            1,
            "unrelated mutation must not discard another path's flight"
        );
        assert!(cache.cache.lock().unwrap().loads.is_empty());
        assert_eq!(inner.active.load(Ordering::SeqCst), 0);
    }
}
