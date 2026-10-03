use std::io;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{Connection, VarInt};

use super::connection_events::ConnectionErrorKind;
use super::core::eio;
use super::exec_protocol::EXEC_ALPN;
use super::handshake_limits::ApplicationHandshakePermit;
use super::node::{ShareIrohNode, ALPN};

impl ShareIrohNode {
    pub(super) fn spawn_accept_loop(self: &Arc<Self>) {
        let node = self.clone();
        self.rt.spawn(async move {
            while let Some(incoming) = node.endpoint.accept().await {
                if node.require_sharing_active().is_err() {
                    incoming.refuse();
                    continue;
                }
                let permit = match node.handshake_slots.clone().try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        incoming.refuse();
                        continue;
                    }
                };
                let node = node.clone();
                tokio::spawn(async move {
                    // TLS identity is not known before this bounded phase.
                    let connected = tokio::time::timeout(UNKNOWN_HANDSHAKE, incoming).await;
                    drop(permit);
                    let connection = match connected {
                        Ok(Ok(connection)) => connection,
                        Ok(Err(error)) => {
                            node.emit_connection_error(ConnectionErrorKind::Accept, error.to_string());
                            return;
                        }
                        Err(_) => return,
                    };
                    let remote = connection.remote_id().to_string();
                    let known = node.known_endpoint(&remote);
                    let timeout = if known { KNOWN_HANDSHAKE } else { UNKNOWN_HANDSHAKE };
                    let deadline = tokio::time::Instant::now() + timeout;
                    let ticket = match node.application_handshakes.enqueue(remote, known) {
                        Ok(ticket) => ticket,
                        Err(_) => { refuse(&connection); return; }
                    };
                    let admitted = match tokio::time::timeout_at(deadline, ticket.acquire()).await {
                        Ok(Ok(permit)) => permit,
                        _ => { refuse(&connection); return; }
                    };
                    let (permit, completion) = ApplicationHandshakePermit::admitted(admitted);
                    let error_kind = match connection.alpn() {
                        ALPN => ConnectionErrorKind::FsConnection,
                        EXEC_ALPN => ConnectionErrorKind::ExecConnection,
                        _ => ConnectionErrorKind::Accept,
                    };
                    let dispatch = dispatch_connection(node.clone(), connection.clone(), permit);
                    tokio::pin!(dispatch);
                    // Dropping the application permit after verified Hello ends
                    // only the handshake deadline, never a live FS/Exec session.
                    let result = tokio::select! {
                        result = &mut dispatch => result,
                        () = completion.wait() => dispatch.await,
                        () = tokio::time::sleep_until(deadline) => {
                            refuse(&connection);
                            Err(io::Error::new(io::ErrorKind::TimedOut, "Share-Anmeldung abgelaufen"))
                        }
                    };
                    if let Err(error) = result {
                        node.emit_connection_error(error_kind, error.to_string());
                    }
                });
            }
        });
    }

    fn known_endpoint(&self, remote: &str) -> bool {
        let Ok(auth) = self.auth.lock() else { return false };
        let matches = |key: &str, node: &str| !remote.is_empty()
            && ((!node.is_empty() && node == remote) || (node.is_empty() && key == remote));
        auth.direct_grants.iter().any(|grant| grant.state == super::types::DirectGrantState::Accepted
            && matches(&grant.public_key, &grant.node_id))
            || auth.direct_contacts.iter().any(|contact| contact.access_state == super::types::DirectAccessState::Accepted
                && matches(contact.remote_public_key.as_deref().or(contact.accepted_public_key.as_deref()).unwrap_or_default(),
                    &contact.expected_node_id))
            || auth.rooms.iter().filter(|room| room.auto_join).any(|room| room.members.iter()
                .any(|member| member.is_admitted() && matches(&member.public_key, &member.node_id)))
    }
}

// Unknown application peers cannot hold the 20 s window reserved for pinned
// identities. Separate pools preserve that capacity under untrusted load.
const UNKNOWN_HANDSHAKE: Duration = Duration::from_secs(3);
const KNOWN_HANDSHAKE: Duration = Duration::from_secs(20);
fn refuse(connection: &Connection) {
    connection.close(VarInt::from_u32(2), b"Share admission rejected");
}

async fn dispatch_connection(
    node: Arc<ShareIrohNode>,
    connection: Connection,
    permit: ApplicationHandshakePermit,
) -> io::Result<()> {
    match connection.alpn() {
        ALPN => super::server::handle_connection(node, connection, permit).await,
        EXEC_ALPN => super::exec_server::handle_connection(node, connection, permit).await,
        _ => Err(eio("Unbekanntes Share-Protokoll")),
    }
}
