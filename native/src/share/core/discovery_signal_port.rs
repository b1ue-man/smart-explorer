use std::fmt;

use super::direct_protocol::DirectPeerIdentity;
use super::direct_reciprocal::{DirectReciprocalPeer, DirectRelationMaterial};
use super::discovery_relation_store::{DiscoveryRelationOutcome, RelationStoreCommit};
use super::discovery_signal_types::{
    DiscoveryAdvertisement, DiscoveryKind, DiscoveryPublishTarget, PairingCloseReason,
    PairingPacketKind,
};
use super::identity::ShareIdentity;

pub(crate) trait DiscoveryDirectPeerSource: Send {
    fn current_direct_peer(&mut self) -> Result<DirectReciprocalPeer, String>;
}

impl<F> DiscoveryDirectPeerSource for F
where
    F: FnMut() -> Result<DirectReciprocalPeer, String> + Send,
{
    fn current_direct_peer(&mut self) -> Result<DirectReciprocalPeer, String> {
        self()
    }
}

/// Build current local Direct material from the worker's identity snapshot.
pub(crate) fn direct_peer_from_identity(
    identity: &ShareIdentity,
) -> Result<DirectReciprocalPeer, String> {
    let peer_identity = DirectPeerIdentity {
        device_id: identity.device_id.clone(),
        device_name: identity.device_name.clone(),
        node_id: identity.node_id.clone(),
        public_key: identity.public_key.clone(),
        fingerprint: identity.fingerprint.clone(),
    };
    let material = DirectRelationMaterial::new(
        identity.direct_lookup_id.clone(),
        identity.direct_secret(),
    )
    .map_err(|error| error.to_string())?;
    DirectReciprocalPeer::authenticated(peer_identity, material)
        .map_err(|error| error.to_string())
}

/// Public metadata prepared by the cryptographic exchange implementation. The
/// signaling layer may publish this descriptor, but never the PIN, registration
/// record, relation secret, identity, or application bundle behind it.
pub(crate) struct PreparedDiscoveryOffer {
    pub(crate) kind: DiscoveryKind,
    pub(crate) suite: String,
    pub(crate) version: u32,
}

/// One exact, already encoded packet which signaling may forward unchanged.
pub(crate) struct DiscoveryPortPacket {
    pub(crate) kind: PairingPacketKind,
    pub(crate) payload: Vec<u8>,
}

/// A durable profile snapshot and the only packet authorized by that commit.
/// The authoritative, secret-free result is `commit.outcome()`; keeping it in
/// the commit prevents a UI label or a second copy from drifting from storage.
pub(crate) struct PersistedDiscoveryPacket {
    pub(crate) commit: RelationStoreCommit,
    pub(crate) packet: DiscoveryPortPacket,
}

impl PersistedDiscoveryPacket {
    pub(crate) fn outcome(&self) -> &DiscoveryRelationOutcome {
        self.commit.outcome()
    }

    pub(crate) fn into_parts(self) -> (RelationStoreCommit, DiscoveryPortPacket) {
        (self.commit, self.packet)
    }
}

/// One opaque protocol action returned to signaling. Ciphertext prepared
/// before persistence is never exposed unless it appears in one of these
/// successful actions.
pub(crate) enum DiscoveryPortAction {
    StartPairing { payload: Vec<u8> },
    SendPacket(DiscoveryPortPacket),
    PersistedAndSend(PersistedDiscoveryPacket),
    ExchangeReady {
        outcome: DiscoveryRelationOutcome,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DiscoveryPortError {
    InvalidRequest(String),
    TargetUnavailable(String),
    Protocol(String),
    Persistence(String),
}

impl DiscoveryPortError {
    pub(crate) fn close_reason(&self) -> PairingCloseReason {
        match self {
            Self::TargetUnavailable(_) => PairingCloseReason::TargetUnavailable,
            Self::InvalidRequest(_) | Self::Protocol(_) | Self::Persistence(_) => {
                PairingCloseReason::ProtocolError
            }
        }
    }
}

impl fmt::Display for DiscoveryPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(error) => write!(formatter, "invalid discovery request: {error}"),
            Self::TargetUnavailable(error) => {
                write!(formatter, "discovery target unavailable: {error}")
            }
            Self::Protocol(error) => write!(formatter, "discovery protocol failed: {error}"),
            Self::Persistence(error) => {
                write!(formatter, "discovery persistence failed: {error}")
            }
        }
    }
}

impl std::error::Error for DiscoveryPortError {}

/// Narrow crypto/orchestration port owned by the share worker. Implementations
/// retain all PAKE typestate and secret application bundles; signaling retains
/// only ephemeral public routing state and invokes these methods off the UI
/// thread.
pub(crate) trait DiscoveryExchangePort: Send {
    fn prepare_offer(
        &mut self,
        offer_id: &str,
        target: DiscoveryPublishTarget,
        pin: &[u8],
    ) -> Result<PreparedDiscoveryOffer, DiscoveryPortError>;

    fn remove_offer(&mut self, offer_id: &str);

    /// Revalidates the live publication target immediately before renewal.
    fn revalidate_offer(&mut self, offer_id: &str) -> Result<(), DiscoveryPortError>;

    fn start_connector(
        &mut self,
        exchange_id: &str,
        advertisement: &DiscoveryAdvertisement,
        pin: &[u8],
    ) -> Result<DiscoveryPortAction, DiscoveryPortError>;

    fn start_publisher(
        &mut self,
        exchange_id: &str,
        discovery_id: &str,
        offer_id: &str,
        payload: Vec<u8>,
    ) -> Result<DiscoveryPortAction, DiscoveryPortError>;

    fn handle_packet(
        &mut self,
        exchange_id: &str,
        kind: PairingPacketKind,
        payload: Vec<u8>,
    ) -> Result<Option<DiscoveryPortAction>, DiscoveryPortError>;

    /// A completed server route is only UI-complete when this returns
    /// `ExchangeReady`, after final cryptographic verification and persistence.
    fn finish_exchange(
        &mut self,
        exchange_id: &str,
        reason: PairingCloseReason,
    ) -> Result<Option<DiscoveryPortAction>, DiscoveryPortError>;

    fn cancel_exchange(&mut self, exchange_id: &str);
}
