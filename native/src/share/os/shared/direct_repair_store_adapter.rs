use std::collections::{HashMap, VecDeque};
use std::fmt;

use super::direct_reciprocal_store::{
    DirectRepairPersistPhase, DirectRepairPersistRequest, DirectRepairStore,
    DirectRepairStoreError, DirectRepairStoreReceipt,
};
use super::direct_reciprocal_wire::{
    DirectRepairDigest, DirectRepairId, DirectRepairPersisted,
};
use super::discovery_relation_store::{RelationStore, RelationStoreError};

const DEFAULT_REPLAY_CAPACITY: usize = 1_024;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ReplayKey {
    phase: u8,
    repair_id: DirectRepairId,
}

#[derive(Clone, Copy)]
struct ReplayEntry {
    digest: DirectRepairDigest,
    persisted: DirectRepairPersisted,
}

/// Adapts the common durable relation transaction to the synchronous repair
/// callback used by the Iroh protocol.
///
/// `RelationStore::persist_direct` remains the authority for reloading current
/// policy and applying the exact-material compare-and-set. In particular,
/// `SystemRelationStore` performs that check in the same durable profile
/// transaction. A receipt is constructed only after that call has returned a
/// committed canonical snapshot.
///
/// The replay index is deliberately bounded and process-local. Repair ids are
/// generated for one authenticated stream exchange and are never reused by
/// the local initiator. Across a restart, exact relation material remains
/// idempotent in `persist_direct`, while changed relation or identity material
/// fails the durable compare-and-set. The in-memory digest binding rejects id
/// reuse while it remains in the bounded current-process replay window.
pub(crate) struct DirectRepairRelationStoreAdapter<S> {
    relation_store: S,
    replay: HashMap<ReplayKey, ReplayEntry>,
    replay_order: VecDeque<ReplayKey>,
    replay_capacity: usize,
}

impl<S> DirectRepairRelationStoreAdapter<S>
where
    S: RelationStore,
{
    pub(crate) fn new(relation_store: S) -> Self {
        Self::with_replay_capacity(relation_store, DEFAULT_REPLAY_CAPACITY)
    }

    pub(crate) fn with_replay_capacity(relation_store: S, replay_capacity: usize) -> Self {
        let replay_capacity = replay_capacity.clamp(1, DEFAULT_REPLAY_CAPACITY);
        Self {
            relation_store,
            replay: HashMap::with_capacity(replay_capacity),
            replay_order: VecDeque::with_capacity(replay_capacity),
            replay_capacity,
        }
    }

    pub(crate) fn into_inner(self) -> S {
        self.relation_store
    }

    fn replay_key(request: &DirectRepairPersistRequest<'_>) -> ReplayKey {
        let phase = match request.phase() {
            DirectRepairPersistPhase::ReceivedHello => 1,
            DirectRepairPersistPhase::ReceivedOffer => 2,
        };
        ReplayKey {
            phase,
            repair_id: request.repair_id(),
        }
    }

    fn remember(&mut self, key: ReplayKey, entry: ReplayEntry) {
        while self.replay.len() >= self.replay_capacity {
            let Some(oldest) = self.replay_order.pop_front() else {
                break;
            };
            self.replay.remove(&oldest);
        }
        self.replay.insert(key, entry);
        self.replay_order.push_back(key);
    }
}

impl<S> fmt::Debug for DirectRepairRelationStoreAdapter<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectRepairRelationStoreAdapter")
            .field("relation_store", &"[INJECTED]")
            .field("replay_entries", &self.replay.len())
            .field("replay_capacity", &self.replay_capacity)
            .finish()
    }
}

impl<S> DirectRepairStore for DirectRepairRelationStoreAdapter<S>
where
    S: RelationStore,
{
    fn persist_reciprocal_peer(
        &mut self,
        request: &DirectRepairPersistRequest<'_>,
    ) -> Result<DirectRepairStoreReceipt, DirectRepairStoreError> {
        let key = Self::replay_key(request);
        if let Some(entry) = self.replay.get(&key).copied() {
            if entry.digest != request.transcript_digest() {
                return Err(DirectRepairStoreError::ReplayConflict);
            }
            return Ok(request.receipt_after_durable_commit(entry.persisted));
        }

        let commit = self
            .relation_store
            .persist_direct(request.peer())
            .map_err(map_relation_store_error)?;
        let persisted = if commit.changed() {
            DirectRepairPersisted::Changed
        } else {
            DirectRepairPersisted::AlreadyComplete
        };
        self.remember(
            key,
            ReplayEntry {
                digest: request.transcript_digest(),
                persisted,
            },
        );
        Ok(request.receipt_after_durable_commit(persisted))
    }
}

fn map_relation_store_error(error: RelationStoreError) -> DirectRepairStoreError {
    match error {
        RelationStoreError::Unavailable(_) => DirectRepairStoreError::Unavailable,
        RelationStoreError::Persistence(_) => DirectRepairStoreError::Retryable,
        RelationStoreError::PolicyDenied(_) => DirectRepairStoreError::PolicyDenied,
        RelationStoreError::Invalid(_) | RelationStoreError::Conflict(_) => {
            DirectRepairStoreError::RelationConflict
        }
    }
}
