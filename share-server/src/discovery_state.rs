use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::protocol::{
    DiscoveryAdvertisement, DiscoveryOfferRequest, PairingCloseReason, PairingPacketKind,
};
use super::state::State;
use super::{Out, Writer};

pub(super) const MAX_DISCOVERY_LIST_ENTRIES: usize = 256;
pub(super) const MAX_OFFER_ID_BYTES: usize = 128;
pub(super) const MAX_EXCHANGE_ID_BYTES: usize = 128;
pub(super) const MAX_SERVER_LEASE: Duration = Duration::from_secs(5 * 60);

const MAX_DISCOVERY_OFFERS_GLOBAL: usize = 256;
const MAX_DISCOVERY_OFFERS_PER_CLIENT: usize = 8;
const MAX_ACTIVE_EXCHANGES_GLOBAL: usize = 256;
const MAX_ACTIVE_EXCHANGES_PER_CLIENT: usize = 8;
const MAX_ACTIVE_EXCHANGES_PER_OFFER: usize = 4;
const MAX_PAIRING_PAYLOAD_BYTES: usize = 192 * 1024;
const MAX_PAIRING_TOTAL_BYTES: usize = 1024 * 1024;
const MAX_PAIRING_PACKETS: u16 = 16;
const MAX_DISPLAY_ALIAS_BYTES: usize = 256;
const MAX_SUITE_BYTES: usize = 96;
const MAX_EXCHANGE_LIFETIME: Duration = Duration::from_secs(2 * 60);
const ATTEMPT_WINDOW: Duration = Duration::from_secs(60);
const MAX_STARTS_PER_OFFER_WINDOW: usize = 12;

pub(super) struct DiscoveryOffer {
    pub(super) owner_id: u64,
    pub(super) advertisement: DiscoveryAdvertisement,
    pub(super) deadline: Instant,
    pub(super) recent_starts: VecDeque<Instant>,
}

impl DiscoveryOffer {
    pub(super) fn new(
        owner_id: u64,
        advertisement: DiscoveryAdvertisement,
        deadline: Instant,
    ) -> Self {
        Self {
            owner_id,
            advertisement,
            deadline,
            recent_starts: VecDeque::new(),
        }
    }
}

pub(super) struct DiscoveryExchange {
    pub(super) discovery_id: String,
    pub(super) publisher_id: u64,
    pub(super) connector_id: u64,
    deadline: Instant,
    next_step: PairingStep,
    packet_count: u16,
    payload_bytes: usize,
}

#[derive(Clone, Copy)]
enum PairingStep {
    PublisherKe2,
    ConnectorKe3Bundle,
    PublisherBundle,
    ConnectorCommit,
    PublisherCommit,
}

pub(super) enum PacketRoute {
    Forward { target: Writer, completes: bool },
    Reject { message: &'static str, close: bool },
}

pub(super) fn prepare_exchange_locked(
    state: &mut State,
    connector_id: u64,
    discovery_id: &str,
    exchange_id: &str,
    payload_bytes: usize,
    now: Instant,
) -> Result<Writer, &'static str> {
    if state.discovery_exchanges.contains_key(exchange_id) {
        return Err("pairing exchange id is already active");
    }
    if state.discovery_exchanges.len() >= MAX_ACTIVE_EXCHANGES_GLOBAL {
        return Err("server pairing exchange limit reached");
    }
    if state
        .discovery_exchanges
        .values()
        .filter(|exchange| {
            exchange.publisher_id == connector_id || exchange.connector_id == connector_id
        })
        .count()
        >= MAX_ACTIVE_EXCHANGES_PER_CLIENT
    {
        return Err("client pairing exchange limit reached");
    }
    let Some(offer) = state.discovery_offers.get(discovery_id) else {
        return Err("discovery offer is unavailable");
    };
    let publisher_id = offer.owner_id;
    let offer_deadline = offer.deadline;
    if publisher_id == connector_id {
        return Err("cannot pair with own discovery offer");
    }
    if state
        .discovery_exchanges
        .values()
        .filter(|exchange| exchange.discovery_id == discovery_id)
        .count()
        >= MAX_ACTIVE_EXCHANGES_PER_OFFER
    {
        return Err("discovery offer pairing limit reached");
    }
    let Some(publisher) = state.clients.get(&publisher_id) else {
        return Err("discovery publisher is offline");
    };
    if !publisher
        .capabilities
        .contains(super::discovery::CAPABILITY)
    {
        return Err("discovery publisher capability is unavailable");
    }
    let publisher = publisher.writer.clone();
    let offer = state
        .discovery_offers
        .get_mut(discovery_id)
        .ok_or("discovery offer disappeared before rate limiting")?;
    while offer
        .recent_starts
        .front()
        .is_some_and(|started| now.saturating_duration_since(*started) >= ATTEMPT_WINDOW)
    {
        offer.recent_starts.pop_front();
    }
    if offer.recent_starts.len() >= MAX_STARTS_PER_OFFER_WINDOW {
        return Err("discovery offer pairing attempt rate exceeded");
    }
    offer.recent_starts.push_back(now);
    state.discovery_exchanges.insert(
        exchange_id.to_string(),
        DiscoveryExchange {
            discovery_id: discovery_id.to_string(),
            publisher_id,
            connector_id,
            deadline: offer_deadline.min(now + MAX_EXCHANGE_LIFETIME),
            next_step: PairingStep::PublisherKe2,
            packet_count: 1,
            payload_bytes,
        },
    );
    Ok(publisher)
}

pub(super) fn route_packet_locked(
    state: &mut State,
    client_id: u64,
    exchange_id: &str,
    kind: PairingPacketKind,
    payload_bytes: usize,
) -> PacketRoute {
    let Some(exchange) = state.discovery_exchanges.get(exchange_id) else {
        return PacketRoute::Reject {
            message: "pairing exchange is unavailable",
            close: false,
        };
    };
    let from_publisher = exchange.publisher_id == client_id;
    let from_connector = exchange.connector_id == client_id;
    if !from_publisher && !from_connector {
        return PacketRoute::Reject {
            message: "pairing exchange does not belong to this client",
            close: false,
        };
    }
    let transition = match (exchange.next_step, from_publisher, kind) {
        (PairingStep::PublisherKe2, true, PairingPacketKind::OpaqueKe2) => Some((
            PairingStep::ConnectorKe3Bundle,
            exchange.connector_id,
            false,
        )),
        (PairingStep::ConnectorKe3Bundle, false, PairingPacketKind::OpaqueKe3Bundle)
            if from_connector =>
        {
            Some((PairingStep::PublisherBundle, exchange.publisher_id, false))
        }
        (PairingStep::PublisherBundle, true, PairingPacketKind::PublisherBundle) => {
            Some((PairingStep::ConnectorCommit, exchange.connector_id, false))
        }
        (PairingStep::ConnectorCommit, false, PairingPacketKind::ConnectorCommit)
            if from_connector =>
        {
            Some((PairingStep::PublisherCommit, exchange.publisher_id, false))
        }
        (PairingStep::PublisherCommit, true, PairingPacketKind::PublisherCommit) => {
            Some((PairingStep::PublisherCommit, exchange.connector_id, true))
        }
        _ => None,
    };
    let Some((next_step, target_id, completes)) = transition else {
        return PacketRoute::Reject {
            message: "pairing packet role or stage is invalid",
            close: true,
        };
    };
    let Some(next_count) = exchange.packet_count.checked_add(1) else {
        return PacketRoute::Reject {
            message: "pairing packet count overflow",
            close: true,
        };
    };
    let Some(next_bytes) = exchange.payload_bytes.checked_add(payload_bytes) else {
        return PacketRoute::Reject {
            message: "pairing payload size overflow",
            close: true,
        };
    };
    if next_count > MAX_PAIRING_PACKETS || next_bytes > MAX_PAIRING_TOTAL_BYTES {
        return PacketRoute::Reject {
            message: "pairing exchange limits exceeded",
            close: true,
        };
    }
    let Some(target) = state
        .clients
        .get(&target_id)
        .map(|client| client.writer.clone())
    else {
        return PacketRoute::Reject {
            message: "pairing peer is offline",
            close: true,
        };
    };
    let Some(exchange) = state.discovery_exchanges.get_mut(exchange_id) else {
        return PacketRoute::Reject {
            message: "pairing exchange disappeared during transition",
            close: false,
        };
    };
    exchange.next_step = next_step;
    exchange.packet_count = next_count;
    exchange.payload_bytes = next_bytes;
    PacketRoute::Forward { target, completes }
}

pub(super) fn prune_locked(state: &mut State, now: Instant) -> Vec<(Writer, Out)> {
    let expired_offers = state
        .discovery_offers
        .iter()
        .filter_map(|(discovery_id, offer)| (offer.deadline <= now).then(|| discovery_id.clone()))
        .collect::<Vec<_>>();
    let mut notifications = Vec::new();
    for discovery_id in expired_offers {
        notifications.extend(remove_offer_locked(
            state,
            &discovery_id,
            PairingCloseReason::OfferExpired,
            None,
        ));
    }
    let expired_exchanges = state
        .discovery_exchanges
        .iter()
        .filter_map(|(exchange_id, exchange)| {
            (exchange.deadline <= now).then(|| exchange_id.clone())
        })
        .collect::<Vec<_>>();
    for exchange_id in expired_exchanges {
        notifications.extend(finish_exchange_locked(
            state,
            &exchange_id,
            PairingCloseReason::TimedOut,
            None,
        ));
    }
    notifications
}

pub(super) fn remove_offer_locked(
    state: &mut State,
    discovery_id: &str,
    reason: PairingCloseReason,
    skip_client: Option<u64>,
) -> Vec<(Writer, Out)> {
    let Some(offer) = state.discovery_offers.remove(discovery_id) else {
        return Vec::new();
    };
    state
        .discovery_offer_index
        .remove(&(offer.owner_id, offer.advertisement.offer_id));
    let exchange_ids = state
        .discovery_exchanges
        .iter()
        .filter_map(|(exchange_id, exchange)| {
            (exchange.discovery_id == discovery_id).then(|| exchange_id.clone())
        })
        .collect::<Vec<_>>();
    let mut notifications = Vec::new();
    for exchange_id in exchange_ids {
        notifications.extend(finish_exchange_locked(
            state,
            &exchange_id,
            reason,
            skip_client,
        ));
    }
    notifications
}

pub(super) fn finish_exchange_locked(
    state: &mut State,
    exchange_id: &str,
    reason: PairingCloseReason,
    skip_client: Option<u64>,
) -> Vec<(Writer, Out)> {
    let Some(exchange) = state.discovery_exchanges.remove(exchange_id) else {
        return Vec::new();
    };
    [exchange.publisher_id, exchange.connector_id]
        .into_iter()
        .filter(|client_id| Some(*client_id) != skip_client)
        .filter_map(|client_id| {
            state.clients.get(&client_id).map(|client| {
                (
                    client.writer.clone(),
                    Out::PairingFinished {
                        exchange_id: exchange_id.to_string(),
                        reason,
                    },
                )
            })
        })
        .collect()
}

pub(super) fn validate_offer_request(request: &DiscoveryOfferRequest) -> Result<(), &'static str> {
    if !valid_text(&request.offer_id, MAX_OFFER_ID_BYTES, false) {
        return Err("invalid offer id");
    }
    if !valid_text(&request.display_alias, MAX_DISPLAY_ALIAS_BYTES, false) {
        return Err("invalid discovery display alias");
    }
    if !valid_text(&request.suite, MAX_SUITE_BYTES, false) || request.version == 0 {
        return Err("invalid discovery protocol suite");
    }
    if request.lease_secs == 0 {
        return Err("discovery lease must be positive");
    }
    Ok(())
}

pub(super) fn same_public_offer(
    advertisement: &DiscoveryAdvertisement,
    request: &DiscoveryOfferRequest,
) -> bool {
    advertisement.offer_id == request.offer_id
        && advertisement.kind == request.kind
        && advertisement.display_alias == request.display_alias
        && advertisement.suite == request.suite
        && advertisement.version == request.version
}

pub(super) fn valid_payload(payload: &str) -> bool {
    valid_text(payload, MAX_PAIRING_PAYLOAD_BYTES, false)
}

pub(super) fn valid_text(value: &str, max_bytes: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.len() <= max_bytes
        && !value.chars().any(char::is_control)
}

pub(super) fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

pub(super) fn offer_capacity_available(state: &State, client_id: u64) -> Result<(), &'static str> {
    if state.discovery_offers.len() >= MAX_DISCOVERY_OFFERS_GLOBAL {
        return Err("server discovery offer limit reached");
    }
    if state
        .discovery_offers
        .values()
        .filter(|offer| offer.owner_id == client_id)
        .count()
        >= MAX_DISCOVERY_OFFERS_PER_CLIENT
    {
        return Err("client discovery offer limit reached");
    }
    Ok(())
}
