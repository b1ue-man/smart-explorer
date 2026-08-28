#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum DiscoveryUiKind {
    Direct,
    Room,
}

impl DiscoveryUiKind {
    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Direct => "Direktgeraet",
            Self::Room => "Raum",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::app) enum DiscoveryPublishTarget {
    Direct,
    Room { room_id: String, room_name: String },
}

impl DiscoveryPublishTarget {
    fn key(&self) -> &str {
        match self {
            Self::Direct => "direct",
            Self::Room { room_id, .. } => room_id,
        }
    }

    fn label(&self) -> &str {
        match self {
            Self::Direct => "Dieses Geraet",
            Self::Room { room_name, .. } => room_name,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::app) enum DiscoveryCompatibility {
    Compatible,
    UnsupportedSuite,
    UnsupportedVersion,
}

impl DiscoveryCompatibility {
    pub(in crate::app) fn label(&self) -> &'static str {
        match self {
            Self::Compatible => "kompatibel",
            Self::UnsupportedSuite => "nicht unterstuetztes Schluesselverfahren",
            Self::UnsupportedVersion => "nicht unterstuetzte Protokollversion",
        }
    }

    pub(in crate::app) fn can_connect(&self) -> bool {
        matches!(self, Self::Compatible)
    }
}

#[derive(Clone, Debug)]
pub(in crate::app) struct DiscoveryListEntry {
    pub(in crate::app) discovery_id: String,
    pub(in crate::app) kind: DiscoveryUiKind,
    pub(in crate::app) display_alias: String,
    pub(in crate::app) expires_at: i64,
    pub(in crate::app) compatibility: DiscoveryCompatibility,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct ActiveDiscoveryOffer {
    pub(in crate::app) offer_id: String,
    pub(in crate::app) target: DiscoveryPublishTarget,
    pub(in crate::app) expires_at: i64,
    pub(in crate::app) phase: DiscoveryOfferPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum DiscoveryOfferPhase {
    Prepared,
    Published,
}

#[derive(Clone, Debug)]
pub(in crate::app) enum DiscoveryExchangeState {
    Exchanging,
    Cancelling,
    Cancelled,
    Complete(String),
    Failed(String),
}

impl DiscoveryExchangeState {
    pub(in crate::app) fn label(&self) -> String {
        match self {
            Self::Exchanging => "Schluessel werden im Hintergrund ausgetauscht".to_string(),
            Self::Cancelling => "Austausch wird abgebrochen".to_string(),
            Self::Cancelled => "Austausch abgebrochen".to_string(),
            Self::Complete(alias) => format!("Verbunden mit {alias}"),
            Self::Failed(error) => format!("Fehlgeschlagen: {error}"),
        }
    }

    pub(in crate::app) fn is_pending(&self) -> bool {
        matches!(self, Self::Exchanging | Self::Cancelling)
    }
}

#[derive(Clone, Debug)]
pub(in crate::app) struct DiscoveryExchangeRecord {
    pub(in crate::app) discovery_id: String,
    pub(in crate::app) state: DiscoveryExchangeState,
}

pub(in crate::app) enum DiscoveryUiAction {
    Publish {
        target: DiscoveryPublishTarget,
        display_alias: String,
        pin: crate::share::DiscoveryPin,
        duration_secs: u64,
    },
    Stop {
        offer_id: String,
    },
    Refresh,
    Connect {
        discovery_id: String,
        pin: crate::share::DiscoveryPin,
    },
    Cancel {
        exchange_id: String,
    },
}

pub(in crate::app) struct DiscoveryUiState {
    pub(in crate::app) duration_minutes: u64,
    pub(in crate::app) direct_pin: DiscoveryPinDraft,
    pub(in crate::app) room_pin: DiscoveryPinDraft,
    pub(in crate::app) selected_room_id: String,
    pub(in crate::app) entries: Vec<DiscoveryListEntry>,
    pub(in crate::app) entry_pins: std::collections::HashMap<String, DiscoveryPinDraft>,
    pub(in crate::app) exchanges: std::collections::HashMap<String, DiscoveryExchangeRecord>,
    pub(in crate::app) exchange_by_discovery: std::collections::HashMap<String, String>,
    pub(in crate::app) starting_discoveries: std::collections::HashSet<String>,
    pub(in crate::app) active_offers: Vec<ActiveDiscoveryOffer>,
    pub(in crate::app) pending_direct_publish: bool,
    pub(in crate::app) pending_room_publish: Option<DiscoveryPublishTarget>,
    pub(in crate::app) pending_stops: std::collections::HashSet<String>,
    pub(in crate::app) refreshing: bool,
    pub(in crate::app) initial_refresh_requested: bool,
    pub(in crate::app) status: Option<String>,
    pub(in crate::app) dispatcher: super::share_discovery_events::DiscoveryCommandDispatcher,
}

impl Default for DiscoveryUiState {
    fn default() -> Self {
        let dispatcher = super::share_discovery_events::DiscoveryCommandDispatcher::new();
        let status = dispatcher.startup_error().map(str::to_string);
        Self {
            duration_minutes: 5,
            direct_pin: DiscoveryPinDraft::default(),
            room_pin: DiscoveryPinDraft::default(),
            selected_room_id: String::new(),
            entries: Vec::new(),
            entry_pins: std::collections::HashMap::new(),
            exchanges: std::collections::HashMap::new(),
            exchange_by_discovery: std::collections::HashMap::new(),
            starting_discoveries: std::collections::HashSet::new(),
            active_offers: Vec::new(),
            pending_direct_publish: false,
            pending_room_publish: None,
            pending_stops: std::collections::HashSet::new(),
            refreshing: false,
            initial_refresh_requested: false,
            status,
            dispatcher,
        }
    }
}

impl DiscoveryUiState {
    pub(in crate::app) fn duration_secs(&self) -> u64 {
        self.duration_minutes.saturating_mul(60)
    }

    pub(in crate::app) fn begin_publish(&mut self, target: &DiscoveryPublishTarget) -> bool {
        if self.offer_for_target(target).is_some() {
            return false;
        }
        match target {
            DiscoveryPublishTarget::Direct if !self.pending_direct_publish => {
                self.pending_direct_publish = true;
                true
            }
            DiscoveryPublishTarget::Room { .. } if self.pending_room_publish.is_none() => {
                self.pending_room_publish = Some(target.clone());
                true
            }
            _ => false,
        }
    }

    pub(in crate::app) fn publish_command_failed(&mut self, target: &DiscoveryPublishTarget) {
        match target {
            DiscoveryPublishTarget::Direct => self.pending_direct_publish = false,
            DiscoveryPublishTarget::Room { .. }
                if self.pending_room_publish.as_ref() == Some(target) =>
            {
                self.pending_room_publish = None;
            }
            DiscoveryPublishTarget::Room { .. } => {}
        }
    }

    pub(in crate::app) fn offer_updated(
        &mut self,
        offer_id: String,
        target: DiscoveryPublishTarget,
        expires_at: i64,
        phase: DiscoveryOfferPhase,
    ) {
        if let Some(existing) = self
            .active_offers
            .iter_mut()
            .find(|entry| entry.offer_id == offer_id)
        {
            existing.expires_at = expires_at;
            existing.phase = phase;
        } else {
            if let Some(existing) = self
                .active_offers
                .iter_mut()
                .find(|entry| entry.target.key() == target.key())
            {
                existing.offer_id = offer_id;
                existing.expires_at = expires_at;
                existing.phase = phase;
            } else {
                self.active_offers.push(ActiveDiscoveryOffer {
                    offer_id,
                    target: target.clone(),
                    expires_at,
                    phase,
                });
            }
        }
        match &target {
            DiscoveryPublishTarget::Direct => self.pending_direct_publish = false,
            DiscoveryPublishTarget::Room { room_id, .. } => {
                if self
                    .pending_room_publish
                    .as_ref()
                    .is_some_and(|pending| pending.key() == room_id)
                {
                    self.pending_room_publish = None;
                }
            }
        }
        let target_label = target.label();
        self.status = Some(match phase {
            DiscoveryOfferPhase::Prepared => {
                format!("{target_label} ist vorbereitet; Server-Bestaetigung steht aus")
            }
            DiscoveryOfferPhase::Published => format!("{target_label} ist jetzt suchbar"),
        });
    }

    pub(in crate::app) fn replace_list(&mut self, entries: Vec<DiscoveryListEntry>) {
        self.entries = entries;
        let entries = &self.entries;
        self.entry_pins
            .retain(|id, _| entries.iter().any(|entry| &entry.discovery_id == id));
        self.refreshing = false;
        self.status = Some("Auffindbare Ziele aktualisiert".to_string());
    }

    pub(in crate::app) fn offer_for_target(
        &self,
        target: &DiscoveryPublishTarget,
    ) -> Option<&ActiveDiscoveryOffer> {
        self.active_offers
            .iter()
            .find(|offer| offer.target.key() == target.key())
    }

    pub(in crate::app) fn stop_started(&mut self, offer_id: &str) -> bool {
        self.pending_stops.insert(offer_id.to_string())
    }

    pub(in crate::app) fn stop_command_failed(&mut self, offer_id: &str) {
        self.pending_stops.remove(offer_id);
    }

    pub(in crate::app) fn stopped(&mut self, offer_id: &str) {
        self.pending_stops.remove(offer_id);
        self.active_offers
            .retain(|offer| offer.offer_id != offer_id);
        self.status = Some("Sichtbarkeit beendet".to_string());
    }

    pub(in crate::app) fn connect_started(&mut self, discovery_id: &str) -> bool {
        if self.starting_discoveries.contains(discovery_id)
            || self
                .exchange_for_discovery(discovery_id)
                .is_some_and(|(_, exchange)| exchange.state.is_pending())
        {
            return false;
        }
        self.starting_discoveries.insert(discovery_id.to_string());
        true
    }

    pub(in crate::app) fn connect_command_failed(&mut self, discovery_id: &str) {
        self.starting_discoveries.remove(discovery_id);
    }

    pub(in crate::app) fn exchange_started(&mut self, exchange_id: String, discovery_id: String) {
        self.starting_discoveries.remove(&discovery_id);
        self.exchange_by_discovery
            .insert(discovery_id.clone(), exchange_id.clone());
        self.exchanges.insert(
            exchange_id,
            DiscoveryExchangeRecord {
                discovery_id,
                state: DiscoveryExchangeState::Exchanging,
            },
        );
        self.status = Some("Schluesselaustausch laeuft im Hintergrund".to_string());
    }

    pub(in crate::app) fn exchange_completed(
        &mut self,
        exchange_id: String,
        discovery_id: String,
        outcome: String,
    ) {
        self.starting_discoveries.remove(&discovery_id);
        self.exchange_by_discovery
            .insert(discovery_id.clone(), exchange_id.clone());
        self.exchanges.insert(
            exchange_id,
            DiscoveryExchangeRecord {
                discovery_id,
                state: DiscoveryExchangeState::Complete(outcome.clone()),
            },
        );
        self.status = Some(format!("Verbunden: {outcome}"));
    }

    pub(in crate::app) fn exchange_failed(
        &mut self,
        exchange_id: Option<String>,
        discovery_id: Option<String>,
        error: String,
    ) {
        let known_discovery = discovery_id.or_else(|| {
            exchange_id
                .as_ref()
                .and_then(|id| self.exchanges.get(id))
                .map(|record| record.discovery_id.clone())
        });
        if let Some(discovery_id) = &known_discovery {
            self.starting_discoveries.remove(discovery_id);
        }
        if let (Some(exchange_id), Some(discovery_id)) = (exchange_id, known_discovery) {
            self.exchange_by_discovery
                .insert(discovery_id.clone(), exchange_id.clone());
            self.exchanges.insert(
                exchange_id,
                DiscoveryExchangeRecord {
                    discovery_id,
                    state: DiscoveryExchangeState::Failed(error.clone()),
                },
            );
        }
        self.status = Some(format!("Exchange fehlgeschlagen: {error}"));
    }

    pub(in crate::app) fn cancel_started(&mut self, exchange_id: &str) -> bool {
        let Some(exchange) = self.exchanges.get_mut(exchange_id) else {
            return false;
        };
        if !matches!(exchange.state, DiscoveryExchangeState::Exchanging) {
            return false;
        }
        exchange.state = DiscoveryExchangeState::Cancelling;
        true
    }

    pub(in crate::app) fn cancel_command_failed(&mut self, exchange_id: &str) {
        if let Some(exchange) = self.exchanges.get_mut(exchange_id) {
            if matches!(exchange.state, DiscoveryExchangeState::Cancelling) {
                exchange.state = DiscoveryExchangeState::Exchanging;
            }
        }
    }

    pub(in crate::app) fn exchange_cancelled(
        &mut self,
        exchange_id: String,
        discovery_id: Option<String>,
    ) {
        if let Some(exchange) = self.exchanges.get_mut(&exchange_id) {
            exchange.state = DiscoveryExchangeState::Cancelled;
        } else if let Some(discovery_id) = discovery_id {
            self.exchange_by_discovery
                .insert(discovery_id.clone(), exchange_id.clone());
            self.exchanges.insert(
                exchange_id,
                DiscoveryExchangeRecord {
                    discovery_id,
                    state: DiscoveryExchangeState::Cancelled,
                },
            );
        }
        self.status = Some("Discovery-Austausch abgebrochen".to_string());
    }

    pub(in crate::app) fn exchange_for_discovery(
        &self,
        discovery_id: &str,
    ) -> Option<(&str, &DiscoveryExchangeRecord)> {
        let exchange_id = self.exchange_by_discovery.get(discovery_id)?;
        self.exchanges
            .get(exchange_id)
            .map(|exchange| (exchange_id.as_str(), exchange))
    }

    pub(in crate::app) fn starting(&self, discovery_id: &str) -> bool {
        self.starting_discoveries.contains(discovery_id)
    }

    pub(in crate::app) fn command_error(&mut self, error: String) {
        self.status = Some(format!("Discovery-Befehl fehlgeschlagen: {error}"));
    }

    pub(in crate::app) fn prune_expired(&mut self, now: i64) {
        self.entries.retain(|entry| entry.expires_at > now);
        let entries = &self.entries;
        self.entry_pins
            .retain(|id, _| entries.iter().any(|entry| &entry.discovery_id == id));
    }
}
use zeroize::Zeroize;

pub(in crate::app) struct DiscoveryPinDraft(String);

impl Default for DiscoveryPinDraft {
    fn default() -> Self {
        Self(String::new())
    }
}

impl std::fmt::Debug for DiscoveryPinDraft {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DiscoveryPinDraft([REDACTED])")
    }
}

impl Drop for DiscoveryPinDraft {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl DiscoveryPinDraft {
    pub(in crate::app) fn text_mut(&mut self) -> &mut String {
        &mut self.0
    }

    pub(in crate::app) fn take(&mut self) -> String {
        std::mem::take(&mut self.0)
    }

    pub(in crate::app) fn byte_len(&self) -> usize {
        self.0.len()
    }

    pub(in crate::app) fn trivially_guessable(&self) -> bool {
        self.0.is_empty() || self.0 == "0"
    }
}
