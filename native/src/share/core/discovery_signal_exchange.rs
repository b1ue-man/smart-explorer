use std::io;

use super::core::{b64, b64_decode, eio, random_token};
use super::configuration_runtime::RuntimeConfiguration;
use super::discovery_signal_commands::{send_discovery_event, DiscoverySignalRuntime};
use super::discovery_signal_port::{DiscoveryPortAction, DiscoveryPortError};
use super::discovery_signal_state::{
    ActiveDiscoveryExchange, MAX_ACTIVE_DISCOVERY_EXCHANGES,
    MAX_PAIRING_PAYLOAD_TEXT_BYTES, MAX_PUBLISHER_EXCHANGES_PER_OFFER,
};
use super::discovery_signal_types::{
    DiscoveryEvent, DiscoveryExchangeHandle, DiscoveryPin, PairingPacketKind,
    DISCOVERY_PIN_MAX_BYTES,
};
use super::discovery_signal_wire::DiscoveryClientMsg;
use super::discovery_signal_validation::validate_discovery_identifier;
use super::signal_connection::{send_line, SignalConnection};
use super::types::ShareEvent;

pub(super) enum DiscoveryExchangeCommandError {
    Local(io::Error),
    Transport(io::Error),
}

impl DiscoveryExchangeCommandError {
    pub(super) fn should_reconnect(&self) -> bool {
        matches!(self, Self::Transport(_))
    }

    pub(super) fn into_io(self) -> io::Error {
        match self {
            Self::Local(error) | Self::Transport(error) => error,
        }
    }
}

impl DiscoverySignalRuntime {
    pub(super) fn start_connector(
        &mut self,
        signal: &mut SignalConnection,
        discovery_id: String,
        pin: &DiscoveryPin,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> Result<DiscoveryExchangeHandle, DiscoveryExchangeCommandError> {
        if self
            .state
            .exchanges
            .len()
            .saturating_add(self.state.pending_publisher_count())
            >= MAX_ACTIVE_DISCOVERY_EXCHANGES
        {
            return Err(DiscoveryExchangeCommandError::Local(eio(
                "lokales Limit aktiver Discovery-Austausche erreicht",
            )));
        }
        if pin.as_bytes().len() > DISCOVERY_PIN_MAX_BYTES {
            return Err(DiscoveryExchangeCommandError::Local(eio(
                "Discovery-PIN ueberschreitet das lokale Ressourcenlimit",
            )));
        }
        let advertisement = self
            .state
            .advertised(&discovery_id)
            .cloned()
            .ok_or_else(|| {
                DiscoveryExchangeCommandError::Local(eio(
                    "Discovery-Angebot ist nicht mehr in der aktuellen Liste",
                ))
            })?;
        if !advertisement.is_compatible() {
            return Err(DiscoveryExchangeCommandError::Local(eio(
                "Discovery-Angebot verwendet eine inkompatible Crypto-Suite",
            )));
        }
        let exchange_id = random_token(18)
            .map_err(|error| DiscoveryExchangeCommandError::Local(eio(error)))?;
        if self.state.exchanges.contains_key(&exchange_id)
            || self.state.pending_exchange_id_exists(&exchange_id)
        {
            return Err(DiscoveryExchangeCommandError::Local(eio(
                "zufaellige Discovery-Exchange-ID kollidierte",
            )));
        }
        let action = self
            .port
            .start_connector(&exchange_id, &advertisement, pin.as_bytes())
            .map_err(|_| {
                DiscoveryExchangeCommandError::Local(eio(
                    "Discovery-Crypto-Start ist fehlgeschlagen",
                ))
            })?;
        let DiscoveryPortAction::StartPairing { payload } = action else {
            self.port.cancel_exchange(&exchange_id);
            return Err(DiscoveryExchangeCommandError::Local(eio(
                "Discovery-Crypto-Port startete den falschen Nachrichtenfluss",
            )));
        };
        let encoded_payload = match canonical_encode_payload(&payload) {
            Ok(payload) => payload,
            Err(error) => {
                self.port.cancel_exchange(&exchange_id);
                return Err(DiscoveryExchangeCommandError::Local(eio(error)));
            }
        };
        let mut exchange =
            ActiveDiscoveryExchange::connector(exchange_id.clone(), discovery_id.clone());
        if let Err(error) = exchange.record_payload(encoded_payload.len()) {
            self.port.cancel_exchange(&exchange_id);
            return Err(DiscoveryExchangeCommandError::Local(eio(error)));
        }
        self.state.exchanges.insert(
            exchange_id.clone(),
            exchange,
        );
        let message = DiscoveryClientMsg::StartPairing {
            discovery_id,
            exchange_id: exchange_id.clone(),
            payload: encoded_payload,
        };
        if let Err(error) = send_line(signal, &message) {
            self.state.exchanges.remove(&exchange_id);
            self.port.cancel_exchange(&exchange_id);
            send_discovery_event(
                events,
                DiscoveryEvent::ExchangeFailed {
                    exchange_id: Some(exchange_id),
                    discovery_id: None,
                    error: "Discovery-Start konnte nicht gesendet werden".into(),
                },
            );
            return Err(DiscoveryExchangeCommandError::Transport(error));
        }
        send_discovery_event(
            events,
            DiscoveryEvent::ExchangeStarted {
                exchange_id: exchange_id.clone(),
                discovery_id: advertisement.discovery_id,
            },
        );
        Ok(DiscoveryExchangeHandle { exchange_id })
    }

    pub(super) fn start_publisher_exchange(
        &mut self,
        exchange_id: String,
        discovery_id: String,
        offer_id: String,
        payload: Vec<u8>,
        deadline: std::time::Instant,
        signal: &mut SignalConnection,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> Result<(), PublisherStartError> {
        if std::time::Instant::now() >= deadline {
            return Err(PublisherStartError::protocol(&exchange_id));
        }
        if self
            .state
            .exchanges
            .values()
            .filter(|exchange| {
                exchange.publisher_offer_id.as_deref() == Some(offer_id.as_str())
            })
            .count()
            >= MAX_PUBLISHER_EXCHANGES_PER_OFFER
        {
            return Err(PublisherStartError::protocol(&exchange_id));
        }
        let allowed = self
            .state
            .offers
            .get_mut(&offer_id)
            .map_or(false, |offer| {
                offer.allow_pairing_start(std::time::Instant::now())
            });
        if !allowed {
            return Err(PublisherStartError::protocol(&exchange_id));
        }
        let request_payload_text_len = canonical_payload_text_len(payload.len());
        let action = self.port.start_publisher(
            &exchange_id,
            &discovery_id,
            &offer_id,
            payload,
        ).map_err(|error| PublisherStartError::local(&exchange_id, error))?;
        let DiscoveryPortAction::SendPacket(packet) = action
        else {
            self.port.cancel_exchange(&exchange_id);
            return Err(PublisherStartError::protocol(&exchange_id));
        };
        if packet.kind != PairingPacketKind::OpaqueKe2 {
            self.port.cancel_exchange(&exchange_id);
            return Err(PublisherStartError::protocol(&exchange_id));
        }
        let response = packet.payload;
        let mut exchange = ActiveDiscoveryExchange::publisher(
            exchange_id.clone(),
            discovery_id.clone(),
            offer_id,
            deadline,
        );
        if exchange
            .record_payload(request_payload_text_len)
            .is_err()
            || exchange
                .record_payload(canonical_payload_text_len(response.len()))
                .is_err()
        {
            self.port.cancel_exchange(&exchange_id);
            return Err(PublisherStartError::protocol(&exchange_id));
        }
        if send_pairing_packet(
            signal,
            &exchange_id,
            PairingPacketKind::OpaqueKe2,
            &response,
        )
        .is_err()
        {
            self.port.cancel_exchange(&exchange_id);
            return Err(PublisherStartError::transport(&exchange_id));
        }
        self.state.exchanges.insert(exchange_id.clone(), exchange);
        send_discovery_event(
            events,
            DiscoveryEvent::ExchangeStarted {
                exchange_id,
                discovery_id,
            },
        );
        Ok(())
    }

    pub(super) fn handle_pairing_packet(
        &mut self,
        exchange_id: String,
        kind: PairingPacketKind,
        payload: String,
        signal: &mut SignalConnection,
        events: &crossbeam_channel::Sender<ShareEvent>,
        configuration: &mut RuntimeConfiguration<'_>,
        tracked_direct: bool,
    ) -> Result<(), String> {
        validate_discovery_identifier(&exchange_id)?;
        if !self.state.exchanges.contains_key(&exchange_id) {
            if self.state.is_closed_exchange(&exchange_id) {
                return Ok(());
            }
            return Err("Pairing-Paket referenziert keinen lokalen Austausch".into());
        }
        let decoded = match strict_decode_payload(&payload) {
            Ok(decoded) => decoded,
            Err(_) => {
                self.fail_exchange(
                    signal,
                    &exchange_id,
                    None,
                    "Discovery-Payload ist ungueltig",
                    events,
                )?;
                return Ok(());
            }
        };
        let validation = self
            .state
            .exchanges
            .get_mut(&exchange_id)
            .ok_or_else(|| {
                "Discovery-Austausch verschwand waehrend der Verarbeitung".to_string()
            })
            .and_then(|exchange| {
                exchange.accept_packet(kind)?;
                exchange.record_payload(canonical_payload_text_len(decoded.len()))
            });
        if let Err(error) = validation {
            self.fail_exchange(signal, &exchange_id, None, &error, events)?;
            return Ok(());
        }
        let action = match self.port.handle_packet(&exchange_id, kind, decoded) {
            Ok(action) => action,
            Err(_) => {
                self.fail_exchange(
                    signal,
                    &exchange_id,
                    None,
                    "Discovery-Crypto-Pruefung ist fehlgeschlagen",
                    events,
                )?;
                return Ok(());
            }
        };
        match action {
            Some(DiscoveryPortAction::SendPacket(packet)) => {
                let kind = packet.kind;
                let payload = packet.payload;
                let transition = self
                    .state
                    .exchanges
                    .get_mut(&exchange_id)
                    .ok_or_else(|| {
                        "Discovery-Austausch verschwand vor dem Senden".to_string()
                    })
                    .and_then(|exchange| {
                        exchange.accept_port_packet(kind)?;
                        exchange.record_payload(canonical_payload_text_len(payload.len()))
                    });
                if let Err(error) = transition {
                    self.fail_exchange(signal, &exchange_id, None, &error, events)?;
                    return Ok(());
                }
                send_pairing_packet(signal, &exchange_id, kind, &payload)
                    .map_err(|error| error.to_string())?;
            }
            Some(DiscoveryPortAction::PersistedAndSend(persisted)) => {
                self.apply_persisted_and_send(
                    &exchange_id,
                    persisted,
                    signal,
                    events,
                    configuration,
                    tracked_direct,
                )?;
            }
            None => self.state
                .exchanges
                .get_mut(&exchange_id)
                .ok_or("Discovery-Austausch verschwand vor Abschluss")?
                .accept_no_packet()?,
            Some(DiscoveryPortAction::ExchangeReady { .. })
            | Some(DiscoveryPortAction::StartPairing { .. }) => self.fail_exchange(
                signal,
                &exchange_id,
                None,
                "Discovery-Crypto-Port lieferte keine gueltige Folgeaktion",
                events,
            )?,
        }
        Ok(())
    }

}

pub(super) struct PublisherStartError {
    pub(super) exchange_id: String,
    pub(super) transport: bool,
    pub(super) target_unavailable: bool,
}

impl PublisherStartError {
    fn local(exchange_id: &str, error: DiscoveryPortError) -> Self {
        Self {
            exchange_id: exchange_id.to_string(),
            transport: false,
            target_unavailable: matches!(error, DiscoveryPortError::TargetUnavailable(_)),
        }
    }

    fn protocol(exchange_id: &str) -> Self {
        Self { exchange_id: exchange_id.to_string(), transport: false, target_unavailable: false }
    }

    fn transport(exchange_id: &str) -> Self {
        Self {
            exchange_id: exchange_id.to_string(),
            transport: true,
            target_unavailable: false,
        }
    }
}

pub(super) fn send_pairing_packet(
    signal: &mut SignalConnection,
    exchange_id: &str,
    kind: PairingPacketKind,
    payload: &[u8],
) -> io::Result<()> {
    let payload = canonical_encode_payload(payload).map_err(eio)?;
    send_line(
        signal,
        &DiscoveryClientMsg::PairingPacket {
            exchange_id: exchange_id.to_string(),
            kind,
            payload,
        },
    )
}

pub(super) fn strict_decode_payload(payload: &str) -> Result<Vec<u8>, String> {
    if payload.len() > MAX_PAIRING_PAYLOAD_TEXT_BYTES {
        return Err("Pairing-Payload ueberschreitet das lokale Textlimit".into());
    }
    let decoded = b64_decode(payload).map_err(|error| error.to_string())?;
    if b64(&decoded) != payload {
        return Err("Pairing-Payload ist nicht kanonisch base64url-kodiert".into());
    }
    Ok(decoded)
}

pub(super) fn canonical_encode_payload(payload: &[u8]) -> Result<String, String> {
    let encoded = b64(payload);
    if encoded.len() > MAX_PAIRING_PAYLOAD_TEXT_BYTES {
        return Err("Pairing-Payload ueberschreitet das lokale Textlimit".into());
    }
    Ok(encoded)
}

pub(super) fn canonical_payload_text_len(bytes: usize) -> usize {
    bytes.saturating_mul(4).saturating_add(2) / 3
}
