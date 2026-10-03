//! In-memory nodes and current runner facts; never enables privileged uplink.
use super::super::super::fs::ShareExportConfig;
use super::super::super::identity::ShareIdentity;
use super::super::super::types::{DirectAccessState, DirectContact, DirectGrant, DirectGrantState};
use super::super::*;
use super::wait_until;
use crate::net::InterfaceFacts;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tokio::task::JoinHandle;

pub(super) struct RunnerLan {
    pub(super) ip: IpAddr,
    pub(super) facts: Vec<InterfaceFacts>,
    pub(super) interface: InterfaceFacts,
}

impl RunnerLan {
    fn discover() -> Self {
        let ip: Ipv4Addr = std::env::var("SE_REVIEW_LAN_IP")
            .expect("remote suite must discover SE_REVIEW_LAN_IP")
            .parse()
            .expect("private IPv4");
        assert!(
            ip.is_private() || ip.is_link_local(),
            "no loopback/public fallback"
        );
        let index: u32 = std::env::var("SE_REVIEW_LAN_IFINDEX")
            .expect("runner interface index")
            .parse()
            .expect("numeric interface index");
        let name = std::env::var("SE_REVIEW_LAN_IFNAME").expect("runner interface name");
        let facts = crate::net::gather_interface_facts().expect("current OS interface facts");
        let matching: Vec<_> = facts
            .iter()
            .filter(|iface| iface.addrs.contains(&IpAddr::V4(ip)))
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "runner IP must identify exactly one real interface"
        );
        let interface = matching[0].clone();
        assert!(
            index > 0 && interface.up && !interface.loopback && !interface.adapter_id.is_empty()
        );
        assert_eq!(interface.index, index);
        assert_eq!(interface.name, name);
        Self {
            ip: IpAddr::V4(ip),
            facts,
            interface,
        }
    }

    pub(super) fn host(&self, own_uplink: OwnUplink) -> LanLinkHostFacts {
        LanLinkHostFacts {
            enabled: true,
            interfaces: self.facts.clone(),
            shared_ifaces: Vec::new(),
            own_uplink,
        }
    }
}

pub(super) fn fixture_identity(seed: u8) -> ShareIdentity {
    let secret = iroh::SecretKey::from_bytes(&[seed; 32]);
    let peer = DirectPeerIdentity::from_secret(format!("s09-device-{seed}"), "", &secret);
    ShareIdentity {
        device_id: peer.device_id,
        device_name: String::new(),
        direct_lookup_id: format!("s09-lookup-{seed}"),
        public_key: peer.public_key,
        fingerprint: peer.fingerprint,
        node_id: peer.node_id,
        iroh_secret: secret,
        direct_secret: [0; 32],
    }
}

pub(super) fn accepted_contact(peer: &ShareIdentity) -> DirectContact {
    DirectContact {
        id: "s09-peer".into(),
        display_name: String::new(),
        lookup_id: peer.direct_lookup_id.clone(),
        expected_fingerprint: peer.fingerprint.clone(),
        expected_node_id: peer.node_id.clone(),
        remote_device_id: Some(peer.device_id.clone()),
        remote_public_key: Some(peer.public_key.clone()),
        accepted_public_key: Some(peer.public_key.clone()),
        auto_connect: false,
        auto_open: false,
        last_seen: None,
        status: Default::default(),
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: Some(1),
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    }
}

fn accepted_grant(peer: &ShareIdentity) -> DirectGrant {
    DirectGrant {
        device_id: peer.device_id.clone(),
        device_name: String::new(),
        public_key: peer.public_key.clone(),
        fingerprint: peer.fingerprint.clone(),
        node_id: peer.node_id.clone(),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: Default::default(),
        write: false,
    }
}

fn fixture_auth(identity: ShareIdentity) -> ShareAuthState {
    ShareAuthState {
        identity,
        direct_secret: vec![0; 32],
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

fn permits(node: &ShareIrohNode) -> (usize, usize, usize) {
    (
        node.handshake_slots.available_permits(),
        node.direct_repair_slots.available_permits(),
        node.runtime_transition_slot.available_permits(),
    )
}

pub(super) fn assert_status_only(
    node: &ShareIrohNode,
    connection: &Connection,
    baseline: (usize, usize, usize),
) {
    assert!(
        !node.policy.is_bound(connection).unwrap(),
        "paired link must never acquire FS authority"
    );
    assert!(node.sessions.lock().unwrap().is_empty());
    assert!(node.exec_registry().active_views().is_empty());
    assert!(node.exec_registry().redacted_history().is_empty());
    assert_eq!(
        permits(node),
        baseline,
        "status must not borrow application/repair/transition slots"
    );
}

pub(super) struct Fixture {
    pub(super) lan: RunnerLan,
    pub(super) a: Arc<ShareIrohNode>,
    pub(super) b: Arc<ShareIrohNode>,
    pub(super) baseline: [(usize, usize, usize); 2],
    pub(super) rounds: Option<JoinHandle<io::Result<()>>>,
}

impl Fixture {
    pub(super) fn start(seed: u8) -> Self {
        let lan = RunnerLan::discover();
        let a_identity = fixture_identity(seed);
        let b_identity = fixture_identity(seed + 1);
        let mut auth_a = fixture_auth(a_identity.clone());
        auth_a.direct_contacts.push(accepted_contact(&b_identity));
        let mut auth_b = fixture_auth(b_identity.clone());
        auth_b.direct_grants.push(accepted_grant(&a_identity));
        let (tx_b, _rx_b) = crossbeam_channel::unbounded();
        let b = ShareIrohNode::start(
            "relay-disabled://s09-transport",
            &b_identity,
            Arc::new(Mutex::new(auth_b)),
            tx_b,
        )
        .expect("server node");
        let (tx_a, _rx_a) = crossbeam_channel::unbounded();
        let a = match ShareIrohNode::start(
            "relay-disabled://s09-transport",
            &a_identity,
            Arc::new(Mutex::new(auth_a)),
            tx_a,
        ) {
            Ok(node) => node,
            Err(error) => {
                let _ = b.stop_sharing();
                panic!("client node: {error}");
            }
        };
        let baseline = [permits(&a), permits(&b)];
        let fixture = Self {
            lan,
            a,
            b,
            baseline,
            rounds: None,
        };
        fixture.refresh_host();
        fixture
    }

    fn refresh_host(&self) {
        self.a
            .update_lan_link_host(self.lan.host(OwnUplink::Present))
            .unwrap();
        self.b
            .update_lan_link_host(self.lan.host(OwnUplink::Absent))
            .unwrap();
    }

    pub(super) fn expected_pin(&self) -> LanPeerPin {
        lan_link_facts::contact_pin(&self.a.auth.lock().unwrap().direct_contacts[0]).unwrap()
    }

    pub(super) async fn connect(&self) -> Connection {
        self.refresh_host();
        let port = self.b.bound_ports().0.expect("real IPv4 UDP socket");
        let remote = SocketAddr::new(self.lan.ip, port);
        let target = EndpointAddr::from_parts(self.b.endpoint.id(), [TransportAddr::Ip(remote)]);
        let connection = tokio::time::timeout(
            ROUND_DEADLINE,
            self.a.endpoint.connect(target, LAN_LINK_ALPN),
        )
        .await
        .expect("bounded private TLS dial")
        .expect("private TLS connection");
        assert_eq!(connection.remote_id(), self.b.endpoint.id());
        assert_eq!(connection.alpn(), LAN_LINK_ALPN);
        wait_until("actual selected private path", || {
            selected_ip_path(&connection).is_some()
        })
        .await;
        let path = selected_ip_path(&connection).unwrap();
        assert_eq!(
            path.local, self.lan.ip,
            "cannot accept a loopback/local-IP guess"
        );
        assert_eq!(
            path.remote, remote,
            "cannot accept a relay or another candidate"
        );
        self.refresh_host();
        connection
    }

    pub(super) fn start_rounds(&mut self, connection: &Connection, expected: LanPeerPin) {
        assert!(self.rounds.is_none());
        let transport = self.a.lan_links.clone();
        let registered = transport
            .state
            .lock()
            .unwrap()
            .channels
            .contains_key(&connection.stable_id());
        if !registered {
            transport.register(connection).unwrap();
        }
        let node = self.a.clone();
        let connection = connection.clone();
        self.rounds = Some(self.a.rt.spawn(super::super::super::lan_link_exchange::run(
            node,
            transport,
            connection,
            Some(expected),
        )));
    }

    pub(super) async fn wait_status(&self) -> (AuthenticatedLanFact, AuthenticatedLanFact) {
        let mut status = None;
        wait_until("mutually confirmed status", || {
            let Some(a) = self.a.lan_link_snapshot().into_iter().next() else {
                return false;
            };
            let Some(b) = self.b.lan_link_snapshot().into_iter().next() else {
                return false;
            };
            status = Some((a, b));
            true
        })
        .await;
        status.unwrap()
    }

    pub(super) async fn freeze_rounds(&mut self) {
        let rounds = self.rounds.take().unwrap();
        rounds.abort();
        assert!(rounds.await.unwrap_err().is_cancelled());
    }

    pub(super) async fn positive(&mut self) -> (Connection, AuthenticatedLanFact) {
        let connection = self.connect().await;
        self.start_rounds(&connection, self.expected_pin());
        let (fact, _) = self.wait_status().await;
        self.freeze_rounds().await;
        (connection, fact)
    }

    pub(super) fn server_connection(&self) -> Connection {
        self.b
            .lan_links
            .state
            .lock()
            .unwrap()
            .channels
            .values()
            .next()
            .expect("accepted TLS channel")
            .connection
            .clone()
    }

    fn shutdown(&mut self) {
        if let Some(rounds) = self.rounds.take() {
            rounds.abort();
        }
        let _ = self.a.stop_sharing();
        let _ = self.b.stop_sharing();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.shutdown();
    }
}
