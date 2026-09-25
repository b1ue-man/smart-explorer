//! Discovery (PIN pairing) client state: own offers, advertised entries,
//! pending commands and key exchanges, without any UI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryUiKind {
    Direct,
    Room,
}

impl DiscoveryUiKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Direct => "Direktgeraet",
            Self::Room => "Raum",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiscoveryPublishTarget {
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
pub enum DiscoveryCompatibility {
    Compatible,
    UnsupportedSuite,
    UnsupportedVersion,
}

impl DiscoveryCompatibility {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Compatible => "kompatibel",
            Self::UnsupportedSuite => "nicht unterstuetztes Schluesselverfahren",
            Self::UnsupportedVersion => "nicht unterstuetzte Protokollversion",
        }
    }

    pub fn can_connect(&self) -> bool {
        matches!(self, Self::Compatible)
    }
}

#[derive(Clone, Debug)]
pub struct DiscoveryListEntry {
    pub discovery_id: String,
    pub kind: DiscoveryUiKind,
    pub display_alias: String,
    pub expires_at: i64,
    pub compatibility: DiscoveryCompatibility,
}

#[derive(Clone, Debug)]
pub struct ActiveDiscoveryOffer {
    pub offer_id: String,
    pub target: DiscoveryPublishTarget,
    pub expires_at: i64,
    pub phase: DiscoveryOfferPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryOfferPhase {
    Prepared,
    Published,
}

#[derive(Clone, Debug)]
pub enum DiscoveryExchangeState {
    Exchanging,
    Cancelling,
    Cancelled,
    Complete(String),
    Failed(String),
}

impl DiscoveryExchangeState {
    pub fn label(&self) -> String {
        match self {
            Self::Exchanging => "Schluessel werden im Hintergrund ausgetauscht".to_string(),
            Self::Cancelling => "Austausch wird abgebrochen".to_string(),
            Self::Cancelled => "Austausch abgebrochen".to_string(),
            Self::Complete(alias) => format!("Verbunden mit {alias}"),
            Self::Failed(error) => format!("Fehlgeschlagen: {error}"),
        }
    }

    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Exchanging | Self::Cancelling)
    }
}

#[derive(Clone, Debug)]
pub struct DiscoveryExchangeRecord {
    pub discovery_id: String,
    pub state: DiscoveryExchangeState,
}

pub enum DiscoveryUiAction {
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

pub struct DiscoveryUiState {
    pub duration_minutes: u64,
    pub direct_pin: DiscoveryPinDraft,
    pub room_pin: DiscoveryPinDraft,
    pub selected_room_id: String,
    pub entries: Vec<DiscoveryListEntry>,
    pub entry_pins: std::collections::HashMap<String, DiscoveryPinDraft>,
    pub exchanges: std::collections::HashMap<String, DiscoveryExchangeRecord>,
    pub exchange_by_discovery: std::collections::HashMap<String, String>,
    pub starting_discoveries: std::collections::HashSet<String>,
    pub active_offers: Vec<ActiveDiscoveryOffer>,
    pub pending_direct_publish: bool,
    pub pending_room_publish: Option<DiscoveryPublishTarget>,
    pub pending_stops: std::collections::HashSet<String>,
    pub refreshing: bool,
    pub initial_refresh_requested: bool,
    pub status: Option<String>,
    pub dispatcher: super::discovery_events::DiscoveryCommandDispatcher,
}

impl Default for DiscoveryUiState {
    fn default() -> Self {
        let dispatcher = super::discovery_events::DiscoveryCommandDispatcher::new();
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
    pub fn duration_secs(&self) -> u64 {
        self.duration_minutes.saturating_mul(60)
    }

    pub fn begin_publish(&mut self, target: &DiscoveryPublishTarget) -> bool {
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

    pub fn publish_command_failed(&mut self, target: &DiscoveryPublishTarget) {
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

    pub fn offer_updated(
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

    pub fn replace_list(&mut self, entries: Vec<DiscoveryListEntry>) {
        self.entries = entries;
        let entries = &self.entries;
        self.entry_pins
            .retain(|id, _| entries.iter().any(|entry| &entry.discovery_id == id));
        super::discovery_retention::prune_orphaned_terminal_exchanges(self);
        self.refreshing = false;
        self.status = Some("Auffindbare Ziele aktualisiert".to_string());
    }

    pub fn offer_for_target(
        &self,
        target: &DiscoveryPublishTarget,
    ) -> Option<&ActiveDiscoveryOffer> {
        self.active_offers
            .iter()
            .find(|offer| offer.target.key() == target.key())
    }

    pub fn stop_started(&mut self, offer_id: &str) -> bool {
        self.pending_stops.insert(offer_id.to_string())
    }

    pub fn stop_command_failed(&mut self, offer_id: &str) {
        self.pending_stops.remove(offer_id);
    }

    pub fn stopped(&mut self, offer_id: &str) {
        self.pending_stops.remove(offer_id);
        self.active_offers
            .retain(|offer| offer.offer_id != offer_id);
        self.status = Some("Sichtbarkeit beendet".to_string());
    }

    pub fn connect_started(&mut self, discovery_id: &str) -> bool {
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

    pub fn connect_command_failed(&mut self, discovery_id: &str) {
        self.starting_discoveries.remove(discovery_id);
    }

    pub fn exchange_started(&mut self, exchange_id: String, discovery_id: String) {
        self.starting_discoveries.remove(&discovery_id);
        super::discovery_retention::replace_exchange_record(
            self,
            exchange_id,
            DiscoveryExchangeRecord {
                discovery_id,
                state: DiscoveryExchangeState::Exchanging,
            },
        );
        self.status = Some("Schluesselaustausch laeuft im Hintergrund".to_string());
    }

    pub fn exchange_completed(
        &mut self,
        exchange_id: String,
        discovery_id: String,
        outcome: String,
    ) {
        self.starting_discoveries.remove(&discovery_id);
        super::discovery_retention::replace_exchange_record(
            self,
            exchange_id,
            DiscoveryExchangeRecord {
                discovery_id,
                state: DiscoveryExchangeState::Complete(outcome.clone()),
            },
        );
        self.status = Some(format!("Verbunden: {outcome}"));
    }

    pub fn exchange_failed(
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
            super::discovery_retention::replace_exchange_record(
                self,
                exchange_id,
                DiscoveryExchangeRecord {
                    discovery_id,
                    state: DiscoveryExchangeState::Failed(error.clone()),
                },
            );
        }
        self.status = Some(format!("Exchange fehlgeschlagen: {error}"));
    }

    pub fn cancel_started(&mut self, exchange_id: &str) -> bool {
        let Some(exchange) = self.exchanges.get_mut(exchange_id) else {
            return false;
        };
        if !matches!(exchange.state, DiscoveryExchangeState::Exchanging) {
            return false;
        }
        exchange.state = DiscoveryExchangeState::Cancelling;
        true
    }

    pub fn cancel_command_failed(&mut self, exchange_id: &str) {
        if let Some(exchange) = self.exchanges.get_mut(exchange_id) {
            if matches!(exchange.state, DiscoveryExchangeState::Cancelling) {
                exchange.state = DiscoveryExchangeState::Exchanging;
            }
        }
    }

    pub fn exchange_cancelled(&mut self, exchange_id: String, discovery_id: Option<String>) {
        if let Some(exchange) = self.exchanges.get_mut(&exchange_id) {
            exchange.state = DiscoveryExchangeState::Cancelled;
        } else if let Some(discovery_id) = discovery_id {
            super::discovery_retention::replace_exchange_record(
                self,
                exchange_id,
                DiscoveryExchangeRecord {
                    discovery_id,
                    state: DiscoveryExchangeState::Cancelled,
                },
            );
        }
        self.status = Some("Discovery-Austausch abgebrochen".to_string());
    }

    pub fn exchange_for_discovery(
        &self,
        discovery_id: &str,
    ) -> Option<(&str, &DiscoveryExchangeRecord)> {
        let exchange_id = self.exchange_by_discovery.get(discovery_id)?;
        self.exchanges
            .get(exchange_id)
            .map(|exchange| (exchange_id.as_str(), exchange))
    }

    pub fn starting(&self, discovery_id: &str) -> bool {
        self.starting_discoveries.contains(discovery_id)
    }

    pub fn command_error(&mut self, error: String) {
        self.status = Some(format!("Discovery-Befehl fehlgeschlagen: {error}"));
    }

    pub fn prune_expired(&mut self, now: i64) {
        self.entries.retain(|entry| entry.expires_at > now);
        let entries = &self.entries;
        self.entry_pins
            .retain(|id, _| entries.iter().any(|entry| &entry.discovery_id == id));
        super::discovery_retention::prune_orphaned_terminal_exchanges(self);
    }
}
use zeroize::Zeroize;

pub struct DiscoveryPinDraft(String);

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
    pub fn text_mut(&mut self) -> &mut String {
        &mut self.0
    }

    pub fn take(&mut self) -> String {
        std::mem::take(&mut self.0)
    }

    pub fn byte_len(&self) -> usize {
        self.0.len()
    }

    pub fn trivially_guessable(&self) -> bool {
        self.0.is_empty() || self.0 == "0"
    }
}
