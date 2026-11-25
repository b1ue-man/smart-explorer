// Smart Explorer direct retry patch (2026-11-25); contract:
// vendor/SMART_EXPLORER_ROUTE_POLICY.md, „Direct retry on one connection“.
//! A fresh direct attempt for one connection whose application data runs over
//! the relay as the marked fallback.
//!
//! Upstream holepunches on its own triggers only (new candidates, a major
//! network change, the 60-second connection check) and only on the client
//! connection with the lowest id to a remote; other connections receive just
//! the selected path. An application whose route policy ended the finite
//! direct phase of one connection asks for a new attempt on exactly that
//! connection with [`Endpoint::retry_direct_paths`]:
//!
//! 1. this device's current direct addresses go to the connection as NAT
//!    traversal candidates, as after a local address change;
//! 2. the addresses the remote published out of band become known paths of
//!    the remote (`Source::App`, as with `connect`);
//! 3. the selected IP path of the remote (validated on another connection)
//!    and each published address the remote did not announce in band are
//!    opened on this connection; an existing path to the same address gets a
//!    fresh proof under the application path policy instead of a second path;
//! 4. a NAT traversal round starts on this connection with the candidates
//!    both sides announced in band.
//!
//! Only a client connection opens paths and starts rounds; for a server
//! connection the call reports `server_side` and changes nothing. Nothing here
//! admits application data: the application's gate still needs a validated
//! path. At most [`MAX_RETRY_OPENS`] path opens per call; loopback and
//! unspecified addresses and address families this device has no candidate
//! for are skipped.
//!
//! [`Endpoint::retry_direct_paths`]: crate::Endpoint::retry_direct_paths

use std::{collections::BTreeSet, net::SocketAddr};

use iroh_base::{EndpointId, TransportAddr};
use tokio::sync::oneshot;

use super::{
    ConnId, RemoteStateActor, RemoteStateMessage, Source, to_transports_addr, transports,
    update_qnt_candidates,
};
use crate::socket::EndpointInner;

/// Path opens one call may request.
const MAX_RETRY_OPENS: usize = 8;

/// What one [`Endpoint::retry_direct_paths`](crate::Endpoint::retry_direct_paths)
/// call started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirectRetryOutcome {
    /// The connection is a live client connection of this endpoint and the
    /// attempt ran.
    pub started: bool,
    /// The connection is the server side: only the remote can open paths.
    pub server_side: bool,
    /// Path opens requested: new paths or fresh proofs of existing ones.
    pub path_opens: usize,
    /// A NAT traversal round started on the connection.
    pub nat_round: bool,
}

impl EndpointInner {
    /// Sends the attempt to the running actor of `remote`; the default
    /// outcome when no actor runs (no connection to that remote).
    pub(crate) async fn retry_direct(
        &self,
        remote: EndpointId,
        connection: usize,
        remote_addrs: Vec<SocketAddr>,
    ) -> DirectRetryOutcome {
        let (tx, rx) = oneshot::channel();
        let Some(sender) = self.remote_actors.get(&remote) else {
            return DirectRetryOutcome::default();
        };
        let message = RemoteStateMessage::RetryDirect(connection, remote_addrs, tx);
        if sender.send(message).await.is_err() {
            return DirectRetryOutcome::default();
        }
        rx.await.unwrap_or_default()
    }
}

impl RemoteStateActor {
    /// Handles [`RemoteStateMessage::RetryDirect`].
    pub(super) fn retry_direct(
        &mut self,
        connection: usize,
        remote_addrs: Vec<SocketAddr>,
    ) -> DirectRetryOutcome {
        let mut outcome = DirectRetryOutcome::default();
        let conn_id = ConnId(connection);
        let Some(conn) = self
            .connections
            .get(&conn_id)
            .and_then(|state| state.handle.upgrade())
        else {
            return outcome;
        };
        if conn.close_reason().is_some() {
            return outcome;
        }
        if conn.side().is_server() {
            outcome.server_side = true;
            return outcome;
        }
        outcome.started = true;
        let local = self.state.local_candidates();
        update_qnt_candidates(&conn, &local);
        let published = usable_addrs(&remote_addrs, &local);
        self.state.paths.insert_multiple(
            to_transports_addr(
                self.state.endpoint_id,
                published.iter().copied().map(TransportAddr::Ip),
            ),
            Source::App,
        );
        let in_band: BTreeSet<SocketAddr> = conn
            .get_remote_nat_traversal_addresses()
            .map(|addrs| addrs.into_iter().map(canonical).collect())
            .unwrap_or_default();
        let mut opens: Vec<transports::FourTuple> = Vec::new();
        if let Some(selected) = self.state.selected_path.clone()
            && selected.is_ip()
        {
            opens.push(selected);
        }
        for addr in published.iter().filter(|addr| !in_band.contains(addr)) {
            let tuple = transports::FourTuple::from_remote(transports::Addr::from(*addr));
            if !opens.iter().any(|open| open.remote() == tuple.remote()) {
                opens.push(tuple);
            }
        }
        opens.truncate(MAX_RETRY_OPENS);
        if let Some(conn_state) = self.connections.get(&conn_id) {
            for open in &opens {
                self.state
                    .open_path_on_conn(conn_id, conn_state, &conn, open);
            }
            outcome.path_opens = opens.len();
        }
        let before = self
            .state
            .last_holepunch
            .as_ref()
            .map(|attempt| attempt.when);
        self.state.do_holepunching(conn);
        let after = self
            .state
            .last_holepunch
            .as_ref()
            .map(|attempt| attempt.when);
        outcome.nat_round = after.is_some() && after != before;
        outcome
    }
}

fn canonical(addr: SocketAddr) -> SocketAddr {
    SocketAddr::new(addr.ip().to_canonical(), addr.port())
}

/// Published addresses worth a path: canonical, deduplicated, neither
/// loopback nor unspecified, not this device's own, and of a family this
/// device has a candidate for.
fn usable_addrs(remote_addrs: &[SocketAddr], local: &BTreeSet<SocketAddr>) -> Vec<SocketAddr> {
    let v4 = local.iter().any(|addr| addr.is_ipv4());
    let v6 = local.iter().any(|addr| addr.is_ipv6());
    let mut usable: Vec<SocketAddr> = Vec::new();
    for addr in remote_addrs.iter().copied().map(canonical) {
        let family = if addr.is_ipv4() { v4 } else { v6 };
        if family
            && !addr.ip().is_loopback()
            && !addr.ip().is_unspecified()
            && addr.port() != 0
            && !local.contains(&addr)
            && !usable.contains(&addr)
        {
            usable.push(addr);
        }
    }
    usable
}

#[cfg(test)]
mod tests {
    use super::usable_addrs;
    use std::{collections::BTreeSet, net::SocketAddr};

    #[test]
    fn direct_retry_published_addresses_skip_unusable_ones() {
        let local: BTreeSet<SocketAddr> = ["172.20.2.127:4000".parse().unwrap()].into();
        let remote: Vec<SocketAddr> = [
            "46.247.137.247:5000",
            "[2a01::1]:5000",
            "127.0.0.1:5000",
            "0.0.0.0:5000",
            "172.20.2.127:4000",
            "46.247.137.247:5000",
            "[::ffff:10.0.2.10]:5000",
        ]
        .iter()
        .map(|addr| addr.parse().unwrap())
        .collect();
        let usable = usable_addrs(&remote, &local);
        let expected: Vec<SocketAddr> = vec![
            "46.247.137.247:5000".parse().unwrap(),
            "10.0.2.10:5000".parse().unwrap(),
        ];
        assert_eq!(usable, expected);
    }
}
