//! Catalog disclosure, exact app request admission and host execution.
use super::*;

impl ShareIrohNode {
    pub(super) async fn apps_execute(
        self: &Arc<Self>,
        connection: &Connection,
        principal: &ProvenPrincipal,
        command: &AppsCommand,
        send: &mut iroh::endpoint::SendStream,
    ) -> io::Result<AppsResult> {
        self.require_sharing_active()?;
        match command {
            AppsCommand::RdpProbe => {
                self.require_inventory(principal)?;
                let host = tokio::task::spawn_blocking(crate::rdp::probe_local_host)
                    .await
                    .map_err(eio)?;
                crate::share::access_steering::note_own_rdp_ports(
                    host.services.iter().filter_map(|service| service.port),
                );
                self.require_inventory(principal)?;
                Ok(AppsResult::RdpProbe { host })
            }
            AppsCommand::Catalog => {
                let host = self.apps.host(principal)?;
                let catalog = tokio::task::spawn_blocking(move || host.catalog())
                    .await
                    .map_err(eio)??;
                Ok(AppsResult::Catalog {
                    catalog: self.visible_catalog(principal, catalog)?,
                })
            }
            AppsCommand::Icon {
                app_id,
                icon,
                catalog_revision,
            } => {
                icon.validate()?;
                let host = self.apps.host(principal)?;
                let catalog = {
                    let host = host.clone();
                    tokio::task::spawn_blocking(move || host.catalog())
                        .await
                        .map_err(eio)??
                };
                let visible = self.visible_catalog(principal, catalog)?;
                if &visible.revision != catalog_revision
                    || !visible
                        .apps
                        .iter()
                        .any(|a| &a.app_id == app_id && a.icon.as_ref() == Some(icon))
                {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "icon does not belong to the current authorized app catalog",
                    ));
                }
                let id = app_id.clone();
                let expected = icon.clone();
                let renderer = host.clone();
                let reply = tokio::task::spawn_blocking(move || renderer.icon(&id, &expected))
                    .await
                    .map_err(eio)??;
                let fresh = tokio::task::spawn_blocking(move || host.catalog())
                    .await
                    .map_err(eio)??;
                let fresh = self.visible_catalog(principal, fresh)?;
                if &fresh.revision != catalog_revision
                    || !fresh
                        .apps
                        .iter()
                        .any(|app| &app.app_id == app_id && app.icon.as_ref() == Some(icon))
                {
                    return Err(eio("app icon catalog changed while rendering"));
                }
                Ok(AppsResult::Icon { reply })
            }
            AppsCommand::Receipt { request } => {
                let host = self.apps.host(principal)?;
                if !host.bind_peer_request(request, &owner_key(principal)?, false)? {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "app receipt owner/request does not match",
                    ));
                }
                let node = self.clone();
                let proven = principal.clone();
                let request = request.clone();
                let receipt =
                    tokio::task::spawn_blocking(move || window::probe(&node, &proven, &request))
                        .await
                        .map_err(eio)??;
                // The owner-specific persistent ledger is unreachable from a
                // different proven relationship, even after a worker restart.
                let relationship_active = {
                    let state = self
                        .auth
                        .lock()
                        .map_err(|_| eio("authorization unavailable"))?;
                    principal_grant(&state, principal).is_some()
                };
                if !relationship_active {
                    return Err(eio("relationship ended"));
                }
                Ok(AppsResult::Receipt { receipt })
            }
            AppsCommand::Launch { request } => {
                let host = self.apps.host(principal)?;
                if !host.bind_peer_request(request, &owner_key(principal)?, true)? {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "app request ID owner or contents differ",
                    ));
                }
                let catalog = {
                    let host = host.clone();
                    tokio::task::spawn_blocking(move || host.catalog())
                        .await
                        .map_err(eio)??
                };
                let access = access_request(request, &catalog)?;
                let confirmation = self
                    .apps_confirmation(Some(connection), principal, &access, Some(send))
                    .await?;
                let node = self.clone();
                let principal = principal.clone();
                let request = request.clone();
                let live = Arc::new(AtomicBool::new(true));
                let dispatch_live = live.clone();
                let mut job = tokio::task::spawn_blocking(move || {
                    node.run_app(
                        &principal,
                        &request,
                        false,
                        confirmation,
                        Some(dispatch_live),
                    )
                });
                let receipt = tokio::select! {
                    result=&mut job=>result.map_err(eio)??,
                    _=connection.closed()=>{live.store(false,Ordering::Release);let _=job.await;return Err(eio("app requester disconnected before completion"));}
                };
                Ok(AppsResult::Launch { receipt })
            }
        }
    }
    pub(super) fn require_inventory(&self, principal: &ProvenPrincipal) -> io::Result<()> {
        let state = self
            .auth
            .lock()
            .map_err(|_| eio("authorization unavailable"))?;
        if capability_for(&state, principal, Capability::ScreenView, now_secs()).decision
            != CapabilityDecision::Allow
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "host inventory requires explicit screen-view metadata permission",
            ));
        }
        Ok(())
    }
    pub(super) fn visible_catalog(
        &self,
        principal: &ProvenPrincipal,
        mut catalog: CatalogSnapshot,
    ) -> io::Result<CatalogSnapshot> {
        let state = self
            .auth
            .lock()
            .map_err(|_| eio("authorization unavailable"))?;
        let grant = principal_grant(&state, principal)
            .ok_or_else(|| eio("relationship ended"))?
            .access_grant();
        if grant.expired_at(now_secs())
            || !(grant.decision(Capability::AppLaunch).is_on()
                || crate::share::access_steering::steers(&grant, now_secs()))
        {
            return Err(eio("app catalog disclosure is not authorized"));
        }
        catalog.apps.retain(|app| {
            let (Ok(app_id), Ok(session), Ok(user)) = (
                constraints::AppId::parse(app.app_id.as_str()),
                constraints::SessionId::parse(catalog.authorization_scope.as_str()),
                constraints::UserId::parse(catalog.session.user_id.as_str()),
            ) else {
                return false;
            };
            grant
                .decision_for(
                    &AccessRequest::AppLaunch(constraints::AppLaunchRequest {
                        app_id,
                        session,
                        user,
                    }),
                    now_secs(),
                )
                .is_on()
        });
        if catalog.apps.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "no app catalog entries are granted for this login/catalog scope",
            ));
        }
        Ok(catalog)
    }
    pub(super) fn run_app(
        self: &Arc<Self>,
        principal: &ProvenPrincipal,
        request: &LaunchRequest,
        resume: bool,
        mut confirmation: Option<AccessConfirmation>,
        live: Option<Arc<AtomicBool>>,
    ) -> io::Result<LaunchReceipt> {
        let host = self.apps.host(principal)?;
        let owner = owner_key(principal)?;
        if !host.bind_peer_request(request, &owner, true)? {
            return Err(eio("app request owner or contents differ"));
        }
        if crate::apps::os::host::local_consent_supported() {
            let prior = self
                .apps
                .pending
                .lock()
                .map_err(|_| eio("app context unavailable"))?
                .get(request.request_id.as_str())
                .cloned();
            if prior.as_ref().is_some_and(|prior| {
                prior.request != *request
                    || !owner_key(&prior.principal).is_ok_and(|key| key == owner)
            }) {
                return Err(eio("app consent context ownership mismatch"));
            }
            if let Some(receipt) = host.receipt(request)? {
                if receipt.state == crate::apps::LaunchState::ConsentRequired
                    && (prior.is_none()
                        || prior
                            .as_ref()
                            .and_then(|p| p.access.as_ref())
                            .is_some_and(|a| a.reason().is_some()))
                {
                    let denied = host.note_dispatch_result(
                        request,
                        &receipt.dispatch_id,
                        crate::apps::os::shared::platform::DispatchReceipt {
                            state: crate::apps::LaunchState::Denied,
                            reason: "original_app_admission_ended_or_worker_restarted".into(),
                            pid: None,
                        },
                    )?;
                    crate::apps::os::host::end_platform_consent(request, &denied.reason);
                    return Ok(denied);
                }
            }
            self.apps
                .pending
                .lock()
                .map_err(|_| eio("app context unavailable"))?
                .insert(
                    request.request_id.to_string(),
                    PendingApp {
                        principal: principal.clone(),
                        request: request.clone(),
                        access: prior.and_then(|p| p.access),
                    },
                );
        }
        let authorize = |facts: &LaunchFacts| {
            let access = access_request(&facts.request, &facts.catalog)
                .map_err(|e| LaunchDenial::Denied(e.to_string()))?;
            let state = self
                .auth
                .lock()
                .map_err(|_| LaunchDenial::Denied("authorization unavailable".into()))?;
            self.require_sharing_active()
                .map_err(|e| LaunchDenial::Denied(e.to_string()))?;
            if live
                .as_ref()
                .is_some_and(|live| !live.load(Ordering::Acquire))
            {
                return Err(LaunchDenial::Denied("app requester cancelled".into()));
            }
            if self
                .apps
                .pending
                .lock()
                .map_err(|_| LaunchDenial::Denied("app context unavailable".into()))?
                .get(request.request_id.as_str())
                .and_then(|pending| pending.access.as_ref())
                .is_some_and(|access| access.reason().is_some())
            {
                return Err(LaunchDenial::Denied(
                    "original app admission was revoked".into(),
                ));
            }
            let handle = {
                if resume {
                    if request_for(&state, principal, &access, now_secs()).decision
                        == CapabilityDecision::Off
                    {
                        return Err(LaunchDenial::Denied("app grant ended".into()));
                    }
                    self.apps
                        .pending
                        .lock()
                        .map_err(|_| LaunchDenial::Denied("app context unavailable".into()))?
                        .get(request.request_id.as_str())
                        .and_then(|pending| pending.access.clone())
                        .filter(|handle| handle.reason().is_none())
                        .ok_or_else(|| {
                            LaunchDenial::Denied(
                                "original app confirmation is no longer active".into(),
                            )
                        })?
                } else {
                    Arc::new(
                        self.access_registry()
                            .admit(
                                &state,
                                principal,
                                access.clone(),
                                confirmation.take(),
                                now_secs(),
                            )
                            .map_err(|reason| {
                                LaunchDenial::Denied(format!("app admission refused: {reason:?}"))
                            })?,
                    )
                }
            };
            let grant = principal_grant(&state, principal)
                .ok_or_else(|| LaunchDenial::Denied("relationship ended".into()))?
                .access_grant();
            let end = match (
                grant.valid_until,
                grant.constraints.end_for(&access, now_secs()),
            ) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let authority = handle.clone();
            let node = Arc::downgrade(self);
            let live = live.clone();
            if let Some(pending) = self
                .apps
                .pending
                .lock()
                .map_err(|_| LaunchDenial::Denied("app context unavailable".into()))?
                .get_mut(request.request_id.as_str())
            {
                pending.access = Some(handle.clone());
            }
            let valid = Arc::new(move || {
                authority.reason().is_none()
                    && live
                        .as_ref()
                        .is_none_or(|live| live.load(Ordering::Acquire))
                    && node
                        .upgrade()
                        .is_some_and(|n| n.require_sharing_active().is_ok())
            });
            Ok(LaunchAdmission {
                guard: (state, handle),
                expires_at_unix_ms: end.map(|end| end.saturating_mul(1000)),
                valid: Some(valid),
            })
        };
        let mut receipt = if resume {
            host.resume_after_local_consent(request, authorize)?
        } else {
            host.launch(request, authorize)?
        };
        if host.has_window_process(request)
            && matches!(
                receipt.state,
                crate::apps::LaunchState::StartedWindowPending
                    | crate::apps::LaunchState::OutcomeUnknown
            )
        {
            if let Ok(Some(confirmed)) = window::probe(self, principal, request) {
                receipt = confirmed;
            }
            if receipt.window.is_none() {
                window::watch(self, principal.clone(), request.clone());
            }
        }
        if !matches!(
            receipt.state,
            crate::apps::LaunchState::ConsentRequired | crate::apps::LaunchState::OutcomeUnknown
        ) {
            self.apps
                .pending
                .lock()
                .map_err(|_| eio("app consent ownership unavailable"))?
                .remove(request.request_id.as_str());
        }
        Ok(receipt)
    }
}

pub(super) fn access_request(
    request: &LaunchRequest,
    catalog: &CatalogSnapshot,
) -> io::Result<AccessRequest> {
    if request.target_device_id != catalog.device_id
        || request.user_id != catalog.session.user_id
        || request.session_id != catalog.session.session_id
        || request.catalog_revision != catalog.revision
        || !catalog.apps.iter().any(|a| a.app_id == request.app_id)
    {
        return Err(eio("app request does not match fresh host catalog/session"));
    }
    Ok(AccessRequest::AppLaunch(constraints::AppLaunchRequest {
        app_id: constraints::AppId::parse(request.app_id.as_str()).map_err(eio)?,
        session: constraints::SessionId::parse(catalog.authorization_scope.as_str())
            .map_err(eio)?,
        user: constraints::UserId::parse(catalog.session.user_id.as_str()).map_err(eio)?,
    }))
}
