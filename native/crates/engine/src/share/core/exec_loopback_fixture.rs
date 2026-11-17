//! Loopback `ShareIrohNode` fixture shared by the `share-exec/3` pool tests
//! and measurement M8 (`verbindungen_messung_tunnel.rs`). Uses only local
//! loopback sockets (`relay-disabled://`) and the in-memory `TestExecRunner`.
use std::collections::BTreeMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::share::core::{now_secs, public_fingerprint, random_bytes, random_uuid_v4};
use crate::share::exec_client::{ExecClientEvent, ExecClientFailure};
use crate::share::exec_client_messages::disconnected;
use crate::share::exec_job::test_runner::TestExecRunner;
use crate::share::exec_policy::ExecGrant;
use crate::share::exec_types::{ExecAuthorization, ExecCommand, ExecId, ExecStart, ExecTerminal};
use crate::share::fs::ShareExportConfig;
use crate::share::identity::ShareIdentity;
use crate::share::node::ShareIrohNode;
use crate::share::types::{
    DirectGrant, DirectGrantState, PeerEndpoint, PeerPresence, ShareAuthState, ShareScope,
};

pub(in crate::share) const NO_RELAY: &str = "relay-disabled://exec-task";

pub(in crate::share) struct NodeGuard(pub(in crate::share) Arc<ShareIrohNode>);

impl Drop for NodeGuard {
    fn drop(&mut self) {
        let _ = self.0.stop_sharing();
    }
}

pub(in crate::share) struct ExecLoopbackHost {
    pub(in crate::share) identity: ShareIdentity,
    pub(in crate::share) auth: Arc<Mutex<ShareAuthState>>,
    pub(in crate::share) endpoint: PeerEndpoint,
    pub(in crate::share) node: NodeGuard,
}

impl ExecLoopbackHost {
    pub(in crate::share) fn start_for_client(
        client: &ShareIdentity,
        grant: ExecGrant,
    ) -> io::Result<Self> {
        let host = make_identity("exec-host")?;
        let mut state = auth_state(&host);
        state.direct_grants.push(DirectGrant {
            device_id: client.device_id.clone(),
            device_name: client.device_name.clone(),
            public_key: client.public_key.clone(),
            fingerprint: client.fingerprint.clone(),
            node_id: client.node_id.clone(),
            state: DirectGrantState::Accepted,
            updated_at: now_secs(),
            exec: grant,
            write: false,
        });
        let auth = Arc::new(Mutex::new(state));
        let (ev, _rx) = crossbeam_channel::bounded(64);
        let node = NodeGuard(ShareIrohNode::start(NO_RELAY, &host, auth.clone(), ev)?);
        node.0
            .exec_host
            .set_test_runner(Some(TestExecRunner::immediate_ok()));
        let candidates = loopback_candidates(&node.0)?;
        let presence = PeerPresence {
            kind: "direct".into(),
            relation_id: host.direct_lookup_id.clone(),
            device_id: host.device_id.clone(),
            device_name: host.device_name.clone(),
            public_key: host.public_key.clone(),
            fingerprint: host.fingerprint.clone(),
            node_id: host.node_id.clone(),
            relay_url: String::new(),
            candidates,
            expires_at: now_secs() + 15 * 60,
            nonce: random_uuid_v4().map_err(io::Error::other)?,
            proof: String::new(),
        };
        let endpoint = PeerEndpoint {
            label: "exec loopback".into(),
            scope: ShareScope::Direct {
                contact_id: "exec-host".into(),
            },
            presence,
            relation_secret: host.direct_secret(),
            expected_node_id: Some(host.node_id.clone()),
        };
        Ok(Self {
            identity: host,
            auth,
            endpoint,
            node,
        })
    }
}

pub(in crate::share) fn enabled_grant() -> ExecGrant {
    ExecGrant {
        enabled: true,
        policy_revision: 1,
        ..ExecGrant::default()
    }
}

pub(in crate::share) fn make_identity(name: &str) -> io::Result<ShareIdentity> {
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

pub(in crate::share) fn start_client_node(client: &ShareIdentity) -> io::Result<NodeGuard> {
    let auth = Arc::new(Mutex::new(auth_state(client)));
    let (ev, _rx) = crossbeam_channel::bounded(64);
    Ok(NodeGuard(ShareIrohNode::start(NO_RELAY, client, auth, ev)?))
}

pub(in crate::share) fn argv_start(program: &str) -> io::Result<ExecStart> {
    Ok(ExecStart {
        exec_id: ExecId::generate()?,
        command: ExecCommand::Argv {
            program: program.into(),
            args: Vec::new(),
        },
        cwd: None,
        env: BTreeMap::new(),
        timeout_ms: Some(15_000),
        max_output_bytes: None,
        terminal: None,
    })
}

pub(in crate::share) fn run_to_terminal(
    client: &Arc<ShareIrohNode>,
    endpoint: &PeerEndpoint,
    identity: &ShareIdentity,
    start: ExecStart,
) -> Result<(Option<ExecAuthorization>, ExecTerminal), ExecClientFailure> {
    let mut session = client
        .start_exec(endpoint.clone(), identity.clone(), start)
        .map_err(|err| disconnected(err, false))?;
    let mut authorized = None;
    let terminal = loop {
        match session
            .next_event()
            .map_err(|err| disconnected(err, true))?
        {
            ExecClientEvent::Authorized { authorization, .. } => {
                authorized = Some(authorization);
            }
            ExecClientEvent::Terminal(term) => break term,
            ExecClientEvent::Failed(failure) => return Err(failure),
            ExecClientEvent::Started | ExecClientEvent::Stdout(_) | ExecClientEvent::Stderr(_) => {}
        }
    };
    session.finish().map_err(|err| disconnected(err, true))?;
    Ok((authorized, terminal))
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
        authorization_epoch: 1,
    }
}

fn loopback_candidates(node: &ShareIrohNode) -> io::Result<Vec<String>> {
    let deadline = Instant::now() + Duration::from_secs(20);
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
                "exec loopback fixture has no bound candidate",
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
