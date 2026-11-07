use std::fmt;

use super::direct_protocol::DirectPeerIdentity;
use super::direct_reciprocal::{DirectReciprocalPeer, DirectRelationMaterial};
use super::direct_reciprocal_store::{
    DirectRepairPersistPhase, DirectRepairPersistRequest, DirectRepairStore,
    DirectRepairStoreReceipt,
};
use super::direct_reciprocal_wire::{
    DirectRepairCommit, DirectRepairComplete, DirectRepairDigest, DirectRepairHello,
    DirectRepairId, DirectRepairMaterial, DirectRepairOffer, DirectRepairPersisted,
};

#[path = "direct_reciprocal_session_error.rs"]
mod error;
pub use error::DirectRepairSessionError;

/// The authorization decision already enforced by the outer Direct handshake.
/// Fresh requests may use `IncomingFreshNoDecisionGrant` only after their
/// secret proof has been checked, no prior explicit denial exists, and the
/// resulting incoming grant has been durably accepted. Ignored, rejected, or
/// revoked state must be supplied as `ExplicitPolicyDenied`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectSessionAuthorization {
    OutgoingAcceptedContact,
    IncomingAcceptedGrant,
    IncomingFreshNoDecisionGrant,
    ExplicitPolicyDenied,
}

/// Pins captured only after the outer transport has authenticated a Direct
/// session and observed `direct_reciprocal_v1` in its requested capabilities.
pub struct AuthenticatedDirectSession {
    remote_device_id: String,
    tls_node_id: String,
    saved_public_key: String,
    saved_fingerprint: String,
    saved_node_id: String,
    authorization: DirectSessionAuthorization,
}

impl AuthenticatedDirectSession {
    #[allow(clippy::too_many_arguments)]
    pub fn from_verified_handshake(
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
        let node_pin_matches =
            self.saved_node_id.is_empty() || self.saved_node_id == identity.node_id;
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

pub struct DirectRepairInitiator;

pub struct DirectRepairInitiatorAwaitingOffer {
    session: AuthenticatedDirectSession,
    expected_remote_material: Option<DirectRelationMaterial>,
    repair_id: DirectRepairId,
    hello_digest: DirectRepairDigest,
    accepts_renewal: bool,
}

pub struct DirectRepairInitiatorAwaitingStore {
    repair_id: DirectRepairId,
    offer_digest: DirectRepairDigest,
    peer: DirectReciprocalPeer,
    receiver_persisted: DirectRepairPersisted,
}

pub struct DirectRepairInitiatorAwaitingComplete {
    repair_id: DirectRepairId,
    commit_digest: DirectRepairDigest,
    receiver_persisted: DirectRepairPersisted,
    initiator_persisted: DirectRepairPersisted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Safe for live configuration application after `Complete` was received.
pub struct DirectRepairInitiatorComplete {
    pub repair_id: DirectRepairId,
    pub receiver_persisted: DirectRepairPersisted,
    pub initiator_persisted: DirectRepairPersisted,
}

impl DirectRepairInitiator {
    pub fn begin(
        local_identity: DirectPeerIdentity,
        local_material: &DirectRelationMaterial,
        session: AuthenticatedDirectSession,
        expected_remote_material: Option<DirectRelationMaterial>,
    ) -> Result<(DirectRepairInitiatorAwaitingOffer, DirectRepairHello), DirectRepairSessionError>
    {
        Self::begin_with(
            local_identity,
            local_material,
            session,
            expected_remote_material,
            false,
        )
    }

    /// E17: as `begin`, but the hello (kind 5) accepts renewed data of the
    /// host in its offer. Only for a host whose signed presence announced
    /// the renewal (`PeerPresence::announces_code_renewal`).
    pub fn begin_renewal(
        local_identity: DirectPeerIdentity,
        local_material: &DirectRelationMaterial,
        session: AuthenticatedDirectSession,
        expected_remote_material: Option<DirectRelationMaterial>,
    ) -> Result<(DirectRepairInitiatorAwaitingOffer, DirectRepairHello), DirectRepairSessionError>
    {
        Self::begin_with(
            local_identity,
            local_material,
            session,
            expected_remote_material,
            true,
        )
    }

    fn begin_with(
        local_identity: DirectPeerIdentity,
        local_material: &DirectRelationMaterial,
        session: AuthenticatedDirectSession,
        expected_remote_material: Option<DirectRelationMaterial>,
        accepts_renewal: bool,
    ) -> Result<(DirectRepairInitiatorAwaitingOffer, DirectRepairHello), DirectRepairSessionError>
    {
        validate_local(&local_identity, &session)?;
        session.require_outgoing()?;
        let repair_id = DirectRepairId::generate().map_err(DirectRepairSessionError::Wire)?;
        let hello = DirectRepairHello {
            repair_id,
            identity: local_identity,
            material: DirectRepairMaterial::from_domain(local_material),
            accepts_renewal,
        };
        let state = DirectRepairInitiatorAwaitingOffer {
            session,
            expected_remote_material,
            repair_id,
            hello_digest: hello.digest(),
            accepts_renewal,
        };
        Ok((state, hello))
    }
}

impl DirectRepairInitiatorAwaitingOffer {
    pub fn accept_offer(
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
            self.accepts_renewal,
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
    pub fn persist_with(
        self,
        store: &mut (impl DirectRepairStore + ?Sized),
    ) -> Result<(DirectRepairInitiatorAwaitingComplete, DirectRepairCommit), DirectRepairSessionError>
    {
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
    pub fn accept_complete(
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

#[path = "direct_reciprocal_session_receiver.rs"]
mod receiver;
pub use receiver::{
    DirectRepairReceiver, DirectRepairReceiverAwaitingCommit, DirectRepairReceiverAwaitingStore,
    DirectRepairReceiverComplete,
};

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

/// With `allow_renewal` (initiator of a kind-5 hello), other data than the
/// saved ones are the peer's renewed data: the identity is already bound to
/// the TLS key and the saved pins, so only the peer itself can offer them.
fn authenticate_peer(
    session: &AuthenticatedDirectSession,
    identity: DirectPeerIdentity,
    material: DirectRepairMaterial,
    expected: Option<&DirectRelationMaterial>,
    allow_renewal: bool,
) -> Result<DirectReciprocalPeer, DirectRepairSessionError> {
    session.verify_presented(&identity)?;
    let material = material
        .into_domain()
        .map_err(|_| DirectRepairSessionError::InvalidRelationMaterial)?;
    let renewed = expected.is_some_and(|expected| !relation_material_matches(expected, &material));
    if renewed && !allow_renewal {
        return Err(DirectRepairSessionError::RelationMaterialConflict);
    }
    let peer = DirectReciprocalPeer::authenticated(identity, material)
        .map_err(|_| DirectRepairSessionError::InvalidPresentedIdentity)?;
    Ok(if renewed { peer.into_renewal() } else { peer })
}

fn relation_material_matches(
    expected: &DirectRelationMaterial,
    presented: &DirectRelationMaterial,
) -> bool {
    let secret_difference = expected
        .secret()
        .iter()
        .zip(presented.secret())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        });
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
