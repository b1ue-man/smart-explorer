//! r-zugang U1 (Restbeleg M34, E61): the 3 s deadline of an unknown peer's
//! handshake (`UNKNOWN_HANDSHAKE`, `node_accept.rs`) on a mobile-like path.
//! The suite `crates-task` sets netem on `lo` before this test runs
//! (`tc qdisc replace dev lo root netem delay 350ms 50ms loss 2%`). Without
//! that the test measures plain loopback and says nothing about mobile
//! networks.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iroh::endpoint::VarInt;
use iroh::{EndpointAddr, TransportAddr};

use super::UNKNOWN_HANDSHAKE;
use crate::share::backend::ShareIrohNode;
use crate::share::identity::ShareIdentity;
use crate::share::node::ALPN;
use crate::share::power::test_support::{auth_state, identity, NO_RELAY};
use crate::share::types::{ShareAuthState, ShareEvent};

/// Handshakes of the one unknown peer.
const HANDSHAKES: usize = 100;
/// The 99th percentile of the handshake durations must stay below this.
const PERCENTILE_LIMIT: Duration = Duration::from_millis(2000);

/// A running node that stops sharing when the test ends.
struct Node {
    node: Arc<ShareIrohNode>,
    _events: crossbeam_channel::Receiver<ShareEvent>,
}

impl Node {
    fn start(identity: &ShareIdentity, state: ShareAuthState) -> Self {
        let (events, receiver) = crossbeam_channel::bounded(256);
        let node = ShareIrohNode::start(NO_RELAY, identity, Arc::new(Mutex::new(state)), events)
            .expect("start the node");
        Self {
            node,
            _events: receiver,
        }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.node.stop_sharing();
    }
}

fn loopback_address(host: &ShareIrohNode) -> EndpointAddr {
    let socket = match host.bound_ports() {
        (Some(port), _) => SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        (None, Some(port)) => SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port),
        (None, None) => panic!("the host node is bound to no socket"),
    };
    EndpointAddr::from_parts(host.endpoint.id(), [TransportAddr::Ip(socket)])
}

/// Each handshake runs from the start of `connect` to the established
/// connection. A handshake that reaches `UNKNOWN_HANDSHAKE` fails the test;
/// it is counted and the others still run, so the failure names how many
/// reached the deadline and how the durations are spread (one lost packet
/// pair by chance reads differently from a deadline that is too short).
/// Each connection closes at once, so the host never waits for a hello and
/// books nothing against the peer.
#[test]
#[ignore = "netem auf lo (CAP_NET_ADMIN); Suite crates-task"]
fn unbelegt_task_unknown_handshake_mobile_profile() {
    let host_identity = identity("mobile host").expect("host identity");
    let peer_identity = identity("mobile stranger").expect("peer identity");
    let host = Node::start(&host_identity, auth_state(&host_identity));
    let peer = Node::start(&peer_identity, auth_state(&peer_identity));
    let address = loopback_address(&host.node);

    let (mut durations, timed_out) = peer.node.block_on(async {
        let mut durations = Vec::with_capacity(HANDSHAKES);
        let mut timed_out = 0_usize;
        for attempt in 0..HANDSHAKES {
            let started = Instant::now();
            let connected = tokio::time::timeout(
                UNKNOWN_HANDSHAKE,
                peer.node.endpoint.connect(address.clone(), ALPN),
            )
            .await;
            match connected {
                Ok(Ok(connection)) => {
                    durations.push(started.elapsed());
                    connection.close(VarInt::from_u32(0), b"measured");
                }
                Ok(Err(error)) => panic!("handshake {attempt} failed: {error:?}"),
                Err(_) => {
                    timed_out += 1;
                    durations.push(UNKNOWN_HANDSHAKE);
                }
            }
        }
        (durations, timed_out)
    });

    durations.sort();
    // Nearest rank: the element at position ceil(0.99 * n) of the sorted list.
    let rank = (99 * durations.len()).div_ceil(100);
    let p99 = durations[rank - 1];
    let median = durations[durations.len() / 2];
    let slowest = durations[durations.len() - 1];
    assert_eq!(
        timed_out, 0,
        "{timed_out} of {HANDSHAKES} handshakes reached {UNKNOWN_HANDSHAKE:?}: \
         median {median:?}, 99th percentile {p99:?}"
    );
    assert!(slowest < UNKNOWN_HANDSHAKE, "slowest handshake {slowest:?}");
    assert!(
        p99 < PERCENTILE_LIMIT,
        "99th percentile {p99:?} of {HANDSHAKES} handshakes: median {median:?}, slowest {slowest:?}"
    );
}
