use super::direct_reciprocal::DirectReciprocalPeer;
use super::direct_reciprocal_wire::{
    DirectRepairDigest, DirectRepairId, DirectRepairPersisted,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectRepairPersistPhase {
    ReceivedHello,
    ReceivedOffer,
}

/// Synchronous request handed to the host persistence adapter. The adapter
/// atomically rechecks terminal policy and durably applies the peer without
/// overwriting conflicting identity/material. Repair ids are scoped to one
/// authenticated stream exchange. A bounded process-local digest binding
/// rejects reuse while exact durable relation CAS makes restart replay safe.
pub(crate) struct DirectRepairPersistRequest<'a> {
    phase: DirectRepairPersistPhase,
    repair_id: DirectRepairId,
    transcript_digest: DirectRepairDigest,
    peer: &'a DirectReciprocalPeer,
}

impl<'a> DirectRepairPersistRequest<'a> {
    pub(super) fn new(
        phase: DirectRepairPersistPhase,
        repair_id: DirectRepairId,
        transcript_digest: DirectRepairDigest,
        peer: &'a DirectReciprocalPeer,
    ) -> Self {
        Self {
            phase,
            repair_id,
            transcript_digest,
            peer,
        }
    }

    pub(crate) fn phase(&self) -> DirectRepairPersistPhase {
        self.phase
    }

    pub(crate) fn repair_id(&self) -> DirectRepairId {
        self.repair_id
    }

    pub(crate) fn transcript_digest(&self) -> DirectRepairDigest {
        self.transcript_digest
    }

    pub(crate) fn peer(&self) -> &'a DirectReciprocalPeer {
        self.peer
    }

    /// Call only after the durable relation transaction has committed. The
    /// state machine rejects receipts for another request or transcript.
    pub(crate) fn receipt_after_durable_commit(
        &self,
        persisted: DirectRepairPersisted,
    ) -> DirectRepairStoreReceipt {
        DirectRepairStoreReceipt {
            phase: self.phase,
            repair_id: self.repair_id,
            transcript_digest: self.transcript_digest,
            persisted,
        }
    }
}

/// This callback is deliberately synchronous: callers running on an async
/// Iroh task must move the consuming state and adapter through `spawn_blocking`.
pub(crate) trait DirectRepairStore: Send {
    fn persist_reciprocal_peer(
        &mut self,
        request: &DirectRepairPersistRequest<'_>,
    ) -> Result<DirectRepairStoreReceipt, DirectRepairStoreError>;
}

pub(crate) struct UnavailableDirectRepairStore;

impl DirectRepairStore for UnavailableDirectRepairStore {
    fn persist_reciprocal_peer(
        &mut self,
        _request: &DirectRepairPersistRequest<'_>,
    ) -> Result<DirectRepairStoreReceipt, DirectRepairStoreError> {
        Err(DirectRepairStoreError::Unavailable)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DirectRepairStoreReceipt {
    phase: DirectRepairPersistPhase,
    repair_id: DirectRepairId,
    transcript_digest: DirectRepairDigest,
    persisted: DirectRepairPersisted,
}

impl DirectRepairStoreReceipt {
    pub(super) fn matches(&self, request: &DirectRepairPersistRequest<'_>) -> bool {
        self.phase == request.phase
            && self.repair_id == request.repair_id
            && self.transcript_digest == request.transcript_digest
    }

    pub(super) fn persisted(&self) -> DirectRepairPersisted {
        self.persisted
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectRepairStoreError {
    Retryable,
    PolicyDenied,
    RelationConflict,
    ReplayConflict,
    StaleLocalIdentity,
    Unavailable,
}
