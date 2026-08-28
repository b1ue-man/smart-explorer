use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use iroh::endpoint::{presets, Connection, VarInt};
use iroh::{Endpoint, RelayMode};
use tokio::sync::Semaphore;

use super::connection_events::{ConnectionErrorKind, ConnectionEventReporter};
use super::core::eio;
use super::direct_reciprocal_transport::SharedDirectRepairStore;
use super::direct_reciprocal_coordinator::DirectReciprocalCoordinator;
use super::endpoint_routes::{EndpointRoutes, PublishedEndpointRoutes};
use super::exec_protocol::EXEC_ALPN;
use super::exec_registry::{ExecCancelReason, ExecRegistry, ExecRegistryLimits};
use super::handshake_limits::PeerHandshakeLimiter;
use super::identity::ShareIdentity;
use super::io_deadline;
use super::keepalive::iroh_transport_config;
use super::session::endpoint_addr;
use super::types::{PeerEndpoint, ShareAuthState, ShareEvent};

pub(super) const ALPN: &[u8] = b"smart-explorer/share-fs/3";
const MAX_PENDING_APPLICATION_HANDSHAKES: usize = 64;
const MAX_PENDING_HANDSHAKES_PER_ENDPOINT: usize = 4;
const MAX_CONCURRENT_DIRECT_REPAIRS: usize = 4;
const RUNTIME_TRANSITION_PERMITS: u32 = 8;

pub(crate) struct ShareIrohNode {
    pub(super) rt: Arc<tokio::runtime::Runtime>,
    pub(super) endpoint: Endpoint,
    pub(super) auth: Arc<Mutex<ShareAuthState>>,
    pub(super) direct_repair_store: SharedDirectRepairStore,
    direct_repair_coordinator: Mutex<Weak<DirectReciprocalCoordinator>>,
    pub(super) ev: crossbeam_channel::Sender<ShareEvent>,
    pub(super) sessions: Mutex<HashMap<String, Connection>>,
    pub(super) session_connects: Mutex<HashMap<String, Weak<Mutex<()>>>>,
    pub(super) session_epoch: AtomicU64,
    pub(super) mount_leases: Arc<super::mount_lease::PeerMountLeases>,
    sharing_active: AtomicBool,
    incoming_sessions: Mutex<HashMap<u64, Connection>>,
    next_incoming_session: AtomicU64,
    connection_events: ConnectionEventReporter,
    exec_registry: Arc<ExecRegistry>,
    pub(super) handshake_slots: Arc<Semaphore>,
    pub(super) direct_repair_slots: Arc<Semaphore>,
    pub(super) runtime_transition_slot: Arc<Semaphore>,
    pub(super) peer_handshake_slots: PeerHandshakeLimiter,
    pub(super) routes: EndpointRoutes,
}

impl ShareIrohNode {
    pub(crate) fn start(
        server: &str,
        identity: &ShareIdentity,
        auth: Arc<Mutex<ShareAuthState>>,
        ev: crossbeam_channel::Sender<ShareEvent>,
    ) -> io::Result<Arc<Self>> {
        Self::start_with_repair_store(
            server,
            identity,
            auth,
            ev,
            super::direct_reciprocal_transport::shared_direct_repair_store(
                super::direct_reciprocal_store::UnavailableDirectRepairStore,
            ),
        )
    }

    pub(crate) fn start_with_repair_store(
        server: &str,
        identity: &ShareIdentity,
        auth: Arc<Mutex<ShareAuthState>>,
        ev: crossbeam_channel::Sender<ShareEvent>,
        direct_repair_store: SharedDirectRepairStore,
    ) -> io::Result<Arc<Self>> {
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("share-iroh")
                .build()
                .map_err(eio)?,
        );
        let transport_options = super::transport_options::load(server);
        let relay_configured = !transport_options.relay_urls.is_empty();
        let relay_mode = if relay_configured {
            RelayMode::custom(transport_options.relay_urls)
        } else {
            RelayMode::Disabled
        };
        let mut builder = Endpoint::builder(presets::Minimal)
            .secret_key(identity.iroh_secret.clone())
            .alpns(vec![ALPN.to_vec(), EXEC_ALPN.to_vec()])
            .relay_mode(relay_mode)
            .transport_config(iroh_transport_config());
        if transport_options.relay_only {
            builder = builder.clear_ip_transports();
        }
        let endpoint = rt.block_on(async { builder.bind().await.map_err(eio) })?;
        let routes = EndpointRoutes::start(&rt, &endpoint, relay_configured);
        let node = Arc::new(Self {
            rt,
            endpoint,
            auth,
            direct_repair_store,
            direct_repair_coordinator: Mutex::new(Weak::new()),
            ev,
            sessions: Mutex::new(HashMap::new()),
            session_connects: Mutex::new(HashMap::new()),
            session_epoch: AtomicU64::new(0),
            mount_leases: Arc::new(super::mount_lease::PeerMountLeases::default()),
            sharing_active: AtomicBool::new(true),
            incoming_sessions: Mutex::new(HashMap::new()),
            next_incoming_session: AtomicU64::new(0),
            connection_events: ConnectionEventReporter::default(),
            exec_registry: Arc::new(ExecRegistry::new(ExecRegistryLimits::default())),
            handshake_slots: Arc::new(Semaphore::new(MAX_PENDING_APPLICATION_HANDSHAKES)),
            direct_repair_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_DIRECT_REPAIRS)),
            runtime_transition_slot: Arc::new(Semaphore::new(
                RUNTIME_TRANSITION_PERMITS as usize,
            )),
            peer_handshake_slots: PeerHandshakeLimiter::new(
                MAX_PENDING_HANDSHAKES_PER_ENDPOINT,
                MAX_PENDING_APPLICATION_HANDSHAKES,
            ),
            routes,
        });
        node.spawn_accept_loop();
        Ok(node)
    }

    pub(crate) fn relay_url(&self) -> String {
        self.routes.published(&self.endpoint).relay_url
    }

    pub(crate) fn candidates(&self) -> Vec<String> {
        self.routes.published(&self.endpoint).candidates
    }

    pub(super) fn published_routes(&self) -> PublishedEndpointRoutes {
        self.routes.published(&self.endpoint)
    }

    pub(crate) fn incoming_direct_repair_in_flight(&self) -> bool {
        self.direct_repair_slots.available_permits() < MAX_CONCURRENT_DIRECT_REPAIRS
    }

    pub(super) fn begin_runtime_transition(&self) -> io::Result<tokio::sync::OwnedSemaphorePermit> {
        self.runtime_transition_slot
            .clone()
            .try_acquire_many_owned(RUNTIME_TRANSITION_PERMITS)
            .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "Direct repair is active"))
    }

    pub(super) fn route_revision(&self) -> u64 {
        self.routes.revision()
    }

    pub(super) fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.rt.block_on(future)
    }

    pub(crate) fn exec_registry(&self) -> &Arc<ExecRegistry> {
        &self.exec_registry
    }

    pub(super) fn install_direct_repair_coordinator(
        &self,
        coordinator: &Arc<DirectReciprocalCoordinator>,
    ) -> io::Result<()> {
        *self
            .direct_repair_coordinator
            .lock()
            .map_err(|_| eio("Direct repair coordinator is locked"))? = Arc::downgrade(coordinator);
        Ok(())
    }

    pub(super) fn direct_repair_coordinator(&self) -> Option<Arc<DirectReciprocalCoordinator>> {
        self.direct_repair_coordinator.lock().ok()?.upgrade()
    }

    pub(super) fn track_incoming(
        self: &Arc<Self>,
        connection: &Connection,
    ) -> io::Result<IncomingConnectionGuard> {
        let id = self
            .next_incoming_session
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| eio("Share-Session-ID ist erschoepft"))?;
        self.incoming_sessions
            .lock()
            .map_err(|_| eio("Eingehende Share-Sessions sind gesperrt"))?
            .insert(id, connection.clone());
        Ok(IncomingConnectionGuard {
            node: Arc::downgrade(self),
            id,
        })
    }

    pub(super) fn invalidate_sessions(&self) -> io::Result<usize> {
        self.session_epoch.fetch_add(1, Ordering::AcqRel);
        let lease_clear = self.mount_leases.clear();
        if let Ok(mut gates) = self.session_connects.lock() {
            gates.retain(|_, gate| gate.strong_count() > 0);
        }
        let mut connections: Vec<Connection> = self
            .sessions
            .lock()
            .map_err(|_| eio("Ausgehende Share-Sessions sind gesperrt"))?
            .drain()
            .map(|(_, connection)| connection)
            .collect();
        connections.extend(
            self.incoming_sessions
                .lock()
                .map_err(|_| eio("Eingehende Share-Sessions sind gesperrt"))?
                .drain()
                .map(|(_, connection)| connection),
        );
        let count = connections.len();
        for connection in connections {
            connection.close(
                VarInt::from_u32(0x5345),
                b"authorization or export policy changed",
            );
        }
        lease_clear?;
        Ok(count)
    }

    pub(super) fn filesystem_authorization_epoch(&self) -> u64 {
        self.session_epoch.load(Ordering::Acquire)
    }

    pub(super) fn require_sharing_active(&self) -> io::Result<()> {
        self.sharing_active
            .load(Ordering::Acquire)
            .then_some(())
            .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "Share ist gestoppt"))
    }

    pub(super) fn stop_sharing(&self) -> io::Result<()> {
        if !self.sharing_active.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        self.exec_registry
            .as_ref()
            .cancel_all(ExecCancelReason::WorkerStopping);
        let invalidation = self.invalidate_sessions().map(|_| ());
        self.block_on(self.endpoint.close());
        invalidation
    }

    pub(super) fn connect_exec(&self, endpoint: &PeerEndpoint) -> io::Result<Connection> {
        self.require_sharing_active()?;
        if let Some(expected) = endpoint.expected_node_id.as_deref() {
            if !expected.trim().is_empty() && expected != endpoint.presence.node_id {
                return Err(eio("Iroh NodeId passt nicht zur gepinnten Identitaet"));
            }
        }
        let local_addr = self.routes.current(&self.endpoint);
        let addr = endpoint_addr(&endpoint.presence, &local_addr)?;
        self.block_on(io_deadline::run("peer exec connection", async {
            self.endpoint
                .connect(addr, EXEC_ALPN)
                .await
                .map_err(io_deadline::disconnected)
        }))
    }

    pub(super) fn start_exec(
        self: &Arc<Self>,
        endpoint: PeerEndpoint,
        identity: ShareIdentity,
        start: super::exec_types::ExecStart,
    ) -> io::Result<super::exec_session::ShareExecSession> {
        let connection = self.connect_exec(&endpoint)?;
        let _runtime = self.rt.enter();
        let client = super::exec_client::spawn_connected(connection, endpoint, identity, start);
        Ok(super::exec_session::ShareExecSession::new(
            self.clone(),
            client,
        ))
    }

    pub(super) fn emit_connection_error(&self, kind: ConnectionErrorKind, message: String) {
        self.connection_events.report(kind, message, &self.ev);
    }
}

pub(super) struct IncomingConnectionGuard {
    node: Weak<ShareIrohNode>,
    id: u64,
}

impl Drop for IncomingConnectionGuard {
    fn drop(&mut self) {
        if let Some(node) = self.node.upgrade() {
            if let Ok(mut sessions) = node.incoming_sessions.lock() {
                sessions.remove(&self.id);
            };
        }
    }
}
