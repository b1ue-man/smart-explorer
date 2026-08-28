use std::io;
use std::time::Instant;

use super::core::eio;
use super::discovery_signal_commands::{
    offer_state_event, send_discovery_event, DiscoverySignalRuntime,
};
use super::discovery_signal_state::{
    DISCOVERY_LIST_REFRESH_INTERVAL, DISCOVERY_PUBLISH_ACK_TIMEOUT,
};
use super::discovery_signal_types::{
    DiscoveryEvent, DiscoveryOfferStopReason,
};
use super::discovery_signal_wire::DiscoveryClientMsg;
use super::signal_connection::{send_line, SignalConnection};
use super::types::ShareEvent;

impl DiscoverySignalRuntime {
    pub(super) fn maintain_offline(
        &mut self,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) {
        for offer in self.state.expire_offers(Instant::now()) {
            self.state.remember_closed_offer(offer.offer_id.clone());
            self.port.remove_offer(&offer.offer_id);
            send_discovery_event(
                events,
                DiscoveryEvent::OfferStopped {
                    offer_id: offer.offer_id,
                    reason: DiscoveryOfferStopReason::Expired,
                },
            );
        }
    }

    pub(super) fn connected(
        &mut self,
        signal: &mut SignalConnection,
        capability: bool,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> io::Result<()> {
        if !capability {
            self.abort_all_exchanges("Discovery-Faehigkeit ist nicht ausgehandelt", events);
            self.abort_pending("Discovery-Faehigkeit ist nicht ausgehandelt", events);
            let offers: Vec<_> = self.state.offers.drain().map(|(_, offer)| offer).collect();
            for offer in offers {
                self.port.remove_offer(&offer.offer_id);
                send_discovery_event(
                    events,
                    DiscoveryEvent::OfferStopped {
                        offer_id: offer.offer_id,
                        reason: DiscoveryOfferStopReason::CapabilityUnavailable,
                    },
                );
            }
            self.state.advertisements.clear();
            return Ok(());
        }
        for offer in self.state.offers.values_mut() {
            offer.next_publish_at = Instant::now();
        }
        self.maintain(signal, events)
    }

    pub(super) fn maintain(
        &mut self,
        signal: &mut SignalConnection,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> io::Result<()> {
        self.expire_exchanges(signal, events)?;
        self.expire_offers(signal, events)?;
        self.expire_publications(events);
        let now = Instant::now();
        if self.state.offers.values().any(|offer| {
            offer
                .last_publish_sent_at
                .is_some_and(|sent| now.saturating_duration_since(sent) >= DISCOVERY_PUBLISH_ACK_TIMEOUT)
        }) {
            return Err(eio("Discovery-Publish-Bestaetigung blieb aus"));
        }
        for offer_id in self.state.due_offer_ids(now) {
            self.publish_offer_now(&offer_id, signal, events)?;
        }
        if self.state.list_request_outstanding && now >= self.state.next_list_request_at {
            return Err(eio("Discovery-Listenbestaetigung blieb aus"));
        }
        if !self.state.list_request_outstanding && now >= self.state.next_list_request_at {
            self.request_discovery_list(signal)?;
        }
        Ok(())
    }

    pub(super) fn disconnected(&mut self, events: &crossbeam_channel::Sender<ShareEvent>) {
        let prepared: Vec<_> = self
            .state
            .offers
            .values()
            .filter(|offer| offer.published_until.is_some())
            .map(|offer| offer_state_event(offer, false))
            .collect();
        let (exchanges, pending) = self.state.disconnected();
        for event in prepared {
            send_discovery_event(events, event);
        }
        for exchange in exchanges {
            self.port.cancel_exchange(&exchange.exchange_id);
            send_discovery_event(
                events,
                DiscoveryEvent::ExchangeFailed {
                    exchange_id: Some(exchange.exchange_id),
                    discovery_id: Some(exchange.discovery_id),
                    error: "Discovery-Signaling wurde getrennt".into(),
                },
            );
        }
        for start in pending {
            send_discovery_event(
                events,
                DiscoveryEvent::ExchangeFailed {
                    exchange_id: Some(start.exchange_id),
                    discovery_id: Some(start.discovery_id),
                    error: "Discovery-Signaling wurde getrennt".into(),
                },
            );
        }
        send_discovery_event(
            events,
            DiscoveryEvent::DiscoveryList {
                advertisements: Vec::new(),
            },
        );
    }

    pub(super) fn publish_offer_now(
        &mut self,
        offer_id: &str,
        signal: &mut SignalConnection,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> io::Result<bool> {
        let now = Instant::now();
        let request = self
            .state
            .offers
            .get(offer_id)
            .and_then(|offer| offer.request(now));
        let Some(request) = request else {
            return Ok(true);
        };
        if let Err(error) = self.port.revalidate_offer(offer_id) {
            if matches!(error, super::discovery_signal_port::DiscoveryPortError::TargetUnavailable(_)) {
                self.stop_offer_connected(
                    signal,
                    offer_id,
                    DiscoveryOfferStopReason::TargetUnavailable,
                    Some("Discovery-Ziel ist nicht mehr verfuegbar"),
                    events,
                )?;
                return Ok(false);
            }
            return Err(eio("Discovery-Ziel konnte nicht erneut geprueft werden"));
        }
        let lease_secs = request.lease_secs;
        send_line(
            signal,
            &DiscoveryClientMsg::PublishDiscovery { offer: request },
        )?;
        if let Some(offer) = self.state.offers.get_mut(offer_id) {
            offer.mark_publish_sent(now, lease_secs);
        }
        Ok(true)
    }

    pub(super) fn request_discovery_list(
        &mut self,
        signal: &mut SignalConnection,
    ) -> io::Result<()> {
        if self.state.list_request_outstanding {
            return Ok(());
        }
        send_line(signal, &DiscoveryClientMsg::ListDiscoveries)?;
        let now = Instant::now();
        self.state.list_request_outstanding = true;
        self.state.next_list_request_at = now + DISCOVERY_LIST_REFRESH_INTERVAL;
        Ok(())
    }

    fn expire_publications(&mut self, events: &crossbeam_channel::Sender<ShareEvent>) {
        let now = Instant::now();
        let mut prepared = Vec::new();
        for offer in self.state.offers.values_mut() {
            if offer.published_until.is_some_and(|until| now >= until) {
                offer.published_until = None;
                prepared.push(offer_state_event(offer, false));
            }
        }
        for event in prepared {
            send_discovery_event(events, event);
        }
    }

    pub(super) fn expire_offers(
        &mut self,
        signal: &mut SignalConnection,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> io::Result<()> {
        let now = Instant::now();
        let expired: Vec<_> = self
            .state
            .offers
            .values()
            .filter(|offer| offer.is_expired(now))
            .map(|offer| offer.offer_id.clone())
            .collect();
        let mut first_error = None;
        for offer_id in expired {
            if let Err(error) = self.stop_offer_connected(
                signal,
                &offer_id,
                DiscoveryOfferStopReason::Expired,
                None,
                events,
            ) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn expire_exchanges(
        &mut self,
        signal: &mut SignalConnection,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> io::Result<()> {
        let now = Instant::now();
        let mut first_error = None;
        let exchanges = self.state.expire_exchanges(now);
        for exchange in exchanges {
            self.state
                .remember_closed_exchange(exchange.exchange_id.clone());
            self.port.cancel_exchange(&exchange.exchange_id);
            if let Err(error) = send_line(
                signal,
                &DiscoveryClientMsg::CancelPairing {
                    exchange_id: exchange.exchange_id.clone(),
                },
            ) {
                first_error.get_or_insert(error);
            }
            send_discovery_event(
                events,
                DiscoveryEvent::ExchangeFailed {
                    exchange_id: Some(exchange.exchange_id),
                    discovery_id: Some(exchange.discovery_id),
                    error: "Discovery-Austausch hat sein Zeitlimit erreicht".into(),
                },
            );
        }
        for pending in self.state.expire_pending_publisher_starts(now) {
            self.state
                .remember_closed_exchange(pending.exchange_id.clone());
            if let Err(error) = send_line(
                signal,
                &DiscoveryClientMsg::CancelPairing {
                    exchange_id: pending.exchange_id.clone(),
                },
            ) {
                first_error.get_or_insert(error);
            }
            send_discovery_event(
                events,
                DiscoveryEvent::ExchangeFailed {
                    exchange_id: Some(pending.exchange_id),
                    discovery_id: Some(pending.discovery_id),
                    error: "Discovery-Austausch hat sein Zeitlimit erreicht".into(),
                },
            );
        }
        first_error.map_or(Ok(()), Err)
    }

    pub(super) fn abort_all_exchanges(
        &mut self,
        error: &str,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) {
        let exchanges: Vec<_> = self
            .state
            .exchanges
            .drain()
            .map(|(_, exchange)| exchange)
            .collect();
        for exchange in exchanges {
            self.state
                .remember_closed_exchange(exchange.exchange_id.clone());
            self.port.cancel_exchange(&exchange.exchange_id);
            send_discovery_event(
                events,
                DiscoveryEvent::ExchangeFailed {
                    exchange_id: Some(exchange.exchange_id),
                    discovery_id: Some(exchange.discovery_id),
                    error: error.into(),
                },
            );
        }
    }

    pub(super) fn abort_pending(
        &mut self,
        error: &str,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) {
        for start in self.state.drain_pending_publisher_starts() {
            self.state
                .remember_closed_exchange(start.exchange_id.clone());
            send_discovery_event(
                events,
                DiscoveryEvent::ExchangeFailed {
                    exchange_id: Some(start.exchange_id),
                    discovery_id: Some(start.discovery_id),
                    error: error.into(),
                },
            );
        }
    }

}
