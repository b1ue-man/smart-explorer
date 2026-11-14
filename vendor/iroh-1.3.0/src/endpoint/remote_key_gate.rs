// Smart Explorer M6 patch (2026-11-08); see vendor/SMART_EXPLORER_ROUTE_POLICY.md,
// section "M6: remote key gate before any own address".
//! Refusal of remote endpoint ids the application has removed or blocked.
//!
//! Without a gate every key may complete the TLS handshake. The completed
//! connection is then registered with the socket before any application
//! check (`conn_from_noq_conn`), and the registration queues this endpoint's
//! direct addresses as `ADD_ADDRESS` frames (`update_qnt_candidates`) and lets
//! the peer's `REACH_OUT` start probes from this endpoint's sockets.
//!
//! With a gate, a refused key fails the TLS handshake itself: the server
//! verifier refuses its client certificate, the client verifier its server
//! certificate. Noq processes no 1-RTT packet before the handshake completes,
//! and a failed handshake never reaches the registration, so neither frame
//! is sent or processed. TLS session resumption is switched off for a gated
//! endpoint, because a resumed handshake carries no certificate to refuse.
//! The registration step asks the gate once more, for a key that the
//! application refuses while its handshake was running.

use std::{fmt, sync::Arc};

use iroh_base::EndpointId;

type Refuses = dyn Fn(&EndpointId) -> bool + Send + Sync;

/// A predicate over remote endpoint ids, asked on every handshake.
///
/// The predicate runs inside the TLS verifiers on the connection driver and
/// must answer at once: no I/O and no lock held across an `await`.
#[derive(Clone)]
pub struct RemoteKeyGate {
    refuses: Arc<Refuses>,
}

impl RemoteKeyGate {
    /// A gate that refuses every endpoint id for which `refuses` answers `true`.
    pub fn new(refuses: impl Fn(&EndpointId) -> bool + Send + Sync + 'static) -> Self {
        Self {
            refuses: Arc::new(refuses),
        }
    }

    /// Whether the handshake with `remote` must fail.
    pub fn refuses(&self, remote: &EndpointId) -> bool {
        (self.refuses)(remote)
    }
}

impl fmt::Debug for RemoteKeyGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RemoteKeyGate")
    }
}
