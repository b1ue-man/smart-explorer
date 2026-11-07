//! Pre-identity admission of the machine login gateway (r-lokal-6, review
//! 2026-11-07). The gateway's TLS handshakes use the same slot rules as the
//! Share endpoint (`handshake_limits_tls.rs`): at most a quarter of the slots
//! per source, a QUIC Retry for unvalidated direct senders under load, and a
//! quarter reserved for relay senders whose EndpointId holds a login grant.
//! A slot covers only the TLS handshake; the gateway frees it as soon as the
//! peer's identity is known.
use std::sync::Arc;

use iroh::endpoint::{Incoming, IncomingAddr};

use crate::share::handshake_limits::{TlsAdmission, TlsHandshakeSlots, TlsSource};

/// The TLS handshake slots of one machine gateway.
pub struct GatewayAdmission {
    slots: Arc<TlsHandshakeSlots>,
}

/// One taken handshake slot; dropping it frees the slot.
pub struct GatewayTlsSlot {
    _slot: Box<dyn Send + Sync>,
}

impl GatewayAdmission {
    pub fn new(capacity: usize) -> Self {
        Self {
            slots: TlsHandshakeSlots::new(capacity),
        }
    }

    /// Decides one incoming login handshake. `granted` tells whether an
    /// EndpointId holds a login grant; only a relay names its sender before
    /// the handshake. Retry and refusal are answered here: `None` means this
    /// attempt is finished, `Some` hands back the attempt with its slot.
    pub fn admit(
        &self,
        incoming: Incoming,
        granted: impl Fn(&str) -> bool,
    ) -> Option<(Incoming, GatewayTlsSlot)> {
        let addr = incoming.remote_addr();
        let pinned = match &addr {
            IncomingAddr::Relay { endpoint_id, .. } => granted(&endpoint_id.to_string()),
            _ => false,
        };
        let validated = incoming.remote_addr_validated();
        match self.slots.admit(TlsSource::of(&addr), validated, pinned) {
            TlsAdmission::Slot(slot) => Some((
                incoming,
                GatewayTlsSlot {
                    _slot: Box::new(slot),
                },
            )),
            TlsAdmission::Retry => {
                if let Err(error) = incoming.retry() {
                    error.into_incoming().refuse();
                }
                None
            }
            TlsAdmission::Refuse => {
                incoming.refuse();
                None
            }
        }
    }
}
