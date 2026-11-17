//! Open incoming `share-exec/3` connections of a host node.
//!
//! Unlike `/2` (one job per connection), a `/3` connection stays open across
//! jobs until the client's idle timer expires or permissions change. When a
//! grant or relation is revoked or expires:
//! - an idle `/3` connection (`active_streams == 0`) closes immediately with
//!   `0x5345` (`"authorization changed"`);
//! - a `/3` connection with running job stream(s) refuses new streams and
//!   closes as soon as the last running stream finishes delivering its
//!   `Revoked` terminal status and `ResultAcknowledged`.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use iroh::endpoint::{Connection, VarInt};

use super::super::access_authorize::principal_grant;
use super::super::core::now_secs;
use super::super::exec_auth::{authorize_stream_in, ExecRejection};
#[cfg(test)]
use super::super::exec_job::test_runner::TestExecRunner;
use super::super::identity::code_renewal_registry::retiring_secret;
use super::super::relation_proof::ProvenPrincipal;
use super::super::types::ShareAuthState;

const AUTH_CHANGED_CODE: u32 = 0x5345;
const AUTH_CHANGED_REASON: &[u8] = b"authorization changed";

struct HostExecEntry {
    connection: Connection,
    principal: ProvenPrincipal,
    active_streams: AtomicUsize,
    revoked: AtomicBool,
}

/// Registry of open incoming `/3` connections on one `ShareIrohNode`.
/// Lock order: `ShareIrohNode.auth` -> `entries` (never the reverse).
#[derive(Default)]
pub(in crate::share) struct ExecHostConnections {
    entries: Mutex<HashMap<usize, Arc<HostExecEntry>>>,
    #[cfg(test)]
    test_runner: Mutex<Option<Arc<TestExecRunner>>>,
}

impl ExecHostConnections {
    pub(in crate::share) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Registers an authenticated `/3` connection after re-checking `principal`
    /// under the `auth` lock so no revocation can slip between handshake and
    /// registration.
    pub(in crate::share) fn register(
        self: &Arc<Self>,
        auth: &Mutex<ShareAuthState>,
        connection: &Connection,
        principal: ProvenPrincipal,
    ) -> Result<HostExecGuard, ExecRejection> {
        let state = auth.lock().map_err(|_| ExecRejection::NotAuthorized)?;
        authorize_stream_in(&principal, &state, now_secs())?;
        let id = connection.stable_id();
        let entry = Arc::new(HostExecEntry {
            connection: connection.clone(),
            principal,
            active_streams: AtomicUsize::new(0),
            revoked: AtomicBool::new(false),
        });
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, Arc::clone(&entry));
        Ok(HostExecGuard {
            host: Arc::clone(self),
            id,
            entry,
        })
    }

    /// Re-evaluates every open `/3` connection against `state` at `now`.
    pub(in crate::share) fn recheck(&self, state: &ShareAuthState, now: i64) {
        let mut to_close = Vec::new();
        {
            let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
            for entry in entries.values() {
                if authorize_stream_in(&entry.principal, state, now).is_err() {
                    entry.revoked.store(true, Ordering::SeqCst);
                    if entry.active_streams.load(Ordering::SeqCst) == 0 {
                        to_close.push(entry.connection.clone());
                    }
                }
            }
        }
        for connection in to_close {
            close_revoked(&connection);
        }
    }

    /// Earliest `valid_until` among currently authorized `/3` connections so
    /// `node_access` wakes up when an idle connection's grant expires.
    pub(in crate::share) fn next_deadline(&self, state: &ShareAuthState) -> Option<i64> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries
            .values()
            .filter(|entry| !entry.revoked.load(Ordering::Acquire))
            .filter_map(|entry| {
                let mut current = entry.principal.clone();
                if current.relation_kind == "direct"
                    && current.relation_id != state.identity.direct_lookup_id
                    && retiring_secret(&state.identity.node_id, &current.relation_id).is_some()
                {
                    current.relation_id = state.identity.direct_lookup_id.clone();
                }
                principal_grant(state, &current)?.access_grant().valid_until
            })
            .min()
    }

    /// Closes all open `/3` connections when sharing stops or all sessions end.
    pub(in crate::share) fn close_all(&self) {
        let connections: Vec<Connection> = {
            let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
            entries
                .drain()
                .map(|(_, entry)| {
                    entry.revoked.store(true, Ordering::SeqCst);
                    entry.connection.clone()
                })
                .collect()
        };
        for connection in connections {
            close_revoked(&connection);
        }
    }

    #[cfg(test)]
    pub(in crate::share) fn set_test_runner(&self, runner: Option<TestExecRunner>) {
        *self
            .test_runner
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = runner.map(Arc::new);
    }

    #[cfg(test)]
    pub(in crate::share) fn test_runner(&self) -> Option<Arc<TestExecRunner>> {
        self.test_runner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    #[cfg(test)]
    pub(in crate::share) fn open_count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter(|entry| entry.connection.close_reason().is_none())
            .count()
    }
}

pub(in crate::share) struct HostExecGuard {
    host: Arc<ExecHostConnections>,
    id: usize,
    entry: Arc<HostExecEntry>,
}

impl HostExecGuard {
    pub(in crate::share) fn principal(&self) -> &ProvenPrincipal {
        &self.entry.principal
    }

    /// Admits one job stream unless the connection has already been revoked.
    pub(in crate::share) fn open_stream(&self) -> Option<HostStreamGuard> {
        self.entry.active_streams.fetch_add(1, Ordering::SeqCst);
        if self.entry.revoked.load(Ordering::SeqCst) {
            if self.entry.active_streams.fetch_sub(1, Ordering::SeqCst) == 1 {
                close_revoked(&self.entry.connection);
            }
            return None;
        }
        Some(HostStreamGuard {
            entry: Arc::clone(&self.entry),
        })
    }
}

impl Drop for HostExecGuard {
    fn drop(&mut self) {
        self.host
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}

pub(in crate::share) struct HostStreamGuard {
    entry: Arc<HostExecEntry>,
}

impl HostStreamGuard {
    pub(in crate::share) fn is_revoked(&self) -> bool {
        self.entry.revoked.load(Ordering::Acquire)
    }
}

impl Drop for HostStreamGuard {
    fn drop(&mut self) {
        if self.entry.active_streams.fetch_sub(1, Ordering::SeqCst) == 1
            && self.entry.revoked.load(Ordering::SeqCst)
        {
            close_revoked(&self.entry.connection);
        }
    }
}

fn close_revoked(connection: &Connection) {
    connection.close(VarInt::from_u32(AUTH_CHANGED_CODE), AUTH_CHANGED_REASON);
}
