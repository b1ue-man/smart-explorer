//! Secret-safe control protocol for a connection-owned discovery exchange.
use serde::{Deserialize, Serialize};

use crate::share::{DiscoveryAdvertisement, DiscoveryPin, DiscoveryRelationOutcome};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum DiscoveryPairingRequest {
    List,
    Pair {
        discovery_id: String,
        pin: DiscoveryPin,
        #[serde(default)]
        share_back: bool,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DiscoveryPairingStatus {
    Completed,
    Unconfirmed,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DiscoveryPairingError {
    InvalidPin,
    WorkerUnavailable,
    Offline,
    Unsupported,
    OfferUnavailable,
    AlreadyPairing,
    IncompatibleOffer,
    PairingFailed,
    Cancelled,
    Timeout,
    ObservationLost,
    AcknowledgementLost,
    DaemonError,
}

impl DiscoveryPairingError {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::InvalidPin => "invalid_pin",
            Self::WorkerUnavailable => "worker_unavailable",
            Self::Offline => "offline",
            Self::Unsupported => "unsupported",
            Self::OfferUnavailable => "offer_unavailable",
            Self::AlreadyPairing => "already_pairing",
            Self::IncompatibleOffer => "incompatible_offer",
            Self::PairingFailed => "pairing_failed",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
            Self::ObservationLost => "observation_lost",
            Self::AcknowledgementLost => "acknowledgement_lost",
            Self::DaemonError => "daemon_error",
        }
    }

    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::InvalidPin => "PIN input is empty, invalid UTF-8, or exceeds the PIN limit",
            Self::WorkerUnavailable => {
                "Share worker is unavailable; inspect Share status and refresh its configuration"
            }
            Self::Offline => "Share server is not connected",
            Self::Unsupported => "Share server does not support discovery pairing",
            Self::OfferUnavailable => {
                "Discovery offer is unavailable or expired; list offers again"
            }
            Self::AlreadyPairing => "This discovery offer already has an active exchange",
            Self::IncompatibleOffer => "Discovery offer uses an incompatible pairing protocol",
            Self::PairingFailed => "Pairing failed; check the PIN, offer, peer and trust policy",
            Self::Cancelled => "Discovery exchange was cancelled",
            Self::Timeout => "Discovery response did not arrive before its deadline",
            Self::ObservationLost => "Discovery outcome could not be observed reliably",
            Self::AcknowledgementLost => {
                "Worker acknowledgement is missing; the request outcome was not confirmed"
            }
            Self::DaemonError => "Discovery daemon request failed",
        }
    }

    /// Never forward a foreign or crypto error string to logs or the client.
    pub(super) fn from_command(error: &str) -> Self {
        if error.contains("nicht bestaetigt") {
            Self::AcknowledgementLost
        } else if error.contains("bereits") && error.contains("Austausch") {
            Self::AlreadyPairing
        } else if error.contains("nicht mehr in der aktuellen Liste") {
            Self::OfferUnavailable
        } else if error.contains("inkompatible") {
            Self::IncompatibleOffer
        } else if error.contains("nicht verbunden") {
            Self::Offline
        } else if error.contains("unterstuetzt") {
            Self::Unsupported
        } else {
            Self::PairingFailed
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum DiscoveryPairingReply {
    List {
        offers: Vec<DiscoveryAdvertisement>,
    },
    Started {
        discovery_id: String,
        exchange_id: String,
    },
    Finished {
        discovery_id: String,
        exchange_id: String,
        status: DiscoveryPairingStatus,
        outcome: Option<DiscoveryRelationOutcome>,
        error_code: Option<DiscoveryPairingError>,
    },
    Error {
        error_code: DiscoveryPairingError,
    },
}
