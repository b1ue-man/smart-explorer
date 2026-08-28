use std::time::Instant;

use super::configuration_runtime::RuntimeConfiguration;
use super::discovery_signal_commands::{send_discovery_event, DiscoverySignalRuntime};
use super::discovery_signal_exchange::strict_decode_payload;
use super::discovery_signal_port::DiscoveryPortAction;
use super::discovery_signal_state::{
    PendingPublisherStart, DISCOVERY_EXCHANGE_TIMEOUT, DISCOVERY_LIST_REFRESH_INTERVAL,
    DISCOVERY_PUBLISH_RETRY_DELAY, MAX_ACTIVE_DISCOVERY_EXCHANGES,
};
use super::discovery_signal_types::{DiscoveryEvent, DiscoveryOfferStopReason, PairingCloseReason};
use super::discovery_signal_wire::{is_discovery_server_tag, DiscoveryServerMsg};
use super::discovery_signal_validation::{
    close_reason_message, validate_discovery_identifier, validate_list,
};
use super::signal_connection::SignalConnection;
use super::types::ShareEvent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DiscoveryDispatchOutcome {
    NotDiscovery,
    Handled,
    Reconnect,
}

impl DiscoverySignalRuntime {
    pub(super) fn dispatch_server_line(
        &mut self,
        line: &str,
        signal: &mut SignalConnection,
        capability: bool,
        events: &crossbeam_channel::Sender<ShareEvent>,
        configuration: &mut RuntimeConfiguration<'_>,
        tracked_direct: bool,
    ) -> DiscoveryDispatchOutcome {
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => return DiscoveryDispatchOutcome::NotDiscovery,
        };
        let Some(tag) = value.get("t").and_then(serde_json::Value::as_str) else {
            return DiscoveryDispatchOutcome::NotDiscovery;
        };
        if !is_discovery_server_tag(tag) {
            return DiscoveryDispatchOutcome::NotDiscovery;
        }
        if !capability {
            self.fail_protocol(
                "Discovery-Nachricht ohne ausgehandelte Faehigkeit",
                events,
            );
            return DiscoveryDispatchOutcome::Reconnect;
        }
        let message: DiscoveryServerMsg = match serde_json::from_value(value) {
            Ok(message) => message,
            Err(_) => {
                self.fail_protocol("ungueltige Discovery-Servernachricht", events);
                return DiscoveryDispatchOutcome::Reconnect;
            }
        };
        match self.handle_discovery_message(
            message,
            signal,
            events,
            configuration,
            tracked_direct,
        ) {
            Ok(()) => DiscoveryDispatchOutcome::Handled,
            Err(error) => {
                let _ = events.send(ShareEvent::Error(error));
                DiscoveryDispatchOutcome::Reconnect
            }
        }
    }

    pub(super) fn handle_server_discovery_error(
        &mut self,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) {
        self.abort_all_exchanges("Discovery-Server hat den Austausch abgelehnt", events);
        self.abort_pending("Discovery-Server hat den Austausch abgelehnt", events);
        self.state.list_request_outstanding = false;
        self.state.next_list_request_at = Instant::now() + DISCOVERY_LIST_REFRESH_INTERVAL;
        for offer in self.state.offers.values_mut() {
            if offer.last_publish_sent_at.take().is_some() {
                offer.next_publish_at = std::cmp::min(
                    offer.deadline,
                    Instant::now() + DISCOVERY_PUBLISH_RETRY_DELAY,
                );
            }
        }
    }

    fn handle_discovery_message(
        &mut self,
        message: DiscoveryServerMsg,
        signal: &mut SignalConnection,
        events: &crossbeam_channel::Sender<ShareEvent>,
        configuration: &mut RuntimeConfiguration<'_>,
        tracked_direct: bool,
    ) -> Result<(), String> {
        match message {
            DiscoveryServerMsg::DiscoveryPublished { advertisement } => {
                self.handle_published(advertisement, signal, events)
            }
            DiscoveryServerMsg::DiscoveryList { advertisements } => {
                if !self.state.list_request_outstanding {
                    return Ok(());
                }
                validate_list(&advertisements)?;
                self.state.list_request_outstanding = false;
                self.state.advertisements = advertisements.clone();
                send_discovery_event(events, DiscoveryEvent::DiscoveryList { advertisements });
                Ok(())
            }
            DiscoveryServerMsg::PairingOpened {
                exchange_id,
                discovery_id,
            } => self.handle_opened(exchange_id, discovery_id, signal, events),
            DiscoveryServerMsg::PairingStarted {
                exchange_id,
                discovery_id,
                payload,
            } => self.handle_started(exchange_id, discovery_id, payload, signal, events),
            DiscoveryServerMsg::PairingPacket {
                exchange_id,
                kind,
                payload,
            } => self.handle_pairing_packet(
                exchange_id,
                kind,
                payload,
                signal,
                events,
                configuration,
                tracked_direct,
            ),
            DiscoveryServerMsg::PairingFinished {
                exchange_id,
                reason,
            } => self.handle_finished(exchange_id, reason, events),
            DiscoveryServerMsg::DiscoveryRejected {
                operation,
                offer_id,
                discovery_id,
                exchange_id,
                classification,
                retryable,
                msg,
            } => self.handle_rejected(
                operation,
                offer_id,
                discovery_id,
                exchange_id,
                classification,
                retryable,
                msg,
                signal,
                events,
            ),
        }
    }

    fn handle_opened(
        &mut self,
        exchange_id: String,
        discovery_id: String,
        signal: &mut SignalConnection,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> Result<(), String> {
        validate_discovery_identifier(&exchange_id)?;
        validate_discovery_identifier(&discovery_id)?;
        if !self.state.exchanges.contains_key(&exchange_id) {
            if self.state.is_closed_exchange(&exchange_id) {
                return Ok(());
            }
            return Err("PairingOpened referenziert keinen lokalen Austausch".into());
        }
        let result = self
            .state
            .exchanges
            .get_mut(&exchange_id)
            .ok_or("Discovery-Austausch verschwand waehrend PairingOpened")?
            .accept_opened(&discovery_id);
        if let Err(error) = result {
            self.fail_exchange(signal, &exchange_id, Some(discovery_id), &error, events)?;
            return Ok(());
        }
        Ok(())
    }

    fn handle_started(
        &mut self,
        exchange_id: String,
        discovery_id: String,
        payload: String,
        signal: &mut SignalConnection,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> Result<(), String> {
        validate_discovery_identifier(&exchange_id)?;
        validate_discovery_identifier(&discovery_id)?;
        if self.state.exchanges.contains_key(&exchange_id)
            || self.state.pending_exchange_id_exists(&exchange_id)
        {
            return Err("doppelte Discovery-Exchange-ID vom Server".into());
        }
        if self.state.exchanges.len() + self.state.pending_publisher_count()
            >= MAX_ACTIVE_DISCOVERY_EXCHANGES
        {
            self.reject_exchange(signal, &exchange_id)?;
            send_discovery_event(
                events,
                DiscoveryEvent::ExchangeFailed {
                    exchange_id: Some(exchange_id),
                    discovery_id: Some(discovery_id),
                    error: "Lokales Limit aktiver Discovery-Austausche erreicht".into(),
                },
            );
            return Ok(());
        }
        let decoded = match strict_decode_payload(&payload) {
            Ok(decoded) => decoded,
            Err(_) => {
                self.state
                    .remember_closed_exchange(exchange_id.clone());
                self.reject_exchange(signal, &exchange_id)?;
                send_discovery_event(
                    events,
                    DiscoveryEvent::ExchangeFailed {
                        exchange_id: Some(exchange_id),
                        discovery_id: Some(discovery_id),
                        error: "Discovery-Payload ist ungueltig".into(),
                    },
                );
                return Ok(());
            }
        };
        if let Some(offer_id) = self.state.offer_for_discovery(&discovery_id).map(str::to_owned) {
            if let Err(error) = self.start_publisher_exchange(
                exchange_id,
                discovery_id.clone(),
                offer_id.clone(),
                decoded,
                Instant::now() + DISCOVERY_EXCHANGE_TIMEOUT,
                signal,
                events,
            ) {
                let reconnect = error.transport;
                let target_unavailable = error.target_unavailable;
                self.fail_exchange(
                    signal,
                    &error.exchange_id,
                    Some(discovery_id),
                    if target_unavailable {
                        "Discovery-Ziel ist nicht mehr verfuegbar"
                    } else {
                        "Discovery-Publisher konnte den Austausch nicht starten"
                    },
                    events,
                )?;
                if target_unavailable {
                    self.stop_offer_connected(
                        signal,
                        &offer_id,
                        DiscoveryOfferStopReason::TargetUnavailable,
                        Some("Discovery-Ziel ist nicht mehr verfuegbar"),
                        events,
                    )
                    .map_err(|error| error.to_string())?;
                    return Ok(());
                }
                if reconnect {
                    return Err("Discovery-Nachricht konnte nicht gesendet werden".into());
                }
                return Ok(());
            }
            return Ok(());
        }
        let pending_offers: Vec<_> = self
            .state
            .offers
            .values()
            .filter(|offer| offer.discovery_id.is_none() && offer.last_publish_sent_at.is_some())
            .map(|offer| offer.offer_id.clone())
            .collect();
        let [offer_id] = pending_offers.as_slice() else {
            self.state.remember_closed_exchange(exchange_id.clone());
            self.reject_exchange(signal, &exchange_id)?;
            send_discovery_event(
                events,
                DiscoveryEvent::ExchangeFailed {
                    exchange_id: Some(exchange_id),
                    discovery_id: Some(discovery_id),
                    error: "Discovery-Start passt zu keiner eindeutigen Publish-Bestaetigung".into(),
                },
            );
            return Ok(());
        };
        self.state
            .pending_publisher_starts
            .entry(discovery_id.clone())
            .or_default()
            .push(PendingPublisherStart {
                exchange_id,
                discovery_id,
                offer_id: offer_id.clone(),
                payload: decoded,
                deadline: Instant::now() + DISCOVERY_EXCHANGE_TIMEOUT,
            });
        Ok(())
    }

    fn handle_finished(
        &mut self,
        exchange_id: String,
        reason: PairingCloseReason,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> Result<(), String> {
        validate_discovery_identifier(&exchange_id)?;
        if let Some(pending) = self.state.remove_pending_exchange(&exchange_id) {
            self.state
                .remember_closed_exchange(exchange_id.clone());
            let event = if reason == PairingCloseReason::Cancelled {
                DiscoveryEvent::ExchangeCancelled {
                    exchange_id,
                    discovery_id: Some(pending.discovery_id),
                }
            } else {
                DiscoveryEvent::ExchangeFailed {
                    exchange_id: Some(exchange_id),
                    discovery_id: Some(pending.discovery_id),
                    error: close_reason_message(reason).into(),
                }
            };
            send_discovery_event(events, event);
            return Ok(());
        }
        let Some(exchange) = self.state.exchanges.remove(&exchange_id) else {
            if self.state.is_closed_exchange(&exchange_id) {
                return Ok(());
            }
            return Err("PairingFinished referenziert keinen lokalen Austausch".into());
        };
        self.state
            .remember_closed_exchange(exchange_id.clone());
        if reason == PairingCloseReason::Completed {
            if !exchange.awaits_finish() {
                self.port.cancel_exchange(&exchange_id);
                send_discovery_event(
                    events,
                    DiscoveryEvent::ExchangeFailed {
                        exchange_id: Some(exchange_id.clone()),
                        discovery_id: Some(exchange.discovery_id),
                        error: "Discovery-Server meldete einen vorzeitigen Abschluss".into(),
                    },
                );
                return Ok(());
            }
            match self.port.finish_exchange(&exchange_id, reason) {
                Ok(Some(DiscoveryPortAction::ExchangeReady { outcome })) => send_discovery_event(
                    events,
                    DiscoveryEvent::ExchangeCompleted {
                        exchange_id,
                        discovery_id: exchange.discovery_id,
                        outcome,
                    },
                ),
                _ => {
                    self.port.cancel_exchange(&exchange_id);
                    send_discovery_event(
                        events,
                        DiscoveryEvent::ExchangeFailed {
                            exchange_id: Some(exchange_id),
                            discovery_id: Some(exchange.discovery_id),
                            error: "Discovery-Austausch konnte nicht sicher abgeschlossen werden"
                                .into(),
                        },
                    );
                }
            }
        } else {
            let _ = self.port.finish_exchange(&exchange_id, reason);
            self.port.cancel_exchange(&exchange_id);
            if reason == PairingCloseReason::Cancelled {
                send_discovery_event(
                    events,
                    DiscoveryEvent::ExchangeCancelled {
                        exchange_id,
                        discovery_id: Some(exchange.discovery_id),
                    },
                );
            } else {
                send_discovery_event(
                    events,
                    DiscoveryEvent::ExchangeFailed {
                        exchange_id: Some(exchange_id),
                        discovery_id: Some(exchange.discovery_id),
                        error: close_reason_message(reason).into(),
                    },
                );
            }
        }
        Ok(())
    }

    fn fail_protocol(
        &mut self,
        error: &str,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) {
        self.abort_all_exchanges(error, events);
        self.abort_pending("Discovery-Protokoll wurde abgebrochen", events);
        self.state.advertisements.clear();
        let _ = events.send(ShareEvent::Error(error.into()));
    }
}
