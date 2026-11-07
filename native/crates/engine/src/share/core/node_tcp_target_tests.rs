//! G6 (review r-exec-2, batch 2026-11-07): the host decides whether a rule of
//! the proven relation, or its right to steer the screen, names the
//! requested destination before it resolves a name or lists a service.
use crate::share::access_constraints::{
    AccessConstraints, NodeId, TcpDirection, TcpForwardRule, TcpHost, TcpPort,
};
use crate::share::access_grant::{AccessGrantChange, Capability, CapabilityDecision, Expiry};
use crate::share::core::{now_secs, public_fingerprint};
use crate::share::relation_proof::ProvenPrincipal;
use crate::share::service::ShareService;
use crate::share::service_tests::test_service;
use crate::share::tcp_forward::TcpDestination;
use crate::share::types::{DirectGrant, DirectGrantState};
use crate::share::ExecGrant;
use std::net::{IpAddr, Ipv4Addr};

const NAS: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5));

/// A host and one proven peer whose grant each case replaces.
struct Rig {
    service: ShareService,
    peer: ProvenPrincipal,
    host_node: String,
}

impl Rig {
    fn new() -> Self {
        let service = test_service();
        let (relation_id, host_node) = {
            let state = service.auth.lock().unwrap();
            (
                state.identity.direct_lookup_id.clone(),
                state.identity.node_id.clone(),
            )
        };
        let node_id = iroh::SecretKey::from_bytes(&[41u8; 32])
            .public()
            .to_string();
        let peer = ProvenPrincipal {
            relation_kind: "direct".into(),
            relation_id,
            device_id: "g6-peer".into(),
            device_name: "G6 peer".into(),
            public_key: node_id.clone(),
            fingerprint: public_fingerprint(node_id.as_bytes()),
            node_id,
        };
        Self {
            service,
            peer,
            host_node,
        }
    }

    fn rule(&self, host: TcpHost, port: u16, direction: TcpDirection) -> TcpForwardRule {
        TcpForwardRule {
            initiator: NodeId::parse(&self.peer.node_id).unwrap(),
            target: NodeId::parse(&self.host_node).unwrap(),
            direction,
            host,
            ports: vec![TcpPort::try_from(port).unwrap()],
            valid_until: None,
        }
    }

    fn dns(&self, name: &str, port: u16, direction: TcpDirection) -> TcpForwardRule {
        let host = TcpHost::Dns {
            name: name.into(),
            addresses: vec![NAS],
        };
        self.rule(host, port, direction)
    }

    /// A grant with these TCP rules and the capability decision, no more.
    fn tcp(rules: Vec<TcpForwardRule>, decision: CapabilityDecision) -> AccessGrantChange {
        AccessGrantChange {
            capabilities: [(Capability::TcpForward, decision)].into(),
            constraints: Some(AccessConstraints {
                tcp_forward: rules,
                ..AccessConstraints::default()
            }),
            ..AccessGrantChange::default()
        }
    }

    fn screen(capability: Capability) -> AccessGrantChange {
        AccessGrantChange {
            capabilities: [(capability, CapabilityDecision::Allow)].into(),
            ..AccessGrantChange::default()
        }
    }

    /// The peer's only grant on the host; `None` leaves it without one.
    fn grant(&self, change: Option<AccessGrantChange>) {
        let mut state = self.service.auth.lock().unwrap();
        state.direct_grants.clear();
        if let Some(change) = change {
            let mut exec = ExecGrant::default();
            exec.apply_access_change(&change, now_secs())
                .expect("valid grant");
            state.direct_grants.push(DirectGrant {
                device_id: self.peer.device_id.clone(),
                device_name: self.peer.device_name.clone(),
                public_key: self.peer.public_key.clone(),
                fingerprint: self.peer.fingerprint.clone(),
                node_id: self.peer.node_id.clone(),
                state: DirectGrantState::Accepted,
                updated_at: now_secs(),
                exec,
                write: false,
            });
        }
    }

    fn names(&self, destination: &TcpDestination) -> bool {
        self.service
            .iroh
            .names_tcp_destination(&self.peer, destination)
            .expect("the authorization state is readable")
    }
}

fn destination(host: TcpHost, port: u16) -> TcpDestination {
    TcpDestination {
        host,
        port: TcpPort::try_from(port).unwrap(),
    }
}

fn name(host: &str, port: u16) -> TcpDestination {
    destination(
        TcpHost::Dns {
            name: host.into(),
            addresses: Vec::new(),
        },
        port,
    )
}

fn ip(octets: [u8; 4], port: u16) -> TcpDestination {
    destination(
        TcpHost::Ip {
            address: IpAddr::V4(Ipv4Addr::from(octets)),
        },
        port,
    )
}

/// Cases of T-G6: `false` where no rule of the relation, and no right to
/// steer, names the destination (the host then resolves nothing and lists
/// nothing); `true` where one does, so no request that was admitted loses
/// its path.
#[test]
fn exec_task_g6_names_tcp_destination_decides_before_resolving() {
    use CapabilityDecision::{Allow, Ask, Off};
    use TcpDirection::{HostToPeer, PeerToHost};
    let rig = Rig::new();
    let allow = |rules| Some(Rig::tcp(rules, Allow));
    let ended = {
        let mut change = Rig::tcp(vec![rig.dns("nas.local", 80, PeerToHost)], Allow);
        change.expiry = Some(Expiry::At(now_secs() - 60));
        Some(change)
    };
    let loopback = || destination(TcpHost::Loopback, 3000);
    let cases: Vec<(&str, bool, Option<AccessGrantChange>, TcpDestination)> = vec![
        // Not named.
        (
            "rule for another name",
            false,
            allow(vec![rig.dns("nas.local", 80, PeerToHost)]),
            name("secret.internal", 80),
        ),
        (
            "rule for another port",
            false,
            allow(vec![rig.dns("nas.local", 22, PeerToHost)]),
            name("nas.local", 80),
        ),
        (
            "rule for the other direction",
            false,
            allow(vec![rig.dns("nas.local", 80, HostToPeer)]),
            name("nas.local", 80),
        ),
        (
            "rule for another IP",
            false,
            allow(vec![rig.rule(TcpHost::Ip { address: NAS }, 80, PeerToHost)]),
            ip([10, 0, 0, 6], 80),
        ),
        (
            "rule of a capability that is off",
            false,
            Some(Rig::tcp(vec![rig.dns("nas.local", 80, PeerToHost)], Off)),
            name("nas.local", 80),
        ),
        ("ended grant", false, ended, name("nas.local", 80)),
        ("no grant", false, None, name("nas.local", 80)),
        (
            "steering a name that is no loopback destination",
            false,
            Some(Rig::screen(Capability::ScreenControl)),
            name("nas.local", 80),
        ),
        (
            "loopback without a rule or the right to steer",
            false,
            Some(Rig::screen(Capability::ScreenView)),
            loopback(),
        ),
        // Named.
        (
            "same name in another case",
            true,
            allow(vec![rig.dns("NAS.local", 80, PeerToHost)]),
            name("nas.local", 80),
        ),
        (
            "addresses the requester names are no authority",
            true,
            allow(vec![rig.dns("nas.local", 80, PeerToHost)]),
            destination(
                TcpHost::Dns {
                    name: "nas.local".into(),
                    addresses: vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))],
                },
                80,
            ),
        ),
        (
            "rule for the same IP",
            true,
            allow(vec![rig.rule(TcpHost::Ip { address: NAS }, 80, PeerToHost)]),
            ip([10, 0, 0, 5], 80),
        ),
        (
            "rule for loopback",
            true,
            allow(vec![rig.rule(TcpHost::Loopback, 3000, PeerToHost)]),
            loopback(),
        ),
        (
            "rule whose use the host asks about",
            true,
            Some(Rig::tcp(vec![rig.dns("nas.local", 80, PeerToHost)], Ask)),
            name("nas.local", 80),
        ),
        (
            "right to steer, loopback",
            true,
            Some(Rig::screen(Capability::ScreenControl)),
            loopback(),
        ),
    ];
    for (label, expected, change, destination) in cases {
        rig.grant(change);
        assert_eq!(rig.names(&destination), expected, "{label}");
    }
}
