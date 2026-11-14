//! r-zugang U1 (Restbeleg M34, E61): the 3.5 s deadline of an unknown peer's
//! handshake (`UNKNOWN_HANDSHAKE`, `node_accept.rs`) on a mobile-like path.
//! The suite `crates-task` sets netem on `lo` before this test runs
//! (`tc qdisc replace dev lo root netem delay 350ms 50ms loss 2%`). Without
//! that the test measures plain loopback and says nothing about mobile
//! networks.
//!
//! What the profile allows the test to assert. Every datagram is lost with 2 %
//! and every one-way trip takes 300..400 ms, so a round trip takes 600..800 ms.
//! The handshake is 4 critical datagrams (two Initials with the ClientHello,
//! whose hybrid key share does not fit one datagram; the server's Initial and
//! its Handshake flight). It takes one round trip (about 760 ms) unless one of
//! them is lost. A lost second Initial or a lost server Handshake flight is
//! only recovered by the probe timeout (PTO), and once a round trip was sampled
//! the PTO is 3 x RTT (RFC 9002 section 5.3 and 6.2.1: smoothed RTT + 4 x
//! RTT variance, with variance = RTT / 2 after the first sample; noq-proto
//! 1.3.0 `RttEstimator::pto_base`, `Connection::pto_time_and_space`), so such a
//! handshake ends at 3 x 700 + 700 + 60 = about 2.86 s (sd 0.13 s) and
//! exceeds 3 s in about 15 % of those cases; with the largest round trip of
//! the profile (800 ms) it ends at 3 x 800 + 800 + 60 = 3.26 s. The deadline
//! was 3 s until the suite run of 2026-11-14 measured 3 of 100 hits (median
//! 760 ms); per handshake the profile gives 0.9 % to 1.4 % hits at 3 s. At
//! 3.5 s every handshake with one lost datagram completes, and only a second
//! loss during the retransmission reaches the deadline: the rate of a first
//! critical loss is at most 4 x 2 % = 8 %, a further loss of the
//! retransmission or its answer 2 x 2 % = 4 %, together at most 0.32 % per
//! handshake. A 99th percentile below 2 s cannot hold for this profile (4 %
//! to 6 % of the handshakes take longer). The test asserts what the profile
//! leaves intact: the median is one round trip, and the deadline hits stay
//! below the number that 100 handshakes at a rate of 0.5 % (more than the
//! computed 0.32 %) exceed with a probability of 0.2 %.
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
/// The median handshake is one round trip (600..800 ms) plus processing; a
/// median beyond this means the deadline or the path handling is at fault, not
/// a lost datagram.
const MEDIAN_LIMIT: Duration = Duration::from_millis(1000);
/// Handshakes that may reach `UNKNOWN_HANDSHAKE`: P(more than 2 of 100 |
/// 0.5 %) is 0.2 % (binomial); the rate the profile causes is at most 0.32 %
/// (see the header).
const MAX_TIMED_OUT: usize = 2;

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
/// connection. A handshake that reaches `UNKNOWN_HANDSHAKE` is counted and the
/// others still run, so the failure names how many reached the deadline and
/// how the durations are spread. Each connection closes at once, so the host
/// never waits for a hello and books nothing against the peer.
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
    assert!(
        timed_out <= MAX_TIMED_OUT,
        "{timed_out} of {HANDSHAKES} handshakes reached {UNKNOWN_HANDSHAKE:?} \
         (at most {MAX_TIMED_OUT} allowed): median {median:?}, 99th percentile {p99:?}, \
         slowest {slowest:?}"
    );
    assert!(
        median < MEDIAN_LIMIT,
        "median {median:?} of {HANDSHAKES} handshakes: 99th percentile {p99:?}, slowest {slowest:?}"
    );
}
