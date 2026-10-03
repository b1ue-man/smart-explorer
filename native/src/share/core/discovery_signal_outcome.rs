//! How an exchange that ends without completion is reported, and what it
//! means for this device's offer (FC2): a relation that was already
//! installed (or Room material that was handed out) is "gekoppelt –
//! Bestätigung fehlt" with its outcome for "Widerrufen" (S24) and ends the
//! offer; an attempt that never proved the PIN counts towards the offer's
//! limit. The offer itself ends at the next maintenance.

use std::io;

use super::discovery_signal_commands::{send_discovery_event, DiscoverySignalRuntime};
use super::discovery_signal_state::{
    ActiveDiscoveryExchange, DiscoveryExchangeRole, DiscoveryExchangeStage,
};
use super::discovery_signal_types::DiscoveryEvent;
use super::signal_connection::SignalConnection;
use super::types::ShareEvent;

pub(super) enum ExchangeEnd {
    Cancelled,
    Failed(String),
}

impl DiscoverySignalRuntime {
    /// The event for an exchange that ended without completion. Call it
    /// before the port forgets the exchange; `attempt` marks ends that count
    /// against the offer (the peer gave up, the exchange failed or expired).
    pub(super) fn exchange_ended(
        &mut self,
        exchange_id: &str,
        exchange: Option<&ActiveDiscoveryExchange>,
        discovery_id: Option<String>,
        end: ExchangeEnd,
        attempt: bool,
    ) -> DiscoveryEvent {
        let outcome = self.port.take_persisted_outcome(exchange_id);
        if let Some(exchange) =
            exchange.filter(|exchange| exchange.role == DiscoveryExchangeRole::Publisher)
        {
            if let Some(offer_id) = exchange.publisher_offer_id.as_deref() {
                let proved = outcome.is_some()
                    || exchange.stage != DiscoveryExchangeStage::PublisherAwaitOpaqueKe3Bundle;
                if proved {
                    self.state.offer_guards.finish_paired(offer_id);
                } else if attempt {
                    self.state.offer_guards.record_failure(offer_id);
                }
            }
        }
        let exchange_id = exchange_id.to_string();
        let discovery_id =
            discovery_id.or_else(|| exchange.map(|exchange| exchange.discovery_id.clone()));
        match (outcome, end) {
            (Some(outcome), _) => DiscoveryEvent::ExchangeUnconfirmed {
                exchange_id,
                discovery_id,
                outcome,
            },
            (None, ExchangeEnd::Cancelled) => DiscoveryEvent::ExchangeCancelled {
                exchange_id,
                discovery_id,
            },
            (None, ExchangeEnd::Failed(error)) => DiscoveryEvent::ExchangeFailed {
                exchange_id: Some(exchange_id),
                discovery_id,
                error,
            },
        }
    }

    /// A completed exchange of this device's offer ends the offer.
    pub(super) fn exchange_completed(&mut self, exchange: &ActiveDiscoveryExchange) {
        if exchange.role == DiscoveryExchangeRole::Publisher {
            if let Some(offer_id) = exchange.publisher_offer_id.as_deref() {
                self.state.offer_guards.finish_paired(offer_id);
            }
        }
    }

    /// After a connector proved the PIN the offer is single-use: no new
    /// exchange starts, and the other running ones are cancelled.
    pub(super) fn publisher_proved(
        &mut self,
        exchange_id: &str,
        signal: &mut SignalConnection,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> Result<(), String> {
        let Some(offer_id) = self
            .state
            .exchanges
            .get(exchange_id)
            .filter(|exchange| {
                exchange.role == DiscoveryExchangeRole::Publisher
                    && exchange.stage == DiscoveryExchangeStage::PublisherAwaitConnectorCommit
            })
            .and_then(|exchange| exchange.publisher_offer_id.clone())
        else {
            return Ok(());
        };
        self.state.offer_guards.mark_paired(&offer_id);
        let others: Vec<String> = self
            .state
            .exchanges
            .values()
            .filter(|exchange| {
                exchange.exchange_id != exchange_id
                    && exchange.publisher_offer_id.as_deref() == Some(offer_id.as_str())
            })
            .map(|exchange| exchange.exchange_id.clone())
            .chain(
                self.state
                    .pending_publisher_starts
                    .values()
                    .flatten()
                    .filter(|start| start.offer_id == offer_id)
                    .map(|start| start.exchange_id.clone()),
            )
            .collect();
        for other in others {
            self.cancel_exchange(signal, &other, events)
                .map_err(|error| error.into_io().to_string())?;
        }
        Ok(())
    }

    /// Ends the offers whose pairing finished or whose attempts ran out.
    pub(super) fn stop_due_offers(
        &mut self,
        signal: &mut SignalConnection,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> io::Result<()> {
        let mut first_error = None;
        for (offer_id, reason) in self.state.offer_guards.take_due_stops() {
            if !self.state.offers.contains_key(&offer_id) {
                continue;
            }
            if let Err(error) = self.stop_offer_connected(signal, &offer_id, reason, None, events) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Offline variant: nothing is announced at the server any more.
    pub(super) fn stop_due_offers_offline(
        &mut self,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) {
        for (offer_id, reason) in self.state.offer_guards.take_due_stops() {
            if self.state.offers.contains_key(&offer_id) {
                let _ = self.stop_offer(&offer_id, reason, events);
            }
        }
    }

    /// Sends the end event of an exchange that is already out of the state.
    pub(super) fn report_exchange_end(
        &mut self,
        exchange_id: &str,
        exchange: Option<ActiveDiscoveryExchange>,
        discovery_id: Option<String>,
        end: ExchangeEnd,
        attempt: bool,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) {
        let event = self.exchange_ended(exchange_id, exchange.as_ref(), discovery_id, end, attempt);
        send_discovery_event(events, event);
    }
}
