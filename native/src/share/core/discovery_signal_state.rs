use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use super::discovery_signal_types::{
    DiscoveryAdvertisement, DiscoveryKind, DiscoveryPublishTarget, PairingPacketKind,
    DISCOVERY_MAX_SERVER_LEASE_SECS,
};
use super::discovery_signal_wire::DiscoveryOfferRequest;

pub(super) const DISCOVERY_LIST_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
pub(super) const DISCOVERY_PUBLISH_RETRY_DELAY: Duration = Duration::from_secs(5);
pub(super) const DISCOVERY_PUBLISH_ACK_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const MAX_ACTIVE_DISCOVERY_OFFERS: usize = 8;
pub(super) const MAX_ACTIVE_DISCOVERY_EXCHANGES: usize = 8;
pub(super) const MAX_PUBLISHER_EXCHANGES_PER_OFFER: usize = 4;
pub(super) const MAX_DISCOVERY_LIST_ENTRIES: usize = 256;
pub(super) const MAX_DISCOVERY_ALIAS_BYTES: usize = 256;
pub(super) const MAX_DISCOVERY_SUITE_BYTES: usize = 96;
pub(super) const MAX_PAIRING_PAYLOAD_TEXT_BYTES: usize = 192 * 1024;
pub(super) const MAX_EXCHANGE_PAYLOAD_BYTES: usize = 1024 * 1024;
pub(super) const MAX_EXCHANGE_PACKETS: u8 = 16;
pub(super) const DISCOVERY_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(2 * 60);
const MAX_PAIRING_STARTS_PER_OFFER_WINDOW: usize = 12;
const PAIRING_START_WINDOW: Duration = Duration::from_secs(60);

pub(super) struct DiscoverySignalState {
    pub(super) offers: HashMap<String, ActiveDiscoveryOffer>,
    pub(super) advertisements: Vec<DiscoveryAdvertisement>,
    pub(super) exchanges: HashMap<String, ActiveDiscoveryExchange>,
    pub(super) pending_publisher_starts: HashMap<String, Vec<PendingPublisherStart>>,
    closed_exchange_ids: HashMap<String, Instant>,
    closed_offer_ids: HashMap<String, Instant>,
    pub(super) list_request_outstanding: bool,
    pub(super) next_list_request_at: Instant,
}

impl DiscoverySignalState {
    pub(super) fn new() -> Self {
        Self {
            offers: HashMap::new(),
            advertisements: Vec::new(),
            exchanges: HashMap::new(),
            pending_publisher_starts: HashMap::new(),
            closed_exchange_ids: HashMap::new(),
            closed_offer_ids: HashMap::new(),
            list_request_outstanding: false,
            next_list_request_at: Instant::now(),
        }
    }

    pub(super) fn offer_for_discovery(&self, discovery_id: &str) -> Option<&str> {
        self.offers.values().find_map(|offer| {
            (offer.discovery_id.as_deref() == Some(discovery_id)).then_some(offer.offer_id.as_str())
        })
    }

    pub(super) fn advertised(&self, discovery_id: &str) -> Option<&DiscoveryAdvertisement> {
        self.advertisements
            .iter()
            .find(|advertisement| advertisement.discovery_id == discovery_id)
    }

    pub(super) fn remove_offer(&mut self, offer_id: &str) -> Option<ActiveDiscoveryOffer> {
        self.offers.remove(offer_id)
    }

    pub(super) fn expire_offers(&mut self, now: Instant) -> Vec<ActiveDiscoveryOffer> {
        let expired: Vec<_> = self
            .offers
            .iter()
            .filter_map(|(offer_id, offer)| offer.is_expired(now).then_some(offer_id.clone()))
            .collect();
        expired
            .into_iter()
            .filter_map(|offer_id| self.offers.remove(&offer_id))
            .collect()
    }

    pub(super) fn due_offer_ids(&self, now: Instant) -> Vec<String> {
        self.offers
            .values()
            .filter(|offer| offer.publish_due(now))
            .map(|offer| offer.offer_id.clone())
            .collect()
    }

    pub(super) fn disconnected(
        &mut self,
    ) -> (Vec<ActiveDiscoveryExchange>, Vec<PendingPublisherStart>) {
        self.list_request_outstanding = false;
        self.next_list_request_at = Instant::now();
        self.advertisements.clear();
        let pending = self.drain_pending_publisher_starts();
        self.closed_exchange_ids.clear();
        self.closed_offer_ids.clear();
        for offer in self.offers.values_mut() {
            offer.discovery_id = None;
            offer.next_publish_at = Instant::now();
            offer.last_publish_lease_secs = 0;
            offer.last_publish_sent_at = None;
            offer.published_until = None;
        }
        let exchanges = self.exchanges.drain().map(|(_, exchange)| exchange).collect();
        (exchanges, pending)
    }

    pub(super) fn expire_exchanges(&mut self, now: Instant) -> Vec<ActiveDiscoveryExchange> {
        let expired: Vec<_> = self
            .exchanges
            .iter()
            .filter_map(|(exchange_id, exchange)| {
                exchange.is_expired(now).then_some(exchange_id.clone())
            })
            .collect();
        expired
            .into_iter()
            .filter_map(|exchange_id| self.exchanges.remove(&exchange_id))
            .collect()
    }

    pub(super) fn pending_publisher_count(&self) -> usize {
        self.pending_publisher_starts.values().map(Vec::len).sum()
    }

    pub(super) fn drain_pending_publisher_starts(&mut self) -> Vec<PendingPublisherStart> {
        self.pending_publisher_starts
            .drain()
            .flat_map(|(_, starts)| starts)
            .collect()
    }

    pub(super) fn pending_exchange_id_exists(&self, exchange_id: &str) -> bool {
        self.pending_publisher_starts
            .values()
            .flatten()
            .any(|start| start.exchange_id == exchange_id)
    }

    pub(super) fn remove_pending_exchange(
        &mut self,
        exchange_id: &str,
    ) -> Option<PendingPublisherStart> {
        let discovery_ids: Vec<_> = self.pending_publisher_starts.keys().cloned().collect();
        for discovery_id in discovery_ids {
            let Some(starts) = self.pending_publisher_starts.get_mut(&discovery_id) else {
                continue;
            };
            if let Some(index) = starts
                .iter()
                .position(|start| start.exchange_id == exchange_id)
            {
                let start = starts.remove(index);
                if starts.is_empty() {
                    self.pending_publisher_starts.remove(&discovery_id);
                }
                return Some(start);
            }
        }
        None
    }

    pub(super) fn remember_closed_exchange(&mut self, exchange_id: String) {
        let now = Instant::now();
        self.closed_exchange_ids
            .retain(|_, deadline| *deadline > now);
        if self.closed_exchange_ids.len() >= MAX_ACTIVE_DISCOVERY_EXCHANGES * 2 {
            if let Some(oldest) = self
                .closed_exchange_ids
                .iter()
                .min_by_key(|(_, deadline)| **deadline)
                .map(|(exchange_id, _)| exchange_id.clone())
            {
                self.closed_exchange_ids.remove(&oldest);
            }
        }
        self.closed_exchange_ids
            .insert(exchange_id, now + DISCOVERY_EXCHANGE_TIMEOUT);
    }

    pub(super) fn is_closed_exchange(&mut self, exchange_id: &str) -> bool {
        let now = Instant::now();
        self.closed_exchange_ids
            .retain(|_, deadline| *deadline > now);
        self.closed_exchange_ids.contains_key(exchange_id)
    }

    pub(super) fn remember_closed_offer(&mut self, offer_id: String) {
        let now = Instant::now();
        self.closed_offer_ids.retain(|_, deadline| *deadline > now);
        if self.closed_offer_ids.len() >= MAX_ACTIVE_DISCOVERY_OFFERS * 2 {
            if let Some(oldest) = self
                .closed_offer_ids
                .iter()
                .min_by_key(|(_, deadline)| **deadline)
                .map(|(offer_id, _)| offer_id.clone())
            {
                self.closed_offer_ids.remove(&oldest);
            }
        }
        self.closed_offer_ids
            .insert(offer_id, now + DISCOVERY_EXCHANGE_TIMEOUT);
    }

    pub(super) fn is_closed_offer(&mut self, offer_id: &str) -> bool {
        let now = Instant::now();
        self.closed_offer_ids.retain(|_, deadline| *deadline > now);
        self.closed_offer_ids.contains_key(offer_id)
    }

    pub(super) fn expire_pending_publisher_starts(
        &mut self,
        now: Instant,
    ) -> Vec<PendingPublisherStart> {
        let discovery_ids: Vec<_> = self.pending_publisher_starts.keys().cloned().collect();
        let mut expired = Vec::new();
        for discovery_id in discovery_ids {
            let Some(starts) = self.pending_publisher_starts.remove(&discovery_id) else {
                continue;
            };
            let (mut timed_out, current): (Vec<_>, Vec<_>) = starts
                .into_iter()
                .partition(|start| now >= start.deadline);
            expired.append(&mut timed_out);
            if !current.is_empty() {
                self.pending_publisher_starts
                    .insert(discovery_id, current);
            }
        }
        expired
    }
}

pub(super) struct ActiveDiscoveryOffer {
    pub(super) offer_id: String,
    pub(super) target: DiscoveryPublishTarget,
    pub(super) kind: DiscoveryKind,
    pub(super) display_alias: String,
    pub(super) suite: String,
    pub(super) version: u32,
    pub(super) deadline: Instant,
    pub(super) discoverable_until: i64,
    pub(super) next_publish_at: Instant,
    pub(super) last_publish_lease_secs: u32,
    pub(super) last_publish_sent_at: Option<Instant>,
    pub(super) published_until: Option<Instant>,
    pub(super) discovery_id: Option<String>,
    pub(super) pairing_starts: VecDeque<Instant>,
}

impl ActiveDiscoveryOffer {
    pub(super) fn is_expired(&self, now: Instant) -> bool {
        now >= self.deadline
    }

    pub(super) fn publish_due(&self, now: Instant) -> bool {
        !self.is_expired(now)
            && self.last_publish_sent_at.is_none()
            && now >= self.next_publish_at
    }

    pub(super) fn request(&self, now: Instant) -> Option<DiscoveryOfferRequest> {
        if self.is_expired(now) {
            return None;
        }
        let remaining = self.deadline.saturating_duration_since(now);
        let rounded_secs = remaining
            .as_secs()
            .saturating_add(u64::from(remaining.subsec_nanos() != 0));
        let lease_secs = rounded_secs
            .min(u64::from(DISCOVERY_MAX_SERVER_LEASE_SECS))
            .max(1) as u32;
        Some(DiscoveryOfferRequest {
            offer_id: self.offer_id.clone(),
            kind: self.kind,
            display_alias: self.display_alias.clone(),
            suite: self.suite.clone(),
            version: self.version,
            lease_secs,
        })
    }

    pub(super) fn mark_publish_sent(&mut self, now: Instant, lease_secs: u32) {
        self.last_publish_lease_secs = lease_secs;
        self.last_publish_sent_at = Some(now);
        let renew_after = u64::from(lease_secs).saturating_mul(2).saturating_div(3).max(1);
        self.next_publish_at = now + Duration::from_secs(renew_after);
    }

    pub(super) fn accepts_advertisement(
        &self,
        advertisement: &DiscoveryAdvertisement,
    ) -> bool {
        advertisement.offer_id == self.offer_id
            && advertisement.kind == self.kind
            && advertisement.display_alias == self.display_alias
            && advertisement.suite == self.suite
            && advertisement.version == self.version
            && self
                .discovery_id
                .as_ref()
                .map_or(true, |known| known == &advertisement.discovery_id)
    }

    pub(super) fn allow_pairing_start(&mut self, now: Instant) -> bool {
        while self
            .pairing_starts
            .front()
            .map_or(false, |started| {
                now.saturating_duration_since(*started) >= PAIRING_START_WINDOW
            })
        {
            self.pairing_starts.pop_front();
        }
        if self.pairing_starts.len() >= MAX_PAIRING_STARTS_PER_OFFER_WINDOW {
            return false;
        }
        self.pairing_starts.push_back(now);
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DiscoveryExchangeRole {
    Connector,
    Publisher,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DiscoveryExchangeStage {
    ConnectorAwaitOpened,
    ConnectorAwaitOpaqueKe2,
    ConnectorAwaitPublisherBundle,
    ConnectorAwaitPublisherCommit,
    ConnectorAwaitFinish,
    PublisherAwaitOpaqueKe3Bundle,
    PublisherAwaitConnectorCommit,
    PublisherAwaitFinish,
}

pub(super) struct ActiveDiscoveryExchange {
    pub(super) exchange_id: String,
    pub(super) discovery_id: String,
    pub(super) role: DiscoveryExchangeRole,
    pub(super) stage: DiscoveryExchangeStage,
    pub(super) deadline: Instant,
    pub(super) publisher_offer_id: Option<String>,
    payload_bytes: usize,
    packet_count: u8,
}

impl ActiveDiscoveryExchange {
    pub(super) fn connector(exchange_id: String, discovery_id: String) -> Self {
        Self {
            exchange_id,
            discovery_id,
            role: DiscoveryExchangeRole::Connector,
            stage: DiscoveryExchangeStage::ConnectorAwaitOpened,
            deadline: Instant::now() + DISCOVERY_EXCHANGE_TIMEOUT,
            publisher_offer_id: None,
            payload_bytes: 0,
            packet_count: 0,
        }
    }

    pub(super) fn publisher(
        exchange_id: String,
        discovery_id: String,
        offer_id: String,
        deadline: Instant,
    ) -> Self {
        Self {
            exchange_id,
            discovery_id,
            role: DiscoveryExchangeRole::Publisher,
            stage: DiscoveryExchangeStage::PublisherAwaitOpaqueKe3Bundle,
            deadline,
            publisher_offer_id: Some(offer_id),
            payload_bytes: 0,
            packet_count: 0,
        }
    }

    pub(super) fn accept_opened(&mut self, discovery_id: &str) -> Result<(), String> {
        if self.role != DiscoveryExchangeRole::Connector
            || self.stage != DiscoveryExchangeStage::ConnectorAwaitOpened
            || self.discovery_id != discovery_id
        {
            return Err("unerwartete PairingOpened-Reihenfolge".into());
        }
        self.stage = DiscoveryExchangeStage::ConnectorAwaitOpaqueKe2;
        Ok(())
    }

    pub(super) fn accept_packet(&self, kind: PairingPacketKind) -> Result<(), String> {
        let expected = match self.stage {
            DiscoveryExchangeStage::ConnectorAwaitOpaqueKe2 => PairingPacketKind::OpaqueKe2,
            DiscoveryExchangeStage::ConnectorAwaitPublisherBundle => {
                PairingPacketKind::PublisherBundle
            }
            DiscoveryExchangeStage::ConnectorAwaitPublisherCommit => {
                PairingPacketKind::PublisherCommit
            }
            DiscoveryExchangeStage::PublisherAwaitOpaqueKe3Bundle => {
                PairingPacketKind::OpaqueKe3Bundle
            }
            DiscoveryExchangeStage::PublisherAwaitConnectorCommit => {
                PairingPacketKind::ConnectorCommit
            }
            _ => return Err("Pairing-Paket traf in einer nicht empfangenden Phase ein".into()),
        };
        if kind != expected {
            return Err(format!(
                "unerwartete Pairing-Paketart: erwartet {expected:?}, erhalten {kind:?}"
            ));
        }
        Ok(())
    }

    pub(super) fn accept_port_packet(
        &mut self,
        kind: PairingPacketKind,
    ) -> Result<(), String> {
        let next = match (self.stage, kind) {
            (
                DiscoveryExchangeStage::ConnectorAwaitOpaqueKe2,
                PairingPacketKind::OpaqueKe3Bundle,
            ) => DiscoveryExchangeStage::ConnectorAwaitPublisherBundle,
            (
                DiscoveryExchangeStage::ConnectorAwaitPublisherBundle,
                PairingPacketKind::ConnectorCommit,
            ) => DiscoveryExchangeStage::ConnectorAwaitPublisherCommit,
            (
                DiscoveryExchangeStage::PublisherAwaitOpaqueKe3Bundle,
                PairingPacketKind::PublisherBundle,
            ) => DiscoveryExchangeStage::PublisherAwaitConnectorCommit,
            (
                DiscoveryExchangeStage::PublisherAwaitConnectorCommit,
                PairingPacketKind::PublisherCommit,
            ) => DiscoveryExchangeStage::PublisherAwaitFinish,
            _ => return Err("Crypto-Port lieferte ein Paket fuer die falsche Pairing-Phase".into()),
        };
        self.stage = next;
        Ok(())
    }

    pub(super) fn accept_no_packet(&mut self) -> Result<(), String> {
        if self.stage != DiscoveryExchangeStage::ConnectorAwaitPublisherCommit {
            return Err("Crypto-Port beendete eine nicht-terminale Pairing-Phase".into());
        }
        self.stage = DiscoveryExchangeStage::ConnectorAwaitFinish;
        Ok(())
    }

    pub(super) fn awaits_finish(&self) -> bool {
        matches!(
            self.stage,
            DiscoveryExchangeStage::ConnectorAwaitFinish
                | DiscoveryExchangeStage::PublisherAwaitFinish
        )
    }

    pub(super) fn record_payload(&mut self, bytes: usize) -> Result<(), String> {
        let next_bytes = self
            .payload_bytes
            .checked_add(bytes)
            .ok_or("Pairing-Payload-Zaehler ist uebergelaufen")?;
        let next_packets = self
            .packet_count
            .checked_add(1)
            .ok_or("Pairing-Paketzaehler ist uebergelaufen")?;
        if next_bytes > MAX_EXCHANGE_PAYLOAD_BYTES || next_packets > MAX_EXCHANGE_PACKETS {
            return Err("Pairing-Austausch ueberschreitet lokale Ressourcenlimits".into());
        }
        self.payload_bytes = next_bytes;
        self.packet_count = next_packets;
        Ok(())
    }

    pub(super) fn is_expired(&self, now: Instant) -> bool {
        now >= self.deadline
    }
}

pub(super) struct PendingPublisherStart {
    pub(super) exchange_id: String,
    pub(super) discovery_id: String,
    pub(super) offer_id: String,
    pub(super) payload: Vec<u8>,
    pub(super) deadline: Instant,
}
