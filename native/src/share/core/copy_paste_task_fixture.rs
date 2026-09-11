//! Opt-in, authenticated loopback peers. Never loads identities or saved exports.
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::backend::{PeerBackend, ShareIrohNode};
use super::core::{now_secs, public_fingerprint, random_bytes, random_uuid_v4};
use super::fs::{ShareExportConfig, SharedRoot};
use super::identity::ShareIdentity;
use super::types::{
    DirectAccessState, DirectContact, DirectGrant, DirectGrantState, PeerEndpoint,
    PeerPresence, ShareAuthState, ShareScope, ShareStatus,
};
use crate::vfs::BackendHandle;

const NO_RELAY: &str = "relay-disabled://copy-paste-task";

pub(crate) struct CopyPastePeerFixture {
    pub(crate) backend: BackendHandle,
    pub(crate) root_a: PathBuf,
    pub(crate) root_b: PathBuf,
    host_auth: Arc<Mutex<ShareAuthState>>,
    // Fields drop in declaration order: close both endpoints before removing
    // their exported roots. Backend clones held by a caller then fail closed.
    _nodes: [NodeGuard; 2],
    _temporary: tempfile::TempDir,
}

struct NodeGuard(Arc<ShareIrohNode>);

impl Drop for NodeGuard {
    fn drop(&mut self) {
        let _ = self.0.stop_sharing();
    }
}

impl CopyPastePeerFixture {
    pub(crate) fn enabled() -> bool {
        std::env::var("SMART_EXPLORER_COPY_PASTE_TASK").as_deref() == Ok("1")
    }

    pub(crate) fn new() -> io::Result<Self> {
        if !Self::enabled() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied, "copy/paste task fixture is not enabled",
            ));
        }
        let options = super::transport_options::load(NO_RELAY);
        if options.relay_only || !options.relay_urls.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "copy/paste fixture requires SE_SHARE_RELAY_URL unset and SE_SHARE_RELAY_ONLY disabled",
            ));
        }
        let temporary = tempfile::tempdir()?;
        let root_a = temporary.path().join("A");
        let root_b = temporary.path().join("B");
        std::fs::create_dir(&root_a)?;
        std::fs::create_dir(&root_b)?;
        let client = identity("copy/paste client")?;
        let host = identity("copy/paste host")?;
        let mut host_state = auth_state(&host);
        host_state.default_direct_exports = ShareExportConfig {
            roots: vec![export("A", &root_a)?, export("B", &root_b)?],
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
            exec: super::ExecGrant::default(),
        });
        let host_auth = Arc::new(Mutex::new(host_state));
        let (host_events, _host_events_rx) = crossbeam_channel::bounded(64);
        let host_node = NodeGuard(ShareIrohNode::start(
            NO_RELAY, &host, host_auth.clone(), host_events,
        )?);
        let presence = PeerPresence {
            kind: "direct".into(),
            relation_id: host.direct_lookup_id.clone(),
            device_id: host.device_id.clone(),
            device_name: host.device_name.clone(),
            public_key: host.public_key.clone(),
            fingerprint: host.fingerprint.clone(),
            node_id: host.node_id.clone(),
            relay_url: String::new(),
            candidates: loopback_candidates(&host_node.0)?,
            expires_at: now_secs() + 15 * 60,
            nonce: random_uuid_v4().map_err(io::Error::other)?,
            // No discovery publisher is involved; the filesystem connection
            // still performs its real TLS-bound Direct HMAC/grant handshake.
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
        });
        let client_auth = Arc::new(Mutex::new(client_state));
        let (client_events, _client_events_rx) = crossbeam_channel::bounded(64);
        let client_node = NodeGuard(ShareIrohNode::start(
            NO_RELAY, &client, client_auth, client_events,
        )?);
        let endpoint = PeerEndpoint {
            label: "copy/paste loopback".into(),
            scope: ShareScope::Direct { contact_id },
            presence,
            relation_secret: host.direct_secret(),
            expected_node_id: Some(host.node_id.clone()),
        };
        let backend: BackendHandle = Arc::new(PeerBackend::new(
            endpoint, client, client_node.0.clone(),
        ));
        // This is deliberately unleased: two exported roots must remain
        // independently authorized destinations in the same peer namespace.
        let entries = backend.list_dir("/")?;
        if entries.len() != 2 || !["A", "B"].iter().all(|name| {
            entries.iter().any(|entry| entry.name == *name && entry.is_dir)
        }) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected fixture exports"));
        }
        Ok(Self {
            backend, root_a, root_b, host_auth,
            _nodes: [client_node, host_node], _temporary: temporary,
        })
    }

    pub(super) fn revoke_access(&self) -> io::Result<()> {
        self.host_auth.lock()
            .map_err(|_| io::Error::other("fixture authorization is poisoned"))?
            .direct_grants.clear();
        Ok(())
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
        identity: identity.clone(), direct_secret: identity.direct_secret(),
        default_direct_exports: ShareExportConfig::default(),
        direct_contacts: Vec::new(), direct_grants: Vec::new(), rooms: Vec::new(),
        direct_requests: Vec::new(), direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(), direct_online: true, authorization_epoch: 0,
    }
}

fn export(label: &str, path: &Path) -> io::Result<SharedRoot> {
    let path = path.to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "fixture root is not Unicode"))?;
    Ok(SharedRoot { label: label.into(), path: path.replace('\\', "/") })
}

fn loopback_candidates(node: &ShareIrohNode) -> io::Result<Vec<String>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if !node.relay_url().is_empty() {
            return Err(io::Error::other("fixture unexpectedly published a relay route"));
        }
        let mut candidates = node.candidates().into_iter()
            .filter_map(|address| address.parse::<SocketAddr>().ok())
            .filter(|address| address.port() != 0)
            .map(|address| {
                let ip = if address.is_ipv4() { IpAddr::V4(Ipv4Addr::LOCALHOST) }
                    else { IpAddr::V6(Ipv6Addr::LOCALHOST) };
                SocketAddr::new(ip, address.port()).to_string()
            }).collect::<Vec<_>>();
        candidates.sort();
        candidates.dedup();
        if !candidates.is_empty() { return Ok(candidates); }
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "fixture has no bound IP candidate"));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
