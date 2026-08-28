use serde::{Deserialize, Serialize};

use super::discovery_signal_types::{
    DiscoveryAdvertisement, DiscoveryKind, DiscoveryOperation, DiscoveryRejectionClass,
    PairingCloseReason, PairingPacketKind,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct DiscoveryOfferRequest {
    pub(super) offer_id: String,
    pub(super) kind: DiscoveryKind,
    pub(super) display_alias: String,
    pub(super) suite: String,
    pub(super) version: u32,
    pub(super) lease_secs: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(super) enum DiscoveryClientMsg {
    PublishDiscovery {
        offer: DiscoveryOfferRequest,
    },
    UnpublishDiscovery {
        offer_id: String,
    },
    ListDiscoveries,
    StartPairing {
        discovery_id: String,
        exchange_id: String,
        payload: String,
    },
    PairingPacket {
        exchange_id: String,
        kind: PairingPacketKind,
        payload: String,
    },
    CancelPairing {
        exchange_id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(super) enum DiscoveryServerMsg {
    DiscoveryPublished {
        advertisement: DiscoveryAdvertisement,
    },
    DiscoveryList {
        advertisements: Vec<DiscoveryAdvertisement>,
    },
    PairingOpened {
        exchange_id: String,
        discovery_id: String,
    },
    PairingStarted {
        exchange_id: String,
        discovery_id: String,
        payload: String,
    },
    PairingPacket {
        exchange_id: String,
        kind: PairingPacketKind,
        payload: String,
    },
    PairingFinished {
        exchange_id: String,
        reason: PairingCloseReason,
    },
    DiscoveryRejected {
        operation: DiscoveryOperation,
        offer_id: Option<String>,
        discovery_id: Option<String>,
        exchange_id: Option<String>,
        classification: DiscoveryRejectionClass,
        retryable: bool,
        msg: String,
    },
}

pub(super) fn is_discovery_server_tag(tag: &str) -> bool {
    matches!(
        tag,
        "discovery_published"
            | "discovery_list"
            | "pairing_opened"
            | "pairing_started"
            | "pairing_packet"
            | "pairing_finished"
            | "discovery_rejected"
    )
}
