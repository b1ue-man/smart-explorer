//! The viewer side of the screen protocol on the share node (SCR-M2): the
//! endpoint of a direct device or a room member as the share service
//! resolves it, the connection with the screen ALPN (an older device
//! refuses it with `0x178`, `screen_unsupported`; when its refusal never
//! reaches the viewer, the file-session ALPN tells a reachable older device
//! from an unreachable one) and the client hello, signed with the relation
//! secret after the host's identity matched the pinned one.
use std::io;
use std::time::Duration;

use iroh::endpoint::{Connection, VarInt};

use crate::screen::core::viewer_client::{is_unsupported_alpn, ViewerError, ViewerRequest};
use crate::screen::core::wire::{SCREEN_ALPN, SCREEN_PROTOCOL_VERSION};
use crate::screen::core::wire_hello::{screen_transcript, ScreenClientHello, ScreenServerHello};
use crate::share::core::{eio, hmac_proof, now_secs, random_token};
use crate::share::io_deadline;
use crate::share::node::{ShareIrohNode, ALPN};
use crate::share::profiles::{fingerprint_matches, ShareProfiles};
use crate::share::service::ShareService;
use crate::share::session::{endpoint_addr, relation_kind_id, PeerPrincipal};
use crate::share::types::{PeerEndpoint, PeerOpenTarget, ShareAuthState, ShareScope};
use crate::share::ShareIdentity;

/// How long the file-session probe after a silent screen handshake may take.
const SCREEN_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Whether a connect error is the QUIC idle timeout of a handshake that the
/// peer never answered (`ConnectionError::TimedOut`, displayed "timed out").
fn is_handshake_timeout(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(error) = current {
        if matches!(
            error.downcast_ref::<iroh::endpoint::ConnectionError>(),
            Some(iroh::endpoint::ConnectionError::TimedOut)
        ) || error.to_string() == "timed out"
        {
            return true;
        }
        current = error.source();
    }
    false
}

impl ShareIrohNode {
    /// Whether the device at `addr` completes a handshake with the file
    /// session ALPN, which every Share device speaks. After a screen
    /// handshake that nobody answered, a yes means the device is reachable
    /// and just does not know the screen ALPN: its refusal (TLS alert 120,
    /// QUIC `0x178`) is lost when the server finds the ALPN mismatch only in
    /// the second Initial packet of the post-quantum ClientHello, because
    /// noq-proto 1.3 then sends no CONNECTION_CLOSE to a client whose address
    /// it has not validated (`connection/mod.rs`, the close after a packet
    /// while closed requires `validated`; only a refusal on the first Initial
    /// is answered by `Endpoint::accept`).
    async fn answers_file_alpn(&self, addr: iroh::EndpointAddr) -> bool {
        match tokio::time::timeout(SCREEN_PROBE_TIMEOUT, self.endpoint.connect(addr, ALPN)).await {
            Ok(Ok(connection)) => {
                connection.close(VarInt::from_u32(0), b"screen capability probe");
                true
            }
            _ => false,
        }
    }

    /// Viewer side: a connection with the screen ALPN to `endpoint`.
    pub(in crate::share) async fn connect_screen(
        &self,
        endpoint: &PeerEndpoint,
    ) -> Result<Connection, ViewerError> {
        self.require_sharing_active()?;
        if let Some(expected) = endpoint.expected_node_id.as_deref() {
            if !expected.trim().is_empty() && expected != endpoint.presence.node_id {
                return Err(eio("Iroh NodeId passt nicht zur gepinnten Identitaet").into());
            }
        }
        let local_addr = self.routes.current(&self.endpoint);
        let addr = endpoint_addr(&endpoint.presence, &local_addr, &self.transport_options)?;
        let generation = self
            .policy
            .snapshot(PeerPrincipal::from_endpoint(endpoint))?;
        let connecting = self.endpoint.connect(addr.clone(), SCREEN_ALPN);
        let connection = match tokio::time::timeout(io_deadline::PEER_OP_TIMEOUT, connecting).await
        {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) if is_unsupported_alpn(&error) => return Err(ViewerError::Unsupported),
            Ok(Err(error)) if is_handshake_timeout(&error) => {
                return if self.answers_file_alpn(addr).await {
                    Err(ViewerError::Unsupported)
                } else {
                    Err(io_deadline::disconnected(error).into())
                };
            }
            Ok(Err(error)) => return Err(io_deadline::disconnected(error).into()),
            Err(_) => {
                let error =
                    io::Error::new(io::ErrorKind::TimedOut, "peer screen connection timed out");
                return Err(error.into());
            }
        };
        if let Err(error) = self.policy.bind(&connection, generation) {
            connection.close(
                VarInt::from_u32(0x5345),
                b"authorization changed during handshake",
            );
            return Err(error.into());
        }
        Ok(connection)
    }
}

/// The viewer's way to one screen host: the connection and what signs its
/// client hello.
pub(crate) struct ScreenLink {
    pub(crate) connection: Connection,
    pub(super) endpoint: PeerEndpoint,
    pub(super) identity: ShareIdentity,
}

impl ScreenLink {
    /// The client hello for `server`, after the host's identity matched the
    /// pinned one, with the relation proof over the screen transcript.
    pub(crate) fn client_hello(
        &self,
        server: &ScreenServerHello,
        request: &ViewerRequest,
    ) -> io::Result<ScreenClientHello> {
        let presence = &self.endpoint.presence;
        let pinned = server.server_device_id == presence.device_id
            && server.server_public_key == presence.public_key
            && server.server_fingerprint == presence.fingerprint
            && server.server_node_id == presence.node_id
            && fingerprint_matches(&server.server_public_key, &server.server_fingerprint)
            && self
                .endpoint
                .expected_node_id
                .as_ref()
                .is_none_or(|expected| expected.is_empty() || expected == &server.server_node_id);
        if !pinned {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "screen host identity does not match the pinned device",
            ));
        }
        let (relation_kind, relation_id) = relation_kind_id(&self.endpoint);
        let identity = &self.identity;
        let mut hello = ScreenClientHello {
            protocol_version: SCREEN_PROTOCOL_VERSION,
            relation_kind: relation_kind.into(),
            relation_id,
            device_id: identity.device_id.clone(),
            device_name: identity.device_name.clone(),
            public_key: identity.public_key.clone(),
            fingerprint: identity.fingerprint.clone(),
            node_id: identity.node_id.clone(),
            client_nonce: random_token(18).map_err(eio)?,
            proof: String::new(),
            mode: request.mode,
            source: request.source.clone(),
            decoders: request.decoders.clone(),
            display_size: request.display_size,
            max_fps: request.max_fps,
        };
        hello.proof = hmac_proof(
            &self.endpoint.relation_secret,
            &screen_transcript(server, &hello),
        );
        Ok(hello)
    }

    /// How this device names the host.
    pub(crate) fn label(&self) -> &str {
        &self.endpoint.label
    }
}

impl ShareService {
    /// The runtime the screen connections of this service run on.
    pub(crate) fn screen_runtime(&self) -> tokio::runtime::Handle {
        self.iroh.rt.handle().clone()
    }

    /// Connects to the screen host of `target` (blocking: daemon threads).
    pub(crate) fn screen_link(&self, target: &PeerOpenTarget) -> Result<ScreenLink, ViewerError> {
        let endpoint = {
            let state = self.auth.lock().map_err(|_| eio("Share-State gesperrt"))?;
            endpoint_for_screen(&state, target, now_secs()).map_err(eio)?
        };
        let connection = self.iroh.block_on(self.iroh.connect_screen(&endpoint))?;
        Ok(ScreenLink {
            connection,
            endpoint,
            identity: self.identity.clone(),
        })
    }
}

/// The endpoint of `target` as `ShareService::endpoint_for_target` resolves
/// it (same checks and texts; that function is private to `service.rs`).
fn endpoint_for_screen(
    state: &ShareAuthState,
    target: &PeerOpenTarget,
    now: i64,
) -> Result<PeerEndpoint, String> {
    match target {
        PeerOpenTarget::Direct { contact_id } => {
            let contact = state
                .direct_contacts
                .iter()
                .find(|contact| &contact.id == contact_id)
                .ok_or_else(|| "Direktgeraet nicht gefunden".to_string())?;
            let presence = crate::share::lan_presence_match::effective_presence(contact, now)
                .ok_or_else(|| {
                    if contact.presence.is_some() || contact.lan_seen_at.is_some() {
                        "Direktgeraet ist nicht online (gespeicherte Presence ist abgelaufen)"
                            .to_string()
                    } else {
                        "Direktgeraet ist nicht online".to_string()
                    }
                })?;
            let secret = ShareProfiles::direct_secret_checked(contact)?
                .ok_or_else(|| "Direkt-Secret fehlt".to_string())?;
            let expected_node_id = if contact.expected_node_id.trim().is_empty() {
                Some(presence.node_id.clone())
            } else {
                Some(contact.expected_node_id.clone())
            };
            Ok(PeerEndpoint {
                label: format!("Share Direkt: {}", contact.display_name),
                scope: ShareScope::Direct {
                    contact_id: contact.id.clone(),
                },
                presence,
                relation_secret: secret,
                expected_node_id,
            })
        }
        PeerOpenTarget::RoomDevice { room_id, device_id } => {
            let room = state
                .rooms
                .iter()
                .find(|room| &room.id == room_id || &room.room_id == room_id)
                .ok_or_else(|| "Raum nicht gefunden".to_string())?;
            let member = room
                .members
                .iter()
                .find(|member| &member.device_id == device_id)
                .ok_or_else(|| "Geraet nicht im Raum".to_string())?;
            if member.blocked {
                return Err("Geraet ist blockiert".into());
            }
            let presence = member
                .presence
                .clone()
                .ok_or_else(|| "Raumgeraet ist nicht online".to_string())?;
            if !presence.is_current_at(now) {
                return Err(
                    "Raumgeraet ist nicht online (gespeicherte Presence ist abgelaufen)".into(),
                );
            }
            let secret = ShareProfiles::room_secret_checked(room)?
                .ok_or_else(|| "Raum-Secret fehlt".to_string())?;
            Ok(PeerEndpoint {
                label: format!("Share Raum {} / {}", room.name, member.device_name),
                scope: ShareScope::Room {
                    room_id: room.room_id.clone(),
                },
                presence,
                relation_secret: secret,
                expected_node_id: Some(member.node_id.clone()),
            })
        }
    }
}
