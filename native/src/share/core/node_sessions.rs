use std::io;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use iroh::endpoint::{Connection, RecvStream, SendStream, VarInt};

use super::core::{eio, hmac_proof, public_fingerprint, random_token};
use super::direct_protocol::DirectPeerIdentity;
use super::direct_reciprocal::DirectRelationMaterial;
use super::direct_reciprocal_session::{
    AuthenticatedDirectSession, DirectRepairSessionError, DirectSessionAuthorization,
};
use super::direct_reciprocal_transport::DirectReciprocalTransportResult;
use super::direct_reciprocal_wire::DIRECT_RECIPROCAL_CAPABILITY;
use super::framing::{recv_ctrl, send_ctrl};
use super::identity::ShareIdentity;
use super::io_deadline;
use super::node::{ShareIrohNode, ALPN};
use super::session::{
    endpoint_addr, relation_kind_id, session_key, session_payload, transport_label,
    AuthorizedDirectRepair,
};
use super::types::{PeerEndpoint, ShareEvent};
use super::wire::{Ctrl, FsResponse, PeerHello};

pub(super) struct OpenedPeerStream {
    pub(super) send: SendStream,
    pub(super) recv: RecvStream,
    pub(super) session_key: String,
    pub(super) generation: usize,
}

impl ShareIrohNode {
    /// Runs reciprocal repair on a new bi-stream of the cached filesystem
    /// session. Repair failures never evict or close that cached connection.
    pub(crate) fn repair_direct_reciprocal(
        &self,
        endpoint: &PeerEndpoint,
        identity: &ShareIdentity,
        expected_generation: u64,
    ) -> DirectReciprocalTransportResult {
        if self.require_sharing_active().is_err() {
            return DirectReciprocalTransportResult::Transient;
        }
        let transition = match self.runtime_transition_slot.clone().try_acquire_owned() {
            Ok(transition) => transition,
            Err(_) => return DirectReciprocalTransportResult::Transient,
        };
        let runtime_guard =
            super::direct_reciprocal_transport::direct_repair_runtime_guard(transition, None);
        let current_authorization = match self.auth.lock() {
            Ok(state) => state.direct_online && state.authorization_epoch == expected_generation,
            Err(_) => return DirectReciprocalTransportResult::Transient,
        };
        if !current_authorization {
            return DirectReciprocalTransportResult::PolicyDenied;
        }
        let (kind, relation_id) = relation_kind_id(endpoint);
        if kind != "direct" {
            return DirectReciprocalTransportResult::PolicyDenied;
        }
        let deadline = Instant::now() + io_deadline::PEER_OP_TIMEOUT;
        let key = session_key(endpoint);
        let expected_epoch = self.session_epoch.load(Ordering::Acquire);
        let connection = match self.session_connection_until(
            &key,
            endpoint,
            identity,
            expected_epoch,
            deadline,
        ) {
            Ok(connection) => connection,
            Err(error) => return classify_repair_setup_io(&error),
        };
        let session = match AuthenticatedDirectSession::from_verified_handshake(
            endpoint.presence.device_id.clone(),
            connection.remote_id().to_string(),
            endpoint.presence.public_key.clone(),
            endpoint.presence.fingerprint.clone(),
            endpoint.expected_node_id.clone().unwrap_or_default(),
            DirectSessionAuthorization::OutgoingAcceptedContact,
            true,
        ) {
            Ok(session) => session,
            Err(error) => return classify_repair_session_setup(error),
        };
        let local_material = match DirectRelationMaterial::new(
            identity.direct_lookup_id.clone(),
            identity.direct_secret(),
        ) {
            Ok(material) => material,
            Err(_) => return DirectReciprocalTransportResult::Conflict,
        };
        let expected_remote_material = match DirectRelationMaterial::new(
            relation_id,
            endpoint.relation_secret.clone(),
        ) {
            Ok(material) => material,
            Err(_) => return DirectReciprocalTransportResult::Conflict,
        };
        let authorized = AuthorizedDirectRepair {
            local_identity: DirectPeerIdentity {
                device_id: identity.device_id.clone(),
                device_name: identity.device_name.clone(),
                node_id: identity.node_id.clone(),
                public_key: identity.public_key.clone(),
                fingerprint: public_fingerprint(identity.public_key.as_bytes()),
            },
            local_material,
            session,
            expected_remote_material: Some(expected_remote_material),
        };
        let timeout = match io_deadline::remaining(deadline, "reciprocal Direct repair") {
            Ok(timeout) => timeout,
            Err(_) => return DirectReciprocalTransportResult::Transient,
        };
        let store = self.direct_repair_store.clone();
        match self.block_on(tokio::time::timeout(
            timeout,
            super::direct_reciprocal_transport::run_outgoing(
                connection,
                authorized,
                store,
                runtime_guard,
            ),
        )) {
            Ok(result) => result,
            Err(_) => DirectReciprocalTransportResult::Transient,
        }
    }

    pub(super) fn open_stream(
        &self,
        endpoint: &PeerEndpoint,
        identity: &ShareIdentity,
    ) -> io::Result<OpenedPeerStream> {
        self.open_stream_until(
            endpoint,
            identity,
            Instant::now() + io_deadline::PEER_OP_TIMEOUT,
        )
    }

    pub(super) fn open_stream_until(
        &self,
        endpoint: &PeerEndpoint,
        identity: &ShareIdentity,
        deadline: Instant,
    ) -> io::Result<OpenedPeerStream> {
        self.require_sharing_active()?;
        let key = session_key(endpoint);
        let expected_epoch = self.session_epoch.load(Ordering::Acquire);
        let connection =
            self.session_connection_until(&key, endpoint, identity, expected_epoch, deadline)?;
        match self.open_on_connection(&key, &connection, deadline, "peer stream open") {
            Ok(stream) => Ok(stream),
            Err(_) => {
                // No operation payload was sent yet. Replacing the failed
                // physical connection here cannot replay a mutation.
                self.invalidate_outgoing_session(&key, connection.stable_id())?;
                let replacement = self.session_connection_until(
                    &key,
                    endpoint,
                    identity,
                    expected_epoch,
                    deadline,
                )?;
                self.open_on_connection(&key, &replacement, deadline, "peer stream reopen")
            }
        }
    }

    pub(super) fn invalidate_outgoing_session(
        &self,
        key: &str,
        failed_generation: usize,
    ) -> io::Result<bool> {
        let failed = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| eio("Ausgehende Share-Sessions sind gesperrt"))?;
            match sessions.get(key) {
                Some(current) if current.stable_id() == failed_generation => sessions.remove(key),
                _ => None,
            }
        };
        // Removing only the exact failed generation makes future operations
        // reconnect, but does not actively close unrelated concurrent streams
        // (notably an in-flight mutation) that still own this connection.
        Ok(failed.is_some())
    }

    pub(super) fn session_transport(&self, endpoint: &PeerEndpoint) -> Option<&'static str> {
        let key = session_key(endpoint);
        let connection = self.sessions.lock().ok()?.get(&key).cloned()?;
        connection
            .close_reason()
            .is_none()
            .then(|| transport_label(&connection))
    }

    fn open_on_connection(
        &self,
        key: &str,
        connection: &Connection,
        deadline: Instant,
        operation: &'static str,
    ) -> io::Result<OpenedPeerStream> {
        let timeout = io_deadline::remaining(deadline, operation)?;
        let (send, recv) = self.block_on(io_deadline::run_for(operation, timeout, async {
            connection
                .open_bi()
                .await
                .map_err(io_deadline::disconnected)
        }))?;
        Ok(OpenedPeerStream {
            send,
            recv,
            session_key: key.to_string(),
            generation: connection.stable_id(),
        })
    }

    fn session_connection_until(
        &self,
        key: &str,
        endpoint: &PeerEndpoint,
        identity: &ShareIdentity,
        expected_epoch: u64,
        deadline: Instant,
    ) -> io::Result<Connection> {
        if let Some(connection) = self.healthy_cached_session(key)? {
            return Ok(connection);
        }
        let connect_gate = {
            let mut gates = self
                .session_connects
                .lock()
                .map_err(|_| eio("Share-Verbindungsaufbau ist gesperrt"))?;
            gates.retain(|_, gate| gate.strong_count() > 0);
            if let Some(gate) = gates.get(key).and_then(std::sync::Weak::upgrade) {
                gate
            } else {
                let gate = Arc::new(Mutex::new(()));
                gates.insert(key.to_string(), Arc::downgrade(&gate));
                gate
            }
        };
        let _singleflight = connect_gate
            .lock()
            .map_err(|_| eio("Share-Verbindungsaufbau ist gesperrt"))?;
        if let Some(connection) = self.healthy_cached_session(key)? {
            return Ok(connection);
        }
        if self.session_epoch.load(Ordering::Acquire) != expected_epoch {
            return Err(eio(
                "Share-Autorisierung wurde vor dem Verbindungsaufbau geaendert",
            ));
        }
        let connection = self.connect_session_until(endpoint, identity, deadline)?;
        self.cache_session(key.to_string(), connection.clone(), expected_epoch)?;
        Ok(connection)
    }

    fn healthy_cached_session(&self, key: &str) -> io::Result<Option<Connection>> {
        let cached = self
            .sessions
            .lock()
            .map_err(|_| eio("Ausgehende Share-Sessions sind gesperrt"))?
            .get(key)
            .cloned();
        let Some(connection) = cached else {
            return Ok(None);
        };
        if connection.close_reason().is_none() {
            return Ok(Some(connection));
        }
        self.invalidate_outgoing_session(key, connection.stable_id())?;
        Ok(None)
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

    fn connect_session_until(
        &self,
        endpoint: &PeerEndpoint,
        identity: &ShareIdentity,
        deadline: Instant,
    ) -> io::Result<Connection> {
        if let Some(expected) = endpoint.expected_node_id.as_deref() {
            if !expected.trim().is_empty() && expected != endpoint.presence.node_id {
                return Err(eio("Iroh NodeId passt nicht zur gepinnten Identitaet"));
            }
        }
        let local_addr = self.routes.current(&self.endpoint);
        let addr = endpoint_addr(&endpoint.presence, &local_addr)?;
        let (kind, relation_id) = relation_kind_id(endpoint);
        let mut requested_capabilities = vec!["fs".to_string(), "fs_walk_batches_v1".to_string()];
        if kind == "direct" {
            requested_capabilities.push(DIRECT_RECIPROCAL_CAPABILITY.to_string());
        }
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
            requested_capabilities,
        };
        let started = Instant::now();
        let timeout = io_deadline::remaining(deadline, "peer session handshake")?;
        self.block_on(io_deadline::run_for(
            "peer session handshake",
            timeout,
            async {
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
                        let _ = self.ev.try_send(ShareEvent::Status(format!(
                            "Iroh-Session authentifiziert: {} via {} in {} ms",
                            remote_device,
                            transport,
                            started.elapsed().as_millis()
                        )));
                        Ok(connection)
                    }
                    Ctrl::FsResp {
                        resp: FsResponse::Err { msg, .. },
                    } => Err(io::Error::new(io::ErrorKind::PermissionDenied, msg)),
                    _ => Err(eio("Peer akzeptiert die Iroh-Session nicht")),
                }
            },
        ))
    }

    #[cfg(test)]
    pub(super) fn outgoing_generation_for_test(
        &self,
        endpoint: &PeerEndpoint,
    ) -> io::Result<Option<usize>> {
        let key = session_key(endpoint);
        Ok(self
            .sessions
            .lock()
            .map_err(|_| eio("Ausgehende Share-Sessions sind gesperrt"))?
            .get(&key)
            .map(Connection::stable_id))
    }

    #[cfg(test)]
    pub(super) fn disconnect_outgoing_for_test(&self, endpoint: &PeerEndpoint) -> io::Result<bool> {
        let key = session_key(endpoint);
        let generation = self
            .sessions
            .lock()
            .map_err(|_| eio("Ausgehende Share-Sessions sind gesperrt"))?
            .get(&key)
            .map(Connection::stable_id);
        match generation {
            Some(generation) => self.invalidate_outgoing_session(&key, generation),
            None => Ok(false),
        }
    }
}

fn classify_repair_setup_io(error: &io::Error) -> DirectReciprocalTransportResult {
    match error.kind() {
        io::ErrorKind::PermissionDenied => DirectReciprocalTransportResult::PolicyDenied,
        io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput => {
            DirectReciprocalTransportResult::Conflict
        }
        _ => DirectReciprocalTransportResult::Transient,
    }
}

fn classify_repair_session_setup(
    error: DirectRepairSessionError,
) -> DirectReciprocalTransportResult {
    match error {
        DirectRepairSessionError::PolicyDenied => DirectReciprocalTransportResult::PolicyDenied,
        DirectRepairSessionError::CapabilityNotRequested => {
            DirectReciprocalTransportResult::Unsupported
        }
        _ => DirectReciprocalTransportResult::Conflict,
    }
}
