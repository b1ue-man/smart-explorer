//! E43, M21, S2 (L2 point 5, p-erg-15 B8): the machine login target over two
//! real nodes on loopback. A host without any file export answers the
//! viewer's `Capabilities` and `LoginTarget` (assumptions A1 and A2 of
//! p-erg-1); the viewer keeps the target bound to the host's device ID and
//! Share key only while it may steer the host's screen, and a host that no
//! longer lets it steer withdraws a target kept before.
//!
//! The steering set, the login broker's answer and the viewer's target book
//! are process-wide, so the test is ignored and suite part `ignored` runs it
//! alone with an isolated app data folder (group „Login target over two
//! nodes“ of `native/test-crates-task.py`).
use std::collections::BTreeSet;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::login::core::{
    gateway_types::GatewayTarget,
    model::VERSION,
    self_grants::{SelfReply, SELF_FORMAT},
};
use crate::share::access_steering::note_steering_devices;
use crate::share::core::{now_secs, public_fingerprint, random_bytes, random_uuid_v4};
use crate::share::fs::ShareExportConfig;
use crate::share::identity::ShareIdentity;
use crate::share::login_transport::{paired_login_target_bound, target_store, target_wire};
use crate::share::node::ShareIrohNode;
use crate::share::types::{
    DirectAccessState, DirectContact, DirectGrant, DirectGrantState, PeerEndpoint, PeerPresence,
    ShareAuthState, ShareEvent, ShareScope, ShareStatus,
};
use crate::share::ExecGrant;

const NO_RELAY: &str = "relay-disabled://login-ziel-knoten";
const CONTACT: &str = "login-ziel-knoten-host";
/// Bound of every wait for a state the nodes reach on their own.
const SETTLE: Duration = Duration::from_secs(30);

struct NodeGuard(Arc<ShareIrohNode>);

impl Drop for NodeGuard {
    fn drop(&mut self) {
        let _ = self.0.stop_sharing();
    }
}

/// Broker answer and steering set go back when the test ends, also on a panic.
struct Reset;

impl Drop for Reset {
    fn drop(&mut self) {
        crate::login::self_setup::broker_stand_in::set(None);
        note_steering_devices(BTreeSet::new());
    }
}

/// A host without exports that grants the viewer access, and the viewer
/// that pinned the host as accepted Direct contact.
struct Pair {
    host_identity: ShareIdentity,
    viewer_identity: ShareIdentity,
    endpoint: PeerEndpoint,
    _events: [crossbeam_channel::Receiver<ShareEvent>; 2],
    // Fields drop in order: both nodes stop in this order.
    viewer: NodeGuard,
    _host: NodeGuard,
}

impl Pair {
    fn new() -> io::Result<Self> {
        let viewer_identity = identity("login viewer")?;
        let host_identity = identity("login host")?;
        let mut host_state = auth_state(&host_identity);
        host_state.direct_grants.push(DirectGrant {
            device_id: viewer_identity.device_id.clone(),
            device_name: viewer_identity.device_name.clone(),
            public_key: viewer_identity.public_key.clone(),
            fingerprint: viewer_identity.fingerprint.clone(),
            node_id: viewer_identity.node_id.clone(),
            state: DirectGrantState::Accepted,
            updated_at: now_secs(),
            exec: ExecGrant::default(),
            write: false,
        });
        let (host_events, host_events_rx) = crossbeam_channel::bounded(64);
        let host = NodeGuard(ShareIrohNode::start(
            NO_RELAY,
            &host_identity,
            Arc::new(Mutex::new(host_state)),
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
        let mut viewer_state = auth_state(&viewer_identity);
        viewer_state.direct_contacts.push(contact(&host_identity));
        let (viewer_events, viewer_events_rx) = crossbeam_channel::bounded(64);
        let viewer = NodeGuard(ShareIrohNode::start(
            NO_RELAY,
            &viewer_identity,
            Arc::new(Mutex::new(viewer_state)),
            viewer_events,
        )?);
        let endpoint = PeerEndpoint {
            label: "login loopback".into(),
            scope: ShareScope::Direct {
                contact_id: CONTACT.into(),
            },
            presence,
            relation_secret: host_identity.direct_secret(),
            expected_node_id: Some(host_identity.node_id.clone()),
        };
        Ok(Self {
            host_identity,
            viewer_identity,
            endpoint,
            _events: [host_events_rx, viewer_events_rx],
            viewer,
            _host: host,
        })
    }

    /// The viewer's session to the host; the target fetch starts with it.
    fn connect(&self) {
        let deadline = Instant::now() + SETTLE;
        self.viewer
            .0
            .open_stream_until(&self.endpoint, &self.viewer_identity, deadline)
            .expect("the viewer opens a session to the host");
    }

    fn target_kept(&self) -> bool {
        paired_login_target_bound(
            &self.host_identity.device_id,
            &self.host_identity.public_key,
        )
        .is_some()
    }
}

fn gateway() -> GatewayTarget {
    GatewayTarget {
        format: VERSION,
        machine_id: "machine-login-ziel-knoten".into(),
        public_key: iroh::SecretKey::from_bytes(&[61; 32]).public().to_string(),
        direct_addresses: vec!["192.0.2.30:4433".parse().expect("address")],
        relay_urls: vec!["https://relay.example.test/".into()],
        use_default_relays: false,
        allow_plaintext_relays: false,
    }
}

/// The host's login broker: its gateway runs and holds a grant for `viewer`.
fn broker(viewer: &ShareIdentity) -> SelfReply {
    SelfReply {
        format: SELF_FORMAT,
        state: "ok".into(),
        reason: None,
        release: "test".into(),
        account: "1000".into(),
        policy_revision: 1,
        own_grants: vec![viewer.device_id.clone()],
        other_grants: 0,
        target: Some(gateway()),
    }
}

fn steering(viewer: &ShareIdentity) -> BTreeSet<(String, String)> {
    BTreeSet::from([(viewer.device_id.clone(), viewer.public_key.clone())])
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + SETTLE;
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "process-wide steering set, login broker stand-in and target book; Suite crates-task"]
fn login_ziel_task_two_nodes_deliver_the_target_only_with_steering() {
    let _reset = Reset;

    // The viewer steers the host's screen: the target arrives after the
    // session handshake, bound to the host's device ID and Share key.
    let steered = Pair::new().expect("two nodes on loopback");
    crate::login::self_setup::broker_stand_in::set(Some(broker(&steered.viewer_identity)));
    note_steering_devices(steering(&steered.viewer_identity));
    assert!(!steered.target_kept());
    steered.connect();
    wait_until("the steering viewer keeps the host's login target", || {
        steered.target_kept()
    });
    let kept = paired_login_target_bound(
        &steered.host_identity.device_id,
        &steered.host_identity.public_key,
    )
    .expect("kept");
    assert_eq!(
        (kept.target, kept.target_user.as_str()),
        (gateway(), "1000")
    );
    assert!(paired_login_target_bound(
        &steered.host_identity.device_id,
        &steered.viewer_identity.public_key
    )
    .is_none());
    drop(steered);

    // Without the right to steer, the host names no target: a target the
    // viewer kept for that host before is withdrawn by the next session.
    let unsteered = Pair::new().expect("two nodes on loopback");
    crate::login::self_setup::broker_stand_in::set(Some(broker(&unsteered.viewer_identity)));
    note_steering_devices(BTreeSet::new());
    keep_earlier_target(&unsteered);
    assert!(unsteered.target_kept());
    unsteered.connect();
    wait_until("the host without steering withdraws the target", || {
        !unsteered.target_kept()
    });
}

/// The book holds a target of `pair`'s host from an earlier session.
fn keep_earlier_target(pair: &Pair) {
    let grants = vec![pair.viewer_identity.device_id.clone()];
    let target = gateway();
    let message = target_wire::host_message(&target_wire::HostFacts {
        host_device_id: &pair.host_identity.device_id,
        device_id: &pair.viewer_identity.device_id,
        steering: true,
        account: Some("1000"),
        own_grants: &grants,
        target: Some(&target),
    });
    let peer = target_wire::SessionPeer {
        device_id: pair.host_identity.device_id.clone(),
        public_key: pair.host_identity.public_key.clone(),
        node_id: pair.host_identity.node_id.clone(),
        device_name: "Gerät login host".into(),
        relation_id: CONTACT.into(),
        pinned: true,
    };
    let stored = target_store::update(|book| {
        let change = book.apply(&peer, &message, now_secs() as u64);
        let changed = change.is_ok();
        (change, changed)
    })
    .expect("the book is written");
    assert_eq!(stored, Ok(target_store::Change::Stored));
}

fn contact(host: &ShareIdentity) -> DirectContact {
    DirectContact {
        id: CONTACT.into(),
        display_name: "login host".into(),
        lookup_id: host.direct_lookup_id.clone(),
        expected_fingerprint: host.fingerprint.clone(),
        expected_node_id: host.node_id.clone(),
        remote_device_id: Some(host.device_id.clone()),
        remote_public_key: Some(host.public_key.clone()),
        auto_connect: false,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::Offline,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: Some(1),
        accepted_public_key: Some(host.public_key.clone()),
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    }
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
