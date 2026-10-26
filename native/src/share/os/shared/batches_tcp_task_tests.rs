//! B7 real socket/lease fixtures, selected only by the common remote task suite.
use super::*;
use crate::share::access_constraints::{
    AccessConstraints, AccessRequest, NodeId, TcpDirection, TcpForwardRequest, TcpForwardRule,
};
use crate::share::access_grant::{AccessGrantChange, Capability, CapabilityDecision};
use crate::share::access_revocation::{confirmation, AccessLeaseRegistry, AccessRevocation};
use crate::share::identity::ShareIdentity;
use crate::share::relation_proof::ProvenPrincipal;
use crate::share::route_policy::batches_route_task_tests::{pair, runtime};
use crate::share::types::{DirectGrant, DirectGrantState, ShareAuthState};
use std::sync::atomic::Ordering;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const WAIT: Duration = Duration::from_secs(10);

fn identity(name: &str, byte: u8) -> ShareIdentity {
    let secret = iroh::SecretKey::from_bytes(&[byte; 32]);
    let key = secret.public().to_string();
    ShareIdentity {
        device_id: name.into(),
        device_name: name.into(),
        direct_lookup_id: name.into(),
        fingerprint: crate::share::core::public_fingerprint(key.as_bytes()),
        public_key: key.clone(),
        node_id: key,
        iroh_secret: secret,
        direct_secret: [byte; 32],
    }
}

struct Authority {
    state: ShareAuthState,
    principal: ProvenPrincipal,
    request: AccessRequest,
    registry: Arc<AccessLeaseRegistry>,
}
impl Authority {
    fn new(destination: &TcpDestination, admitted: SocketAddr) -> Self {
        let host = identity("tcp-fixture-host", 41);
        let peer = identity("tcp-fixture-peer", 42);
        let principal = ProvenPrincipal {
            relation_kind: "direct".into(),
            relation_id: host.direct_lookup_id.clone(),
            device_id: peer.device_id.clone(),
            device_name: peer.device_name.clone(),
            public_key: peer.public_key.clone(),
            fingerprint: peer.fingerprint.clone(),
            node_id: peer.node_id.clone(),
        };
        let request = TcpForwardRequest {
            initiator: NodeId::parse(&peer.node_id).unwrap(),
            target: NodeId::parse(&host.node_id).unwrap(),
            direction: TcpDirection::PeerToHost,
            host: destination.host.clone(),
            resolved_ip: admitted.ip(),
            port: destination.port,
        };
        let mut policy = crate::share::exec_policy::ExecGrant::default();
        policy
            .apply_access_change(
                &AccessGrantChange {
                    capabilities: [(Capability::TcpForward, CapabilityDecision::Ask)]
                        .into_iter()
                        .collect(),
                    constraints: Some(AccessConstraints {
                        tcp_forward: vec![TcpForwardRule {
                            initiator: request.initiator.clone(),
                            target: request.target.clone(),
                            direction: request.direction,
                            host: request.host.clone(),
                            ports: vec![request.port],
                            valid_until: None,
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                100,
            )
            .unwrap();
        let state = ShareAuthState {
            identity: host,
            direct_secret: vec![41; 32],
            default_direct_exports: Default::default(),
            direct_contacts: Vec::new(),
            direct_grants: vec![DirectGrant {
                device_id: peer.device_id,
                device_name: peer.device_name,
                public_key: peer.public_key,
                fingerprint: peer.fingerprint,
                node_id: peer.node_id,
                state: DirectGrantState::Accepted,
                updated_at: 100,
                exec: policy,
                write: false,
            }],
            rooms: Vec::new(),
            direct_requests: Vec::new(),
            direct_request_tombstones: Vec::new(),
            seen_nonces: Default::default(),
            direct_online: true,
            authorization_epoch: 1,
        };
        Self {
            state,
            principal,
            request: AccessRequest::TcpForward(request),
            registry: Arc::new(AccessLeaseRegistry::default()),
        }
    }
    fn admit(&self) -> Arc<AccessHandle> {
        Arc::new(
            self.registry
                .admit(
                    &self.state,
                    &self.principal,
                    self.request.clone(),
                    Some(confirmation(
                        self.principal.clone(),
                        self.request.clone(),
                        self.state.direct_grants[0].exec.policy_revision,
                    )),
                    100,
                )
                .unwrap(),
        )
    }
}

async fn service() -> (TcpListener, TcpDestination, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let destination =
        TcpDestination::from_host_port(&address.ip().to_string(), address.port()).unwrap();
    (listener, destination, address)
}

#[test]
fn batches_task_b7_pinned_socket_greeting_and_fin_survive_reuse_and_quic_pump() {
    runtime().block_on(async {
        let (listener, destination, admitted) = service().await;
        let authority = Authority::new(&destination, admitted);
        let access = authority.admit();
        let mut initial = connect_bound(&destination, admitted, access.clone())
            .await
            .unwrap();
        let local = initial.stream().local_addr().unwrap();
        let (mut service, peer) = tokio::time::timeout(WAIT, listener.accept())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(peer, local);
        service.write_all(b"greeting-before-tunnel").await.unwrap();
        service.shutdown().await.unwrap();
        // FIN leaves the write direction valid. Reuse must neither probe-read
        // the greeting nor open a replacement connection to this one-shot service.
        let mut retained = connect_bound_reusing(&destination, admitted, access, Some(initial))
            .await
            .unwrap();
        assert_eq!(retained.stream().local_addr().unwrap(), local);
        assert_eq!(retained.stream().peer_addr().unwrap(), admitted);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
        let (left, right, client, target) = pair().await;
        crate::share::route_monitor::start(&client, left.id().to_string(), false, true, Vec::new())
            .unwrap();
        crate::share::route_monitor::start(
            &target,
            right.id().to_string(),
            false,
            true,
            Vec::new(),
        )
        .unwrap();
        let deadline = Some(std::time::Instant::now() + WAIT);
        crate::share::route_monitor::wait_application_route(&client, deadline)
            .await
            .unwrap();
        crate::share::route_monitor::wait_application_route(&target, deadline)
            .await
            .unwrap();
        let (mut outgoing, mut incoming) = client.open_bi().await.unwrap();
        outgoing
            .write_all(b"request-after-service-fin")
            .await
            .unwrap();
        outgoing.finish().unwrap();
        let (mut send, mut recv) = tokio::time::timeout(WAIT, target.accept_bi())
            .await
            .unwrap()
            .unwrap();
        let counters = Arc::new(crate::share::tcp_registry::TcpCounters::default());
        let (pump, greeting, request) = tokio::time::timeout(WAIT, async {
            tokio::join!(
                crate::share::route_protocol::scope(
                    target.clone(),
                    crate::share::tcp_pump::pump(
                        &target,
                        retained.stream(),
                        &mut send,
                        &mut recv,
                        counters.clone()
                    )
                ),
                incoming.read_to_end(1024),
                async {
                    let mut bytes = Vec::new();
                    service.read_to_end(&mut bytes).await.unwrap();
                    bytes
                }
            )
        })
        .await
        .expect("both half-closed directions drain");
        pump.unwrap();
        assert_eq!(greeting.unwrap(), b"greeting-before-tunnel");
        assert_eq!(request, b"request-after-service-fin");
        assert_eq!(counters.active.load(Ordering::Relaxed), 0);
        assert_eq!(
            counters.read.load(Ordering::Relaxed),
            b"greeting-before-tunnel".len() as u64
        );
        assert_eq!(
            counters.written.load(Ordering::Relaxed),
            b"request-after-service-fin".len() as u64
        );
        drop(retained);
        client.close(0u32.into(), b"fixture complete");
        left.close().await;
        right.close().await;
    });
}

#[test]
fn batches_task_b7_numeric_target_and_confirmation_cannot_move_to_another_ip() {
    runtime().block_on(async {
        let (listener, destination, admitted) = service().await;
        let approval_destination = TcpDestination {
            host: TcpHost::Loopback,
            ..destination.clone()
        };
        let authority = Authority::new(&approval_destination, admitted);
        assert!(matches!(
            authority.registry.admit(
                &authority.state,
                &authority.principal,
                authority.request.clone(),
                None,
                100
            ),
            Err(AccessRevocation::PermissionWithdrawn)
        ));
        let mut moved = authority.request.clone();
        let AccessRequest::TcpForward(request) = &mut moved else {
            unreachable!()
        };
        request.resolved_ip = "127.0.0.2".parse().unwrap();
        assert_eq!(
            crate::share::access_authorize::request_for(
                &authority.state,
                &authority.principal,
                &moved,
                100
            )
            .decision,
            CapabilityDecision::Ask,
            "both numeric IPs match the rule; only the first has this confirmation"
        );
        let approval = confirmation(
            authority.principal.clone(),
            authority.request.clone(),
            authority.state.direct_grants[0].exec.policy_revision,
        );
        assert!(matches!(
            authority.registry.admit(
                &authority.state,
                &authority.principal,
                moved,
                Some(approval),
                100
            ),
            Err(AccessRevocation::PermissionWithdrawn)
        ));
        assert_eq!(resolve(&destination).await.unwrap(), vec![admitted]);
        let wrong = SocketAddr::new("127.0.0.2".parse().unwrap(), admitted.port());
        let numeric_authority = Authority::new(&destination, admitted);
        let result = connect_bound(&destination, wrong, numeric_authority.admit()).await;
        assert_eq!(
            result.err().expect("different numeric IP denied").kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
}

#[test]
fn batches_task_b7_retained_socket_rejects_foreign_authority_and_revocation() {
    runtime().block_on(async {
        for revoke in [false, true] {
            let (listener, destination, admitted) = service().await;
            let mut authority = Authority::new(&destination, admitted);
            let access = authority.admit();
            let initial = connect_bound(&destination, admitted, access.clone())
                .await
                .unwrap();
            let (mut service, _) = tokio::time::timeout(WAIT, listener.accept())
                .await
                .unwrap()
                .unwrap();
            let replacement = if revoke {
                authority.state.direct_online = false;
                assert_eq!(authority.registry.recheck(&authority.state, 101), 1);
                assert_eq!(access.reason(), Some(AccessRevocation::RelationEnded));
                access
            } else {
                authority.admit()
            };
            let result =
                connect_bound_reusing(&destination, admitted, replacement, Some(initial)).await;
            assert_eq!(
                result.err().expect("changed authority denied").kind(),
                io::ErrorKind::PermissionDenied
            );
            let mut byte = [0];
            assert_eq!(
                tokio::time::timeout(WAIT, service.read(&mut byte))
                    .await
                    .unwrap()
                    .unwrap(),
                0,
                "rejected retained socket closes the actual peer connection"
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        }
    });
}

#[test]
fn batches_task_b7_listener_requires_acknowledgement_and_binds_only_loopback() {
    runtime().block_on(async {
        let (_, destination, _) = service().await;
        let mut request = TcpStart {
            destination,
            local_port: 0,
            acknowledge_local_access: false,
            lifetime_secs: 60,
        };
        assert!(listen(&request).await.is_err());
        request.acknowledge_local_access = true;
        let listener = listen(&request).await.unwrap();
        assert_eq!(
            listener.local_addr().unwrap().ip(),
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        );
        assert_ne!(listener.local_addr().unwrap().port(), 0);
    });
}
