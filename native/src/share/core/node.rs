use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use iroh::endpoint::{presets, Connection, VarInt};
use iroh::{Endpoint, RelayMode};
use tokio::sync::Semaphore;

use super::connection_events::{ConnectionErrorKind, ConnectionEventReporter};
use super::core::eio;
use super::direct_reciprocal_coordinator::DirectReciprocalCoordinator;
use super::direct_reciprocal_transport::SharedDirectRepairStore;
use super::endpoint_routes::{EndpointRoutes, NodeTransportOptions, PublishedEndpointRoutes};
use super::exec_protocol::EXEC_ALPN;
use super::exec_registry::{ExecCancelReason, ExecRegistry, ExecRegistryLimits};
use super::handshake_limits::HandshakeAdmission;
use super::identity::ShareIdentity;
use super::io_deadline;
use super::keepalive::iroh_transport_config;
use super::power::PowerHub;
use super::session::endpoint_addr;
use super::types::{PeerEndpoint, ShareAuthState, ShareEvent};
use super::wire::FsTransferCapabilities;

#[path = "node_restrictions.rs"]
mod restrictions;

#[path = "node_idle.rs"]
mod idle;
#[path = "node_wake.rs"]
mod wake;

pub(crate) use self::idle::{closed_idle, IncomingActivity, IDLE_CLOSE_CODE, IDLE_CLOSE_REASON};

pub(super) const ALPN: &[u8] = b"smart-explorer/share-fs/3";
const MAX_PENDING_APPLICATION_HANDSHAKES: usize = 64;
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
    pub(super) session_connects: Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
    pub(super) session_epoch: AtomicU64,
    pub(super) policy: super::node_policy::SessionPolicy,
    pub(super) mount_leases: Arc<super::mount_lease::PeerMountLeases>,
    pub(super) lan_links: Arc<super::lan_link_transport::LanLinkTransport>,
    sharing_active: AtomicBool,
    incoming_sessions: Mutex<HashMap<u64, idle::IncomingEntry>>,
    next_incoming_session: AtomicU64,
    connection_events: ConnectionEventReporter,
    exec_registry: Arc<ExecRegistry>,
    pub(super) handshake_slots: Arc<Semaphore>,
    pub(super) direct_repair_slots: Arc<Semaphore>,
    pub(super) runtime_transition_slot: Arc<Semaphore>,
    pub(super) application_handshakes: HandshakeAdmission,
    pub(super) routes: EndpointRoutes,
    relay_configured: bool,
    pub(super) transport_options: NodeTransportOptions,
    power: Arc<PowerHub>,
    wake: wake::NodeWake,
    idle: idle::NodeIdle,
    /// Tests pose this host as one before transfer v1.
    #[cfg(test)]
    legacy_transfer_host: AtomicBool,
    /// Tests shorten the stall bound of transfers (milliseconds, 0 = as
    /// in production) to observe a stalled client losing its slot.
    #[cfg(test)]
    transfer_stall_millis: AtomicU64,
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
        let power = super::power::global().clone();
        Self::start_with_power(server, identity, auth, ev, direct_repair_store, power)
    }

    /// A node whose idle decisions read `power` instead of the process hub.
    #[cfg(test)]
    pub(crate) fn start_with_power_for_test(
        server: &str,
        identity: &ShareIdentity,
        auth: Arc<Mutex<ShareAuthState>>,
        ev: crossbeam_channel::Sender<ShareEvent>,
        power: Arc<PowerHub>,
    ) -> io::Result<Arc<Self>> {
        let store = super::direct_reciprocal_transport::shared_direct_repair_store(
            super::direct_reciprocal_store::UnavailableDirectRepairStore,
        );
        Self::start_with_power(server, identity, auth, ev, store, power)
    }

    fn start_with_power(
        server: &str,
        identity: &ShareIdentity,
        auth: Arc<Mutex<ShareAuthState>>,
        ev: crossbeam_channel::Sender<ShareEvent>,
        direct_repair_store: SharedDirectRepairStore,
        power: Arc<PowerHub>,
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
            RelayMode::custom(transport_options.relay_urls.clone())
        } else {
            RelayMode::Disabled
        };
        let mut builder = Endpoint::builder(presets::Minimal)
            .secret_key(identity.iroh_secret.clone())
            .alpns(vec![ALPN.to_vec(), EXEC_ALPN.to_vec(), super::lan_link_wire::LAN_LINK_ALPN.to_vec()])
            .relay_mode(relay_mode)
            .transport_config(iroh_transport_config());
        if let Some(config) = transport_options.ca_tls_config() {
            builder = builder.ca_tls_config(config);
        }
        if transport_options.relay_only {
            builder = builder.clear_ip_transports();
        }
        let endpoint = rt.block_on(async { builder.bind().await.map_err(eio) })?;
        let routes = EndpointRoutes::start(&rt, &endpoint, relay_configured);
        let wake = wake::NodeWake::new();
        wake.watch(&rt, &endpoint);
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
            policy: super::node_policy::SessionPolicy::default(),
            mount_leases: Arc::new(super::mount_lease::PeerMountLeases::default()),
            lan_links: Arc::new(super::lan_link_transport::LanLinkTransport::default()),
            sharing_active: AtomicBool::new(true),
            incoming_sessions: Mutex::new(HashMap::new()),
            next_incoming_session: AtomicU64::new(0),
            connection_events: ConnectionEventReporter::default(),
            exec_registry: Arc::new(ExecRegistry::new(ExecRegistryLimits::default())),
            handshake_slots: Arc::new(Semaphore::new(MAX_PENDING_APPLICATION_HANDSHAKES)),
            direct_repair_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_DIRECT_REPAIRS)),
            runtime_transition_slot: Arc::new(Semaphore::new(RUNTIME_TRANSITION_PERMITS as usize)),
            application_handshakes: HandshakeAdmission::new(MAX_PENDING_APPLICATION_HANDSHAKES),
            routes,
            relay_configured,
            transport_options,
            power,
            wake,
            idle: idle::NodeIdle::default(),
            #[cfg(test)]
            legacy_transfer_host: AtomicBool::new(false),
            #[cfg(test)]
            transfer_stall_millis: AtomicU64::new(0),
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

    /// `(IPv4 port, IPv6 port)` of the bound Iroh sockets.
    pub(crate) fn bound_ports(&self) -> (Option<u16>, Option<u16>) {
        let mut v4 = None;
        let mut v6 = None;
        for socket in self.endpoint.bound_sockets() {
            match socket {
                std::net::SocketAddr::V4(addr) => v4 = v4.or(Some(addr.port())),
                std::net::SocketAddr::V6(addr) => v6 = v6.or(Some(addr.port())),
            }
        }
        (v4, v6)
    }

    pub(super) fn published_routes(&self) -> PublishedEndpointRoutes {
        self.routes.published(&self.endpoint)
    }

    pub(crate) fn reciprocal_transition_in_flight(&self) -> bool {
        self.runtime_transition_slot.available_permits() < RUNTIME_TRANSITION_PERMITS as usize
    }

    pub(super) fn begin_runtime_transition(&self) -> io::Result<tokio::sync::OwnedSemaphorePermit> {
        self.runtime_transition_slot
            .clone()
            .try_acquire_many_owned(RUNTIME_TRANSITION_PERMITS)
            .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "Direct repair is active"))
    }

    /// Changes with every route or home-relay change the worker must
    /// republish; the node's own watcher also wakes the worker for it.
    pub(super) fn route_revision(&self) -> u64 {
        self.routes
            .revision()
            .wrapping_add(self.wake.route_revision())
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
        self.track_incoming_entry(connection, None)
    }

    /// Tracks a filesystem connection whose streams and leases an idle
    /// sweep may judge.
    pub(super) fn track_incoming_fs(
        self: &Arc<Self>,
        connection: &Connection,
    ) -> io::Result<(IncomingConnectionGuard, Arc<IncomingActivity>)> {
        let activity = Arc::new(IncomingActivity::default());
        let guard = self.track_incoming_entry(connection, Some(activity.clone()))?;
        Ok((guard, activity))
    }

    fn track_incoming_entry(
        self: &Arc<Self>,
        connection: &Connection,
        activity: Option<Arc<IncomingActivity>>,
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
            .insert(
                id,
                idle::IncomingEntry {
                    connection: connection.clone(),
                    activity,
                },
            );
        Ok(IncomingConnectionGuard {
            node: Arc::downgrade(self),
            id,
        })
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
        self.lan_links.disable();
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
        let addr = endpoint_addr(&endpoint.presence, &local_addr, &self.transport_options)?;
        let generation = self.policy.snapshot(super::session::PeerPrincipal::from_endpoint(endpoint))?;
        let connection = self.block_on(io_deadline::run("peer exec connection", async {
            self.endpoint.connect(addr, EXEC_ALPN).await.map_err(io_deadline::disconnected)
        }))?;
        if let Err(error) = self.policy.bind(&connection, generation) {
            connection.close(VarInt::from_u32(0x5345), b"authorization changed during handshake");
            return Err(error);
        }
        Ok(connection)
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

    /// Transfer features this host advertises in every Capabilities reply.
    pub(super) fn transfer_capabilities(&self) -> FsTransferCapabilities {
        if self.legacy_transfer_host() {
            FsTransferCapabilities::default()
        } else {
            FsTransferCapabilities::host()
        }
    }

    /// Whether this host answers like one before transfer v1 (tests only).
    #[cfg(test)]
    pub(super) fn legacy_transfer_host(&self) -> bool {
        self.legacy_transfer_host.load(Ordering::Acquire)
    }

    #[cfg(not(test))]
    pub(super) fn legacy_transfer_host(&self) -> bool {
        false
    }

    #[cfg(test)]
    pub(super) fn pose_as_legacy_transfer_host(&self) {
        self.legacy_transfer_host.store(true, Ordering::Release);
    }

    /// Longest one chunk of a transfer may stall (a client that neither
    /// reads nor sends): the operation deadline, the same bound the client
    /// puts on each of its own chunks. Past it the host frees the slot.
    #[cfg(not(test))]
    pub(super) fn transfer_stall(&self) -> std::time::Duration {
        io_deadline::PEER_OP_TIMEOUT
    }

    #[cfg(test)]
    pub(super) fn transfer_stall(&self) -> std::time::Duration {
        match self.transfer_stall_millis.load(Ordering::Acquire) {
            0 => io_deadline::PEER_OP_TIMEOUT,
            millis => std::time::Duration::from_millis(millis),
        }
    }

    #[cfg(test)]
    pub(super) fn shorten_transfer_stall_for_test(&self, stall: std::time::Duration) {
        let millis = u64::try_from(stall.as_millis()).unwrap_or(u64::MAX).max(1);
        self.transfer_stall_millis.store(millis, Ordering::Release);
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
                if let Some(entry) = sessions.remove(&self.id) { node.policy.unbind(entry.connection.stable_id()); }
            };
        }
    }
}
