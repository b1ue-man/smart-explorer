use std::io;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{Connection, IncomingAddr, VarInt};

use super::connection_events::ConnectionErrorKind;
use super::core::eio;
use super::exec_protocol::EXEC_ALPN;
use super::handshake_limits::{ApplicationHandshakePermit, TlsAdmission, TlsSource};
use super::identity::code_renewal_registry::is_withdrawn;
use super::keepalive::transport_purpose::AcceptProfiles;
use super::node::{ShareIrohNode, ALPN};
use crate::screen::core::wire::SCREEN_ALPN;

impl ShareIrohNode {
    pub(super) fn spawn_accept_loop(self: &Arc<Self>) {
        let node = self.clone();
        self.rt.spawn(async move {
            // E1: every incoming connection gets the profile of its purpose
            // (`transport_purpose.rs`); the checks below keep their order.
            let profiles = Arc::new(AcceptProfiles::new(&node.endpoint));
            while let Some(incoming) = node.endpoint.accept().await {
                if node.require_sharing_active().is_err() {
                    incoming.refuse();
                    continue;
                }
                // R-ZUGANG-01: per-source cap, Retry under load, reserve
                // for pinned relay senders (`handshake_limits_tls.rs`).
                let addr = incoming.remote_addr();
                let pinned = match &addr {
                    IncomingAddr::Relay { endpoint_id, .. } => {
                        node.known_endpoint(&endpoint_id.to_string())
                    }
                    _ => false,
                };
                // E17: Iroh hands every peer that completes TLS this device's
                // direct addresses (ADD_ADDRESS) before the app can refuse it.
                // A device the user removed, named by the relay, gets no TLS.
                if !pinned && relay_names_withdrawn(&addr) {
                    incoming.refuse();
                    continue;
                }
                let source = TlsSource::of(&addr);
                let validated = incoming.remote_addr_validated();
                let permit = match node.handshake_slots.admit(source, validated, pinned) {
                    TlsAdmission::Slot(permit) => permit,
                    TlsAdmission::Retry => {
                        if let Err(error) = incoming.retry() {
                            error.into_incoming().refuse();
                        }
                        continue;
                    }
                    TlsAdmission::Refuse => {
                        incoming.refuse();
                        continue;
                    }
                };
                let node = node.clone();
                let profiles = profiles.clone();
                tokio::spawn(async move {
                    let (config, purpose) = profiles.for_incoming(&incoming);
                    let accepting = match incoming.accept_with(config) {
                        Ok(accepting) => accepting,
                        Err(error) => {
                            drop(permit);
                            node.emit_connection_error(ConnectionErrorKind::Accept, error.to_string());
                            return;
                        }
                    };
                    // TLS identity is not known before this bounded phase.
                    let connected = tokio::time::timeout(UNKNOWN_HANDSHAKE, accepting).await;
                    drop(permit);
                    let connection = match connected {
                        Ok(Ok(connection)) => connection,
                        Ok(Err(error)) => {
                            node.emit_connection_error(ConnectionErrorKind::Accept, error.to_string());
                            return;
                        }
                        Err(_) => return,
                    };
                    // The late-bound purpose: the controller yields (or not)
                    // by the negotiated ALPN from now on.
                    purpose.bind_alpn(connection.alpn());
                    let remote = connection.remote_id().to_string();
                    let known = node.known_endpoint(&remote);
                    if connection.alpn() == super::lan_link_wire::LAN_LINK_ALPN && !known {
                        refuse(&connection);
                        return;
                    }
                    // A removed device that came in directly: close at once.
                    if !known && is_withdrawn(&remote) {
                        refuse(&connection);
                        return;
                    }
                    // Penalized after failed authentication: Exec and screen
                    // always, file sessions only for unknown endpoints, so
                    // automatic reconnects of paired devices are never throttled.
                    let exec = connection.alpn() == EXEC_ALPN;
                    let screen = connection.alpn() == SCREEN_ALPN;
                    let tcp = super::tcp_forward::tcp_version_for_alpn(connection.alpn()).is_some();
                    let apps = connection.alpn() == super::apps_wire::APPS_ALPN;
                    if super::auth_penalty::refusal_applies(exec || screen || tcp || apps, known) {
                        let now = std::time::Instant::now();
                        if let Some(remaining) = node.auth_penalties().refusal(&remote, now) {
                            let protocol = match (exec, screen, tcp, apps) {
                                (true, _, _, _) => "exec",
                                (_, true, _, _) => "screen",
                                (_, _, true, _) => "tcp",
                                (_, _, _, true) => "apps",
                                _ => "fs",
                            };
                            super::exec_audit::record(super::exec_audit::AuditEvent::PenaltyRefused {
                                remote_node: &remote,
                                protocol,
                                remaining_secs: remaining.as_secs(),
                            });
                            refuse(&connection);
                            return;
                        }
                    }
                    let timeout = if known { KNOWN_HANDSHAKE } else { UNKNOWN_HANDSHAKE };
                    let ticket = match node.application_handshakes.enqueue(remote, known) {
                        Ok(ticket) => ticket,
                        Err(_) => { refuse(&connection); return; }
                    };
                    // Bound unauthenticated identities before they can reserve
                    // a route-monitor slot. This local queue consumes no app
                    // bytes and therefore has an ordinary admission deadline.
                    let admitted = match tokio::time::timeout(timeout, ticket.acquire()).await {
                        Ok(Ok(permit)) => permit,
                        _ => { refuse(&connection); return; }
                    };
                    let (permit, completion) = ApplicationHandshakePermit::admitted(admitted);
                    // SessionHello, HMAC and LAN-link status bytes share the same
                    // transport gate; application time starts only after admission.
                    if node.await_route(&connection, None, None).await.is_err() {
                        refuse(&connection);
                        return;
                    }
                    if connection.alpn() == super::lan_link_wire::LAN_LINK_ALPN {
                        drop(permit);
                        // Its existing pinned-peer admission and refresh/session
                        // deadlines own the long-lived LAN status rounds.
                        let _ = node.accept_lan_link(connection).await;
                        return;
                    }
                    // Screen sessions report as accept errors: the reporter keeps
                    // a fixed set of kinds (connection_events.rs).
                    let error_kind = match connection.alpn() {
                        ALPN => ConnectionErrorKind::FsConnection,
                        EXEC_ALPN => ConnectionErrorKind::ExecConnection,
                        _ => ConnectionErrorKind::Accept,
                    };
                    let dispatch = super::route_protocol::scope(connection.clone(),
                        dispatch_connection(node.clone(), connection.clone(), permit));
                    tokio::pin!(dispatch);
                    // Dropping the application permit after verified Hello ends
                    // only the handshake deadline, never a live FS/Exec session.
                    let result = tokio::select! {
                        result = &mut dispatch => result,
                        admitted = super::route_protocol::run_with_route(&connection, timeout, None, async {
                            completion.wait().await; Ok(())
                        }) => {
                            match admitted {
                                Ok(()) => {
                                    // Hello verified: the host logs its route
                                    // changes too (D1, "über Relay …" → "direkt").
                                    super::route_policy::route_start::log_route_changes(
                                        &connection,
                                        connection.remote_id().to_string(),
                                        node.ev.clone(),
                                    );
                                    dispatch.await
                                }
                                Err(error) => {
                                    // R-ZUGANG-04: a hello that never came is
                                    // OpenSSH's "noauth" penalty; the handler
                                    // that books it was dropped with the
                                    // deadline. Same rule as the refusal above.
                                    if error.kind() == io::ErrorKind::TimedOut
                                        && super::auth_penalty::refusal_applies(
                                            exec || screen || tcp || apps,
                                            known,
                                        )
                                    {
                                        node.auth_penalties().record(
                                            &connection.remote_id().to_string(),
                                            super::auth_penalty::PenaltyKind::NoAuthentication,
                                            std::time::Instant::now(),
                                        );
                                    }
                                    refuse(&connection);
                                    Err(error)
                                }
                            }
                        }
                    };
                    if let Err(error) = result {
                        if !super::node::closed_on_purpose(&connection) {
                            node.emit_connection_error(error_kind, error.to_string());
                        }
                    }
                });
            }
        });
    }

    pub(super) fn known_endpoint(&self, remote: &str) -> bool {
        let Ok(auth) = self.auth.lock() else {
            return false;
        };
        let matches = |key: &str, node: &str| {
            !remote.is_empty()
                && ((!node.is_empty() && node == remote) || (node.is_empty() && key == remote))
        };
        auth.direct_grants.iter().any(|grant| {
            grant.state == super::types::DirectGrantState::Accepted
                && matches(&grant.public_key, &grant.node_id)
        }) || auth.direct_contacts.iter().any(|contact| {
            contact.access_state == super::types::DirectAccessState::Accepted
                && matches(
                    contact
                        .remote_public_key
                        .as_deref()
                        .or(contact.accepted_public_key.as_deref())
                        .unwrap_or_default(),
                    &contact.expected_node_id,
                )
        }) || auth.rooms.iter().filter(|room| room.auto_join).any(|room| {
            room.members
                .iter()
                .any(|member| member.is_admitted() && matches(&member.public_key, &member.node_id))
        })
    }
}

/// The relay names the sender of an incoming connection; Iroh's relay
/// protocol authenticates that id with the sender's key.
fn relay_names_withdrawn(addr: &IncomingAddr) -> bool {
    match addr {
        IncomingAddr::Relay { endpoint_id, .. } => is_withdrawn(&endpoint_id.to_string()),
        _ => false,
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
        ALPN => {
            node.quic_stats
                .track(&connection, super::quic_stats_hub::Side::Host);
            super::server::handle_connection(node, connection, permit).await
        }
        EXEC_ALPN => super::exec_server::handle_connection(node, connection, permit).await,
        SCREEN_ALPN => node.handle_screen_connection(connection, permit).await,
        super::tcp_forward::TCP_ALPN | super::tcp_forward::TCP_ALPN_V1 => {
            node.handle_tcp_connection(connection, permit).await
        }
        super::apps_wire::APPS_ALPN => node.handle_apps_connection(connection, permit).await,
        _ => Err(eio("Unbekanntes Share-Protokoll")),
    }
}

#[cfg(test)]
#[path = "node_accept_withdrawn_tests.rs"]
mod withdrawn_tests;

#[cfg(test)]
#[path = "unbelegt_handshake_mobile_task_tests.rs"]
mod unbelegt_handshake_mobile_tests;
