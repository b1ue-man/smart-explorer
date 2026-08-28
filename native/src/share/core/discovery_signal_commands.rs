use std::io;
use std::time::{Duration, Instant};

use super::core::{eio, now_secs, random_token};
use super::discovery_signal_port::DiscoveryExchangePort;
use super::discovery_signal_state::{
    ActiveDiscoveryOffer, DiscoverySignalState, MAX_ACTIVE_DISCOVERY_OFFERS,
    DISCOVERY_PUBLISH_RETRY_DELAY, MAX_DISCOVERY_ALIAS_BYTES, MAX_DISCOVERY_SUITE_BYTES,
};
use super::discovery_signal_types::{
    DiscoveryCommand, DiscoveryEvent, DiscoveryOfferHandle, DiscoveryOfferStopReason,
    DiscoveryPublishTarget, DiscoveryRejectionClass, DISCOVERY_PAIRING_SUITE,
    DISCOVERY_PAIRING_VERSION, DISCOVERY_PIN_MAX_BYTES,
};
use super::signal_connection::SignalConnection;
use super::types::{ShareCmdResult, ShareEvent};

pub(super) struct DiscoverySignalRuntime {
    pub(super) state: DiscoverySignalState,
    pub(super) port: Box<dyn DiscoveryExchangePort>,
}

pub(super) struct DiscoveryCommandOutcome {
    pub(super) result: io::Result<ShareCmdResult>,
    pub(super) should_reconnect: bool,
}

impl DiscoveryCommandOutcome {
    fn result(result: io::Result<ShareCmdResult>) -> Self {
        Self {
            result,
            should_reconnect: false,
        }
    }

    fn reconnect(result: io::Result<ShareCmdResult>) -> Self {
        Self {
            result,
            should_reconnect: true,
        }
    }
}

impl DiscoverySignalRuntime {
    pub(super) fn with_port(port: Box<dyn DiscoveryExchangePort>) -> Self {
        Self {
            state: DiscoverySignalState::new(),
            port,
        }
    }

    pub(super) fn run_offline_command(
        &mut self,
        command: DiscoveryCommand,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> DiscoveryCommandOutcome {
        match command {
            DiscoveryCommand::Publish {
                target,
                display_alias,
                pin,
                duration_secs,
            } => DiscoveryCommandOutcome::result(
                self.prepare_offer(target, display_alias, &pin, duration_secs, events)
                    .map(ShareCmdResult::DiscoveryOffer),
            ),
            DiscoveryCommand::StopPublishing { offer_id } => {
                DiscoveryCommandOutcome::result(
                    self.stop_offer(&offer_id, DiscoveryOfferStopReason::Requested, events)
                        .map(|()| ShareCmdResult::Applied),
                )
            }
            DiscoveryCommand::ListDiscoveries => {
                self.replay_offers(events);
                DiscoveryCommandOutcome::result(Err(eio(
                    "Share-Server nicht verbunden; Discovery-Liste ist nicht verfuegbar",
                )))
            }
            DiscoveryCommand::StartDiscoveryExchange { .. }
            | DiscoveryCommand::CancelDiscoveryExchange { .. } => {
                DiscoveryCommandOutcome::result(Err(eio(
                    "Share-Server nicht verbunden; Discovery-Austausch wurde nicht gestartet",
                )))
            }
        }
    }

    pub(super) fn run_connected_command(
        &mut self,
        command: DiscoveryCommand,
        signal: &mut SignalConnection,
        capability: bool,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> DiscoveryCommandOutcome {
        if !capability {
            return DiscoveryCommandOutcome::result(Err(eio(
                "Share-Server unterstuetzt discovery_exchange_v1 nicht",
            )));
        }
        match command {
            DiscoveryCommand::Publish {
                target,
                display_alias,
                pin,
                duration_secs,
            } => match self.prepare_offer(target, display_alias, &pin, duration_secs, events) {
                Ok(handle) => {
                    match self.publish_offer_now(&handle.offer_id, signal, events) {
                        Ok(true) => DiscoveryCommandOutcome::result(Ok(
                            ShareCmdResult::DiscoveryOffer(handle),
                        )),
                        Ok(false) => DiscoveryCommandOutcome::result(Err(eio(
                            "Discovery-Ziel ist nicht mehr verfuegbar",
                        ))),
                        Err(error) => DiscoveryCommandOutcome::reconnect(Err(error)),
                    }
                }
                Err(error) => DiscoveryCommandOutcome::result(Err(error)),
            },
            DiscoveryCommand::StopPublishing { offer_id } => {
                let existed = self.state.offers.contains_key(&offer_id);
                let result = self.stop_offer_connected(
                        signal,
                        &offer_id,
                        DiscoveryOfferStopReason::Requested,
                        None,
                        events,
                    )
                    .map(|()| ShareCmdResult::Applied);
                if result.is_err() && existed {
                    DiscoveryCommandOutcome::reconnect(result)
                } else {
                    DiscoveryCommandOutcome::result(result)
                }
            }
            DiscoveryCommand::ListDiscoveries => {
                if let Err(error) = self.expire_offers(signal, events) {
                    return DiscoveryCommandOutcome::reconnect(Err(error));
                }
                self.replay_offers(events);
                let result = self
                    .request_discovery_list(signal)
                    .map(|()| ShareCmdResult::Applied);
                if result.is_err() {
                    DiscoveryCommandOutcome::reconnect(result)
                } else {
                    DiscoveryCommandOutcome::result(result)
                }
            }
            DiscoveryCommand::StartDiscoveryExchange { discovery_id, pin } => {
                match self.start_connector(signal, discovery_id, &pin, events) {
                    Ok(handle) => DiscoveryCommandOutcome::result(Ok(
                        ShareCmdResult::DiscoveryExchange(handle),
                    )),
                    Err(error) => {
                        let reconnect = error.should_reconnect();
                        let result = Err(error.into_io());
                        if reconnect {
                            DiscoveryCommandOutcome::reconnect(result)
                        } else {
                            DiscoveryCommandOutcome::result(result)
                        }
                    }
                }
            }
            DiscoveryCommand::CancelDiscoveryExchange { exchange_id } => {
                match self.cancel_exchange(signal, &exchange_id, events) {
                    Ok(()) => DiscoveryCommandOutcome::result(Ok(ShareCmdResult::Applied)),
                    Err(error) => {
                        let reconnect = error.should_reconnect();
                        let result = Err(error.into_io());
                        if reconnect {
                            DiscoveryCommandOutcome::reconnect(result)
                        } else {
                            DiscoveryCommandOutcome::result(result)
                        }
                    }
                }
            }
        }
    }

    fn prepare_offer(
        &mut self,
        target: DiscoveryPublishTarget,
        display_alias: String,
        pin: &super::discovery_signal_types::DiscoveryPin,
        duration_secs: u64,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> io::Result<DiscoveryOfferHandle> {
        if duration_secs == 0 {
            return Err(eio("Discovery-Gesamtdauer muss groesser als null sein"));
        }
        if self.state.offers.len() >= MAX_ACTIVE_DISCOVERY_OFFERS {
            return Err(eio("lokales Limit aktiver Discovery-Offers erreicht"));
        }
        if pin.as_bytes().len() > DISCOVERY_PIN_MAX_BYTES {
            return Err(eio("Discovery-PIN ueberschreitet das lokale Ressourcenlimit"));
        }
        if display_alias.is_empty()
            || display_alias.len() > MAX_DISCOVERY_ALIAS_BYTES
            || display_alias.chars().any(char::is_control)
        {
            return Err(eio("Discovery-Anzeigename ist ungueltig"));
        }
        let offer_id = random_token(18).map_err(eio)?;
        if self.state.offers.contains_key(&offer_id) {
            return Err(eio("zufaellige Discovery-Offer-ID kollidierte"));
        }
        let now = Instant::now();
        let deadline = now
            .checked_add(Duration::from_secs(duration_secs))
            .ok_or_else(|| eio("Discovery-Gesamtdauer ist auf diesem System nicht darstellbar"))?;
        let kind = target.kind();
        let prepared = self
            .port
            .prepare_offer(&offer_id, target.clone(), pin.as_bytes())
            .map_err(|_| eio("Discovery-Crypto-Vorbereitung ist fehlgeschlagen"))?;
        if prepared.kind != kind
            || prepared.suite != DISCOVERY_PAIRING_SUITE
            || prepared.suite.len() > MAX_DISCOVERY_SUITE_BYTES
            || prepared.version != DISCOVERY_PAIRING_VERSION
        {
            self.port.remove_offer(&offer_id);
            return Err(eio("Discovery-Crypto-Port lieferte inkompatible Metadaten"));
        }
        let unix_duration = i64::try_from(duration_secs).unwrap_or(i64::MAX);
        let discoverable_until = now_secs().saturating_add(unix_duration);
        self.state.offers.insert(
            offer_id.clone(),
            ActiveDiscoveryOffer {
                offer_id: offer_id.clone(),
                target: target.clone(),
                kind,
                display_alias: display_alias.clone(),
                suite: prepared.suite,
                version: prepared.version,
                deadline,
                discoverable_until,
                next_publish_at: now,
                last_publish_lease_secs: 0,
                last_publish_sent_at: None,
                published_until: None,
                discovery_id: None,
                pairing_starts: std::collections::VecDeque::new(),
            },
        );
        send_discovery_event(
            events,
            DiscoveryEvent::OfferPrepared {
                offer_id: offer_id.clone(),
                target,
                display_alias,
                discoverable_until,
            },
        );
        Ok(DiscoveryOfferHandle { offer_id })
    }

    fn stop_offer(
        &mut self,
        offer_id: &str,
        reason: DiscoveryOfferStopReason,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> io::Result<()> {
        let offer = self
            .state
            .remove_offer(offer_id)
            .ok_or_else(|| eio("Discovery-Offer ist nicht aktiv"))?;
        self.state
            .remember_closed_offer(offer.offer_id.clone());
        self.port.remove_offer(offer_id);
        send_discovery_event(
            events,
            DiscoveryEvent::OfferStopped {
                offer_id: offer.offer_id,
                reason,
            },
        );
        Ok(())
    }

    fn replay_offers(&self, events: &crossbeam_channel::Sender<ShareEvent>) {
        let now = Instant::now();
        for offer in self.state.offers.values() {
            send_discovery_event(
                events,
                offer_state_event(
                    offer,
                    offer.published_until.is_some_and(|until| until > now),
                ),
            );
        }
    }

    pub(super) fn reject_offer(
        &mut self,
        offer_id: &str,
        classification: DiscoveryRejectionClass,
        retryable: bool,
        signal: &mut SignalConnection,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> io::Result<()> {
        if retryable || classification == DiscoveryRejectionClass::Unavailable {
            if let Some(offer) = self.state.offers.get_mut(offer_id) {
                let was_published = offer.published_until.take().is_some();
                offer.last_publish_sent_at = None;
                offer.next_publish_at = std::cmp::min(
                    offer.deadline,
                    Instant::now() + DISCOVERY_PUBLISH_RETRY_DELAY,
                );
                if was_published {
                    send_discovery_event(events, offer_state_event(offer, false));
                }
            }
            return Ok(());
        }
        let reason = if classification == DiscoveryRejectionClass::Unsupported {
            DiscoveryOfferStopReason::CapabilityUnavailable
        } else {
            DiscoveryOfferStopReason::TransportError
        };
        self.stop_offer_connected(
            signal,
            offer_id,
            reason,
            Some("Discovery-Angebot wurde vom Server abgelehnt"),
            events,
        )
    }
}

pub(super) fn offer_state_event(
    offer: &ActiveDiscoveryOffer,
    published: bool,
) -> DiscoveryEvent {
    if published {
        DiscoveryEvent::OfferPublished {
            offer_id: offer.offer_id.clone(),
            target: offer.target.clone(),
            display_alias: offer.display_alias.clone(),
            discoverable_until: offer.discoverable_until,
        }
    } else {
        DiscoveryEvent::OfferPrepared {
            offer_id: offer.offer_id.clone(),
            target: offer.target.clone(),
            display_alias: offer.display_alias.clone(),
            discoverable_until: offer.discoverable_until,
        }
    }
}

pub(super) fn send_discovery_event(
    events: &crossbeam_channel::Sender<ShareEvent>,
    event: DiscoveryEvent,
) {
    if matches!(&event, DiscoveryEvent::DiscoveryList { .. }) {
        let _ = events.try_send(ShareEvent::Discovery(event));
    } else {
        let _ = events.send(ShareEvent::Discovery(event));
    }
}
