use std::fmt;

use super::direct_protocol::DirectPeerIdentity;
use super::direct_reciprocal::{DirectReciprocalPeer, DirectRelationMaterial};
use super::direct_reciprocal_store::{
    DirectRepairPersistPhase, DirectRepairPersistRequest, DirectRepairStore,
    DirectRepairStoreError, DirectRepairStoreReceipt,
};
use super::direct_reciprocal_wire::{
    DirectRepairCommit, DirectRepairComplete, DirectRepairDigest, DirectRepairHello,
    DirectRepairId, DirectRepairMaterial, DirectRepairOffer, DirectRepairPersisted,
    DirectRepairWireError,
};

/// The authorization decision already enforced by the outer Direct handshake.
/// Fresh requests may use `IncomingFreshNoDecisionGrant` only after their
/// secret proof has been checked, no prior explicit denial exists, and the
/// resulting incoming grant has been durably accepted. Ignored, rejected, or
/// revoked state must be supplied as `ExplicitPolicyDenied`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectSessionAuthorization {
    OutgoingAcceptedContact,
    IncomingAcceptedGrant,
    IncomingFreshNoDecisionGrant,
    ExplicitPolicyDenied,
}

/// Pins captured only after the outer transport has authenticated a Direct
/// session and observed `direct_reciprocal_v1` in its requested capabilities.
pub(crate) struct AuthenticatedDirectSession {
    remote_device_id: String,
    tls_node_id: String,
    saved_public_key: String,
    saved_fingerprint: String,
    saved_node_id: String,
    authorization: DirectSessionAuthorization,
}

impl AuthenticatedDirectSession {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_verified_handshake(
        remote_device_id: String,
        tls_node_id: String,
        saved_public_key: String,
        saved_fingerprint: String,
        saved_node_id: String,
        authorization: DirectSessionAuthorization,
        reciprocal_capability_requested: bool,
    ) -> Result<Self, DirectRepairSessionError> {
        if authorization == DirectSessionAuthorization::ExplicitPolicyDenied {
            return Err(DirectRepairSessionError::PolicyDenied);
        }
        if !reciprocal_capability_requested {
            return Err(DirectRepairSessionError::CapabilityNotRequested);
        }
        let authenticated_pin = DirectPeerIdentity {
            device_id: remote_device_id.clone(),
            device_name: String::new(),
            node_id: tls_node_id.clone(),
            public_key: saved_public_key.clone(),
            fingerprint: saved_fingerprint.clone(),
        };
        authenticated_pin
            .validate()
            .map_err(|_| DirectRepairSessionError::InvalidAuthenticatedBinding)?;
        if !saved_node_id.is_empty() && saved_node_id != tls_node_id {
            return Err(DirectRepairSessionError::InvalidAuthenticatedBinding);
        }
        // `validate` above proves saved_public_key == tls_node_id. This is the
        // sole condition under which an empty legacy saved node pin is healed.
        Ok(Self {
            remote_device_id,
            tls_node_id,
            saved_public_key,
            saved_fingerprint,
            saved_node_id,
            authorization,
        })
    }

    fn require_outgoing(&self) -> Result<(), DirectRepairSessionError> {
        if self.authorization != DirectSessionAuthorization::OutgoingAcceptedContact {
            return Err(DirectRepairSessionError::WrongAuthorizationDirection);
        }
        Ok(())
    }

    fn require_incoming(&self) -> Result<(), DirectRepairSessionError> {
        if matches!(
            self.authorization,
            DirectSessionAuthorization::IncomingAcceptedGrant
                | DirectSessionAuthorization::IncomingFreshNoDecisionGrant
        ) {
            return Ok(());
        }
        Err(DirectRepairSessionError::WrongAuthorizationDirection)
    }

    fn verify_presented(
        &self,
        identity: &DirectPeerIdentity,
    ) -> Result<(), DirectRepairSessionError> {
        identity
            .validate()
            .map_err(|_| DirectRepairSessionError::InvalidPresentedIdentity)?;
        let node_pin_matches = self.saved_node_id.is_empty()
            || self.saved_node_id == identity.node_id;
        if identity.device_id != self.remote_device_id
            || identity.node_id != self.tls_node_id
            || identity.public_key != self.saved_public_key
            || identity.fingerprint != self.saved_fingerprint
            || !node_pin_matches
        {
            return Err(DirectRepairSessionError::IdentityConflict);
        }
        Ok(())
    }
}

impl fmt::Debug for AuthenticatedDirectSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedDirectSession")
            .field("authorization", &self.authorization)
            .field("legacy_node_pin", &self.saved_node_id.is_empty())
            .field("identity", &"[AUTHENTICATED]")
            .finish()
    }
}

pub(crate) struct DirectRepairInitiator;

pub(crate) struct DirectRepairInitiatorAwaitingOffer {
    session: AuthenticatedDirectSession,
    expected_remote_material: Option<DirectRelationMaterial>,
    repair_id: DirectRepairId,
    hello_digest: DirectRepairDigest,
}

pub(crate) struct DirectRepairInitiatorAwaitingStore {
    repair_id: DirectRepairId,
    offer_digest: DirectRepairDigest,
    peer: DirectReciprocalPeer,
    receiver_persisted: DirectRepairPersisted,
}

pub(crate) struct DirectRepairInitiatorAwaitingComplete {
    repair_id: DirectRepairId,
    commit_digest: DirectRepairDigest,
    receiver_persisted: DirectRepairPersisted,
    initiator_persisted: DirectRepairPersisted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Safe for live configuration application after `Complete` was received.
pub(crate) struct DirectRepairInitiatorComplete {
    pub(crate) repair_id: DirectRepairId,
    pub(crate) receiver_persisted: DirectRepairPersisted,
    pub(crate) initiator_persisted: DirectRepairPersisted,
}

impl DirectRepairInitiator {
    pub(crate) fn begin(
        local_identity: DirectPeerIdentity,
        local_material: &DirectRelationMaterial,
        session: AuthenticatedDirectSession,
        expected_remote_material: Option<DirectRelationMaterial>,
    ) -> Result<(DirectRepairInitiatorAwaitingOffer, DirectRepairHello), DirectRepairSessionError>
    {
        validate_local(&local_identity, &session)?;
        session.require_outgoing()?;
        let repair_id = DirectRepairId::generate().map_err(DirectRepairSessionError::Wire)?;
        let hello = DirectRepairHello {
            repair_id,
            identity: local_identity,
            material: DirectRepairMaterial::from_domain(local_material),
        };
        let state = DirectRepairInitiatorAwaitingOffer {
            session,
            expected_remote_material,
            repair_id,
            hello_digest: hello.digest(),
        };
        Ok((state, hello))
    }
}

impl DirectRepairInitiatorAwaitingOffer {
    pub(crate) fn accept_offer(
        self,
        offer: DirectRepairOffer,
    ) -> Result<DirectRepairInitiatorAwaitingStore, DirectRepairSessionError> {
        let offer_digest = offer.digest();
        if offer.repair_id != self.repair_id {
            return Err(DirectRepairSessionError::RepairIdMismatch);
        }
        if offer.hello_digest != self.hello_digest {
            return Err(DirectRepairSessionError::TranscriptMismatch);
        }
        let peer = authenticate_peer(
            &self.session,
            offer.identity,
            offer.material,
            self.expected_remote_material.as_ref(),
        )?;
        Ok(DirectRepairInitiatorAwaitingStore {
            repair_id: self.repair_id,
            offer_digest,
            peer,
            receiver_persisted: offer.persisted,
        })
    }
}

impl DirectRepairInitiatorAwaitingStore {
    pub(crate) fn persist_with(
        self,
        store: &mut (impl DirectRepairStore + ?Sized),
    ) -> Result<
        (DirectRepairInitiatorAwaitingComplete, DirectRepairCommit),
        DirectRepairSessionError,
    > {
        let request = DirectRepairPersistRequest::new(
            DirectRepairPersistPhase::ReceivedOffer,
            self.repair_id,
            self.offer_digest,
            &self.peer,
        );
        let receipt = store.persist_reciprocal_peer(&request)?;
        validate_receipt(&request, receipt)?;
        let commit = DirectRepairCommit {
            repair_id: self.repair_id,
            offer_digest: self.offer_digest,
            persisted: receipt.persisted(),
        };
        let state = DirectRepairInitiatorAwaitingComplete {
            repair_id: self.repair_id,
            commit_digest: commit.digest(),
            receiver_persisted: self.receiver_persisted,
            initiator_persisted: receipt.persisted(),
        };
        Ok((state, commit))
    }
}

impl DirectRepairInitiatorAwaitingComplete {
    pub(crate) fn accept_complete(
        self,
        complete: DirectRepairComplete,
    ) -> Result<DirectRepairInitiatorComplete, DirectRepairSessionError> {
        if complete.repair_id != self.repair_id {
            return Err(DirectRepairSessionError::RepairIdMismatch);
        }
        if complete.commit_digest != self.commit_digest {
            return Err(DirectRepairSessionError::TranscriptMismatch);
        }
        Ok(DirectRepairInitiatorComplete {
            repair_id: self.repair_id,
            receiver_persisted: self.receiver_persisted,
            initiator_persisted: self.initiator_persisted,
        })
    }
}

pub(crate) struct DirectRepairReceiver {
    local_identity: DirectPeerIdentity,
    local_material: DirectRelationMaterial,
    session: AuthenticatedDirectSession,
    expected_remote_material: Option<DirectRelationMaterial>,
}

pub(crate) struct DirectRepairReceiverAwaitingStore {
    local_identity: DirectPeerIdentity,
    local_material: DirectRelationMaterial,
    repair_id: DirectRepairId,
    hello_digest: DirectRepairDigest,
    peer: DirectReciprocalPeer,
}

pub(crate) struct DirectRepairReceiverAwaitingCommit {
    repair_id: DirectRepairId,
    offer_digest: DirectRepairDigest,
    receiver_persisted: DirectRepairPersisted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The outer receiver must flush/ack the paired `Complete` before applying the
/// live configuration, so it cannot invalidate its own repair stream.
pub(crate) struct DirectRepairReceiverComplete {
    pub(crate) repair_id: DirectRepairId,
    pub(crate) receiver_persisted: DirectRepairPersisted,
    pub(crate) initiator_persisted: DirectRepairPersisted,
}

impl DirectRepairReceiver {
    pub(crate) fn new(
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
        })
    }

    pub(crate) fn accept_hello(
        self,
        hello: DirectRepairHello,
    ) -> Result<DirectRepairReceiverAwaitingStore, DirectRepairSessionError> {
        let hello_digest = hello.digest();
        let peer = authenticate_peer(
            &self.session,
            hello.identity,
            hello.material,
            self.expected_remote_material.as_ref(),
        )?;
        Ok(DirectRepairReceiverAwaitingStore {
            local_identity: self.local_identity,
            local_material: self.local_material,
            repair_id: hello.repair_id,
            hello_digest,
            peer,
        })
    }
}

impl DirectRepairReceiverAwaitingStore {
    pub(crate) fn persist_with(
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
    pub(crate) fn accept_commit(
        self,
        commit: DirectRepairCommit,
    ) -> Result<(DirectRepairReceiverComplete, DirectRepairComplete), DirectRepairSessionError> {
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

fn validate_local(
    local: &DirectPeerIdentity,
    session: &AuthenticatedDirectSession,
) -> Result<(), DirectRepairSessionError> {
    local
        .validate()
        .map_err(|_| DirectRepairSessionError::InvalidLocalIdentity)?;
    if local.device_id == session.remote_device_id || local.public_key == session.saved_public_key {
        return Err(DirectRepairSessionError::SelfRelation);
    }
    Ok(())
}

fn authenticate_peer(
    session: &AuthenticatedDirectSession,
    identity: DirectPeerIdentity,
    material: DirectRepairMaterial,
    expected: Option<&DirectRelationMaterial>,
) -> Result<DirectReciprocalPeer, DirectRepairSessionError> {
    session.verify_presented(&identity)?;
    let material = material
        .into_domain()
        .map_err(|_| DirectRepairSessionError::InvalidRelationMaterial)?;
    if expected.is_some_and(|expected| !relation_material_matches(expected, &material)) {
        return Err(DirectRepairSessionError::RelationMaterialConflict);
    }
    DirectReciprocalPeer::authenticated(identity, material)
        .map_err(|_| DirectRepairSessionError::InvalidPresentedIdentity)
}

fn relation_material_matches(
    expected: &DirectRelationMaterial,
    presented: &DirectRelationMaterial,
) -> bool {
    let secret_difference = expected
        .secret()
        .iter()
        .zip(presented.secret())
        .fold(0_u8, |difference, (left, right)| difference | (left ^ right));
    expected.lookup_id() == presented.lookup_id() && secret_difference == 0
}

fn validate_receipt(
    request: &DirectRepairPersistRequest<'_>,
    receipt: DirectRepairStoreReceipt,
) -> Result<(), DirectRepairSessionError> {
    if !receipt.matches(request) {
        return Err(DirectRepairSessionError::InvalidStoreReceipt);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectRepairSessionError {
    PolicyDenied,
    CapabilityNotRequested,
    WrongAuthorizationDirection,
    InvalidAuthenticatedBinding,
    InvalidLocalIdentity,
    InvalidPresentedIdentity,
    IdentityConflict,
    RelationMaterialConflict,
    InvalidRelationMaterial,
    SelfRelation,
    RepairIdMismatch,
    TranscriptMismatch,
    InvalidStoreReceipt,
    Store(DirectRepairStoreError),
    Wire(DirectRepairWireError),
}

impl From<DirectRepairStoreError> for DirectRepairSessionError {
    fn from(error: DirectRepairStoreError) -> Self {
        Self::Store(error)
    }
}

impl fmt::Display for DirectRepairSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PolicyDenied => "local Direct policy explicitly denies reciprocal repair",
            Self::CapabilityNotRequested => "reciprocal Direct capability was not requested",
            Self::WrongAuthorizationDirection => "Direct authorization has the wrong direction",
            Self::InvalidAuthenticatedBinding => "authenticated Direct binding is invalid",
            Self::InvalidLocalIdentity => "local Direct identity is invalid",
            Self::InvalidPresentedIdentity => "presented Direct identity is invalid",
            Self::IdentityConflict => "presented Direct identity conflicts with authenticated pins",
            Self::RelationMaterialConflict => "Direct relation material conflicts with saved state",
            Self::InvalidRelationMaterial => "presented Direct relation material is invalid",
            Self::SelfRelation => "reciprocal Direct repair cannot target the local identity",
            Self::RepairIdMismatch => "reciprocal Direct repair id does not match",
            Self::TranscriptMismatch => "reciprocal Direct transcript does not match",
            Self::InvalidStoreReceipt => "durable Direct store receipt does not match",
            Self::Store(DirectRepairStoreError::Retryable) => "durable store should be retried",
            Self::Store(DirectRepairStoreError::PolicyDenied) => "local policy denied store",
            Self::Store(DirectRepairStoreError::RelationConflict) => "relation conflict in store",
            Self::Store(DirectRepairStoreError::ReplayConflict) => "replay conflict in store",
            Self::Store(DirectRepairStoreError::StaleLocalIdentity) => "local identity changed",
            Self::Store(DirectRepairStoreError::Unavailable) => "durable store is unavailable",
            Self::Wire(error) => return fmt::Display::fmt(error, formatter),
        })
    }
}

impl std::error::Error for DirectRepairSessionError {}
