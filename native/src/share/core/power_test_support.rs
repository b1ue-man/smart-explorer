//! Fixtures of the `android_background_task_` tests: a power hub with a
//! manual clock, identities and two real loopback Iroh nodes.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::clock::{ManualClock, PowerClock};
use super::PowerHub;
use crate::share::backend::{PeerBackend, ShareIrohNode};
use crate::share::core::{now_secs, public_fingerprint, random_bytes, random_uuid_v4};
use crate::share::fs::{ShareExportConfig, SharedRoot};
use crate::share::identity::ShareIdentity;
use crate::share::types::{
    DirectAccessState, DirectContact, DirectGrant, DirectGrantState, PeerEndpoint, PeerPresence,
    ShareAuthState, ShareEvent, ShareScope, ShareStatus,
};

/// A transport option string without relay (as in the transfer fixtures).
pub(crate) const NO_RELAY: &str = "relay-disabled://android-background-task";

pub(crate) fn manual_hub(wall_ms: i64) -> (Arc<ManualClock>, Arc<PowerHub>) {
    let clock = Arc::new(ManualClock::new(wall_ms));
    let dynamic: Arc<dyn PowerClock> = clock.clone();
    (clock, Arc::new(PowerHub::new(dynamic)))
}

pub(crate) fn identity(name: &str) -> io::Result<ShareIdentity> {
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

pub(crate) fn auth_state(identity: &ShareIdentity) -> ShareAuthState {
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

pub(crate) fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A client with an accepted Direct contact to a host that exports one
/// directory; the host judges idleness with `host_power`.
pub(crate) struct LoopbackPeers {
    pub(crate) host: Arc<ShareIrohNode>,
    pub(crate) client: Arc<ShareIrohNode>,
    pub(crate) peer: Arc<PeerBackend>,
    pub(crate) endpoint: PeerEndpoint,
    _events: [crossbeam_channel::Receiver<ShareEvent>; 2],
    _root: tempfile::TempDir,
}

impl LoopbackPeers {
    pub(crate) fn new(host_power: Arc<PowerHub>) -> io::Result<Self> {
        let root = tempfile::tempdir()?;
        let client = identity("background client")?;
        let host = identity("background host")?;
        let mut host_state = auth_state(&host);
        host_state.default_direct_exports = ShareExportConfig {
            roots: vec![SharedRoot {
                label: "A".into(),
                path: root
                    .path()
                    .to_str()
                    .ok_or_else(|| io::Error::other("temporary root is not Unicode"))?
                    .replace('\\', "/"),
            }],
            include_connections: false,
        };
        host_state.direct_grants.push(DirectGrant {
            device_id: client.device_id.clone(),
            device_name: client.device_name.clone(),
            public_key: client.public_key.clone(),
            fingerprint: client.fingerprint.clone(),
            node_id: client.node_id.clone(),
            state: DirectGrantState::Accepted,
            updated_at: now_secs(),
            exec: crate::share::ExecGrant::default(),
        });
        let (host_events, host_events_rx) = crossbeam_channel::bounded(256);
        let host_node = ShareIrohNode::start_with_power_for_test(
            NO_RELAY,
            &host,
            Arc::new(Mutex::new(host_state)),
            host_events,
            host_power,
        )?;
        let presence = PeerPresence {
            kind: "direct".into(),
            relation_id: host.direct_lookup_id.clone(),
            device_id: host.device_id.clone(),
            device_name: host.device_name.clone(),
            public_key: host.public_key.clone(),
            fingerprint: host.fingerprint.clone(),
            node_id: host.node_id.clone(),
            relay_url: String::new(),
            candidates: loopback_candidates(&host_node)?,
            expires_at: now_secs() + 15 * 60,
            nonce: random_uuid_v4().map_err(io::Error::other)?,
            proof: String::new(),
        };
        let contact_id = random_uuid_v4().map_err(io::Error::other)?;
        let mut client_state = auth_state(&client);
        client_state.direct_contacts.push(DirectContact {
            id: contact_id.clone(),
            display_name: host.device_name.clone(),
            lookup_id: host.direct_lookup_id.clone(),
            expected_fingerprint: host.fingerprint.clone(),
            expected_node_id: host.node_id.clone(),
            remote_device_id: Some(host.device_id.clone()),
            remote_public_key: Some(host.public_key.clone()),
            auto_connect: false,
            auto_open: false,
            last_seen: None,
            status: ShareStatus::Available,
            last_error: None,
            presence: Some(presence.clone()),
            access_state: DirectAccessState::Accepted,
            request_sent_at: None,
            accepted_at: Some(now_secs()),
            accepted_public_key: Some(host.public_key.clone()),
            lan_candidates: Vec::new(),
            lan_seen_at: None,
            lan_uplink: None,
        });
        let (client_events, client_events_rx) = crossbeam_channel::bounded(256);
        let client_node = ShareIrohNode::start(
            NO_RELAY,
            &client,
            Arc::new(Mutex::new(client_state)),
            client_events,
        )?;
        let endpoint = PeerEndpoint {
            label: "background loopback".into(),
            scope: ShareScope::Direct { contact_id },
            presence,
            relation_secret: host.direct_secret(),
            expected_node_id: Some(host.node_id.clone()),
        };
        let peer = Arc::new(PeerBackend::new(
            endpoint.clone(),
            client,
            client_node.clone(),
        ));
        Ok(Self {
            host: host_node,
            client: client_node,
            peer,
            endpoint,
            _events: [host_events_rx, client_events_rx],
            _root: root,
        })
    }
}

impl Drop for LoopbackPeers {
    fn drop(&mut self) {
        let _ = self.client.stop_sharing();
        let _ = self.host.stop_sharing();
    }
}

fn loopback_candidates(node: &ShareIrohNode) -> io::Result<Vec<String>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut candidates = node
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
            .collect::<Vec<_>>();
        candidates.sort();
        candidates.dedup();
        if !candidates.is_empty() {
            return Ok(candidates);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "loopback node has no bound IP candidate",
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
