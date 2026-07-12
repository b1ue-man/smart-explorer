use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use iroh::endpoint::{presets, Connection, RecvStream, SendStream, VarInt};
use iroh::{Endpoint, RelayMap, RelayMode, RelayUrl};

use super::core::{eio, hmac_proof, random_token};
use super::framing::{recv_ctrl, send_ctrl};
use super::identity::ShareIdentity;
use super::io_deadline;
use super::keepalive::iroh_transport_config;
use super::session::{
    endpoint_addr, relation_kind_id, relay_url_from_signal, session_key, session_payload,
    transport_label,
};
use super::types::{PeerEndpoint, ShareAuthState, ShareEvent};
use super::wire::{Ctrl, FsResponse, PeerHello};

const ALPN: &[u8] = b"smart-explorer/share-fs/3";

pub(crate) struct ShareIrohNode {
    rt: Arc<tokio::runtime::Runtime>,
    endpoint: Endpoint,
    pub(super) auth: Arc<Mutex<ShareAuthState>>,
    pub(super) ev: crossbeam_channel::Sender<ShareEvent>,
    sessions: Mutex<HashMap<String, Connection>>,
    session_epoch: AtomicU64,
    incoming_sessions: Mutex<HashMap<u64, Connection>>,
    next_incoming_session: AtomicU64,
    accept_errors: Mutex<HashMap<String, (Instant, u32)>>,
    relay_url: String,
}

impl ShareIrohNode {
    pub(crate) fn start(
        server: &str,
        identity: &ShareIdentity,
        auth: Arc<Mutex<ShareAuthState>>,
        ev: crossbeam_channel::Sender<ShareEvent>,
    ) -> io::Result<Arc<Self>> {
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("share-iroh")
                .build()
                .map_err(eio)?,
        );
        let relay_url = relay_url_from_signal(server);
        let relay_mode = relay_url
            .parse::<RelayUrl>()
            .ok()
            .map(|url| RelayMode::Custom(RelayMap::from(url)))
            .unwrap_or(RelayMode::Disabled);
        let endpoint = rt.block_on(async {
            Endpoint::builder(presets::Minimal)
                .secret_key(identity.iroh_secret.clone())
                .alpns(vec![ALPN.to_vec()])
                .relay_mode(relay_mode)
                .transport_config(iroh_transport_config())
                .bind()
                .await
                .map_err(eio)
        })?;
        let node = Arc::new(Self {
            rt,
            endpoint,
            auth,
            ev,
            sessions: Mutex::new(HashMap::new()),
            session_epoch: AtomicU64::new(0),
            incoming_sessions: Mutex::new(HashMap::new()),
            next_incoming_session: AtomicU64::new(0),
            accept_errors: Mutex::new(HashMap::new()),
            relay_url,
        });
        node.spawn_accept_loop();
        Ok(node)
    }

    pub(crate) fn relay_url(&self) -> &str {
        &self.relay_url
    }

    pub(crate) fn candidates(&self) -> Vec<String> {
        self.endpoint
            .addr()
            .ip_addrs()
            .map(|addr| addr.to_string())
            .collect()
    }

    pub(super) fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.rt.block_on(future)
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
        Ok(count)
    }

    pub(super) fn open_stream(
        &self,
        endpoint: &PeerEndpoint,
        identity: &ShareIdentity,
    ) -> io::Result<(SendStream, RecvStream)> {
        let key = session_key(endpoint);
        let cached = self.sessions.lock().ok().and_then(|s| s.get(&key).cloned());
        let connection = if let Some(connection) = cached {
            connection
        } else {
            let epoch = self.session_epoch.load(Ordering::Acquire);
            let connection = self.connect_session(endpoint, identity)?;
            self.cache_session(key.clone(), connection.clone(), epoch)?;
            connection
        };
        match self.block_on(io_deadline::run("peer stream open", async {
            connection
                .open_bi()
                .await
                .map_err(io_deadline::disconnected)
        })) {
            Ok(streams) => Ok(streams),
            Err(_) => {
                // open_bi has not sent an operation payload, so replacing even
                // a blackholed connection timeout is safe and cannot replay a
                // mutation. Preserve a newer generation installed by another
                // caller instead of deleting it with the failed cache entry.
                let replacement = self.replacement_after_open_failure(
                    &key,
                    connection.stable_id(),
                    endpoint,
                    identity,
                )?;
                self.block_on(io_deadline::run("peer stream reopen", async {
                    replacement
                        .open_bi()
                        .await
                        .map_err(io_deadline::disconnected)
                }))
            }
        }
    }

    fn replacement_after_open_failure(
        &self,
        key: &str,
        failed_id: usize,
        endpoint: &PeerEndpoint,
        identity: &ShareIdentity,
    ) -> io::Result<Connection> {
        {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| eio("Ausgehende Share-Sessions sind gesperrt"))?;
            if let Some(current) = sessions.get(key) {
                if current.stable_id() != failed_id {
                    return Ok(current.clone());
                }
            }
            sessions.remove(key);
        }
        let epoch = self.session_epoch.load(Ordering::Acquire);
        let connection = self.connect_session(endpoint, identity)?;
        self.cache_session(key.to_string(), connection.clone(), epoch)?;
        Ok(connection)
    }

    fn cache_session(
        &self,
        key: String,
        connection: Connection,
        expected_epoch: u64,
    ) -> io::Result<()> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| eio("Ausgehende Share-Sessions sind gesperrt"))?;
        if self.session_epoch.load(Ordering::Acquire) != expected_epoch {
            connection.close(
                VarInt::from_u32(0x5345),
                b"authorization changed during session handshake",
            );
            return Err(eio(
                "Share-Autorisierung wurde waehrend des Handshakes geaendert",
            ));
        }
        sessions.insert(key, connection);
        Ok(())
    }

    pub(super) fn session_transport(&self, endpoint: &PeerEndpoint) -> Option<&'static str> {
        let key = session_key(endpoint);
        let connection = self.sessions.lock().ok()?.get(&key).cloned()?;
        Some(transport_label(&connection))
    }

    fn connect_session(
        &self,
        endpoint: &PeerEndpoint,
        identity: &ShareIdentity,
    ) -> io::Result<Connection> {
        if let Some(expected) = endpoint.expected_node_id.as_deref() {
            if !expected.trim().is_empty() && expected != endpoint.presence.node_id {
                return Err(eio("Iroh NodeId passt nicht zur gepinnten Identitaet"));
            }
        }
        let addr = endpoint_addr(&endpoint.presence)?;
        let (kind, relation_id) = relation_kind_id(endpoint);
        let remote_device = endpoint.presence.device_id.clone();
        let remote_node = endpoint.presence.node_id.clone();
        let nonce = random_token(12).map_err(eio)?;
        let payload = session_payload(
            kind,
            &relation_id,
            &identity.device_id,
            &remote_device,
            &identity.node_id,
            &remote_node,
            &nonce,
        );
        let proof = hmac_proof(&endpoint.relation_secret, &payload);
        let hello = PeerHello {
            protocol_version: 3,
            relation_kind: kind.to_string(),
            relation_id,
            device_id: identity.device_id.clone(),
            public_key: identity.public_key.clone(),
            node_id: identity.node_id.clone(),
            session_nonce: nonce,
            session_proof: proof,
            requested_capabilities: vec!["fs".to_string(), "fs_walk_batches_v1".to_string()],
        };
        let started = Instant::now();
        self.block_on(io_deadline::run("peer session handshake", async {
            let connection = self
                .endpoint
                .connect(addr, ALPN)
                .await
                .map_err(io_deadline::disconnected)?;
            let (mut send, mut recv) = connection
                .open_bi()
                .await
                .map_err(io_deadline::disconnected)?;
            send_ctrl(&mut send, &Ctrl::PeerHello { hello }).await?;
            match recv_ctrl(&mut recv).await? {
                Ctrl::PeerHelloOk => {
                    let transport = transport_label(&connection);
                    let _ = self.ev.send(ShareEvent::Status(format!(
                        "Iroh-Session authentifiziert: {} via {} in {} ms",
                        remote_device,
                        transport,
                        started.elapsed().as_millis()
                    )));
                    Ok(connection)
                }
                Ctrl::FsResp {
                    resp: FsResponse::Err { msg, .. },
                } => Err(eio(msg)),
                _ => Err(eio("Peer akzeptiert die Iroh-Session nicht")),
            }
        }))
    }

    fn spawn_accept_loop(self: &Arc<Self>) {
        let node = self.clone();
        self.rt.spawn(async move {
            while let Some(incoming) = node.endpoint.accept().await {
                let node = node.clone();
                tokio::spawn(async move {
                    match incoming.await {
                        Ok(connection) => {
                            if let Err(error) =
                                super::server::handle_connection(node.clone(), connection).await
                            {
                                let _ = node
                                    .ev
                                    .send(ShareEvent::Error(format!("Iroh-Peer: {error}")));
                            }
                        }
                        Err(error) => node.emit_accept_error(error.to_string()),
                    }
                });
            }
        });
    }

    fn emit_accept_error(&self, message: String) {
        let mut send = Some(format!("Iroh-Accept: {message}"));
        if let Ok(mut seen) = self.accept_errors.lock() {
            let now = Instant::now();
            let entry = seen.entry(message.clone()).or_insert((now, 0));
            entry.1 = entry.1.saturating_add(1);
            if entry.1 > 1 && entry.0.elapsed() < Duration::from_secs(30) {
                send = None;
            } else if entry.1 > 1 {
                send = Some(format!(
                    "Iroh-Accept: {message} ({} Wiederholungen in 30s)",
                    entry.1
                ));
                *entry = (now, 1);
            } else {
                entry.0 = now;
            }
        }
        if let Some(message) = send {
            let _ = self.ev.send(ShareEvent::Error(message));
        }
    }
}

pub(super) struct IncomingConnectionGuard {
    node: Weak<ShareIrohNode>,
    id: u64,
}

impl Drop for IncomingConnectionGuard {
    fn drop(&mut self) {
        let Some(node) = self.node.upgrade() else {
            return;
        };
        if let Ok(mut sessions) = node.incoming_sessions.lock() {
            sessions.remove(&self.id);
        };
    }
}
