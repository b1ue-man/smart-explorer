use std::io;
use std::sync::Arc;

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
                    match incoming.await {
                        Ok(connection) => {
                            let remote = connection.remote_id().to_string();
                            let peer_permit = match node.peer_handshake_slots.try_acquire(&remote) {
                                Ok(permit) => permit,
                                Err(_) => {
                                    connection.close(
                                        VarInt::from_u32(2),
                                        b"application handshake admission limit reached",
                                    );
                                    return;
                                }
                            };
                            let permit = ApplicationHandshakePermit::new(permit, peer_permit);
                            let error_kind = match connection.alpn() {
                                ALPN => ConnectionErrorKind::FsConnection,
                                EXEC_ALPN => ConnectionErrorKind::ExecConnection,
                                _ => ConnectionErrorKind::Accept,
                            };
                            if let Err(error) =
                                dispatch_connection(node.clone(), connection, permit).await
                            {
                                node.emit_connection_error(error_kind, error.to_string());
                            }
                        }
                        Err(error) => node
                            .emit_connection_error(ConnectionErrorKind::Accept, error.to_string()),
                    }
                });
            }
        });
    }
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
