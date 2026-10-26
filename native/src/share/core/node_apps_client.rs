//! Authenticated app-command streams and pending-confirmation coordination.
use super::*;

impl ShareIrohNode {
    pub(in super::super) async fn handle_apps_connection(
        self: Arc<Self>,
        connection: Connection,
        permit: ApplicationHandshakePermit,
    ) -> io::Result<()> {
        let _close = super::super::node_tcp::CloseTcp(connection.clone());
        let _incoming = self.track_incoming(&connection)?;
        let _slot = self
            .apps
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| eio("device-command admission exhausted"))?;
        self.require_sharing_active()?;
        super::super::node_tcp::require_policy(&connection)?;
        self.await_route(&connection, None, None).await?;
        super::super::route_protocol::scope(connection.clone(), self.serve_apps(connection, permit))
            .await
    }
    pub(super) async fn serve_apps(
        self: &Arc<Self>,
        connection: Connection,
        permit: ApplicationHandshakePermit,
    ) -> io::Result<()> {
        let deadline = super::super::route_protocol::active_now() + Duration::from_secs(20);
        let (mut send, mut recv) =
            super::super::route_protocol::run_until_active(deadline, "apps hello timeout", async {
                connection.open_bi().await.map_err(eio)
            })
            .await?;
        let identity = self
            .auth
            .lock()
            .map_err(|_| eio("authorization unavailable"))?
            .identity
            .clone();
        let server = AppsServerHello {
            version: apps_wire::APPS_VERSION,
            challenge: format!("rp1.{}", random_token(32).map_err(eio)?),
            device_id: identity.device_id,
            public_key: identity.public_key,
            fingerprint: identity.fingerprint,
            node_id: identity.node_id,
        };
        super::super::route_protocol::run_until_active(
            deadline,
            "apps server hello timeout",
            apps_wire::send(&mut send, &server),
        )
        .await?;
        let remote = connection.remote_id().to_string();
        let client: AppsClientHello = super::super::route_protocol::run_until_active(
            deadline,
            "apps client hello timeout",
            apps_wire::receive_hello(&mut recv),
        )
        .await
        .map_err(|error| {
            self.auth_penalties().record(
                &remote,
                super::super::auth_penalty::PenaltyKind::NoAuthentication,
                std::time::Instant::now(),
            );
            error
        })?;
        client.validate()?;
        let principal = {
            let state = self
                .auth
                .lock()
                .map_err(|_| eio("authorization unavailable"))?;
            prove_relation(
                &server.relation(&client)?,
                &client.relation(),
                &remote,
                &state,
            )
        }
        .map_err(|rejection| {
            if rejection.counts_as_failed_authentication() {
                self.auth_penalties().record(
                    &remote,
                    super::super::auth_penalty::PenaltyKind::AuthFailure,
                    std::time::Instant::now(),
                );
            }
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "device relation proof refused",
            )
        })?;
        self.bind_incoming_principal(
            &connection,
            PeerPrincipal::new(
                &principal.relation_kind,
                &principal.relation_id,
                &principal.device_id,
                &principal.public_key,
                &principal.node_id,
            ),
        )
        .await?;
        super::super::route_monitor::authenticated_remote_capability(
            &connection,
            client.nonce.starts_with("rp1."),
        );
        drop(permit);
        let result = self
            .apps_execute(&connection, &principal, &client.command, &mut send)
            .await;
        let mut reply = match result {
            Ok(result) => AppsReply::Completed {
                id: client.id.clone(),
                request_hash: client.request_hash()?,
                result,
            },
            Err(error) => AppsReply::Refused {
                detail: error.to_string(),
            },
        };
        if serde_json::to_vec(&reply).map_err(eio)?.len() > apps_wire::MAX_PACKET {
            reply=AppsReply::Refused {detail:"complete_device_inventory_exceeds_ipc_frame_budget; no partial catalog or RDP inventory returned".into()};
        }
        super::super::route_protocol::timeout_for(
            Duration::from_secs(20),
            apps_wire::send(&mut send, &reply),
        )
        .await??;
        send.finish().map_err(eio)?;
        let _ = tokio::time::timeout(Duration::from_secs(2), connection.closed()).await;
        Ok(())
    }
    pub(super) async fn apps_confirmation(
        &self,
        connection: Option<&Connection>,
        principal: &ProvenPrincipal,
        request: &AccessRequest,
        send: Option<&mut iroh::endpoint::SendStream>,
    ) -> io::Result<Option<AccessConfirmation>> {
        let capability = {
            let state = self
                .auth
                .lock()
                .map_err(|_| eio("authorization unavailable"))?;
            request_for(&state, principal, request, now_secs())
        };
        match capability.decision {
            CapabilityDecision::Allow => return Ok(None),
            CapabilityDecision::Off => return Err(eio("app launch is off")),
            CapabilityDecision::Ask => {}
        }
        let mut ticket = super::super::consent_launch::approvals().request_access(
            principal,
            request.clone(),
            ApprovalSummary {
                policy_revision: capability.policy_revision,
                ..Default::default()
            },
        );
        if let Some(send) = send {
            super::super::route_protocol::timeout_for(
                Duration::from_secs(20),
                apps_wire::send(send, &AppsReply::Pending),
            )
            .await??;
        }
        loop {
            tokio::select! {
                _=async {if let Some(connection)=connection {connection.closed().await;} else {std::future::pending::<()>().await;}}=>return Err(eio("app requester disconnected")),
                outcome=ticket.outcome()=>return match outcome {ApprovalOutcome::Once|ApprovalOutcome::Always=>Ok(ticket.take_confirmation()),_=>Err(eio("app confirmation refused"))},
                _=tokio::time::sleep(Duration::from_secs(1))=>{
                    self.require_sharing_active()?;
                    let state=self.auth.lock().map_err(|_|eio("authorization unavailable"))?;
                    let current=request_for(&state,principal,request,now_secs());
                    if current.decision==CapabilityDecision::Off||current.policy_revision!=capability.policy_revision {return Err(eio("app policy changed during confirmation"));}
                }
            }
        }
    }
}
