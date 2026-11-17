//! Client-side pool of authenticated `share-exec/3` connections (plan
//! 2026-11-14 E10, D4 B3; modeled after OpenSSH `ControlMaster` /
//! `ControlPersist`).
//!
//! At most one primary `/3` connection is pooled per `(local identity,
//! relation, remote peer)` key. Concurrent jobs share that connection by
//! opening independent bidirectional QUIC streams up to the peer's stream
//! limit (`max_concurrent_bidi_streams`). After the last job on a pooled
//! connection finishes, the connection stays open for 60 s (`ControlPersist`)
//! and then closes with `IDLE_CLOSE_CODE`. Peers that negotiate `/2` in the
//! TLS handshake are remembered for the lifetime of the node so later jobs
//! dial `/2` directly without retry overhead.
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use iroh::endpoint::{Connection, VarInt};
use sha2::{Digest, Sha256};

use crate::share::exec_client::v3::V3ConnectionInfo;
use crate::share::identity::ShareIdentity;
use crate::share::node::{IDLE_CLOSE_CODE, IDLE_CLOSE_REASON};
use crate::share::session::relation_kind_id;
use crate::share::types::PeerEndpoint;

/// Idle keep-open window after the last job completes (`ControlPersist`).
pub(in crate::share) const DEFAULT_EXEC_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::share) struct ExecPoolKey {
    local_device_id: String,
    local_public_key: String,
    local_node_id: String,
    remote_node_id: String,
    remote_device_id: String,
    remote_public_key: String,
    relation_kind: String,
    relation_id: String,
    secret_digest: [u8; 32],
}

impl ExecPoolKey {
    pub(in crate::share) fn of(identity: &ShareIdentity, endpoint: &PeerEndpoint) -> Self {
        let (relation_kind, relation_id) = relation_kind_id(endpoint);
        Self {
            local_device_id: identity.device_id.clone(),
            local_public_key: identity.public_key.clone(),
            local_node_id: identity.node_id.clone(),
            remote_node_id: endpoint.presence.node_id.clone(),
            remote_device_id: endpoint.presence.device_id.clone(),
            remote_public_key: endpoint.presence.public_key.clone(),
            relation_kind: relation_kind.into(),
            relation_id,
            secret_digest: Sha256::digest(&endpoint.relation_secret).into(),
        }
    }
}

struct PooledExecEntry {
    entry_id: u64,
    connection: Connection,
    info: V3ConnectionInfo,
    active_jobs: AtomicUsize,
    last_idle_seq: AtomicU64,
}

pub(in crate::share) struct ExecClientPool {
    rt: Weak<tokio::runtime::Runtime>,
    entries: Mutex<HashMap<ExecPoolKey, Arc<PooledExecEntry>>>,
    connect_gates: Mutex<HashMap<ExecPoolKey, Weak<tokio::sync::Mutex<()>>>>,
    legacy_v2_peers: Mutex<HashSet<String>>,
    next_entry_id: AtomicU64,
    idle_timeout_ms: AtomicU64,
}

impl ExecClientPool {
    pub(in crate::share) fn new(rt: &Arc<tokio::runtime::Runtime>) -> Arc<Self> {
        Arc::new(Self {
            rt: Arc::downgrade(rt),
            entries: Mutex::new(HashMap::new()),
            connect_gates: Mutex::new(HashMap::new()),
            legacy_v2_peers: Mutex::new(HashSet::new()),
            next_entry_id: AtomicU64::new(0),
            idle_timeout_ms: AtomicU64::new(DEFAULT_EXEC_IDLE_TIMEOUT.as_millis() as u64),
        })
    }

    pub(in crate::share) fn is_legacy_v2(&self, remote_node_id: &str) -> bool {
        !remote_node_id.is_empty()
            && self
                .legacy_v2_peers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(remote_node_id)
    }

    pub(in crate::share) fn mark_legacy_v2(&self, remote_node_id: String) {
        if !remote_node_id.is_empty() {
            self.legacy_v2_peers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(remote_node_id);
        }
    }

    pub(in crate::share) fn connect_gate(&self, key: &ExecPoolKey) -> Arc<tokio::sync::Mutex<()>> {
        let mut gates = self
            .connect_gates
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        gates.retain(|_, weak| weak.strong_count() > 0);
        if let Some(existing) = gates.get(key).and_then(Weak::upgrade) {
            return existing;
        }
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        gates.insert(key.clone(), Arc::downgrade(&gate));
        gate
    }

    /// Acquires a lease on the currently pooled `/3` connection for `key`,
    /// evicting it if it has already closed.
    pub(in crate::share) fn try_lease_existing(
        self: &Arc<Self>,
        key: &ExecPoolKey,
    ) -> Option<PoolLeaseGuard> {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = entries.get(key)?;
        if entry.connection.close_reason().is_some() {
            entries.remove(key);
            return None;
        }
        entry.active_jobs.fetch_add(1, Ordering::AcqRel);
        entry.last_idle_seq.fetch_add(1, Ordering::AcqRel);
        let entry = Arc::clone(entry);
        drop(entries);
        Some(PoolLeaseGuard {
            pool: Arc::clone(self),
            key: key.clone(),
            entry,
            pooled: true,
        })
    }

    /// Inserts a newly authenticated `/3` connection (when `pooled` is true)
    /// and returns the initial job lease (`active_jobs == 1`).
    pub(in crate::share) fn insert_and_lease(
        self: &Arc<Self>,
        key: ExecPoolKey,
        connection: Connection,
        info: V3ConnectionInfo,
        pooled: bool,
    ) -> PoolLeaseGuard {
        let entry_id = self.next_entry_id.fetch_add(1, Ordering::Relaxed) + 1;
        let entry = Arc::new(PooledExecEntry {
            entry_id,
            connection,
            info,
            active_jobs: AtomicUsize::new(1),
            last_idle_seq: AtomicU64::new(1),
        });
        if pooled {
            let replaced = self
                .entries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key.clone(), Arc::clone(&entry));
            if let Some(old) = replaced {
                if old.active_jobs.load(Ordering::Acquire) == 0 {
                    old.connection
                        .close(VarInt::from_u32(IDLE_CLOSE_CODE), IDLE_CLOSE_REASON);
                }
            }
        }
        PoolLeaseGuard {
            pool: Arc::clone(self),
            key,
            entry,
            pooled,
        }
    }

    pub(in crate::share) fn close_all(&self) {
        let drained: Vec<Arc<PooledExecEntry>> = self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain()
            .map(|(_, entry)| entry)
            .collect();
        for entry in drained {
            entry
                .connection
                .close(VarInt::from_u32(IDLE_CLOSE_CODE), IDLE_CLOSE_REASON);
        }
    }

    fn idle_timeout(&self) -> Duration {
        Duration::from_millis(self.idle_timeout_ms.load(Ordering::Acquire))
    }

    fn is_current_entry(&self, key: &ExecPoolKey, entry_id: u64) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .is_some_and(|current| current.entry_id == entry_id)
    }

    fn evict_entry(&self, key: &ExecPoolKey, entry_id: u64) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if entries
            .get(key)
            .is_some_and(|current| current.entry_id == entry_id)
        {
            entries.remove(key);
        }
    }

    #[cfg(test)]
    pub(in crate::share) fn set_idle_timeout_for_test(&self, timeout: Duration) {
        let millis = u64::try_from(timeout.as_millis())
            .unwrap_or(u64::MAX)
            .max(1);
        self.idle_timeout_ms.store(millis, Ordering::Release);
    }

    #[cfg(test)]
    pub(in crate::share) fn pooled_connection(&self, key: &ExecPoolKey) -> Option<Connection> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .map(|entry| entry.connection.clone())
    }
}

pub(in crate::share) struct PoolLeaseGuard {
    pool: Arc<ExecClientPool>,
    key: ExecPoolKey,
    entry: Arc<PooledExecEntry>,
    pooled: bool,
}

impl PoolLeaseGuard {
    pub(in crate::share) fn connection(&self) -> &Connection {
        &self.entry.connection
    }

    pub(in crate::share) fn info(&self) -> &V3ConnectionInfo {
        &self.entry.info
    }

    pub(in crate::share) fn active_jobs(&self) -> usize {
        self.entry.active_jobs.load(Ordering::Acquire)
    }

    pub(in crate::share) fn is_sole_owner(&self) -> bool {
        self.active_jobs() <= 1
    }

    pub(in crate::share) fn evict(&self) {
        if self.pooled {
            self.pool.evict_entry(&self.key, self.entry.entry_id);
        }
    }

    pub(in crate::share) fn evict_and_close_if_sole(&self) {
        self.evict();
        if self.is_sole_owner() {
            self.entry.connection.close(
                VarInt::from_u32(0x4558),
                b"Exec pooled connection discarded",
            );
        }
    }
}

impl Drop for PoolLeaseGuard {
    fn drop(&mut self) {
        let prev = self.entry.active_jobs.fetch_sub(1, Ordering::AcqRel);
        if prev != 1 {
            return;
        }
        if !self.pooled {
            self.entry
                .connection
                .close(VarInt::from_u32(IDLE_CLOSE_CODE), IDLE_CLOSE_REASON);
            return;
        }
        if self.entry.connection.close_reason().is_some() {
            self.pool.evict_entry(&self.key, self.entry.entry_id);
            return;
        }
        if !self.pool.is_current_entry(&self.key, self.entry.entry_id) {
            self.entry
                .connection
                .close(VarInt::from_u32(IDLE_CLOSE_CODE), IDLE_CLOSE_REASON);
            return;
        }
        let seq = self.entry.last_idle_seq.fetch_add(1, Ordering::AcqRel) + 1;
        let Some(rt) = self.pool.rt.upgrade() else {
            return;
        };
        let pool = Arc::clone(&self.pool);
        let key = self.key.clone();
        let entry = Arc::clone(&self.entry);
        let timeout = pool.idle_timeout();
        rt.spawn(async move {
            tokio::select! {
                () = tokio::time::sleep(timeout) => {}
                _ = entry.connection.closed() => {}
            }
            let should_close = {
                let mut entries = pool.entries.lock().unwrap_or_else(PoisonError::into_inner);
                let idle = entries.get(&key).is_some_and(|current| {
                    current.entry_id == entry.entry_id
                        && current.active_jobs.load(Ordering::Acquire) == 0
                        && current.last_idle_seq.load(Ordering::Acquire) == seq
                });
                if idle {
                    entries.remove(&key);
                }
                idle
            };
            if should_close {
                entry
                    .connection
                    .close(VarInt::from_u32(IDLE_CLOSE_CODE), IDLE_CLOSE_REASON);
            }
        });
    }
}
