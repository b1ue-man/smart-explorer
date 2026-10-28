//! Active tunnels only. Handles remove entries on every exit path; neither
//! caller supplied IDs nor serialized status views grant remote authority.
//!
//! A listener tunnel also carries its origin rule (design 4.4, 9.3): every
//! accepted local connection is checked by `Gate::admit` before a single
//! byte reaches the peer. An `AwaitProcess` tunnel learns its one client
//! process later (`bind_process`); connections that arrive earlier wait for
//! that binding and are then checked. `spawn_tunnel` runs a registered
//! listener tunnel until its connection, a close or its source ends.
use super::core::eio;
use super::node::ShareIrohNode;
use super::node_tcp::{cancelled_stop, CloseTcp};
use super::tcp_forward::{
    ListenerAdmission, TunnelId, TunnelView, MAX_TCP_STREAMS, MAX_TCP_TUNNELS,
};
use super::tcp_wire::{self, TcpDataOpen, TcpDataReply};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::{
    collections::BTreeMap,
    io,
    net::{Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;

#[derive(Default)]
pub(crate) struct TcpCounters {
    pub(crate) active: AtomicUsize,
    pub(crate) read: AtomicU64,
    pub(crate) written: AtomicU64,
}

/// The one client process an `AwaitProcess` listener forwards for: PID and
/// process start time (Linux: clock ticks of `/proc/<pid>/stat` field 22,
/// Windows: creation FILETIME), so a reused PID never matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoundProcess {
    pub(crate) pid: u32,
    pub(crate) started: u64,
}

/// The OS facts behind an origin rule. Implemented in `os/` (socket owner
/// tables); this layer only decides which question to ask.
pub(crate) trait OriginCheck: Send + Sync {
    /// The connecting process runs under this background service's account
    /// (design 4.4 `SameAccount`).
    fn same_account(&self, peer: SocketAddr, listener: SocketAddr) -> Result<(), String>;
    /// The connection socket belongs to exactly `bound` (design 2.7.1).
    fn process(
        &self,
        peer: SocketAddr,
        listener: SocketAddr,
        bound: BoundProcess,
    ) -> Result<(), String>;
}

/// How the viewer's listener accepts local connections.
#[derive(Clone)]
pub(crate) struct ListenerOptions {
    pub(crate) admission: ListenerAdmission,
    /// The device address of design 4.3; `None` = 127.0.0.1.
    pub(crate) bind: Option<Ipv4Addr>,
    /// The OS facts for `SameAccount`/`AwaitProcess`; unused by `AnyLocal`.
    pub(crate) check: Option<Arc<dyn OriginCheck>>,
}
impl ListenerOptions {
    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.bind.is_some_and(|ip| !ip.is_loopback()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "TCP-Weiterleitungen lauschen nur auf Loopbackadressen",
            ));
        }
        if self.admission != ListenerAdmission::AnyLocal && self.check.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Diese Herkunftsregel braucht eine Prüfung der Verbindungsbesitzer",
            ));
        }
        Ok(())
    }
}

/// The admission of one listener tunnel, cloned into every accept task.
#[derive(Clone)]
pub(crate) struct Gate {
    admission: ListenerAdmission,
    check: Option<Arc<dyn OriginCheck>>,
    bound: watch::Receiver<Option<BoundProcess>>,
}
impl Gate {
    pub(crate) fn admission(&self) -> ListenerAdmission {
        self.admission
    }
    /// `Ok` when the connection from `peer` to `listener` may be forwarded.
    /// The owner lookup reads OS tables and runs on the blocking pool.
    pub(crate) async fn admit(
        &mut self,
        peer: SocketAddr,
        listener: SocketAddr,
    ) -> Result<(), String> {
        let check = match self.admission {
            ListenerAdmission::AnyLocal => return Ok(()),
            _ => self.check.clone().ok_or_else(|| {
                "Diese Weiterleitung hat keine Herkunftsprüfung; Verbindung abgewiesen".to_string()
            })?,
        };
        let bound = match self.admission {
            ListenerAdmission::AwaitProcess => {
                // No time limit: the tunnel ends when the IPC stream that
                // must deliver the binding ends (design 2.6), and then this
                // receiver reports the closed channel.
                let bound = self
                    .bound
                    .wait_for(Option::is_some)
                    .await
                    .map_err(|_| "Tunnel endete vor der Prozessbindung".to_string())?;
                *bound
            }
            _ => None,
        };
        let verdict = tokio::task::spawn_blocking(move || match bound {
            Some(bound) => check.process(peer, listener, bound),
            None => check.same_account(peer, listener),
        })
        .await;
        verdict.map_err(|error| format!("Herkunftsprüfung abgebrochen: {error}"))?
    }
}

struct Entry {
    view: TunnelView,
    connection: Connection,
    stop: watch::Sender<bool>,
    counters: Arc<TcpCounters>,
    bound: watch::Sender<Option<BoundProcess>>,
}
#[derive(Default)]
pub(crate) struct TcpRegistry {
    entries: Mutex<BTreeMap<TunnelId, Entry>>,
}
impl TcpRegistry {
    pub(crate) fn insert(
        self: &Arc<Self>,
        view: TunnelView,
        connection: Connection,
    ) -> io::Result<TcpRegistration> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| io::Error::other("TCP registry locked"))?;
        if entries.len() >= MAX_TCP_TUNNELS || entries.contains_key(&view.id) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "TCP tunnel admission exhausted or duplicate ID",
            ));
        }
        let id = view.id.clone();
        let counters = Arc::new(TcpCounters::default());
        let (stop, cancelled) = watch::channel(false);
        let (bound, bound_rx) = watch::channel(None);
        entries.insert(
            id.clone(),
            Entry {
                view,
                connection,
                stop,
                counters: counters.clone(),
                bound,
            },
        );
        Ok(TcpRegistration {
            registry: self.clone(),
            id,
            cancelled,
            counters,
            bound: bound_rx,
        })
    }
    pub(crate) fn list(&self) -> Vec<TunnelView> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries
            .values()
            .map(|entry| {
                let mut view = entry.view.clone();
                view.active_connections = entry.counters.active.load(Ordering::Relaxed);
                view.socket_read_bytes = entry.counters.read.load(Ordering::Relaxed);
                view.socket_written_bytes = entry.counters.written.load(Ordering::Relaxed);
                view.closing = *entry.stop.borrow();
                view
            })
            .collect()
    }
    pub(crate) fn status(&self, id: &TunnelId) -> Option<TunnelView> {
        self.list().into_iter().find(|view| &view.id == id)
    }
    pub(crate) fn close(&self, id: &TunnelId) -> bool {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(entry) = entries.get(id) else {
            return false;
        };
        entry.stop.send_replace(true);
        super::node_tcp::close_connection(&entry.connection);
        true
    }
    pub(crate) fn close_all(&self) {
        let ids: Vec<_> = self.list().into_iter().map(|view| view.id).collect();
        for id in ids {
            self.close(&id);
        }
    }
    pub(crate) fn record_error(&self, id: &TunnelId, error: &io::Error) {
        self.record_text(id, &error.to_string());
    }
    /// Shown as the tunnel's `last_error` (sanitized like peer text).
    pub(crate) fn record_text(&self, id: &TunnelId, text: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            if let Some(entry) = entries.get_mut(id) {
                entry.view.last_error = Some(super::tcp_forward::shown_error(text));
            }
        }
    }
    /// Binds the one client process of an `AwaitProcess` listener. Refused
    /// for any other listener and for a second binding: the rule of a
    /// running tunnel never widens.
    pub(crate) fn bind_process(&self, id: &TunnelId, bound: BoundProcess) -> io::Result<()> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = entries.get(id).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "TCP-Tunnel besteht nicht mehr")
        })?;
        if entry.view.admission != Some(ListenerAdmission::AwaitProcess) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Dieser TCP-Tunnel nimmt keine Prozessbindung an",
            ));
        }
        if entry.bound.borrow().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Dieser TCP-Tunnel ist bereits an einen Prozess gebunden",
            ));
        }
        entry.bound.send_replace(Some(bound));
        Ok(())
    }
}
pub(crate) struct TcpRegistration {
    registry: Arc<TcpRegistry>,
    pub(crate) id: TunnelId,
    pub(crate) cancelled: watch::Receiver<bool>,
    pub(crate) counters: Arc<TcpCounters>,
    bound: watch::Receiver<Option<BoundProcess>>,
}
impl TcpRegistration {
    pub(crate) fn registry(&self) -> Arc<TcpRegistry> {
        self.registry.clone()
    }
    /// The admission gate of this listener tunnel.
    pub(crate) fn gate(
        &self,
        admission: ListenerAdmission,
        check: Option<Arc<dyn OriginCheck>>,
    ) -> Gate {
        Gate {
            admission,
            check,
            bound: self.bound.clone(),
        }
    }
}
impl Drop for TcpRegistration {
    fn drop(&mut self) {
        if let Some(entry) = self
            .registry
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id)
        {
            super::node_tcp::close_connection(&entry.connection);
        }
    }
}

/// The pieces of one authenticated tunnel that keep running.
pub(crate) struct TunnelParts {
    pub(crate) close: CloseTcp,
    pub(crate) send: SendStream,
    pub(crate) recv: RecvStream,
    /// Only an explicitly chosen lifetime ends a tunnel by time (design 2.6).
    pub(crate) lifetime: Option<Duration>,
}

/// Runs one registered tunnel on the share runtime until it ends.
pub(crate) fn spawn_tunnel(
    node: &Arc<ShareIrohNode>,
    slot: OwnedSemaphorePermit,
    parts: TunnelParts,
    source: Source,
    registration: TcpRegistration,
) {
    let weak = Arc::downgrade(node);
    node.rt.spawn(async move {
        let _slot = slot;
        let TunnelParts {
            close,
            send,
            mut recv,
            lifetime,
        } = parts;
        let connection = close.0.clone();
        run_link(weak, connection, source, &mut recv, registration, lifetime).await;
        // The control stream and the transport live exactly as long as the
        // tunnel: dropping them here tells the host.
        drop(send);
        drop(close);
    });
}

/// Where a tunnel's local connections come from.
pub(crate) enum Source {
    /// Own listener; each connection passes the gate first.
    Listener { listener: TcpListener, gate: Gate },
    /// Sockets an auto-forward listener admitted already.
    Channel(mpsc::Receiver<TcpStream>),
}
impl Source {
    /// The next local connection with the gate it must pass; `None` when
    /// the source ended (failed accept, all channel senders gone).
    async fn next(&mut self) -> Option<(TcpStream, Option<(Gate, SocketAddr, SocketAddr)>)> {
        match self {
            Self::Listener { listener, gate } => {
                let (socket, peer) = listener.accept().await.ok()?;
                let local = socket.local_addr().ok()?;
                Some((socket, Some((gate.clone(), peer, local))))
            }
            Self::Channel(receiver) => receiver.recv().await.map(|socket| (socket, None)),
        }
    }
}

async fn run_link(
    node: std::sync::Weak<ShareIrohNode>,
    connection: Connection,
    mut source: Source,
    control_recv: &mut RecvStream,
    registration: TcpRegistration,
    lifetime: Option<Duration>,
) {
    let mut stop = registration.cancelled.clone();
    let streams = Arc::new(Semaphore::new(MAX_TCP_STREAMS));
    let activity = Arc::new(super::node::IncomingActivity::default());
    let mut children: JoinSet<io::Result<()>> = JoinSet::new();
    // Only an explicitly chosen lifetime ends the tunnel by time (design 2.6).
    let ttl = async move {
        match lifetime {
            Some(lifetime) => tokio::time::sleep(lifetime).await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(ttl);
    let channel = matches!(source, Source::Channel(_));
    let mut close_byte = [0];
    let mut source_open = true;
    'run: loop {
        let mut accepted = Vec::new();
        tokio::select! {
            biased;
            () = cancelled_stop(&mut stop) => break,
            () = &mut ttl => break,
            _ = connection.closed() => break,
            _ = AsyncReadExt::read(control_recv, &mut close_byte) => break,
            Some(_) = children.join_next(), if !children.is_empty() => {
                if children.is_empty() {
                    if !source_open {
                        break;
                    }
                    if let Source::Channel(receiver) = &mut source {
                        // Last local connection ended: refuse new sockets
                        // (the caller then opens a new tunnel) and serve the
                        // ones queued before the close. With no child left
                        // every stream slot is free.
                        receiver.close();
                        while let Ok(socket) = receiver.try_recv() {
                            if let Ok(slot) = streams.clone().try_acquire_owned() {
                                accepted.push((slot, socket, None));
                            }
                        }
                        if accepted.is_empty() {
                            break;
                        }
                    }
                }
            },
            incoming = async {
                // A free stream slot first: more than MAX_TCP_STREAMS local
                // connections wait in the OS backlog, as before.
                let slot = streams.clone().acquire_owned().await.ok()?;
                let (socket, gate) = source.next().await?;
                Some((slot, socket, gate))
            }, if source_open => match incoming {
                Some(incoming) => accepted.push(incoming),
                // A failed accept ends a listener tunnel; a channel whose
                // senders all left ends after its open connections.
                None if !channel || children.is_empty() => break,
                None => source_open = false,
            },
        }
        for (slot, socket, gate) in accepted {
            let Some(current) = node.upgrade() else {
                break 'run;
            };
            let hold = current.incoming_stream_started(&activity);
            drop(current);
            let connection = connection.clone();
            let id = registration.id.clone();
            let counters = registration.counters.clone();
            let registry = registration.registry();
            children.spawn(async move {
                let _slot = slot;
                let _hold = hold;
                if let Some((mut gate, peer, local)) = gate {
                    if let Err(text) = gate.admit(peer, local).await {
                        registry.record_text(&id, &text);
                        return Err(io::Error::new(io::ErrorKind::PermissionDenied, text));
                    }
                }
                let result = forward(connection, socket, id.clone(), counters).await;
                if let Err(error) = &result {
                    registry.record_error(&id, error);
                }
                result
            });
        }
    }
    super::node_tcp::close_connection(&connection);
    children.shutdown().await;
    drop(source);
    drop(registration);
}

/// One local connection over its own QUIC stream pair.
async fn forward(
    connection: Connection,
    mut socket: TcpStream,
    id: TunnelId,
    counters: Arc<TcpCounters>,
) -> io::Result<()> {
    super::route_protocol::scope(connection.clone(), async {
        socket.set_nodelay(true)?;
        let (mut send, mut recv) =
            super::route_protocol::timeout_for(Duration::from_secs(20), connection.open_bi())
                .await?
                .map_err(eio)?;
        super::route_protocol::timeout_for(
            Duration::from_secs(20),
            tcp_wire::send_json(&mut send, &TcpDataOpen { tunnel: id }),
        )
        .await??;
        let reply: TcpDataReply = super::route_protocol::timeout_for(
            Duration::from_secs(20),
            tcp_wire::recv_json(&mut recv),
        )
        .await??;
        match reply {
            TcpDataReply::Connected => {
                super::tcp_pump::pump(&connection, &mut socket, &mut send, &mut recv, counters)
                    .await
            }
            TcpDataReply::Refused { detail } => {
                Err(io::Error::other(super::tcp_forward::shown_error(&detail)))
            }
        }
    })
    .await
}
