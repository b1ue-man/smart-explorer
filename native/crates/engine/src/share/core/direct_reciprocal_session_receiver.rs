//! Receiving side of the reciprocal Direct session (moved out of
//! `direct_reciprocal_session.rs`; E17 adds the retiring data).
use super::*;

pub struct DirectRepairReceiver {
    local_identity: DirectPeerIdentity,
    local_material: DirectRelationMaterial,
    session: AuthenticatedDirectSession,
    expected_remote_material: Option<DirectRelationMaterial>,
    /// E17: the data of the retiring lookup id the session used.
    retiring_material: Option<DirectRelationMaterial>,
}

pub struct DirectRepairReceiverAwaitingStore {
    local_identity: DirectPeerIdentity,
    local_material: DirectRelationMaterial,
    repair_id: DirectRepairId,
    hello_digest: DirectRepairDigest,
    peer: DirectReciprocalPeer,
    offers_current: bool,
}

pub struct DirectRepairReceiverAwaitingCommit {
    repair_id: DirectRepairId,
    offer_digest: DirectRepairDigest,
    receiver_persisted: DirectRepairPersisted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The outer receiver must flush/ack the paired `Complete` before applying the
/// live configuration, so it cannot invalidate its own repair stream.
pub struct DirectRepairReceiverComplete {
    pub repair_id: DirectRepairId,
    pub receiver_persisted: DirectRepairPersisted,
    pub initiator_persisted: DirectRepairPersisted,
}

impl DirectRepairReceiver {
    pub fn new(
        local_identity: DirectPeerIdentity,
        local_material: DirectRelationMaterial,
        session: AuthenticatedDirectSession,
        expected_remote_material: Option<DirectRelationMaterial>,
    ) -> Result<Self, DirectRepairSessionError> {
        validate_local(&local_identity, &session)?;
        session.require_incoming()?;
        Ok(Self {
            local_identity,
            local_material,
            session,
            expected_remote_material,
            retiring_material: None,
        })
    }

    /// E17: the session used a retiring lookup id with these data.
    pub fn with_retiring_material(mut self, material: DirectRelationMaterial) -> Self {
        self.retiring_material = Some(material);
        self
    }

    pub fn accept_hello(
        self,
        hello: DirectRepairHello,
    ) -> Result<DirectRepairReceiverAwaitingStore, DirectRepairSessionError> {
        let hello_digest = hello.digest();
        let accepts_renewal = hello.accepts_renewal;
        let peer = authenticate_peer(
            &self.session,
            hello.identity,
            hello.material,
            self.expected_remote_material.as_ref(),
            false,
        )?;
        // E17: a peer on a retiring lookup id that did not ask for renewed
        // data keeps exactly the data it used; nothing new reaches it.
        let (local_material, offers_current) = match (accepts_renewal, self.retiring_material) {
            (false, Some(retiring)) => (retiring, false),
            _ => (self.local_material, true),
        };
        Ok(DirectRepairReceiverAwaitingStore {
            local_identity: self.local_identity,
            local_material,
            repair_id: hello.repair_id,
            hello_digest,
            peer,
            offers_current,
        })
    }
}

impl DirectRepairReceiverAwaitingStore {
    /// The offer carries this device's current data (not retiring ones).
    pub fn offers_current(&self) -> bool {
        self.offers_current
    }

    pub fn persist_with(
        self,
        store: &mut (impl DirectRepairStore + ?Sized),
    ) -> Result<(DirectRepairReceiverAwaitingCommit, DirectRepairOffer), DirectRepairSessionError>
    {
        let request = DirectRepairPersistRequest::new(
            DirectRepairPersistPhase::ReceivedHello,
            self.repair_id,
            self.hello_digest,
            &self.peer,
        );
        let receipt = store.persist_reciprocal_peer(&request)?;
        validate_receipt(&request, receipt)?;
        let offer = DirectRepairOffer {
            repair_id: self.repair_id,
            hello_digest: self.hello_digest,
            identity: self.local_identity,
            material: DirectRepairMaterial::from_domain(&self.local_material),
            persisted: receipt.persisted(),
        };
        let state = DirectRepairReceiverAwaitingCommit {
            repair_id: self.repair_id,
            offer_digest: offer.digest(),
            receiver_persisted: receipt.persisted(),
        };
        Ok((state, offer))
    }
}

impl DirectRepairReceiverAwaitingCommit {
    pub fn accept_commit(
        self,
        commit: DirectRepairCommit,
    ) -> Result<(DirectRepairReceiverComplete, DirectRepairComplete), DirectRepairSessionError>
    {
        if commit.repair_id != self.repair_id {
            return Err(DirectRepairSessionError::RepairIdMismatch);
        }
        if commit.offer_digest != self.offer_digest {
            return Err(DirectRepairSessionError::TranscriptMismatch);
        }
        let complete = DirectRepairComplete {
            repair_id: self.repair_id,
            commit_digest: commit.digest(),
        };
        let result = DirectRepairReceiverComplete {
            repair_id: self.repair_id,
            receiver_persisted: self.receiver_persisted,
            initiator_persisted: commit.persisted,
        };
        Ok((result, complete))
    }
}
