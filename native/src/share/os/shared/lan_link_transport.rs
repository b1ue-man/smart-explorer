//! Dedicated bounded status transport; never borrows filesystem/Exec pools.
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iroh::endpoint::{Connection, LocalTransportAddr, VarInt};
use iroh::{EndpointAddr, TransportAddr};
use tokio::sync::Semaphore;

use super::core::{eio, now_secs};
use super::direct_protocol::DirectPeerIdentity;
use super::lan_link_facts::{
    self, AuthenticatedLanFact, LanLinkHostFacts, LanPeerPin, OwnUplink, MAX_FACT_LIFETIME_SECS,
    MAX_LINK_PEERS,
};
use super::lan_link_wire::LAN_LINK_ALPN;
use super::node::ShareIrohNode;
use super::types::ShareAuthState;

#[path = "lan_link_dial.rs"]
mod dial;

pub(super) const ROUND_DEADLINE: Duration = Duration::from_secs(3);
pub(super) const SESSION_DEADLINE: Duration = Duration::from_secs(45);
pub(super) const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const FACT_MONOTONIC_TTL: Duration = Duration::from_secs(6);
const HOST_FACT_TTL: Duration = Duration::from_secs(6);
const PROBE_BACKOFF: Duration = Duration::from_secs(5);
const MAX_INTERFACES: usize = 128;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct SelectedIpPath {
    pub(super) local: std::net::IpAddr,
    pub(super) remote: std::net::SocketAddr,
    pub(super) id: String,
}

pub(super) fn selected_ip_path(connection: &Connection) -> Option<SelectedIpPath> {
    if connection.close_reason().is_some() {
        return None;
    }
    let paths = connection.paths();
    let mut selected = paths.iter().filter(|path| path.is_selected());
    let path = selected.next()?;
    if selected.next().is_some() || !path.is_ip() || path.is_relay() {
        return None;
    }
    match (path.local_addr(), path.remote_addr()) {
        (LocalTransportAddr::Ip(Some(local)), TransportAddr::Ip(remote)) => Some(SelectedIpPath {
            local: *local,
            remote: *remote,
            id: format!("{:?}", path.id()),
        }),
        _ => None,
    }
}

struct Channel {
    connection: Connection,
    revision: u64,
    control_epoch: u64,
}
struct Cached {
    fact: AuthenticatedLanFact,
    confirmed: Instant,
    revision: u64,
}
struct State {
    host: Option<(LanLinkHostFacts, Instant)>,
    channels: HashMap<usize, Channel>,
    cache: HashMap<usize, Cached>,
    probes: HashMap<String, Instant>,
    cursor: usize,
}

pub(crate) struct LanLinkTransport {
    enabled: AtomicBool,
    control_epoch: AtomicU64,
    state: Mutex<State>,
    inbound: Arc<Semaphore>,
    outbound: Arc<Semaphore>,
}

impl Default for LanLinkTransport {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            control_epoch: AtomicU64::new(0),
            state: Mutex::new(State {
                host: None,
                channels: HashMap::new(),
                cache: HashMap::new(),
                probes: HashMap::new(),
                cursor: 0,
            }),
            inbound: Arc::new(Semaphore::new(8)),
            outbound: Arc::new(Semaphore::new(4)),
        }
    }
}

impl LanLinkTransport {
    pub(crate) fn disable(&self) {
        self.enabled.store(false, Ordering::Release);
        let _ = self
            .control_epoch
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |epoch| {
                epoch.checked_add(1)
            });
        if let Ok(mut state) = self.state.try_lock() {
            state.host = None;
            state.cache.clear();
            state.probes.clear();
            for channel in state.channels.values() {
                close(&channel.connection);
            }
            state.channels.clear();
        }
    }

    fn update(&self, mut facts: LanLinkHostFacts) -> io::Result<()> {
        if !facts.enabled {
            self.disable();
            return Ok(());
        }
        if self.control_epoch.load(Ordering::Acquire) == u64::MAX {
            return Err(eio("LAN-Link-Epoche erschoepft"));
        }
        if facts.interfaces.len() > MAX_INTERFACES
            || facts.shared_ifaces.len() > MAX_INTERFACES
            || facts.interfaces.iter().any(|iface| iface.addrs.len() > 64)
        {
            self.disable();
            return Err(eio("Zu viele LAN-Interface-Fakten"));
        }
        if facts.interfaces.is_empty() {
            facts.own_uplink = OwnUplink::Unknown;
        }
        let mut state = self.state.try_lock().map_err(|_| busy())?;
        state.host = Some((facts, Instant::now()));
        self.enabled.store(true, Ordering::Release);
        Ok(())
    }

    pub(super) fn host(&self) -> io::Result<LanLinkHostFacts> {
        if !self.enabled.load(Ordering::Acquire) {
            return Err(eio("LAN-Link ist ausgeschaltet"));
        }
        let state = self.state.try_lock().map_err(|_| busy())?;
        let (facts, at) = state
            .host
            .as_ref()
            .ok_or_else(|| eio("LAN-Link-Fakten fehlen"))?;
        if at.elapsed() > HOST_FACT_TTL {
            return Err(eio("LAN-Link-Fakten sind veraltet"));
        }
        Ok(facts.clone())
    }

    fn register(&self, connection: &Connection) -> io::Result<()> {
        self.host()?;
        connection.set_max_concurrent_bi_streams(VarInt::from_u32(1));
        connection.set_max_concurrent_uni_streams(VarInt::from_u32(0));
        connection.set_receive_window(VarInt::from_u32(16 * 1024));
        let mut state = self.state.try_lock().map_err(|_| busy())?;
        if !self.enabled.load(Ordering::Acquire) || state.channels.len() >= 12 {
            return Err(eio("LAN-Link-Zulassung erschoepft"));
        }
        state.channels.insert(
            connection.stable_id(),
            Channel {
                connection: connection.clone(),
                revision: 0,
                control_epoch: self.control_epoch.load(Ordering::Acquire),
            },
        );
        Ok(())
    }

    pub(super) fn path_revision(&self, connection: &Connection) -> io::Result<u64> {
        let state = self.state.try_lock().map_err(|_| busy())?;
        state
            .channels
            .get(&connection.stable_id())
            .filter(|channel| channel.control_epoch == self.control_epoch.load(Ordering::Acquire))
            .map(|channel| channel.revision)
            .ok_or_else(|| eio("LAN-Link geschlossen"))
    }

    pub(super) fn path_changed(&self, connection: &Connection) {
        if let Ok(mut state) = self.state.lock() {
            state.cache.remove(&connection.stable_id());
            if let Some(channel) = state.channels.get_mut(&connection.stable_id()) {
                match channel.revision.checked_add(1) {
                    Some(revision) => channel.revision = revision,
                    None => close(connection),
                }
            }
        } else {
            close(connection);
        }
    }

    fn remove(&self, connection: &Connection) {
        close(connection);
        if let Ok(mut state) = self.state.lock() {
            state.channels.remove(&connection.stable_id());
            state.cache.remove(&connection.stable_id());
        }
    }

    pub(super) fn confirm(
        &self,
        node: &ShareIrohNode,
        connection: &Connection,
        pin: LanPeerPin,
        path: SelectedIpPath,
        revision: u64,
        challenge: [u8; 32],
        uplink: OwnUplink,
    ) -> io::Result<()> {
        node.require_sharing_active()?;
        let auth = node.auth.try_lock().map_err(|_| busy())?;
        if !lan_link_facts::pin_current(&pin, &auth.direct_contacts, &auth.direct_grants)
            || connection.remote_id().to_string() != pin.node_id
            || selected_ip_path(connection).as_ref() != Some(&path)
        {
            return Err(eio("LAN-Link-Autoritaet geaendert"));
        }
        let host = self.host()?;
        let interface =
            lan_link_facts::private_interface(path.local, path.remote, &host.interfaces)
                .ok_or_else(|| eio("LAN-Link-Interface ist nicht eindeutig privat"))?;
        let mut state = self.state.try_lock().map_err(|_| busy())?;
        if state
            .channels
            .get(&connection.stable_id())
            .is_none_or(|channel| {
                channel.revision != revision
                    || channel.control_epoch != self.control_epoch.load(Ordering::Acquire)
            })
        {
            return Err(eio("LAN-Link-Pfad hat gewechselt"));
        }
        state.cache.remove(&connection.stable_id());
        if let Some(peer_uplink) = uplink.known() {
            let confirmed_at = now_secs();
            state.cache.insert(
                connection.stable_id(),
                Cached {
                    fact: AuthenticatedLanFact {
                        pin,
                        interface,
                        remote_addr: path.remote,
                        path_id: path.id,
                        connection_id: connection.stable_id() as u64,
                        challenge,
                        peer_uplink,
                        confirmed_at,
                        expires_at: confirmed_at.saturating_add(MAX_FACT_LIFETIME_SECS),
                    },
                    confirmed: Instant::now(),
                    revision,
                },
            );
        }
        Ok(())
    }

    fn snapshot(&self, auth: &ShareAuthState) -> Vec<AuthenticatedLanFact> {
        let Ok(host) = self.host() else {
            return Vec::new();
        };
        let Ok(mut state) = self.state.try_lock() else {
            return Vec::new();
        };
        let now = now_secs();
        let invalid: Vec<_> = state
            .cache
            .iter()
            .filter_map(|(id, cached)| {
                let valid = cached.confirmed.elapsed() <= FACT_MONOTONIC_TTL
                    && cached.fact.current(
                        now,
                        &auth.direct_contacts,
                        &auth.direct_grants,
                        &host.interfaces,
                    )
                    && state.channels.get(id).is_some_and(|channel| {
                        channel.revision == cached.revision
                            && channel.control_epoch == self.control_epoch.load(Ordering::Acquire)
                            && channel.connection.close_reason().is_none()
                            && channel.connection.remote_id().to_string() == cached.fact.pin.node_id
                            && selected_ip_path(&channel.connection).is_some_and(|path| {
                                path.local == cached.fact.interface.local_ip
                                    && path.remote == cached.fact.remote_addr
                                    && path.id == cached.fact.path_id
                            })
                    });
                (!valid).then_some(*id)
            })
            .collect();
        for id in invalid {
            if let Some(cached) = state.cache.remove(&id) {
                if !lan_link_facts::pin_current(
                    &cached.fact.pin,
                    &auth.direct_contacts,
                    &auth.direct_grants,
                ) {
                    if let Some(channel) = state.channels.get(&id) {
                        close(&channel.connection);
                    }
                }
            }
        }
        state
            .cache
            .values()
            .take(MAX_LINK_PEERS)
            .map(|cached| cached.fact.clone())
            .collect()
    }
}

impl ShareIrohNode {
    /// Parent exposes this through ShareService; performs no network I/O.
    pub(crate) fn update_lan_link_host(
        self: &Arc<Self>,
        facts: LanLinkHostFacts,
    ) -> io::Result<()> {
        self.require_sharing_active()?;
        self.lan_links.update(facts)?;
        if self.lan_links.enabled.load(Ordering::Acquire) {
            self.lan_links.probe(self.clone())?;
        }
        Ok(())
    }

    pub(crate) fn lan_link_snapshot(&self) -> Vec<AuthenticatedLanFact> {
        if self.require_sharing_active().is_err() {
            self.lan_links.disable();
            return Vec::new();
        }
        let Ok(auth) = self.auth.try_lock() else {
            return Vec::new();
        };
        self.lan_links.snapshot(&auth)
    }

    pub(super) async fn accept_lan_link(self: Arc<Self>, connection: Connection) -> io::Result<()> {
        let transport = self.lan_links.clone();
        let permit = match transport.inbound.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                close(&connection);
                return Err(busy());
            }
        };
        let known = self.auth.try_lock().ok().is_some_and(|auth| {
            let remote = connection.remote_id().to_string();
            auth.direct_contacts
                .iter()
                .filter_map(lan_link_facts::contact_pin)
                .chain(
                    auth.direct_grants
                        .iter()
                        .filter_map(lan_link_facts::grant_pin),
                )
                .any(|pin| pin.node_id == remote)
        });
        if !known || connection.alpn() != LAN_LINK_ALPN || transport.register(&connection).is_err()
        {
            close(&connection);
            return Err(eio("Kein aktueller Direct-Pin fuer LAN-Link"));
        }
        let result =
            super::lan_link_exchange::run(self, transport.clone(), connection.clone(), None).await;
        transport.remove(&connection);
        drop(permit);
        result
    }
}

pub(super) fn local_identity(node: &ShareIrohNode) -> io::Result<DirectPeerIdentity> {
    node.require_sharing_active()?;
    let auth = node.auth.try_lock().map_err(|_| busy())?;
    let identity =
        DirectPeerIdentity::from_secret(&auth.identity.device_id, "", &auth.identity.iroh_secret);
    identity.validate().map_err(eio)?;
    Ok(identity)
}

pub(super) fn current_pin(
    node: &ShareIrohNode,
    connection: &Connection,
    identity: &DirectPeerIdentity,
) -> io::Result<LanPeerPin> {
    node.require_sharing_active()?;
    let auth = node.auth.try_lock().map_err(|_| busy())?;
    lan_link_facts::accepted_pin_for_tls(
        &auth.direct_contacts,
        &auth.direct_grants,
        identity,
        &connection.remote_id().to_string(),
    )
    .ok_or_else(|| eio("LAN-Link-Pins passen nicht"))
}

fn busy() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "LAN-Link-Zulassung oder Snapshot belegt",
    )
}
fn close(connection: &Connection) {
    connection.close(VarInt::from_u32(0x534c), b"paired link status ended");
}

#[cfg(test)]
#[path = "lan_link_transport_task_tests.rs"]
mod task_tests;
