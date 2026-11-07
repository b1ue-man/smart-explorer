//! G6 and G7 (batch 2026-11-07, reviews r-exec-2 and r-exec-3) against two
//! real nodes on loopback: a host with a grant for a viewer, and the viewer
//! that asks it for TCP tunnels. Never loads identities or saved profiles; no
//! relay and no third-party server take part.
use crate::share::access_constraints::{
    AccessConstraints, NodeId, TcpDirection, TcpForwardRule, TcpHost, TcpPort,
};
use crate::share::access_grant::{AccessGrantChange, Capability, CapabilityDecision};
use crate::share::core::{now_secs, public_fingerprint, random_bytes, random_uuid_v4};
use crate::share::fs::ShareExportConfig;
use crate::share::identity::ShareIdentity;
use crate::share::node::ShareIrohNode;
use crate::share::node_tcp_link::OpenedLink;
use crate::share::tcp_forward::{shown_error, TcpDestination, TcpStart, TunnelId, MAX_TCP_TUNNELS};
use crate::share::types::{
    DirectGrant, DirectGrantState, PeerEndpoint, PeerPresence, ShareAuthState, ShareScope,
};
use crate::share::ExecGrant;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const NO_RELAY: &str = "relay-disabled://tcp-task";
/// Bound of one request for a tunnel, from the connection to the answer.
const OPEN_LIMIT: Duration = Duration::from_secs(45);
/// Bound of every wait for a state the nodes reach on their own.
const SETTLE: Duration = Duration::from_secs(20);

struct NodeGuard(Arc<ShareIrohNode>);

impl Drop for NodeGuard {
    fn drop(&mut self) {
        let _ = self.0.stop_sharing();
    }
}

struct Pair {
    host_auth: Arc<Mutex<ShareAuthState>>,
    endpoint: PeerEndpoint,
    viewer_identity: ShareIdentity,
    // Fields drop in order: both nodes stop in this order.
    viewer: NodeGuard,
    host: NodeGuard,
}

impl Pair {
    /// `allowed_port`: the host grants the viewer a rule for the DNS name
    /// `localhost` (loopback addresses) on that port; `None`: a grant
    /// without any TCP rule.
    fn new(allowed_port: Option<u16>) -> io::Result<Self> {
        let viewer_identity = identity("tcp viewer")?;
        let host_identity = identity("tcp host")?;
        let exec = match allowed_port {
            Some(port) => rule_grant(port, &viewer_identity, &host_identity)?,
            None => ExecGrant::default(),
        };
        let mut host_state = auth_state(&host_identity);
        host_state.direct_grants.push(DirectGrant {
            device_id: viewer_identity.device_id.clone(),
            device_name: viewer_identity.device_name.clone(),
            public_key: viewer_identity.public_key.clone(),
            fingerprint: viewer_identity.fingerprint.clone(),
            node_id: viewer_identity.node_id.clone(),
            state: DirectGrantState::Accepted,
            updated_at: now_secs(),
            exec,
            write: false,
        });
        let host_auth = Arc::new(Mutex::new(host_state));
        let (host_events, _host_events_rx) = crossbeam_channel::bounded(64);
        let host = NodeGuard(ShareIrohNode::start(
            NO_RELAY,
            &host_identity,
            host_auth.clone(),
            host_events,
        )?);
        let presence = PeerPresence {
            kind: "direct".into(),
            relation_id: host_identity.direct_lookup_id.clone(),
            device_id: host_identity.device_id.clone(),
            device_name: host_identity.device_name.clone(),
            public_key: host_identity.public_key.clone(),
            fingerprint: host_identity.fingerprint.clone(),
            node_id: host_identity.node_id.clone(),
            relay_url: String::new(),
            candidates: loopback_candidates(&host.0)?,
            expires_at: now_secs() + 15 * 60,
            nonce: random_uuid_v4().map_err(io::Error::other)?,
            proof: String::new(),
        };
        let (viewer_events, _viewer_events_rx) = crossbeam_channel::bounded(64);
        let viewer = NodeGuard(ShareIrohNode::start(
            NO_RELAY,
            &viewer_identity,
            Arc::new(Mutex::new(auth_state(&viewer_identity))),
            viewer_events,
        )?);
        let endpoint = PeerEndpoint {
            label: "tcp loopback".into(),
            scope: ShareScope::Direct {
                contact_id: "tcp-host".into(),
            },
            presence,
            relation_secret: host_identity.direct_secret(),
            expected_node_id: Some(host_identity.node_id.clone()),
        };
        Ok(Self {
            host_auth,
            endpoint,
            viewer_identity,
            viewer,
            host,
        })
    }

    /// The viewer asks the host for a tunnel to `destination`.
    fn open(&self, destination: TcpDestination) -> io::Result<OpenedLink> {
        let request = TcpStart {
            destination,
            local_port: 0,
            acknowledge_local_access: true,
            lifetime_secs: 0,
        };
        let id = TunnelId::generate().map_err(io::Error::other)?;
        let node = &self.viewer.0;
        let answer = node.block_on(async {
            tokio::time::timeout(
                OPEN_LIMIT,
                node.open_tcp_link(
                    &self.viewer_identity,
                    &self.endpoint,
                    request,
                    &id,
                    None,
                    None,
                ),
            )
            .await
        });
        answer.unwrap_or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "TCP open timed out",
            ))
        })
    }
}

/// The text a refusal shows the viewer.
fn refusal(answer: io::Result<OpenedLink>) -> String {
    match answer {
        Ok(_) => panic!("the host opened a tunnel it had to refuse"),
        Err(error) => {
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
            error.to_string()
        }
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + SETTLE;
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn destination(host: TcpHost, port: u16) -> TcpDestination {
    TcpDestination {
        host,
        port: TcpPort::try_from(port).unwrap(),
    }
}

/// A DNS host as the peer names it; `localhost` resolves on every host.
fn name(host: &str, port: u16) -> TcpDestination {
    let host = TcpHost::Dns {
        name: host.into(),
        addresses: Vec::new(),
    };
    destination(host, port)
}

/// A local service the host can reach: only its port matters.
fn local_service() -> (TcpListener, u16) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("local service");
    let port = listener.local_addr().expect("local service port").port();
    (listener, port)
}

/// The grant: TCP forwarding allowed to `localhost` on `port`, no more.
fn rule_grant(port: u16, viewer: &ShareIdentity, host: &ShareIdentity) -> io::Result<ExecGrant> {
    let rule = TcpForwardRule {
        initiator: NodeId::parse(&viewer.node_id).map_err(io::Error::other)?,
        target: NodeId::parse(&host.node_id).map_err(io::Error::other)?,
        direction: TcpDirection::PeerToHost,
        host: TcpHost::Dns {
            name: "localhost".into(),
            addresses: vec![
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                IpAddr::V6(Ipv6Addr::LOCALHOST),
            ],
        },
        ports: vec![TcpPort::try_from(port).map_err(io::Error::other)?],
        valid_until: None,
    };
    let change = AccessGrantChange {
        capabilities: [(Capability::TcpForward, CapabilityDecision::Allow)].into(),
        constraints: Some(AccessConstraints {
            tcp_forward: vec![rule],
            ..AccessConstraints::default()
        }),
        ..AccessGrantChange::default()
    };
    let mut grant = ExecGrant::default();
    grant
        .apply_access_change(&change, now_secs())
        .map_err(io::Error::other)?;
    Ok(grant)
}

fn identity(name: &str) -> io::Result<ShareIdentity> {
    let secret = iroh::SecretKey::from_bytes(&random_bytes::<32>().map_err(io::Error::other)?);
    let node_id = secret.public().to_string();
    Ok(ShareIdentity {
        device_id: random_uuid_v4().map_err(io::Error::other)?,
        device_name: name.to_string(),
        direct_lookup_id: random_uuid_v4().map_err(io::Error::other)?,
        public_key: node_id.clone(),
        fingerprint: public_fingerprint(node_id.as_bytes()),
        node_id,
        iroh_secret: secret,
        direct_secret: random_bytes::<32>().map_err(io::Error::other)?,
    })
}

fn auth_state(identity: &ShareIdentity) -> ShareAuthState {
    ShareAuthState {
        identity: identity.clone(),
        direct_secret: identity.direct_secret(),
        default_direct_exports: ShareExportConfig::default(),
        direct_contacts: Vec::new(),
        direct_grants: Vec::new(),
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 0,
    }
}

/// The host's loopback addresses once its endpoint is bound.
fn loopback_candidates(node: &ShareIrohNode) -> io::Result<Vec<String>> {
    let deadline = Instant::now() + SETTLE;
    loop {
        let mut candidates: Vec<String> = node
            .candidates()
            .into_iter()
            .filter_map(|address| address.parse::<SocketAddr>().ok())
            .filter(|address| address.port() != 0)
            .map(|address| {
                let ip = if address.is_ipv4() {
                    IpAddr::V4(Ipv4Addr::LOCALHOST)
                } else {
                    IpAddr::V6(Ipv6Addr::LOCALHOST)
                };
                SocketAddr::new(ip, address.port()).to_string()
            })
            .collect();
        candidates.sort();
        candidates.dedup();
        if !candidates.is_empty() {
            return Ok(candidates);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the host has no bound address",
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// T-G6: a relation without a rule for the destination reads the same
/// refusal whether the name resolves or not, and for loopback without the
/// right to steer. Before this change an unresolvable name answered with the
/// resolver's error, which told the peer what this host's DNS knows.
#[test]
fn exec_task_g6_target_refuses_unlisted_names_alike_before_resolving() {
    let pair = Pair::new(None).expect("two nodes on loopback");
    let expected = shown_error(super::target::TARGET_NOT_AUTHORIZED);
    let cases = [
        (
            "a name that cannot resolve",
            name("does-not-exist.invalid", 80),
        ),
        ("a name that resolves", name("localhost", 80)),
        ("loopback", destination(TcpHost::Loopback, 80)),
    ];
    for (label, destination) in cases {
        assert_eq!(refusal(pair.open(destination)), expected, "{label}");
    }
}

/// Erhalt of T-G6 and of the target place: a relation with a rule for the
/// name still gets its tunnel (`Opened`), which holds one target place while
/// it lives, none of the host's own forwards, and returns it when it ends.
#[test]
fn exec_task_g6_rule_keeps_its_tunnel_and_it_holds_one_target_place() {
    let (_service, port) = local_service();
    let pair = Pair::new(Some(port)).expect("two nodes on loopback");
    let tcp = &pair.host.0.tcp;
    assert_eq!(tcp.target_slots.available_permits(), MAX_TCP_TUNNELS);
    let link = pair
        .open(name("localhost", port))
        .map_err(|error| error.to_string())
        .expect("a rule for the name opens the tunnel");
    assert_eq!(tcp.target_slots.available_permits(), MAX_TCP_TUNNELS - 1);
    assert_eq!(tcp.slots.available_permits(), MAX_TCP_TUNNELS);
    drop(link);
    wait_until("the ended tunnel returns its target place", || {
        tcp.target_slots.available_permits() == MAX_TCP_TUNNELS
    });
}

/// T-G7: tunnels peers hold on this host (target role) and its own forwards
/// (listener role) have separate places; the 65th target tunnel reads
/// `TCP tunnel admission exhausted`; a sender whose relation is not proven
/// reads the relation refusal even while every target place is held, so it
/// takes none.
#[test]
fn exec_task_g7_target_places_are_separate_and_taken_after_the_relation_proof() {
    let (_service, port) = local_service();
    let pair = Pair::new(Some(port)).expect("two nodes on loopback");
    let tcp = &pair.host.0.tcp;
    let all = u32::try_from(MAX_TCP_TUNNELS).unwrap();
    assert!(!Arc::ptr_eq(&tcp.slots, &tcp.target_slots));

    // 64 target tunnels held: the host can still start its own forward, and
    // the next peer tunnel is refused with the text of the limit.
    let held = tcp.target_slots.clone().try_acquire_many_owned(all);
    let held = held.expect("all target places are free at the start");
    assert_eq!(tcp.slots.available_permits(), MAX_TCP_TUNNELS);
    assert!(tcp.slots.clone().try_acquire_owned().is_ok());
    assert_eq!(
        refusal(pair.open(name("localhost", port))),
        "TCP tunnel admission exhausted"
    );

    // A sender without a proven relation reads the relation refusal, not the
    // limit: the place is looked at only after the proof.
    pair.host_auth.lock().unwrap().direct_grants.clear();
    let unproven = refusal(pair.open(name("localhost", port)));
    assert!(!unproven.is_empty());
    assert!(!unproven.contains("admission exhausted"), "{unproven}");

    // Released, nothing is taken by the refused attempts.
    drop(held);
    assert!(pair.open(name("localhost", port)).is_err());
    assert_eq!(tcp.target_slots.available_permits(), MAX_TCP_TUNNELS);
    assert_eq!(tcp.slots.available_permits(), MAX_TCP_TUNNELS);
}
