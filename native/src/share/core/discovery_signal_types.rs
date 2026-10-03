use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

pub const DISCOVERY_EXCHANGE_CAPABILITY: &str = "discovery_exchange_v1";
pub const DISCOVERY_PAIRING_SUITE: &str =
    "se-discovery-opaque-r255-sha512-argon2id-chacha20poly1305";
pub const DISCOVERY_PAIRING_VERSION: u32 = 1;
pub const DISCOVERY_MAX_SERVER_LEASE_SECS: u32 = 300;
pub const DISCOVERY_PIN_MAX_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryKind {
    Direct,
    Room,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryOperation {
    PublishDiscovery,
    UnpublishDiscovery,
    ListDiscoveries,
    StartPairing,
    PairingPacket,
    CancelPairing,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryRejectionClass {
    Unsupported,
    InvalidRequest,
    Conflict,
    Forbidden,
    Unavailable,
    Capacity,
    RateLimited,
    Protocol,
    Internal,
}

impl DiscoveryKind {
    pub const fn wire_tag(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Room => "room",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryAdvertisement {
    pub discovery_id: String,
    pub offer_id: String,
    pub kind: DiscoveryKind,
    pub display_alias: String,
    pub suite: String,
    pub version: u32,
    pub expires_at: i64,
}

impl DiscoveryAdvertisement {
    pub fn is_compatible(&self) -> bool {
        self.suite == DISCOVERY_PAIRING_SUITE && self.version == DISCOVERY_PAIRING_VERSION
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiscoveryPublishTarget {
    Direct,
    Room { room_profile_id: String },
}

impl DiscoveryPublishTarget {
    pub const fn kind(&self) -> DiscoveryKind {
        match self {
            Self::Direct => DiscoveryKind::Direct,
            Self::Room { .. } => DiscoveryKind::Room,
        }
    }
}

impl std::fmt::Debug for DiscoveryPublishTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Direct => formatter.write_str("Direct"),
            Self::Room { .. } => formatter.write_str("Room { room_profile_id: [REDACTED] }"),
        }
    }
}

/// Exact UTF-8 PIN bytes with redacted diagnostics and drop-time wiping. No
/// normalization applies; short or trivial PINs need the explicit
/// `allow_weak_pin` of the publishing command (`discovery_pin`).
#[derive(PartialEq, Eq)]
pub struct DiscoveryPin(Vec<u8>);

impl DiscoveryPin {
    pub fn new(pin: String) -> Self {
        Self(pin.into_bytes())
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Clone for DiscoveryPin {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl std::fmt::Debug for DiscoveryPin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl Drop for DiscoveryPin {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl Serialize for DiscoveryPin {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let pin = std::str::from_utf8(&self.0).map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(pin)
    }
}

impl<'de> Deserialize<'de> for DiscoveryPin {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut pin = String::deserialize(deserializer)?;
        if pin.len() > DISCOVERY_PIN_MAX_BYTES {
            pin.zeroize();
            return Err(serde::de::Error::custom(
                "Discovery-PIN ueberschreitet das lokale Ressourcenlimit",
            ));
        }
        Ok(Self::new(pin))
    }
}

/// Public UI-to-worker port. PIN text is consumed byte-for-byte: the transport
/// never trims, parses or normalizes it. Publishing checks the PIN strength
/// unless `allow_weak_pin` is set, and the duration (at most 30 minutes).
#[derive(Clone, Serialize, Deserialize)]
pub enum DiscoveryCommand {
    Publish {
        target: DiscoveryPublishTarget,
        display_alias: String,
        pin: DiscoveryPin,
        duration_secs: u64,
        /// The user chose "Unsichere PIN erlauben" for a short or trivial PIN.
        #[serde(default)]
        allow_weak_pin: bool,
    },
    StopPublishing {
        offer_id: String,
    },
    ListDiscoveries,
    StartDiscoveryExchange {
        discovery_id: String,
        pin: DiscoveryPin,
        /// The user chose "Auch meine Freigaben für dieses Gerät öffnen":
        /// a Direct pairing then also opens this device's exports (FC1/S21).
        #[serde(default)]
        share_back: bool,
    },
    CancelDiscoveryExchange {
        exchange_id: String,
    },
}

impl std::fmt::Debug for DiscoveryCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Publish {
                target,
                display_alias,
                duration_secs,
                allow_weak_pin,
                ..
            } => formatter
                .debug_struct("Publish")
                .field("target", target)
                .field("display_alias", display_alias)
                .field("pin", &"[REDACTED]")
                .field("duration_secs", duration_secs)
                .field("allow_weak_pin", allow_weak_pin)
                .finish(),
            Self::StopPublishing { offer_id } => formatter
                .debug_struct("StopPublishing")
                .field("offer_id", offer_id)
                .finish(),
            Self::ListDiscoveries => formatter.write_str("ListDiscoveries"),
            Self::StartDiscoveryExchange {
                discovery_id,
                share_back,
                ..
            } => formatter
                .debug_struct("StartDiscoveryExchange")
                .field("discovery_id", discovery_id)
                .field("pin", &"[REDACTED]")
                .field("share_back", share_back)
                .finish(),
            Self::CancelDiscoveryExchange { exchange_id } => formatter
                .debug_struct("CancelDiscoveryExchange")
                .field("exchange_id", exchange_id)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryOfferStopReason {
    Requested,
    Expired,
    TargetUnavailable,
    CapabilityUnavailable,
    TransportError,
    /// The daemon stopped or replaced the Share worker that held the offer.
    WorkerStopped,
    /// A device paired with the offer; it is single-use (FC2).
    Paired,
    /// Too many attempts failed to prove the PIN; someone may be guessing.
    TooManyFailedAttempts,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiscoveryEvent {
    OfferPrepared {
        offer_id: String,
        target: DiscoveryPublishTarget,
        display_alias: String,
        discoverable_until: i64,
    },
    OfferPublished {
        offer_id: String,
        target: DiscoveryPublishTarget,
        display_alias: String,
        discoverable_until: i64,
    },
    OfferStopped {
        offer_id: String,
        reason: DiscoveryOfferStopReason,
    },
    DiscoveryList {
        advertisements: Vec<DiscoveryAdvertisement>,
    },
    ExchangeStarted {
        exchange_id: String,
        discovery_id: String,
    },
    ExchangeCompleted {
        exchange_id: String,
        discovery_id: String,
        outcome: super::discovery_relation_store::DiscoveryRelationOutcome,
    },
    ExchangeCancelled {
        exchange_id: String,
        discovery_id: Option<String>,
    },
    ExchangeFailed {
        exchange_id: Option<String>,
        discovery_id: Option<String>,
        error: String,
    },
    /// The relation was already installed (or the Room material handed out)
    /// when the exchange ended without the other side's confirmation:
    /// "gekoppelt – Bestätigung fehlt", with `outcome` for "Widerrufen" (S24).
    ExchangeUnconfirmed {
        exchange_id: String,
        discovery_id: Option<String>,
        outcome: super::discovery_relation_store::DiscoveryRelationOutcome,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryOfferHandle {
    pub offer_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryExchangeHandle {
    pub exchange_id: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PairingPacketKind {
    OpaqueKe2,
    OpaqueKe3Bundle,
    PublisherBundle,
    ConnectorCommit,
    PublisherCommit,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PairingCloseReason {
    Completed,
    Cancelled,
    TimedOut,
    OfferExpired,
    OfferWithdrawn,
    PeerDisconnected,
    TargetUnavailable,
    ProtocolError,
}
