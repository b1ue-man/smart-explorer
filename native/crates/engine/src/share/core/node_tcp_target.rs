//! Select an actually connected target before freezing the tunnel's exact IP.
use super::*;
use crate::share::access_approval::{ApprovalOutcome, ApprovalSummary};
use crate::share::access_authorize::{principal_grant, request_for};
use crate::share::access_constraints::{
    AccessRequest, NodeId, TcpDirection, TcpForwardRequest, TcpHost,
};
use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::access_revocation::AccessConfirmation;
use crate::share::core::now_secs;
use crate::share::relation_proof::ProvenPrincipal;
use crate::share::tcp_forward::{TcpDestination, TunnelRole, TunnelView};
use crate::share::tcp_registry::TcpRegistration;

/// The refusal a peer reads when no rule of its relation admits the target;
/// the same text whether the name resolves or not (r-exec-2).
pub(super) const TARGET_NOT_AUTHORIZED: &str =
    "TCP target host, address, port or direction is not authorized in this relationship";

pub(super) struct AdmittedTarget {
    pub(super) registration: TcpRegistration,
    pub(super) access: Arc<AccessHandle>,
    pub(super) address: SocketAddr,
    pub(super) socket: crate::share::tcp_sockets::AuthorizedSocket,
}

impl ShareIrohNode {
    pub(super) async fn admit_tcp_target(
        &self,
        connection: &Connection,
        principal: &ProvenPrincipal,
        client: &TcpClientHello,
        server: &TcpServerHello,
        send: &mut iroh::endpoint::SendStream,
    ) -> io::Result<AdmittedTarget> {
        // Freeze host-resolved candidates. Later DNS replies may invalidate a
        // candidate but can never add a new IP under a prior Ask confirmation.
        let addresses = super::super::tcp_sockets::resolve(&client.request.destination).await?;
        let mut last_connect_error = None;
        let mut pending_sent = false;
        for target in addresses {
            let request = {
                let state = self
                    .auth
                    .lock()
                    .map_err(|_| eio("Share authorization locked"))?;
                let request = AccessRequest::TcpForward(TcpForwardRequest {
                    initiator: NodeId::parse(&principal.node_id).map_err(eio)?,
                    target: NodeId::parse(&state.identity.node_id).map_err(eio)?,
                    direction: TcpDirection::PeerToHost,
                    host: client.request.destination.host.clone(),
                    resolved_ip: target.ip(),
                    port: client.request.destination.port,
                });
                if request_for(&state, principal, &request, now_secs()).decision
                    == CapabilityDecision::Off
                {
                    continue;
                }
                request
            };
            // Every IP owns a new concrete request, ticket and live lease.
            // Denial/revocation ends setup; it is never treated as reachability.
            let confirmation = self
                .tcp_confirmation(connection, principal, &request, send, &mut pending_sent)
                .await?;
            let (_, access) = self
                .admit_access(principal, request, confirmation, |_| Ok(()))
                .map_err(access_error)?;
            let access = Arc::new(access);
            let socket = match super::super::tcp_sockets::connect_bound(
                &client.request.destination,
                target,
                access.clone(),
            )
            .await
            {
                Ok(socket) => socket,
                Err(error) if retry_connect_error(&error) => {
                    last_connect_error = Some(error);
                    continue;
                }
                Err(error) => return Err(error),
            };
            let now = now_secs();
            let view = TunnelView {
                id: client.tunnel.clone(),
                role: TunnelRole::Target,
                remote_node: principal.node_id.clone(),
                initiator_node: principal.node_id.clone(),
                target_node: server.node_id.clone(),
                direction: TcpDirection::PeerToHost,
                relation_kind: principal.relation_kind.clone(),
                relation_id: principal.relation_id.clone(),
                destination: client.request.destination.clone(),
                bound_target: Some(target),
                listener: None,
                started_at: now,
                expires_at: client.request.expires_at(now),
                admission: None,
                active_connections: 0,
                socket_read_bytes: 0,
                socket_written_bytes: 0,
                closing: false,
                last_error: None,
            };
            let registration = self
                .admit_existing_access(&access, |_| {
                    self.tcp
                        .registry
                        .insert(view, connection.clone())
                        .map_err(|_| AccessRevocation::Unavailable)
                })
                .map_err(access_error)?;
            return Ok(AdmittedTarget {
                registration,
                access,
                address: target,
                socket,
            });
        }
        Err(last_connect_error.unwrap_or_else(|| {
            io::Error::new(io::ErrorKind::PermissionDenied, TARGET_NOT_AUTHORIZED)
        }))
    }

    /// r-exec-2: whether a rule of the proven relation, or its right to steer
    /// the screen, names `destination` (host, port, direction) before this
    /// host resolves a name or lists its services. A superset of
    /// `request_for` for each resolved IP, which still decides.
    pub(super) fn names_tcp_destination(
        &self,
        principal: &ProvenPrincipal,
        destination: &TcpDestination,
    ) -> io::Result<bool> {
        let state = self
            .auth
            .lock()
            .map_err(|_| eio("Share authorization locked"))?;
        let Some(grant) = principal_grant(&state, principal) else {
            return Ok(false);
        };
        let access = grant.access_grant();
        let now = now_secs();
        if access.expired_at(now) {
            return Ok(false);
        }
        if destination.host == TcpHost::Loopback
            && crate::share::access_steering::steers(&access, now)
        {
            return Ok(true);
        }
        if access.decision(Capability::TcpForward) == CapabilityDecision::Off {
            return Ok(false);
        }
        Ok(access.constraints.tcp_forward.iter().any(|rule| {
            rule.initiator.as_str() == principal.node_id
                && rule.direction == TcpDirection::PeerToHost
                && rule.ports.contains(&destination.port)
                && same_host(&rule.host, &destination.host)
        }))
    }

    async fn tcp_confirmation(
        &self,
        connection: &Connection,
        principal: &ProvenPrincipal,
        request: &AccessRequest,
        send: &mut iroh::endpoint::SendStream,
        pending_sent: &mut bool,
    ) -> io::Result<Option<AccessConfirmation>> {
        let capability = {
            let state = self
                .auth
                .lock()
                .map_err(|_| eio("Share authorization locked"))?;
            request_for(&state, principal, request, now_secs())
        };
        match capability.decision {
            CapabilityDecision::Allow => return Ok(None),
            CapabilityDecision::Off => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "TCP access is off",
                ))
            }
            CapabilityDecision::Ask => {}
        }
        let mut ticket = crate::share::consent_launch::approvals().request_access(
            principal,
            request.clone(),
            ApprovalSummary {
                policy_revision: capability.policy_revision,
                ..Default::default()
            },
        );
        // The existing client expects one Pending transition, even if a later
        // candidate needs a different IP-bound human confirmation.
        if !*pending_sent {
            crate::share::route_protocol::timeout_for(
                Duration::from_secs(20),
                tcp_wire::send_json(send, &TcpHelloReply::Pending),
            )
            .await??;
            *pending_sent = true;
        }
        loop {
            tokio::select! {
                biased;
                _ = connection.closed() => return Err(io::Error::new(io::ErrorKind::Interrupted, "TCP requester left")),
                outcome = ticket.outcome() => {
                    return match outcome {
                        ApprovalOutcome::Once | ApprovalOutcome::Always => Ok(ticket.take_confirmation()),
                        _ => Err(io::Error::new(io::ErrorKind::PermissionDenied, "TCP confirmation refused or unavailable")),
                    };
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    let state = self.auth.lock().map_err(|_| eio("Share authorization locked"))?;
                    let current = request_for(&state, principal, request, now_secs());
                    if current.decision == CapabilityDecision::Off || (current.decision == CapabilityDecision::Ask
                        && current.policy_revision != capability.policy_revision) {
                        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "TCP policy changed while awaiting confirmation"));
                    }
                }
            }
        }
    }
}

/// The host of a rule names the requested host: the same kind and the same
/// address or name, whatever the name resolves to.
fn same_host(rule: &TcpHost, requested: &TcpHost) -> bool {
    match (rule, requested) {
        (TcpHost::Loopback, TcpHost::Loopback) => true,
        (TcpHost::Ip { address }, TcpHost::Ip { address: wanted }) => address == wanted,
        (TcpHost::Dns { name, .. }, TcpHost::Dns { name: wanted, .. }) => {
            name.eq_ignore_ascii_case(wanted)
        }
        _ => false,
    }
}

fn access_error(reason: AccessRevocation) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("TCP access admission refused: {reason:?}"),
    )
}
fn retry_connect_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::NotConnected
            | io::ErrorKind::AddrNotAvailable
            | io::ErrorKind::TimedOut
            | io::ErrorKind::NetworkUnreachable
            | io::ErrorKind::HostUnreachable
    )
}

#[cfg(test)]
#[path = "node_tcp_target_tests.rs"]
mod target_tests;
