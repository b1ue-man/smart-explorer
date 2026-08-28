use std::collections::HashMap;
#[path = "discovery_exchange_port_state.rs"]
mod exchange_port_state;

use exchange_port_state::{ExchangeState, PreparedOfferState, UsedIdTracker};

use super::direct_reciprocal::DirectReciprocalPeer;
use super::discovery_bundle::{ConnectorApplicationBundle, PublisherApplicationBundle};
use super::discovery_domain::{DiscoveryId, DiscoveryOfferBinding, ExchangeId, OfferId, PairingBundle};
use super::discovery_pake::{ConnectorAwaitingKe2, PublisherOffer};
use super::discovery_relation_store::{
    DiscoveryRelationOutcome, RelationStore, RelationStoreCommit, RelationStoreError,
};
use super::discovery_signal_port::{
    DiscoveryDirectPeerSource, DiscoveryExchangePort, DiscoveryPortAction, DiscoveryPortError,
    DiscoveryPortPacket, PersistedDiscoveryPacket, PreparedDiscoveryOffer,
};
use super::discovery_signal_types::{
    DiscoveryAdvertisement, DiscoveryKind, DiscoveryPublishTarget, PairingCloseReason,
    PairingPacketKind, DISCOVERY_PAIRING_SUITE, DISCOVERY_PAIRING_VERSION,
    DISCOVERY_PIN_MAX_BYTES,
};
use super::discovery_wire::{
    ConnectorCommit, OpaqueKe1, OpaqueKe2, OpaqueKe3ConnectorBundle, PublisherBundle,
    PublisherCommit,
};
use super::room_relation::RoomJoinIntent;

pub(crate) struct DiscoveryExchangePortImpl {
    direct_peer_source: Box<dyn DiscoveryDirectPeerSource>,
    relation_store: Box<dyn RelationStore>,
    offers: HashMap<String, PreparedOfferState>,
    exchanges: HashMap<String, ExchangeState>,
    used_offer_ids: UsedIdTracker,
    used_exchange_ids: UsedIdTracker,
}

impl DiscoveryExchangePortImpl {
    pub(crate) fn new(
        direct_peer_source: Box<dyn DiscoveryDirectPeerSource>,
        relation_store: Box<dyn RelationStore>,
    ) -> Self {
        Self {
            direct_peer_source,
            relation_store,
            offers: HashMap::new(),
            exchanges: HashMap::new(),
            used_offer_ids: UsedIdTracker::default(),
            used_exchange_ids: UsedIdTracker::default(),
        }
    }

    fn current_direct_peer(&mut self) -> Result<DirectReciprocalPeer, DiscoveryPortError> {
        self.direct_peer_source
            .current_direct_peer()
            .map_err(DiscoveryPortError::TargetUnavailable)
    }

    fn revalidate_target(
        &mut self,
        target: &DiscoveryPublishTarget,
    ) -> Result<(), DiscoveryPortError> {
        match target {
            DiscoveryPublishTarget::Direct => {
                let _current = self.current_direct_peer()?;
            }
            DiscoveryPublishTarget::Room { room_profile_id } => {
                let _current = self
                    .relation_store
                    .load_room(room_profile_id)
                    .map_err(target_error)?;
            }
        }
        Ok(())
    }

    fn connector_bundle(
        &mut self,
        kind: DiscoveryKind,
    ) -> Result<PairingBundle, DiscoveryPortError> {
        let application = match kind {
            DiscoveryKind::Direct => {
                ConnectorApplicationBundle::Direct(self.current_direct_peer()?)
            }
            DiscoveryKind::Room => ConnectorApplicationBundle::Room(RoomJoinIntent),
        };
        pairing_bundle_from_connector(&application)
    }

    fn advance_exchange(
        &mut self,
        state: ExchangeState,
        packet_kind: PairingPacketKind,
        payload: Vec<u8>,
    ) -> Result<(Option<ExchangeState>, Option<DiscoveryPortAction>), DiscoveryPortError> {
        match state {
            ExchangeState::ConnectorAwaitingKe2(state) => {
                require_packet(packet_kind, PairingPacketKind::OpaqueKe2)?;
                let ke2 = OpaqueKe2::from_bytes(payload).map_err(protocol_error)?;
                let connector_bundle = self.connector_bundle(state.binding().offer().kind())?;
                let (next, packet) = state
                    .finish(ke2, connector_bundle)
                    .map_err(protocol_error)?;
                Ok((
                    Some(ExchangeState::ConnectorAwaitingPublisherBundle(next)),
                    Some(send_packet(
                        PairingPacketKind::OpaqueKe3Bundle,
                        packet.to_bytes(),
                    )),
                ))
            }
            ExchangeState::ConnectorAwaitingPublisherBundle(state) => {
                require_packet(packet_kind, PairingPacketKind::PublisherBundle)?;
                let packet = PublisherBundle::from_bytes(payload).map_err(protocol_error)?;
                let (received, encrypted_bundle) = state
                    .accept_publisher_bundle(packet)
                    .map_err(protocol_error)?;
                let application = PublisherApplicationBundle::decode_plaintext(
                    encrypted_bundle.payload().to_vec(),
                )
                .map_err(protocol_error)?;
                if application.kind() != received.binding().offer().kind() {
                    return Err(protocol_message("publisher bundle kind does not match exchange"));
                }

                // Do not expose the exact commit ciphertext before persistence.
                let (next, packet) = received.commit().map_err(protocol_error)?;
                let commit = match application {
                    PublisherApplicationBundle::Direct(peer) => self
                        .relation_store
                        .persist_direct(&peer)
                        .map_err(persistence_error)?,
                    PublisherApplicationBundle::Room(offer) => self
                        .relation_store
                        .persist_room(offer.material(), offer.display_name())
                        .map_err(persistence_error)?,
                };
                let completion = commit.outcome().clone();
                Ok((
                    Some(ExchangeState::ConnectorAwaitingPublisherCommit {
                        state: next,
                        completion,
                    }),
                    Some(persisted_packet(
                        commit,
                        PairingPacketKind::ConnectorCommit,
                        packet.into_bytes(),
                    )),
                ))
            }
            ExchangeState::ConnectorAwaitingPublisherCommit { state, completion } => {
                require_packet(packet_kind, PairingPacketKind::PublisherCommit)?;
                let packet = PublisherCommit::from_bytes(payload).map_err(protocol_error)?;
                let complete = state.finish(packet).map_err(protocol_error)?;
                Ok((
                    Some(ExchangeState::ConnectorComplete {
                        state: complete,
                        completion,
                    }),
                    None,
                ))
            }
            ExchangeState::PublisherAwaitingKe3 { state, target } => {
                require_packet(packet_kind, PairingPacketKind::OpaqueKe3Bundle)?;
                let packet = OpaqueKe3ConnectorBundle::from_bytes(payload)
                    .map_err(protocol_error)?;
                let (received, encrypted_bundle) =
                    state.finish(packet).map_err(protocol_error)?;
                let application = ConnectorApplicationBundle::decode_plaintext(
                    encrypted_bundle.payload().to_vec(),
                )
                .map_err(protocol_error)?;
                if application.kind() != received.binding().offer().kind()
                    || application.kind() != target.kind()
                {
                    return Err(protocol_message("connector bundle kind does not match target"));
                }

                match (target, application) {
                    (DiscoveryPublishTarget::Direct, ConnectorApplicationBundle::Direct(peer)) => {
                        let own = PublisherApplicationBundle::Direct(self.current_direct_peer()?);
                        let outgoing = pairing_bundle_from_publisher(&own)?;
                        let (next, packet) = received
                            .publisher_bundle(outgoing)
                            .map_err(protocol_error)?;
                        let commit = self
                            .relation_store
                            .persist_direct(&peer)
                            .map_err(persistence_error)?;
                        let completion = commit.outcome().clone();
                        Ok((
                            Some(ExchangeState::PublisherAwaitingConnectorCommit {
                                state: next,
                                completion,
                            }),
                            Some(persisted_packet(
                                commit,
                                PairingPacketKind::PublisherBundle,
                                packet.into_bytes(),
                            )),
                        ))
                    }
                    (
                        DiscoveryPublishTarget::Room { room_profile_id },
                        ConnectorApplicationBundle::Room(_),
                    ) => {
                        // Re-read both profile and credential immediately before
                        // producing the publisher bundle. Offer-time aliases and
                        // stale Room material are never completion authority.
                        let room = self
                            .relation_store
                            .load_room(&room_profile_id)
                            .map_err(target_error)?;
                        let completion = DiscoveryRelationOutcome::RoomShared {
                            room_profile_id: room.room_profile_id().to_string(),
                            display_name: room.offer().display_name().to_string(),
                        };
                        let outgoing = PublisherApplicationBundle::Room(room.into_offer());
                        let outgoing = pairing_bundle_from_publisher(&outgoing)?;
                        let (next, packet) = received
                            .publisher_bundle(outgoing)
                            .map_err(protocol_error)?;
                        Ok((
                            Some(ExchangeState::PublisherAwaitingConnectorCommit {
                                state: next,
                                completion,
                            }),
                            Some(send_packet(
                                PairingPacketKind::PublisherBundle,
                                packet.into_bytes(),
                            )),
                        ))
                    }
                    _ => Err(protocol_message("application bundle direction mismatch")),
                }
            }
            ExchangeState::PublisherAwaitingConnectorCommit { state, completion } => {
                require_packet(packet_kind, PairingPacketKind::ConnectorCommit)?;
                let packet = ConnectorCommit::from_bytes(payload).map_err(protocol_error)?;
                let ready = state
                    .accept_connector_commit(packet)
                    .map_err(protocol_error)?;
                let (complete, packet) = ready.commit().map_err(protocol_error)?;
                Ok((
                    Some(ExchangeState::PublisherComplete {
                        state: complete,
                        completion,
                    }),
                    Some(send_packet(
                        PairingPacketKind::PublisherCommit,
                        packet.into_bytes(),
                    )),
                ))
            }
            ExchangeState::ConnectorComplete { .. }
            | ExchangeState::PublisherComplete { .. } => {
                Err(protocol_message("pairing packet received after cryptographic completion"))
            }
        }
    }
}

impl DiscoveryExchangePort for DiscoveryExchangePortImpl {
    fn prepare_offer(
        &mut self,
        offer_id: &str,
        target: DiscoveryPublishTarget,
        pin: &[u8],
    ) -> Result<PreparedDiscoveryOffer, DiscoveryPortError> {
        let offer_id = OfferId::new(offer_id).map_err(invalid_error)?;
        if self.used_offer_ids.contains(offer_id.as_str()) {
            return Err(protocol_message("discovery offer identifier was reused"));
        }
        validate_pin_length(pin)?;
        self.revalidate_target(&target)?;
        let kind = target.kind();
        let binding = DiscoveryOfferBinding::new(kind, offer_id);
        let offer = PublisherOffer::register(binding, pin).map_err(invalid_error)?;
        let offer_key = offer.binding().offer_id().as_str().to_string();
        self.used_offer_ids.remember(offer_key.clone())?;
        self.offers
            .insert(offer_key, PreparedOfferState { target, offer });
        Ok(PreparedDiscoveryOffer {
            kind,
            suite: DISCOVERY_PAIRING_SUITE.to_string(),
            version: DISCOVERY_PAIRING_VERSION,
        })
    }

    fn remove_offer(&mut self, offer_id: &str) {
        self.offers.remove(offer_id);
        self.exchanges
            .retain(|_, state| !state.is_publisher_offer(offer_id));
    }

    fn revalidate_offer(&mut self, offer_id: &str) -> Result<(), DiscoveryPortError> {
        let target = self
            .offers
            .get(offer_id)
            .map(|offer| offer.target.clone())
            .ok_or_else(|| DiscoveryPortError::TargetUnavailable("offer is not active".into()))?;
        self.revalidate_target(&target)
    }

    fn start_connector(
        &mut self,
        exchange_id: &str,
        advertisement: &DiscoveryAdvertisement,
        pin: &[u8],
    ) -> Result<DiscoveryPortAction, DiscoveryPortError> {
        if !advertisement.is_compatible() {
            return Err(protocol_message("advertisement uses an incompatible pairing suite"));
        }
        let offer_id = OfferId::new(advertisement.offer_id.clone()).map_err(invalid_error)?;
        let discovery_id =
            DiscoveryId::new(advertisement.discovery_id.clone()).map_err(invalid_error)?;
        let exchange = ExchangeId::new(exchange_id).map_err(invalid_error)?;
        if self.exchanges.contains_key(exchange.as_str())
            || self.used_exchange_ids.contains(exchange.as_str())
        {
            return Err(protocol_message("pairing exchange identifier was reused"));
        }
        validate_pin_length(pin)?;
        if advertisement.kind == DiscoveryKind::Direct {
            let _current = self.current_direct_peer()?;
        }
        let binding = DiscoveryOfferBinding::new(advertisement.kind, offer_id)
            .for_exchange(discovery_id, exchange);
        let (state, ke1) = ConnectorAwaitingKe2::start(binding, pin).map_err(invalid_error)?;
        self.used_exchange_ids.remember(exchange_id.to_string())?;
        self.exchanges.insert(
            exchange_id.to_string(),
            ExchangeState::ConnectorAwaitingKe2(state),
        );
        Ok(DiscoveryPortAction::StartPairing {
            payload: ke1.into_bytes(),
        })
    }

    fn start_publisher(
        &mut self,
        exchange_id: &str,
        discovery_id: &str,
        offer_id: &str,
        payload: Vec<u8>,
    ) -> Result<DiscoveryPortAction, DiscoveryPortError> {
        let offer_id = OfferId::new(offer_id).map_err(invalid_error)?;
        let target = self
            .offers
            .get(offer_id.as_str())
            .map(|prepared| prepared.target.clone())
            .ok_or_else(|| DiscoveryPortError::TargetUnavailable("offer is not active".into()))?;
        self.revalidate_target(&target)?;
        let discovery_id = DiscoveryId::new(discovery_id).map_err(invalid_error)?;
        let exchange = ExchangeId::new(exchange_id).map_err(invalid_error)?;
        if self.exchanges.contains_key(exchange.as_str())
            || self.used_exchange_ids.contains(exchange.as_str())
        {
            return Err(protocol_message("pairing exchange identifier was reused"));
        }
        let ke1 = OpaqueKe1::from_bytes(payload).map_err(protocol_error)?;
        let prepared = self
            .offers
            .get(offer_id.as_str())
            .ok_or_else(|| DiscoveryPortError::TargetUnavailable("offer is not active".into()))?;
        let binding = prepared
            .offer
            .binding()
            .for_exchange(discovery_id, exchange);
        let (state, ke2) = prepared
            .offer
            .start_exchange(binding, ke1)
            .map_err(protocol_error)?;
        self.used_exchange_ids.remember(exchange_id.to_string())?;
        self.exchanges.insert(
            exchange_id.to_string(),
            ExchangeState::PublisherAwaitingKe3 { state, target },
        );
        Ok(send_packet(
            PairingPacketKind::OpaqueKe2,
            ke2.into_bytes(),
        ))
    }

    fn handle_packet(
        &mut self,
        exchange_id: &str,
        kind: PairingPacketKind,
        payload: Vec<u8>,
    ) -> Result<Option<DiscoveryPortAction>, DiscoveryPortError> {
        let state = self
            .exchanges
            .remove(exchange_id)
            .ok_or_else(|| protocol_message("unknown or already consumed pairing exchange"))?;
        let (next, action) = self.advance_exchange(state, kind, payload)?;
        if let Some(next) = next {
            self.exchanges.insert(exchange_id.to_string(), next);
        }
        Ok(action)
    }

    fn finish_exchange(
        &mut self,
        exchange_id: &str,
        reason: PairingCloseReason,
    ) -> Result<Option<DiscoveryPortAction>, DiscoveryPortError> {
        let state = self.exchanges.remove(exchange_id);
        if reason != PairingCloseReason::Completed {
            return Ok(None);
        }
        let state = state.ok_or_else(|| {
            protocol_message("server completed an unknown or already consumed exchange")
        })?;
        let outcome = match state {
            ExchangeState::ConnectorComplete { completion, .. }
            | ExchangeState::PublisherComplete { completion, .. } => completion,
            _ => {
                return Err(protocol_message(
                    "server completed exchange before local cryptographic completion",
                ))
            }
        };
        Ok(Some(DiscoveryPortAction::ExchangeReady { outcome }))
    }

    fn cancel_exchange(&mut self, exchange_id: &str) {
        self.exchanges.remove(exchange_id);
    }
}

fn pairing_bundle_from_connector(
    bundle: &ConnectorApplicationBundle,
) -> Result<PairingBundle, DiscoveryPortError> {
    let plaintext = bundle.encode_plaintext().map_err(protocol_error)?;
    PairingBundle::new(bundle.kind(), plaintext.as_slice().to_vec()).map_err(protocol_error)
}

fn pairing_bundle_from_publisher(
    bundle: &PublisherApplicationBundle,
) -> Result<PairingBundle, DiscoveryPortError> {
    let plaintext = bundle.encode_plaintext().map_err(protocol_error)?;
    PairingBundle::new(bundle.kind(), plaintext.as_slice().to_vec()).map_err(protocol_error)
}

fn send_packet(kind: PairingPacketKind, payload: Vec<u8>) -> DiscoveryPortAction {
    DiscoveryPortAction::SendPacket(DiscoveryPortPacket { kind, payload })
}

fn persisted_packet(
    commit: RelationStoreCommit,
    kind: PairingPacketKind,
    payload: Vec<u8>,
) -> DiscoveryPortAction {
    DiscoveryPortAction::PersistedAndSend(PersistedDiscoveryPacket {
        commit,
        packet: DiscoveryPortPacket { kind, payload },
    })
}

fn require_packet(
    actual: PairingPacketKind,
    expected: PairingPacketKind,
) -> Result<(), DiscoveryPortError> {
    if actual == expected {
        Ok(())
    } else {
        Err(protocol_message("pairing packet arrived in the wrong typestate"))
    }
}

fn validate_pin_length(pin: &[u8]) -> Result<(), DiscoveryPortError> {
    if pin.len() > DISCOVERY_PIN_MAX_BYTES {
        Err(DiscoveryPortError::InvalidRequest(
            "PIN exceeds the supported byte limit".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn invalid_error(error: impl ToString) -> DiscoveryPortError {
    DiscoveryPortError::InvalidRequest(error.to_string())
}

fn protocol_error(error: impl ToString) -> DiscoveryPortError {
    DiscoveryPortError::Protocol(error.to_string())
}

fn protocol_message(message: &str) -> DiscoveryPortError {
    DiscoveryPortError::Protocol(message.to_string())
}

fn target_error(error: RelationStoreError) -> DiscoveryPortError {
    DiscoveryPortError::TargetUnavailable(error.to_string())
}

fn persistence_error(error: RelationStoreError) -> DiscoveryPortError {
    DiscoveryPortError::Persistence(error.to_string())
}
