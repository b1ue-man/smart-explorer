use super::super::discovery_exchange::{
    ConnectorAwaitingPublisherBundle, ConnectorAwaitingPublisherCommit, ConnectorPairingComplete,
    PublisherAwaitingConnectorCommit, PublisherPairingComplete,
};
use super::super::discovery_pake::{
    ConnectorAwaitingKe2, PublisherAwaitingKe3Bundle, PublisherOffer,
};
use super::super::discovery_relation_store::DiscoveryRelationOutcome;
use super::super::discovery_signal_port::DiscoveryPortError;
use super::super::discovery_signal_types::DiscoveryPublishTarget;

const MAX_TRACKED_DISCOVERY_IDS: usize = 4096;
const USED_ID_RETENTION: Duration = Duration::from_secs(30 * 60);

#[derive(Default)]
pub(super) struct UsedIdTracker {
    entries: HashMap<String, Instant>,
    order: VecDeque<(Instant, String)>,
}

impl UsedIdTracker {
    pub(super) fn contains(&mut self, id: &str) -> bool {
        self.prune(Instant::now());
        self.entries.contains_key(id)
    }

    pub(super) fn remember(&mut self, id: String) -> Result<(), DiscoveryPortError> {
        let now = Instant::now();
        self.prune(now);
        while self.entries.len() >= MAX_TRACKED_DISCOVERY_IDS {
            let Some((expires_at, oldest)) = self.order.pop_front() else {
                break;
            };
            if self.entries.get(&oldest) == Some(&expires_at) {
                self.entries.remove(&oldest);
            }
        }
        let expires_at = now + USED_ID_RETENTION;
        self.entries.insert(id.clone(), expires_at);
        self.order.push_back((expires_at, id));
        Ok(())
    }

    fn prune(&mut self, now: Instant) {
        while self
            .order
            .front()
            .is_some_and(|(expires_at, _)| *expires_at <= now)
        {
            let Some((expires_at, id)) = self.order.pop_front() else {
                break;
            };
            if self.entries.get(&id) == Some(&expires_at) {
                self.entries.remove(&id);
            }
        }
    }
}

pub(super) struct PreparedOfferState {
    pub(super) target: DiscoveryPublishTarget,
    pub(super) offer: PublisherOffer,
}

pub(super) enum ExchangeState {
    ConnectorAwaitingKe2(ConnectorAwaitingKe2),
    ConnectorAwaitingPublisherBundle(ConnectorAwaitingPublisherBundle),
    ConnectorAwaitingPublisherCommit {
        state: ConnectorAwaitingPublisherCommit,
        completion: DiscoveryRelationOutcome,
    },
    ConnectorComplete {
        state: ConnectorPairingComplete,
        completion: DiscoveryRelationOutcome,
    },
    PublisherAwaitingKe3 {
        state: PublisherAwaitingKe3Bundle,
        target: DiscoveryPublishTarget,
    },
    PublisherAwaitingConnectorCommit {
        state: PublisherAwaitingConnectorCommit,
        completion: DiscoveryRelationOutcome,
    },
    PublisherComplete {
        state: PublisherPairingComplete,
        completion: DiscoveryRelationOutcome,
    },
    /// A later malformed packet consumed the crypto state, but the relation
    /// was already installed. Keep its outcome until the runtime reports it.
    Unconfirmed {
        completion: DiscoveryRelationOutcome,
        publisher_offer_id: Option<String>,
    },
}

impl ExchangeState {
    /// The outcome already persisted or handed out at this stage.
    pub(super) fn persisted_outcome(&self) -> Option<&DiscoveryRelationOutcome> {
        match self {
            Self::ConnectorAwaitingPublisherCommit { completion, .. }
            | Self::ConnectorComplete { completion, .. }
            | Self::PublisherAwaitingConnectorCommit { completion, .. }
            | Self::PublisherComplete { completion, .. }
            | Self::Unconfirmed { completion, .. } => Some(completion),
            Self::ConnectorAwaitingKe2(_)
            | Self::ConnectorAwaitingPublisherBundle(_)
            | Self::PublisherAwaitingKe3 { .. } => None,
        }
    }

    pub(super) fn is_publisher_offer(&self, offer_id: &str) -> bool {
        self.publisher_offer_id() == Some(offer_id)
    }

    pub(super) fn failed_after_persistence(&self) -> Option<Self> {
        self.persisted_outcome()
            .map(|completion| Self::Unconfirmed {
                completion: completion.clone(),
                publisher_offer_id: self.publisher_offer_id().map(str::to_string),
            })
    }

    fn publisher_offer_id(&self) -> Option<&str> {
        match self {
            Self::PublisherAwaitingKe3 { state, .. } => {
                Some(state.binding().offer().offer_id().as_str())
            }
            Self::PublisherAwaitingConnectorCommit { state, .. } => {
                Some(state.binding().offer().offer_id().as_str())
            }
            Self::PublisherComplete { state, .. } => {
                Some(state.binding().offer().offer_id().as_str())
            }
            Self::Unconfirmed {
                publisher_offer_id, ..
            } => publisher_offer_id.as_deref(),
            _ => None,
        }
    }
}
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
