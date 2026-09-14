//! Free helpers of the discovery exchange port: bundle conversion, packet
//! actions and the error constructors shared by every exchange step.
use super::super::discovery_bundle::{ConnectorApplicationBundle, PublisherApplicationBundle};
use super::super::discovery_domain::PairingBundle;
use super::super::discovery_relation_store::{RelationStoreCommit, RelationStoreError};
use super::super::discovery_signal_port::{
    DiscoveryPortAction, DiscoveryPortError, DiscoveryPortPacket, PersistedDiscoveryPacket,
};
use super::super::discovery_signal_types::{PairingPacketKind, DISCOVERY_PIN_MAX_BYTES};

pub(super) fn pairing_bundle_from_connector(
    bundle: &ConnectorApplicationBundle,
) -> Result<PairingBundle, DiscoveryPortError> {
    let plaintext = bundle.encode_plaintext().map_err(protocol_error)?;
    PairingBundle::new(bundle.kind(), plaintext.as_slice().to_vec()).map_err(protocol_error)
}

pub(super) fn pairing_bundle_from_publisher(
    bundle: &PublisherApplicationBundle,
) -> Result<PairingBundle, DiscoveryPortError> {
    let plaintext = bundle.encode_plaintext().map_err(protocol_error)?;
    PairingBundle::new(bundle.kind(), plaintext.as_slice().to_vec()).map_err(protocol_error)
}

pub(super) fn send_packet(kind: PairingPacketKind, payload: Vec<u8>) -> DiscoveryPortAction {
    DiscoveryPortAction::SendPacket(DiscoveryPortPacket { kind, payload })
}

pub(super) fn persisted_packet(
    commit: RelationStoreCommit,
    kind: PairingPacketKind,
    payload: Vec<u8>,
) -> DiscoveryPortAction {
    DiscoveryPortAction::PersistedAndSend(PersistedDiscoveryPacket {
        commit,
        packet: DiscoveryPortPacket { kind, payload },
    })
}

pub(super) fn require_packet(
    actual: PairingPacketKind,
    expected: PairingPacketKind,
) -> Result<(), DiscoveryPortError> {
    if actual == expected {
        Ok(())
    } else {
        Err(protocol_message(
            "pairing packet arrived in the wrong typestate",
        ))
    }
}

pub(super) fn validate_pin_length(pin: &[u8]) -> Result<(), DiscoveryPortError> {
    if pin.len() > DISCOVERY_PIN_MAX_BYTES {
        Err(DiscoveryPortError::InvalidRequest(
            "PIN exceeds the supported byte limit".to_string(),
        ))
    } else {
        Ok(())
    }
}

pub(super) fn invalid_error(error: impl ToString) -> DiscoveryPortError {
    DiscoveryPortError::InvalidRequest(error.to_string())
}

pub(super) fn protocol_error(error: impl ToString) -> DiscoveryPortError {
    DiscoveryPortError::Protocol(error.to_string())
}

pub(super) fn protocol_message(message: &str) -> DiscoveryPortError {
    DiscoveryPortError::Protocol(message.to_string())
}

pub(super) fn target_error(error: RelationStoreError) -> DiscoveryPortError {
    DiscoveryPortError::TargetUnavailable(error.to_string())
}

pub(super) fn persistence_error(error: RelationStoreError) -> DiscoveryPortError {
    DiscoveryPortError::Persistence(error.to_string())
}
