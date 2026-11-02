//! Two real Iroh nodes on loopback for the screen task tests (SCR-M2): a
//! host that grants the viewer the screen rights of a test, and a viewer
//! that connects with the screen ALPN. Never loads identities or saved
//! profiles. The host asks through its own approval service, whose
//! requests the tests answer; it logs into `capture` when a test reads the
//! access log. `with_server` starts both nodes for another share server (the
//! relay-only test).
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::screen::core::host_backend::ConsentBroker;
use crate::screen::core::viewer_client::{handshake, ViewerError, ViewerRequest, ViewerSession};
use crate::share::access_approval::{ApprovalService, ApprovalView, ConsentNotifier};
use crate::share::access_grant::{AccessGrantChange, Capability, CapabilityDecision, Expiry};
use crate::share::core::{now_secs, public_fingerprint, random_bytes, random_uuid_v4};
use crate::share::exec_access_view::ApprovalSurface;
use crate::share::fs::{ShareExportConfig, SharedRoot};
use crate::share::identity::ShareIdentity;
use crate::share::node::ShareIrohNode;
use crate::share::node_screen::ScreenLink;
use crate::share::signal_connector::{spawn_connect, NegotiatedSignal};
use crate::share::types::{
    DirectGrant, DirectGrantState, PeerEndpoint, PeerPresence, ShareAuthState, ShareScope,
};
use crate::share::ExecGrant;

pub(super) const NO_RELAY: &str = "relay-disabled://screen-task";

/// Every host request is shown in a (pretend) window.
struct Shown;

impl ConsentNotifier for Shown {
    fn show(&self, _request: &ApprovalView) -> Option<ApprovalSurface> {
        Some(ApprovalSurface::Window)
    }
}

pub(super) struct NodeGuard(pub(super) Arc<ShareIrohNode>);

impl Drop for NodeGuard {
    fn drop(&mut self) {
        let _ = self.0.stop_sharing();
    }
}

/// The screen rights the host grants the viewer.
pub(super) fn screen_grant(
    view: CapabilityDecision,
    control: CapabilityDecision,
    valid_until: Option<i64>,
) -> ExecGrant {
    let mut grant = ExecGrant::default();
    let change = AccessGrantChange {
        capabilities: BTreeMap::from([
            (Capability::ScreenView, view),
            (Capability::ScreenControl, control),
        ]),
        expiry: valid_until.map(Expiry::At),
        ..AccessGrantChange::default()
    };
    grant
        .apply_access_change(&change, now_secs())
        .expect("valid screen grant");
    grant
}

pub(super) struct ScreenPair {
    pub(super) host_auth: Arc<Mutex<ShareAuthState>>,
    pub(super) endpoint: PeerEndpoint,
    pub(super) viewer_identity: ShareIdentity,
    pub(super) approvals: ApprovalService,
    // Fields drop in order: both nodes stop before the exported root goes.
    pub(super) viewer: NodeGuard,
    pub(super) host: NodeGuard,
    /// The signaling sessions that admit both nodes at the server's relay.
    _signals: Vec<NegotiatedSignal>,
    _root: tempfile::TempDir,
}

impl ScreenPair {
    pub(super) fn new(view: CapabilityDecision, control: CapabilityDecision) -> Self {
        Self::with_server(NO_RELAY, view, control, None).expect("screen pair on loopback")
    }

    /// Both nodes for `server`; without a relay the viewer reaches the host
    /// over its loopback addresses, with one over the host's home relay.
    pub(super) fn with_server(
        server: &str,
        view: CapabilityDecision,
        control: CapabilityDecision,
        valid_until: Option<i64>,
    ) -> std::io::Result<Self> {
        let root = tempfile::tempdir()?;
        let docs = root.path().join("Docs");
        std::fs::create_dir(&docs)?;
        std::fs::write(docs.join("note.txt"), b"screen task")?;
        let viewer = identity("screen viewer")?;
        let host = identity("screen host")?;
        // The share server's relay admits only endpoints that are signed in
        // at its signaling (`relay_access.rs`); a bare Iroh node never is,
        // so both sign in before their nodes dial the relay.
        let mut signals = Vec::new();
        if server != NO_RELAY {
            for identity in [&host, &viewer] {
                signals.push(sign_in(server, identity)?);
            }
        }
        let mut host_state = auth_state(&host);
        let path = docs.to_string_lossy().replace('\\', "/");
        host_state.default_direct_exports = ShareExportConfig {
            roots: vec![SharedRoot::new("Docs", path)],
            ..Default::default()
        };
        host_state.direct_grants.push(DirectGrant {
            device_id: viewer.device_id.clone(),
            device_name: viewer.device_name.clone(),
            public_key: viewer.public_key.clone(),
            fingerprint: viewer.fingerprint.clone(),
            node_id: viewer.node_id.clone(),
            state: DirectGrantState::Accepted,
            updated_at: now_secs(),
            exec: screen_grant(view, control, valid_until),
            write: false,
        });
        let host_auth = Arc::new(Mutex::new(host_state));
        let (host_events, _host_rx) = crossbeam_channel::bounded(64);
        let host_node = NodeGuard(ShareIrohNode::start(
            server,
            &host,
            host_auth.clone(),
            host_events,
        )?);
        let approvals = ApprovalService::new(Arc::new(Shown));
        let broker: Arc<dyn ConsentBroker> = Arc::new(approvals.broker());
        *host_node.0.screen.broker.write().unwrap() = broker;
        let (relay_url, candidates) = routes(&host_node.0, server == NO_RELAY)?;
        let presence = PeerPresence {
            kind: "direct".into(),
            relation_id: host.direct_lookup_id.clone(),
            device_id: host.device_id.clone(),
            device_name: host.device_name.clone(),
            public_key: host.public_key.clone(),
            fingerprint: host.fingerprint.clone(),
            node_id: host.node_id.clone(),
            relay_url,
            candidates,
            expires_at: now_secs() + 15 * 60,
            nonce: random_uuid_v4().map_err(std::io::Error::other)?,
            proof: String::new(),
        };
        let (viewer_events, _viewer_rx) = crossbeam_channel::bounded(64);
        let viewer_node = NodeGuard(ShareIrohNode::start(
            server,
            &viewer,
            Arc::new(Mutex::new(auth_state(&viewer))),
            viewer_events,
        )?);
        let endpoint = PeerEndpoint {
            label: "screen loopback".into(),
            scope: ShareScope::Direct {
                contact_id: "screen-host".into(),
            },
            presence,
            relation_secret: host.direct_secret(),
            expected_node_id: Some(host.node_id.clone()),
        };
        Ok(Self {
            host_auth,
            endpoint,
            viewer_identity: viewer,
            approvals,
            viewer: viewer_node,
            host: host_node,
            _signals: signals,
            _root: root,
        })
    }

    /// Changes the viewer's grant on the host directly (no narrowing seen).
    pub(super) fn set_grant(&self, grant: ExecGrant) {
        let mut state = self.host_auth.lock().unwrap();
        state.direct_grants[0].exec = grant;
    }

    /// The device id of the viewer, as the host's access log names it.
    pub(super) fn viewer_id(&self) -> &str {
        &self.viewer_identity.device_id
    }

    pub(super) fn link(&self) -> Result<ScreenLink, ViewerError> {
        let connection = self
            .viewer
            .0
            .block_on(self.viewer.0.connect_screen(&self.endpoint))?;
        Ok(ScreenLink {
            connection,
            endpoint: self.endpoint.clone(),
            identity: self.viewer_identity.clone(),
        })
    }

    /// A session with `request`; `on_pending` hears `APPROVAL_PENDING`.
    pub(super) fn open(
        &self,
        request: &ViewerRequest,
        on_pending: impl FnMut(Capability),
    ) -> Result<ViewerSession, ViewerError> {
        let link = self.link()?;
        self.viewer.0.block_on(async {
            let build = |server: &_| link.client_hello(server, request);
            handshake(link.connection.clone(), build, on_pending).await
        })
    }

    /// Answers the host's next request with `decide` once it is open.
    pub(super) fn answer_next(
        &self,
        decide: impl FnOnce(&ApprovalService, &ApprovalView) + Send + 'static,
    ) -> std::thread::JoinHandle<bool> {
        let service = self.approvals.clone();
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(20);
            while Instant::now() < deadline {
                if let Some(request) = service.pending().into_iter().next() {
                    decide(&service, &request);
                    return true;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            false
        })
    }
}

/// A signaling session of `identity` with key login on `server`.
fn sign_in(server: &str, identity: &ShareIdentity) -> std::io::Result<NegotiatedSignal> {
    let connecting = spawn_connect(server.to_string(), identity.clone())?;
    connecting
        .recv_timeout(Duration::from_secs(20))
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "signaling sign-in"))?
}

fn identity(name: &str) -> std::io::Result<ShareIdentity> {
    let secret = iroh::SecretKey::from_bytes(&random_bytes::<32>().map_err(std::io::Error::other)?);
    let node_id = secret.public().to_string();
    Ok(ShareIdentity {
        device_id: random_uuid_v4().map_err(std::io::Error::other)?,
        device_name: name.to_string(),
        direct_lookup_id: random_uuid_v4().map_err(std::io::Error::other)?,
        public_key: node_id.clone(),
        fingerprint: public_fingerprint(node_id.as_bytes()),
        node_id,
        iroh_secret: secret,
        direct_secret: random_bytes::<32>().map_err(std::io::Error::other)?,
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

/// The host's relay (with a server) or its loopback addresses (without).
fn routes(node: &ShareIrohNode, loopback: bool) -> std::io::Result<(String, Vec<String>)> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if !loopback && !node.relay_url().is_empty() {
            return Ok((node.relay_url(), Vec::new()));
        }
        if loopback {
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
                return Ok((String::new(), candidates));
            }
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "screen fixture has no route to the host",
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
