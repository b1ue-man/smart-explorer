//! The port through which a route actor starts direct retries in relay
//! fallback (`route_monitor_retry.rs`), and its implementation for a Share
//! node: fresh candidates from the endpoint and the peer address book, the
//! attempt itself through the transport (`Endpoint::retry_direct_paths`,
//! vendored Iroh `application_direct_retry.rs`).
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use iroh::endpoint::{Connection, DirectRetryOutcome};
use iroh::Endpoint;
use tokio::sync::watch;

use crate::share::core::now_secs;
use crate::share::peer_address_book::PeerAddressBook;
use crate::share::types::ShareAuthState;

/// The running attempt of [`DirectRetryPort::attempt`].
pub(in crate::share) type AttemptFuture<'a> =
    Pin<Box<dyn Future<Output = Option<DirectRetryOutcome>> + Send + 'a>>;

/// Candidates of one attempt, each sorted and without duplicates.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::share) struct RetryCandidates {
    /// This device's current direct addresses.
    pub(in crate::share) local: Vec<SocketAddr>,
    /// The direct addresses the peer published last.
    pub(in crate::share) peer: Vec<SocketAddr>,
}

pub(in crate::share) trait DirectRetryPort: Send + Sync {
    /// The current candidates for `remote` (an Iroh node id).
    fn candidates(&self, remote: &str) -> RetryCandidates;
    /// Advances whenever local addresses or a peer's published ones may
    /// have changed.
    fn subscribe(&self) -> watch::Receiver<u64>;
    /// One direct attempt on `connection` with the peer's addresses.
    fn attempt<'a>(
        &'a self,
        connection: &'a Connection,
        peer: Vec<SocketAddr>,
    ) -> AttemptFuture<'a>;
}

/// [`DirectRetryPort`] of a Share node (`ShareIrohNode::direct_retry_port`).
pub(in crate::share) struct NodeDirectRetry {
    pub(in crate::share) endpoint: Endpoint,
    pub(in crate::share) auth: Arc<Mutex<ShareAuthState>>,
    pub(in crate::share) book: Arc<PeerAddressBook>,
}

impl DirectRetryPort for NodeDirectRetry {
    fn candidates(&self, remote: &str) -> RetryCandidates {
        let mut local: Vec<SocketAddr> = self.endpoint.addr().ip_addrs().copied().collect();
        local.sort();
        local.dedup();
        let peer = self
            .auth
            .lock()
            .map(|state| {
                crate::share::peer_dial::peer_addresses(&state, &self.book, remote, now_secs())
            })
            .unwrap_or_default();
        RetryCandidates { local, peer }
    }

    fn subscribe(&self) -> watch::Receiver<u64> {
        self.book.subscribe()
    }

    fn attempt<'a>(
        &'a self,
        connection: &'a Connection,
        peer: Vec<SocketAddr>,
    ) -> AttemptFuture<'a> {
        Box::pin(async move { Some(self.endpoint.retry_direct_paths(connection, peer).await) })
    }
}
