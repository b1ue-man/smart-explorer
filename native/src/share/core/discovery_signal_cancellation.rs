use std::io;

use super::core::eio;
use super::discovery_signal_commands::{send_discovery_event, DiscoverySignalRuntime};
use super::discovery_signal_exchange::DiscoveryExchangeCommandError;
use super::discovery_signal_outcome::ExchangeEnd;
use super::discovery_signal_types::{DiscoveryEvent, DiscoveryOfferStopReason};
use super::discovery_signal_wire::DiscoveryClientMsg;
use super::signal_connection::{send_line, SignalConnection};
use super::types::ShareEvent;

impl DiscoverySignalRuntime {
    pub(super) fn stop_offer_connected(
        &mut self,
        signal: &mut SignalConnection,
        offer_id: &str,
        reason: DiscoveryOfferStopReason,
        exchange_error: Option<&str>,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> io::Result<()> {
        let Some(offer) = self.state.offers.get(offer_id) else {
            return Err(eio("Discovery-Offer ist nicht aktiv"));
        };
        let was_announced = offer.last_publish_sent_at.is_some() || offer.discovery_id.is_some();
        let mut first_error = self
            .cancel_offer_exchanges(signal, offer_id, exchange_error, events)
            .err();
        if was_announced {
            if let Err(error) = send_line(
                signal,
                &DiscoveryClientMsg::UnpublishDiscovery {
                    offer_id: offer_id.to_string(),
                },
            ) {
                first_error.get_or_insert(error);
            }
        }
        let offer = self
            .state
            .remove_offer(offer_id)
            .ok_or_else(|| eio("Discovery-Offer verschwand beim Beenden"))?;
        self.state.remember_closed_offer(offer.offer_id.clone());
        self.port.remove_offer(&offer.offer_id);
        send_discovery_event(
            events,
            DiscoveryEvent::OfferStopped {
                offer_id: offer.offer_id,
                reason,
            },
        );
        first_error.map_or(Ok(()), Err)
    }

    fn cancel_offer_exchanges(
        &mut self,
        signal: &mut SignalConnection,
        offer_id: &str,
        exchange_error: Option<&str>,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> io::Result<()> {
        let active_ids: Vec<_> = self
            .state
            .exchanges
            .values()
            .filter(|exchange| exchange.publisher_offer_id.as_deref() == Some(offer_id))
            .map(|exchange| exchange.exchange_id.clone())
            .collect();
        let pending_ids: Vec<_> = self
            .state
            .pending_publisher_starts
            .values()
            .flatten()
            .filter(|start| start.offer_id == offer_id)
            .map(|start| start.exchange_id.clone())
            .collect();
        let mut first_error = None;
        for exchange_id in active_ids.into_iter().chain(pending_ids) {
            if let Err(error) = send_line(
                signal,
                &DiscoveryClientMsg::CancelPairing {
                    exchange_id: exchange_id.clone(),
                },
            ) {
                first_error.get_or_insert(error);
            }
            let active = self.state.exchanges.remove(&exchange_id);
            let pending = self.state.remove_pending_exchange(&exchange_id);
            let discovery_id = pending.map(|start| start.discovery_id);
            let end = match exchange_error {
                Some(error) => ExchangeEnd::Failed(error.into()),
                None => ExchangeEnd::Cancelled,
            };
            let event =
                self.exchange_ended(&exchange_id, active.as_ref(), discovery_id, end, false);
            self.port.cancel_exchange(&exchange_id);
            self.state.remember_closed_exchange(exchange_id.clone());
            send_discovery_event(events, event);
        }
        first_error.map_or(Ok(()), Err)
    }

    pub(super) fn cancel_exchange(
        &mut self,
        signal: &mut SignalConnection,
        exchange_id: &str,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> Result<(), DiscoveryExchangeCommandError> {
        if self.state.is_closed_exchange(exchange_id) {
            return Ok(());
        }
        if !self.state.exchanges.contains_key(exchange_id)
            && !self.state.pending_exchange_id_exists(exchange_id)
        {
            return Err(DiscoveryExchangeCommandError::Local(eio(
                "Discovery-Austausch ist nicht aktiv",
            )));
        }
        send_line(
            signal,
            &DiscoveryClientMsg::CancelPairing {
                exchange_id: exchange_id.to_string(),
            },
        )
        .map_err(DiscoveryExchangeCommandError::Transport)?;
        let active = self.state.exchanges.remove(exchange_id);
        let pending = self.state.remove_pending_exchange(exchange_id);
        let discovery_id = pending.map(|start| start.discovery_id);
        let event = self.exchange_ended(
            exchange_id,
            active.as_ref(),
            discovery_id,
            ExchangeEnd::Cancelled,
            false,
        );
        self.port.cancel_exchange(exchange_id);
        self.state.remember_closed_exchange(exchange_id.to_string());
        send_discovery_event(events, event);
        Ok(())
    }

    pub(super) fn terminate_exchange_local(
        &mut self,
        exchange_id: &str,
        discovery_id: Option<String>,
        error: &str,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> bool {
        let active = self.state.exchanges.remove(exchange_id);
        let pending = self.state.remove_pending_exchange(exchange_id);
        if active.is_none() && pending.is_none() {
            return false;
        }
        let discovery_id = discovery_id.or_else(|| pending.map(|start| start.discovery_id));
        let end = ExchangeEnd::Failed(error.into());
        let event = self.exchange_ended(exchange_id, active.as_ref(), discovery_id, end, false);
        self.port.cancel_exchange(exchange_id);
        self.state.remember_closed_exchange(exchange_id.to_string());
        send_discovery_event(events, event);
        true
    }

    pub(super) fn fail_exchange(
        &mut self,
        signal: &mut SignalConnection,
        exchange_id: &str,
        discovery_id: Option<String>,
        error: &str,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> Result<(), String> {
        let active = self.state.exchanges.remove(exchange_id);
        let pending = self.state.remove_pending_exchange(exchange_id);
        let discovery_id = discovery_id.or_else(|| pending.map(|start| start.discovery_id));
        let end = ExchangeEnd::Failed(error.to_string());
        // A wrong PIN fails here on the publisher (KE3): it counts.
        let event = self.exchange_ended(exchange_id, active.as_ref(), discovery_id, end, true);
        self.port.cancel_exchange(exchange_id);
        self.state.remember_closed_exchange(exchange_id.to_string());
        self.reject_exchange(signal, exchange_id)?;
        send_discovery_event(events, event);
        Ok(())
    }

    pub(super) fn reject_exchange(
        &mut self,
        signal: &mut SignalConnection,
        exchange_id: &str,
    ) -> Result<(), String> {
        send_line(
            signal,
            &DiscoveryClientMsg::CancelPairing {
                exchange_id: exchange_id.to_string(),
            },
        )
        .map_err(|error| error.to_string())
    }
}
