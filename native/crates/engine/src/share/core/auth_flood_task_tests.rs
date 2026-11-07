//! R-ZUGANG-04 over real QUIC on loopback: an unknown EndpointId that opens
//! the file-session ALPN and never sends a hello is booked as "never
//! authenticated" when the 3 s deadline of the host ends. Booked often
//! enough within the penalty window, the host closes its connections before
//! any application handshake. A paired device that stays as silent is never
//! booked and never turned away.
//!
//! A penalty of an EndpointId adds 1 s per such connection and runs down in
//! real time, and the threshold is 15 s (`auth_penalty.rs`). So the
//! connections must time out together: the host takes at most eight unknown
//! handshakes at once (a pool of 64 / 8), which is the size of one wave. Each
//! wave adds 8 s to the penalty and takes about 3 s; the third wave crosses
//! the threshold. Waves repeat up to a bound instead of fixing their number.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iroh::endpoint::{ConnectionError, VarInt};
use iroh::{EndpointAddr, TransportAddr};

use crate::share::backend::ShareIrohNode;
use crate::share::identity::ShareIdentity;
use crate::share::node::ALPN;
use crate::share::power::test_support::{auth_state, identity, NO_RELAY};
use crate::share::types::{DirectGrant, DirectGrantState, ShareAuthState, ShareEvent};

/// Unknown handshakes the host runs at once.
const WAVE: usize = 8;
const MAX_WAVES: usize = 8;
/// Silent connections of the paired device (more than the 15 s threshold, so
/// booking them would show).
const PAIRED: usize = 20;
/// The whole scenario; the waves and the paired device's 20 s take about 25 s.
const LIMIT: Duration = Duration::from_secs(120);
const PATIENCE: Duration = Duration::from_secs(15);
/// Application close code of "Share admission rejected" (`node_accept.rs`).
const REFUSED: u32 = 2;

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

/// What one silent connection saw.
struct Outcome {
    /// From the end of the QUIC handshake to the end of the wait.
    after: Duration,
    connected: bool,
    /// The host closed it with "Share admission rejected".
    rejected: bool,
    still_open: bool,
}

/// One connection of `node` to the host's file-session ALPN that never
/// opens a stream, up to `patience`.
async fn silent(node: Arc<ShareIrohNode>, address: EndpointAddr, patience: Duration) -> Outcome {
    let started = Instant::now();
    let Ok(connection) = node.endpoint.connect(address, ALPN).await else {
        return Outcome {
            after: started.elapsed(),
            connected: false,
            rejected: false,
            still_open: false,
        };
    };
    let connected = Instant::now();
    // As a real client does after the handshake; the host waits for it too.
    let _ = node
        .await_route(&connection, None, Some(connected + patience))
        .await;
    let _ = tokio::time::timeout(patience, connection.closed()).await;
    let close = connection.close_reason();
    Outcome {
        after: connected.elapsed(),
        connected: true,
        rejected: matches!(
            &close,
            Some(ConnectionError::ApplicationClosed(close))
                if close.error_code == VarInt::from_u32(REFUSED)
        ),
        still_open: close.is_none(),
    }
}

/// `PAIRED` silent connections, opened one after the other (so their
/// deadlines end together); signals once all are open, and returns how long
/// each lasted.
async fn paired_connections(
    node: Arc<ShareIrohNode>,
    address: EndpointAddr,
    open: tokio::sync::oneshot::Sender<()>,
) -> Vec<(Duration, bool)> {
    let mut connections = Vec::new();
    for _ in 0..PAIRED {
        let connection = node
            .endpoint
            .connect(address.clone(), ALPN)
            .await
            .expect("the paired device connects");
        let _ = node
            .await_route(&connection, None, Some(Instant::now() + PATIENCE))
            .await;
        connections.push((Instant::now(), connection));
    }
    let _ = open.send(());
    let mut lasted = Vec::new();
    for (opened, connection) in connections {
        let closed = tokio::time::timeout(Duration::from_secs(60), connection.closed()).await;
        lasted.push((opened.elapsed(), closed.is_ok()));
    }
    lasted
}

#[test]
fn zugang_task_silent_connections_refuse_an_unknown_endpoint_but_not_a_paired_one() {
    let host_identity = identity("flood host").expect("host identity");
    let stranger_identity = identity("flood stranger").expect("stranger identity");
    let paired_identity = identity("flood paired").expect("paired identity");
    let mut host_state = auth_state(&host_identity);
    host_state.direct_grants.push(DirectGrant {
        device_id: paired_identity.device_id.clone(),
        device_name: paired_identity.device_name.clone(),
        public_key: paired_identity.public_key.clone(),
        fingerprint: paired_identity.fingerprint.clone(),
        node_id: paired_identity.node_id.clone(),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: crate::share::ExecGrant::default(),
        write: false,
    });
    let host = Node::start(&host_identity, host_state);
    let stranger = Node::start(&stranger_identity, auth_state(&stranger_identity));
    let paired = Node::start(&paired_identity, auth_state(&paired_identity));
    let address = loopback_address(&host.node);
    let refused = |id: &str| host.node.auth_penalties().refusal(id, Instant::now());
    assert!(refused(&stranger_identity.node_id).is_none());

    let finished = stranger.node.block_on(async {
        tokio::time::timeout(LIMIT, async {
            // The paired device's silent connections run for their 20 s
            // while the stranger's waves run.
            let (open, opened) = tokio::sync::oneshot::channel();
            let paired_run = paired.node.rt.spawn(paired_connections(
                paired.node.clone(),
                address.clone(),
                open,
            ));
            opened.await.expect("the paired device's connections open");

            let mut waves = 0;
            while refused(&stranger_identity.node_id).is_none() {
                waves += 1;
                assert!(waves <= MAX_WAVES, "no refusal after {MAX_WAVES} waves");
                let wave: Vec<_> = (0..WAVE)
                    .map(|_| tokio::spawn(silent(stranger.node.clone(), address.clone(), PATIENCE)))
                    .collect();
                for attempt in wave {
                    attempt.await.expect("silent connection task");
                }
            }

            // Turned away before the application handshake: at once, not
            // after the 3 s deadline.
            let turned = silent(stranger.node.clone(), address.clone(), PATIENCE).await;
            assert!(
                turned.after < Duration::from_millis(2500),
                "{:?}",
                turned.after
            );
            assert!(!turned.connected || turned.rejected);

            // The paired device is not affected: no refusal, and a further
            // silent connection stays open (its deadline is 20 s).
            assert!(refused(&paired_identity.node_id).is_none());
            let further = paired
                .node
                .rt
                .spawn(silent(
                    paired.node.clone(),
                    address.clone(),
                    Duration::from_secs(2),
                ))
                .await
                .expect("further connection task");
            assert!(further.connected && further.still_open);

            paired_run.await.expect("paired connections task")
        })
        .await
    });
    let lasted = finished.expect("the scenario ends within its limit");
    assert_eq!(lasted.len(), PAIRED);
    for (index, (duration, closed)) in lasted.iter().enumerate() {
        // Held as long as the 20 s of a paired device, not the 3 s of an
        // unknown one, and then closed by the host.
        assert!(
            *duration >= Duration::from_secs(10),
            "paired connection {index} lasted {duration:?}"
        );
        assert!(closed, "paired connection {index} never ended");
    }
    // Their deadlines booked nothing: 20 bookings within a second would be
    // a refusal for minutes.
    assert!(refused(&paired_identity.node_id).is_none());
}
