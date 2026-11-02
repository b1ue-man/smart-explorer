//! Locks keyed by a path or a Drive namespace slot (parent ID + name): only
//! operations on the same key wait for each other; unrelated folders, uploads
//! and renames run in parallel.
use std::collections::HashSet;
use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

pub(super) struct KeyLocks {
    active: Mutex<HashSet<String>>,
    released: Condvar,
    poisoned: &'static str,
}

/// Holds one key until dropped.
pub(super) struct KeyGuard {
    locks: Arc<KeyLocks>,
    key: String,
}

impl KeyLocks {
    pub(super) fn new(poisoned: &'static str) -> Arc<Self> {
        Arc::new(Self {
            active: Mutex::new(HashSet::new()),
            released: Condvar::new(),
            poisoned,
        })
    }

    fn active(&self) -> io::Result<MutexGuard<'_, HashSet<String>>> {
        self.active
            .lock()
            .map_err(|_| io::Error::other(self.poisoned))
    }
}

/// Wait until nobody holds `key`, then hold it.
pub(super) fn lock(locks: &Arc<KeyLocks>, key: &str) -> io::Result<KeyGuard> {
    let mut active = locks.active()?;
    while active.contains(key) {
        active = locks
            .released
            .wait(active)
            .map_err(|_| io::Error::other(locks.poisoned))?;
    }
    active.insert(key.to_string());
    Ok(KeyGuard {
        locks: Arc::clone(locks),
        key: key.to_string(),
    })
}

/// Hold every key, taken in sorted order so two callers with overlapping sets
/// never wait for each other in a cycle.
pub(super) fn lock_all(
    locks: &Arc<KeyLocks>,
    keys: impl IntoIterator<Item = String>,
) -> io::Result<Vec<KeyGuard>> {
    let mut keys: Vec<String> = keys.into_iter().collect();
    keys.sort();
    keys.dedup();
    keys.iter().map(|key| lock(locks, key)).collect()
}

/// Key of the Drive namespace slot `name` below folder `parent_id`. Drive IDs
/// never contain '/', so the key is unambiguous for every name.
pub(super) fn slot(parent_id: &str, name: &str) -> String {
    format!("{parent_id}/{name}")
}

impl Drop for KeyGuard {
    fn drop(&mut self) {
        // The set only gains or loses whole keys, so it stays usable after a
        // panic elsewhere; releasing must never leave a key held forever.
        let mut active = self
            .locks
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        active.remove(&self.key);
        drop(active);
        self.locks.released.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn transfer_engine_task_drive_key_locks_block_only_the_same_key() {
        let locks = KeyLocks::new("test lock poisoned");
        let held = lock(&locks, "root/a").unwrap();
        // Another key is free at once, even while "root/a" is held.
        drop(lock(&locks, "root/b").unwrap());

        let (tx, rx) = mpsc::channel();
        let waiter = {
            let locks = Arc::clone(&locks);
            std::thread::spawn(move || {
                let guards =
                    lock_all(&locks, ["root/b".to_string(), "root/a".to_string()]).unwrap();
                tx.send(guards.len()).unwrap();
            })
        };
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        drop(held);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
        waiter.join().unwrap();
        assert_eq!(slot("parent", "a/b"), "parent/a/b");
        assert_eq!(
            lock_all(&locks, ["x".to_string(), "x".to_string()])
                .unwrap()
                .len(),
            1
        );
    }
}
