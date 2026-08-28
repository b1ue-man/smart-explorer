use std::collections::HashSet;
use std::time::Instant;

use super::discovery_signal_commands::DiscoverySignalRuntime;
use super::discovery_signal_state::{
    DiscoveryExchangeStage, DISCOVERY_LIST_REFRESH_INTERVAL, MAX_DISCOVERY_ALIAS_BYTES,
    MAX_DISCOVERY_LIST_ENTRIES, MAX_DISCOVERY_SUITE_BYTES,
};
use super::discovery_signal_types::{
    DiscoveryAdvertisement, DiscoveryOperation, DiscoveryRejectionClass, PairingCloseReason,
};
use super::types::ShareEvent;

pub(super) fn validate_list(
    advertisements: &[DiscoveryAdvertisement],
) -> Result<(), String> {
    if advertisements.len() > MAX_DISCOVERY_LIST_ENTRIES {
        return Err("Discovery-Liste ueberschreitet das lokale Eintragslimit".into());
    }
    let mut discovery_ids = HashSet::with_capacity(advertisements.len());
    for advertisement in advertisements {
        validate_advertisement(advertisement)?;
        if !discovery_ids.insert(&advertisement.discovery_id) {
            return Err("Discovery-Liste enthaelt doppelte IDs".into());
        }
    }
    Ok(())
}

pub(super) fn validate_advertisement(
    advertisement: &DiscoveryAdvertisement,
) -> Result<(), String> {
    if !valid_text(&advertisement.discovery_id, 128)
        || !valid_text(&advertisement.offer_id, 128)
        || !valid_text(&advertisement.display_alias, MAX_DISCOVERY_ALIAS_BYTES)
        || !valid_text(&advertisement.suite, MAX_DISCOVERY_SUITE_BYTES)
        || advertisement.version == 0
        || advertisement.expires_at <= 0
    {
        return Err("Discovery-Advertisement ist ungueltig".into());
    }
    Ok(())
}

pub(super) fn validate_discovery_identifier(value: &str) -> Result<(), String> {
    if valid_text(value, 128) {
        Ok(())
    } else {
        Err("Discovery-Protokoll-ID ist ungueltig".into())
    }
}

pub(super) fn validate_rejection(
    operation: DiscoveryOperation,
    offer_id: Option<&str>,
    discovery_id: Option<&str>,
    exchange_id: Option<&str>,
    classification: DiscoveryRejectionClass,
    msg: &str,
) -> Result<(), String> {
    if msg.is_empty() || msg.len() > 256 || msg.chars().any(char::is_control) {
        return Err("Discovery-Ablehnung enthaelt eine ungueltige Meldung".into());
    }
    let sanitized_invalid = classification == DiscoveryRejectionClass::InvalidRequest;
    let valid = match operation {
        DiscoveryOperation::PublishDiscovery | DiscoveryOperation::UnpublishDiscovery => {
            (offer_id.is_some() || sanitized_invalid)
                && discovery_id.is_none()
                && exchange_id.is_none()
        }
        DiscoveryOperation::ListDiscoveries => {
            offer_id.is_none() && discovery_id.is_none() && exchange_id.is_none()
        }
        DiscoveryOperation::StartPairing => {
            offer_id.is_none()
                && (discovery_id.is_some() && exchange_id.is_some() || sanitized_invalid)
        }
        DiscoveryOperation::PairingPacket | DiscoveryOperation::CancelPairing => {
            offer_id.is_none()
                && discovery_id.is_none()
                && (exchange_id.is_some() || sanitized_invalid)
        }
    };
    if !valid {
        return Err("Discovery-Ablehnung hat ungueltige Korrelationsfelder".into());
    }
    for value in [offer_id, discovery_id, exchange_id].into_iter().flatten() {
        validate_discovery_identifier(value)?;
    }
    Ok(())
}

impl DiscoverySignalRuntime {
    pub(super) fn handle_rejected(
        &mut self,
        operation: DiscoveryOperation,
        offer_id: Option<String>,
        discovery_id: Option<String>,
        exchange_id: Option<String>,
        classification: DiscoveryRejectionClass,
        retryable: bool,
        msg: String,
        signal: &mut super::signal_connection::SignalConnection,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> Result<(), String> {
        validate_rejection(
            operation,
            offer_id.as_deref(),
            discovery_id.as_deref(),
            exchange_id.as_deref(),
            classification,
            &msg,
        )?;
        match operation {
            DiscoveryOperation::PublishDiscovery => {
                let Some(offer_id) = offer_id else {
                    let _ = events.send(ShareEvent::Error(msg));
                    return Ok(());
                };
                if self
                    .state
                    .offers
                    .get(&offer_id)
                    .is_some_and(|offer| offer.last_publish_sent_at.is_some())
                {
                    self.reject_offer(&offer_id, classification, retryable, signal, events)
                        .map_err(|error| error.to_string())?;
                }
            }
            DiscoveryOperation::UnpublishDiscovery => {}
            DiscoveryOperation::ListDiscoveries => {
                if self.state.list_request_outstanding {
                    self.state.list_request_outstanding = false;
                    self.state.next_list_request_at =
                        Instant::now() + DISCOVERY_LIST_REFRESH_INTERVAL;
                    let _ = events.send(ShareEvent::Error(
                        "Discovery-Liste wurde vom Server abgelehnt".into(),
                    ));
                }
            }
            DiscoveryOperation::StartPairing => {
                let Some(exchange_id) = exchange_id else {
                    let _ = events.send(ShareEvent::Error(msg));
                    return Ok(());
                };
                if self.state.is_closed_exchange(&exchange_id) {
                    return Ok(());
                }
                let Some(exchange) = self.state.exchanges.get(&exchange_id) else {
                    return Ok(());
                };
                if exchange.stage != DiscoveryExchangeStage::ConnectorAwaitOpened {
                    return Ok(());
                }
                if discovery_id
                    .as_deref()
                    .is_some_and(|id| id != exchange.discovery_id.as_str())
                {
                    return Err("Discovery-Ablehnung passt nicht zum lokalen Austausch".into());
                }
                let local_discovery_id = exchange.discovery_id.clone();
                self.terminate_exchange_local(
                    &exchange_id,
                    Some(local_discovery_id),
                    "Discovery-Austausch wurde vom Server abgelehnt",
                    events,
                );
            }
            DiscoveryOperation::PairingPacket => {
                let Some(exchange_id) = exchange_id else {
                    let _ = events.send(ShareEvent::Error(msg));
                    return Ok(());
                };
                if self.state.is_closed_exchange(&exchange_id) {
                    return Ok(());
                }
                let Some(discovery_id) = self
                    .state
                    .exchanges
                    .get(&exchange_id)
                    .map(|exchange| exchange.discovery_id.clone())
                else {
                    return Ok(());
                };
                self.terminate_exchange_local(
                    &exchange_id,
                    Some(discovery_id),
                    "Discovery-Austausch wurde vom Server abgelehnt",
                    events,
                );
            }
            DiscoveryOperation::CancelPairing => {}
        }
        Ok(())
    }
}

fn valid_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

pub(super) fn close_reason_message(reason: PairingCloseReason) -> &'static str {
    match reason {
        PairingCloseReason::Completed => "Discovery-Austausch wurde abgeschlossen",
        PairingCloseReason::Cancelled => "Discovery-Austausch wurde abgebrochen",
        PairingCloseReason::TimedOut => "Discovery-Austausch hat sein Zeitlimit erreicht",
        PairingCloseReason::OfferExpired => "Discovery-Angebot ist abgelaufen",
        PairingCloseReason::OfferWithdrawn => "Discovery-Angebot wurde beendet",
        PairingCloseReason::PeerDisconnected => "Discovery-Gegenstelle wurde getrennt",
        PairingCloseReason::TargetUnavailable => "Discovery-Ziel ist nicht verfuegbar",
        PairingCloseReason::ProtocolError => "Discovery-Protokoll wurde abgebrochen",
    }
}
