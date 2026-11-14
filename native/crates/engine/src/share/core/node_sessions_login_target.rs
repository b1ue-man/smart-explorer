//! Viewer side of `login_target_v1` (E22, E43): after every authenticated
//! outgoing Share session (`node_sessions::connect_session_until`) the
//! viewer asks the host for the public target of its machine login gateway
//! and keeps it bound to the host's device ID and Share key. A host without
//! the feature is noted as an older peer (`login_target_legacy_peer`); a
//! withdrawal deletes the stored target. The fetch never delays or fails
//! the session: it starts after the handshake on the node runtime, and its
//! errors only reach the log.
use std::io;
use std::sync::{Arc, Mutex};

use iroh::endpoint::Connection;

use super::SessionPeer;
use crate::share::framing::{recv_ctrl, send_ctrl};
use crate::share::io_deadline;
use crate::share::node::ShareIrohNode;
use crate::share::types::{
    DirectAccessState, PeerEndpoint, ShareAuthState, ShareEvent, ShareScope,
};
use crate::share::wire::{Ctrl, FsRequest, FsResponse};

impl ShareIrohNode {
    /// Starts the background fetch for the new session `connection` to
    /// `endpoint` and hands the connection back unchanged.
    pub(in crate::share) fn with_login_target(
        &self,
        endpoint: &PeerEndpoint,
        connection: Connection,
    ) -> Connection {
        let (auth, events) = (self.auth.clone(), self.ev.clone());
        let (endpoint, session) = (endpoint.clone(), connection.clone());
        self.rt.spawn(async move {
            let device = endpoint.presence.device_id.clone();
            let fetched = tokio::time::timeout(
                io_deadline::PEER_OP_TIMEOUT,
                fetch(&session, auth, endpoint),
            )
            .await;
            let error = match fetched {
                Ok(Ok(())) => return,
                Ok(Err(error)) => error,
                Err(_) => io::Error::new(io::ErrorKind::TimedOut, "keine Antwort"),
            };
            let _ = events.try_send(ShareEvent::Status(format!(
                "Anmeldeziel von {device} nicht abgerufen: {error}"
            )));
        });
        connection
    }
}

async fn fetch(
    connection: &Connection,
    auth: Arc<Mutex<ShareAuthState>>,
    endpoint: PeerEndpoint,
) -> io::Result<()> {
    let remote = connection.remote_id().to_string();
    // A thread may hold the state while it builds this very session: the
    // lock is taken off the session's path, on the blocking pool.
    let peer = blocking(move || {
        let state = auth
            .lock()
            .map_err(|_| io::Error::other("Share-State gesperrt"))?;
        Ok(session_peer(&state, &endpoint, &remote))
    })
    .await?;
    let capabilities = FsRequest::Capabilities {
        path: "/".into(),
        acquire_lease: false,
        lease_request_id: None,
    };
    let features = match request(connection, capabilities).await? {
        FsResponse::Capabilities { capabilities, .. } => capabilities.features,
        FsResponse::Err { msg, .. } => return Err(io::Error::other(msg)),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unerwartete Antwort auf die Merkmalsabfrage",
            ))
        }
    };
    if !super::asks_login_target(&features) {
        return blocking(move || super::viewer_login_target_unsupported(&peer).map(drop)).await;
    }
    let message = super::login_answer(request(connection, FsRequest::LoginTarget).await?)?;
    blocking(move || super::viewer_login_target(&peer, &message).map(drop)).await
}

/// One request on its own stream of the session, like every Share request.
async fn request(connection: &Connection, req: FsRequest) -> io::Result<FsResponse> {
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(io_deadline::disconnected)?;
    send_ctrl(&mut send, &Ctrl::Fs { req, lease: None }).await?;
    match recv_ctrl(&mut recv).await? {
        Ctrl::FsResp { resp } => Ok(resp),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Antwort der Gegenstelle ist keine Dateiantwort",
        )),
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(io::Error::other)?
}

/// The session as the target book binds it. `pinned` holds only when the
/// TLS-authenticated node is the identity this device pinned for the
/// relation: the accepted Direct contact's `expected_node_id`, or the node
/// of the admitted room member with this device ID and Share key.
fn session_peer(state: &ShareAuthState, endpoint: &PeerEndpoint, remote: &str) -> SessionPeer {
    let presence = &endpoint.presence;
    let pins = |expected: &str| !expected.trim().is_empty() && expected == remote;
    let (relation_id, pinned) = match &endpoint.scope {
        ShareScope::Direct { contact_id } => {
            let pinned = state.direct_contacts.iter().any(|contact| {
                contact.id == *contact_id
                    && contact.access_state == DirectAccessState::Accepted
                    && pins(&contact.expected_node_id)
                    && contact
                        .remote_device_id
                        .as_deref()
                        .is_none_or(|device| device == presence.device_id)
            });
            (contact_id.clone(), pinned)
        }
        ShareScope::Room { room_id } => {
            let room = state
                .rooms
                .iter()
                .find(|room| room.room_id == *room_id || room.id == *room_id);
            let pinned = room.is_some_and(|room| {
                room.members.iter().any(|member| {
                    member.device_id == presence.device_id
                        && member.public_key == presence.public_key
                        && member.is_admitted()
                        && pins(&member.node_id)
                })
            });
            let relation = room.map_or_else(|| room_id.clone(), |room| room.id.clone());
            (relation, pinned)
        }
    };
    SessionPeer {
        device_id: presence.device_id.clone(),
        public_key: presence.public_key.clone(),
        node_id: remote.to_string(),
        device_name: endpoint.label.clone(),
        relation_id,
        pinned,
    }
}
