//! Purpose profiles (plan 2026-11-14 E1, E5): every QUIC connection of the
//! share endpoint gets the transport configuration of its purpose when it is
//! made. All profiles keep the endpoint values of
//! `keepalive::base_transport_builder(true)` (keepalive 5 s, idle 20 s,
//! 64/0 streams, windows, `application_path_policy`); a profile changes only
//! the congestion controller, the send batches and (for measurements) the
//! ACK frequency.
//!
//! | purpose | ALPN | controller | send batch (segments/datagrams) |
//! |---|---|---|---|
//! | `Files` | `share-fs/3` (sessions, bulk extras, rooms, repair) | Cubic with yielding (E2) | 40/80 |
//! | `Screen` | `share-screen/1` (pictures and the device feed) | Cubic | 10/20 |
//! | `Exec` | `share-exec/2` | Cubic | 10/20 |
//! | `Apps` | `share-apps/1` | Cubic | 10/20 |
//! | `Tcp` | `share-tcp/2`, `/1` | Cubic | 40/80 |
//! | `LanLink` | `paired-link/1` | Cubic | 10/20 |
//!
//! Outgoing connections pass the profile through `ConnectOptions`.
//! Incoming ones are accepted with `Incoming::accept_with` and a server
//! configuration of their own: the ALPN offered in the first Initial packet
//! picks the profile when it can be read (best effort, a post-quantum
//! ClientHello often spans two packets), and the controller carries a late
//! bound purpose slot that the accept path sets from the negotiated ALPN
//! right after the handshake. Until then the controller is plain Cubic.
//! "Cubic" stands for the controller `SE_SHARE_QUIC_CC` selects (default
//! Cubic, `bbr3` for measurements, `keepalive.rs`).
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

#[cfg(any(test, feature = "test-support"))]
use iroh::endpoint::AckFrequencyConfig;
use iroh::endpoint::{
    ConnectError, ConnectOptions, Connection, ControllerFactory, Incoming, QuicTransportConfig,
    SendStream, ServerConfig, ServerConfigBuilder,
};
use iroh::{Endpoint, EndpointAddr};

use super::interactive_presence;
use super::yield_controller::{Engagement, YieldingFactory, DEFAULT_TARGET};

/// What one connection is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum TransportPurpose {
    /// File sessions and their bulk connections.
    Files = 1,
    /// Screen pictures with input, and the device feed (metadata).
    Screen = 2,
    Exec = 3,
    Apps = 4,
    Tcp = 5,
    LanLink = 6,
}

impl TransportPurpose {
    pub(crate) const ALL: [TransportPurpose; 6] = [
        TransportPurpose::Files,
        TransportPurpose::Screen,
        TransportPurpose::Exec,
        TransportPurpose::Apps,
        TransportPurpose::Tcp,
        TransportPurpose::LanLink,
    ];

    /// The purpose of a connection with `alpn`.
    pub(crate) fn for_alpn(alpn: &[u8]) -> Option<Self> {
        Some(match alpn {
            crate::share::node::ALPN => TransportPurpose::Files,
            crate::screen::core::wire::SCREEN_ALPN => TransportPurpose::Screen,
            crate::share::exec_protocol::EXEC_ALPN => TransportPurpose::Exec,
            crate::share::apps_wire::APPS_ALPN => TransportPurpose::Apps,
            crate::share::tcp_forward::TCP_ALPN | crate::share::tcp_forward::TCP_ALPN_V1 => {
                TransportPurpose::Tcp
            }
            crate::share::lan_link_wire::LAN_LINK_ALPN => TransportPurpose::LanLink,
            _ => return None,
        })
    }

    fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|purpose| *purpose as u8 == code)
    }

    /// Whether bulk traffic of this purpose yields to interactive sessions.
    pub(crate) fn yields(self) -> bool {
        matches!(self, TransportPurpose::Files)
    }

    /// Whether this purpose moves bulk data (large send batches, E5).
    pub(crate) fn throughput(self) -> bool {
        matches!(self, TransportPurpose::Files | TransportPurpose::Tcp)
    }

    /// Segments per transmit and datagrams per driver pass.
    pub(crate) fn transmit_batch(self) -> (NonZeroUsize, NonZeroUsize) {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(batch) = overrides::current().transmit.filter(|_| self.throughput()) {
            return batch;
        }
        if self.throughput() {
            (BULK_SEGMENTS, BULK_DATAGRAMS)
        } else {
            (DEFAULT_SEGMENTS, DEFAULT_DATAGRAMS)
        }
    }
}

const fn count(value: usize) -> NonZeroUsize {
    match NonZeroUsize::new(value) {
        Some(value) => value,
        None => panic!("a send batch is never zero"),
    }
}

const DEFAULT_SEGMENTS: NonZeroUsize = count(10);
const DEFAULT_DATAGRAMS: NonZeroUsize = count(20);
/// E5: noq PR #636/#645 measured larger batches (estimate; measurement M1
/// decides). 40 × 1452 bytes stays below the relay's 64 KiB packet limit;
/// the vendored noq caps larger values (`vendor/SMART_EXPLORER_QUIC_PROFILES.md`).
const BULK_SEGMENTS: NonZeroUsize = count(40);
const BULK_DATAGRAMS: NonZeroUsize = count(80);

/// The ALPNs the share endpoint accepts. `accept_with` replaces the
/// endpoint's server configuration for every incoming connection, so this
/// list (not the one given to the endpoint builder in `node.rs`) decides
/// which protocols are accepted; it is that list, in the same order.
pub(crate) fn accepted_alpns() -> Vec<Vec<u8>> {
    vec![
        crate::share::node::ALPN.to_vec(),
        crate::share::exec_protocol::EXEC_ALPN.to_vec(),
        crate::share::tcp_forward::TCP_ALPN.to_vec(),
        crate::share::tcp_forward::TCP_ALPN_V1.to_vec(),
        crate::share::apps_wire::APPS_ALPN.to_vec(),
        crate::share::lan_link_wire::LAN_LINK_ALPN.to_vec(),
        crate::screen::core::wire::SCREEN_ALPN.to_vec(),
    ]
}

/// The purpose slot of one connection: set when the connection is made
/// (outgoing) or bound after the handshake (incoming). Unbound, the
/// controller does not yield.
#[derive(Debug, Default)]
pub(crate) struct PurposeSlot(AtomicU8);

impl PurposeSlot {
    pub(crate) fn new(purpose: Option<TransportPurpose>) -> Self {
        Self(AtomicU8::new(purpose.map_or(0, |purpose| purpose as u8)))
    }

    pub(crate) fn purpose(&self) -> Option<TransportPurpose> {
        TransportPurpose::from_code(self.0.load(Ordering::Acquire))
    }

    /// Binds the purpose of the negotiated `alpn`; an unknown ALPN leaves
    /// the slot as it is.
    pub(crate) fn bind_alpn(&self, alpn: &[u8]) {
        if let Some(purpose) = TransportPurpose::for_alpn(alpn) {
            self.0.store(purpose as u8, Ordering::Release);
        }
    }

    pub(crate) fn yields(&self) -> bool {
        self.purpose().is_some_and(TransportPurpose::yields)
    }
}

/// The congestion controller factory of a connection with `slot`.
pub(crate) fn controller_factory(
    slot: Arc<PurposeSlot>,
    purpose: Option<TransportPurpose>,
) -> Arc<dyn ControllerFactory + Send + Sync> {
    let inner = super::congestion_from_environment();
    // Outgoing non-yielding purposes keep the inner controller itself.
    if purpose.is_some_and(|purpose| !purpose.yields()) {
        return inner;
    }
    #[cfg(any(test, feature = "test-support"))]
    let switches = overrides::current();
    #[cfg(any(test, feature = "test-support"))]
    let (enabled, target) = (
        switches.yielding.unwrap_or(true),
        switches.target.unwrap_or(DEFAULT_TARGET),
    );
    #[cfg(not(any(test, feature = "test-support")))]
    let (enabled, target) = (true, DEFAULT_TARGET);
    Arc::new(YieldingFactory {
        inner,
        engagement: Engagement {
            slot,
            presence: interactive_presence::global(),
            enabled,
        },
        target,
    })
}

/// The transport configuration of `purpose` with `factory` and `batch`.
fn profile(
    purpose: Option<TransportPurpose>,
    factory: Arc<dyn ControllerFactory + Send + Sync>,
) -> QuicTransportConfig {
    let (segments, datagrams) = purpose.map_or(
        (DEFAULT_SEGMENTS, DEFAULT_DATAGRAMS),
        TransportPurpose::transmit_batch,
    );
    let builder = super::base_transport_builder(true)
        .congestion_controller_factory(factory)
        .max_transmit_segments(segments)
        .max_transmit_datagrams(datagrams);
    #[cfg(any(test, feature = "test-support"))]
    let builder = match overrides::current().ack_frequency {
        Some(true) if purpose.is_some_and(TransportPurpose::throughput) => {
            builder.ack_frequency_config(Some(AckFrequencyConfig::default()))
        }
        _ => builder,
    };
    builder.build()
}

/// The transport configuration of an outgoing connection for `purpose`.
pub(crate) fn transport_config(purpose: TransportPurpose) -> QuicTransportConfig {
    let slot = Arc::new(PurposeSlot::new(Some(purpose)));
    profile(Some(purpose), controller_factory(slot, Some(purpose)))
}

/// `ConnectOptions` carrying the profile of `purpose`.
pub(crate) fn connect_options(purpose: TransportPurpose) -> ConnectOptions {
    ConnectOptions::new().with_transport_config(transport_config(purpose))
}

/// `ConnectOptions` of a TCP tunnel connection (`Tcp` profile).
pub(crate) fn tcp_options() -> ConnectOptions {
    connect_options(TransportPurpose::Tcp)
}

/// `Endpoint::connect` with the profile of `purpose`: same result and
/// errors, only the transport configuration differs.
pub(crate) async fn connect(
    endpoint: &Endpoint,
    addr: impl Into<EndpointAddr>,
    alpn: &[u8],
    purpose: TransportPurpose,
) -> Result<Connection, ConnectError> {
    let connecting = endpoint
        .connect_with_opts(addr, alpn, connect_options(purpose))
        .await?;
    Ok(connecting.await?)
}

/// A file session connection (`share-fs/3`) with the `Files` profile.
pub(crate) async fn connect_files(
    endpoint: &Endpoint,
    addr: impl Into<EndpointAddr>,
) -> Result<Connection, ConnectError> {
    connect(
        endpoint,
        addr,
        crate::share::node::ALPN,
        TransportPurpose::Files,
    )
    .await
}

/// The purpose all ALPNs of `offer` share, if every one is known.
pub(crate) fn offered_purpose<A, E>(
    offer: impl IntoIterator<Item = Result<A, E>>,
) -> Option<TransportPurpose>
where
    A: AsRef<[u8]>,
{
    let mut found = None;
    for alpn in offer {
        let purpose = TransportPurpose::for_alpn(alpn.ok()?.as_ref())?;
        if found.is_some_and(|known| known != purpose) {
            return None;
        }
        found = Some(purpose);
    }
    found
}

/// Server configurations for incoming connections, made from one template
/// of the endpoint (TLS identity, ALPNs, token key) per accept loop.
pub(crate) struct AcceptProfiles {
    template: ServerConfigBuilder,
}

impl AcceptProfiles {
    pub(crate) fn new(endpoint: &Endpoint) -> Self {
        Self {
            template: endpoint.create_server_config_builder(accepted_alpns()),
        }
    }

    /// The configuration for `incoming` and its purpose slot.
    pub(crate) fn for_incoming(
        &self,
        incoming: &Incoming,
    ) -> (Arc<ServerConfig>, Arc<PurposeSlot>) {
        let hint = incoming
            .decrypt()
            .and_then(|initial| initial.alpns())
            .and_then(|alpns| offered_purpose(alpns));
        self.with_hint(hint)
    }

    /// The configuration for a connection whose offered purpose is `hint`.
    pub(crate) fn with_hint(
        &self,
        hint: Option<TransportPurpose>,
    ) -> (Arc<ServerConfig>, Arc<PurposeSlot>) {
        let slot = Arc::new(PurposeSlot::new(hint));
        // Always the late-bound controller: a wrong or missing hint is
        // corrected by `bind_alpn` after the handshake.
        let factory = controller_factory(Arc::clone(&slot), None);
        let config = self
            .template
            .clone()
            .set_transport_config(profile(hint, factory))
            .build();
        (Arc::new(config), slot)
    }
}

/// Priority of transfer streams (E4): below the other streams of a file
/// session (0), so listings, walks and status never wait behind transfers.
pub(crate) const TRANSFER_STREAM_PRIORITY: i32 = -1;

/// The priority transfer streams get now (measurement switch M3).
pub(crate) fn transfer_stream_priority() -> i32 {
    #[cfg(any(test, feature = "test-support"))]
    if overrides::current().transfer_priority == Some(false) {
        return 0;
    }
    TRANSFER_STREAM_PRIORITY
}

/// Gives the transfer stream `send` its priority. A stream that is already
/// closed keeps none; its next write reports the closure.
pub(crate) fn prioritize_transfer(send: &SendStream) {
    let _ = send.set_priority(transfer_stream_priority());
}

#[cfg(any(test, feature = "test-support"))]
#[path = "transport_purpose_overrides.rs"]
pub(crate) mod overrides;

#[cfg(test)]
#[path = "transport_purpose_tests.rs"]
mod tests;
