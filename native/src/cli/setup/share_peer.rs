pub(crate) fn add_peer(code: &str, name: &str, request: bool) -> Result<String, String> {
    let mut profiles = crate::share::ShareProfiles::load_checked(Some(default_home()))
        .map_err(|error| format!("share profile laden: {error}"))?;
    let existing = profiles.direct_contact_id_from_code(code)?;
    let mut identity = if request && existing.is_none() {
        Some(load_identity().map_err(|error| {
            format!("signed access request was not created; peer contact was not saved: {error}")
        })?)
    } else {
        None
    };
    let (contact_id, created) = match existing {
        Some(id) => (id, false),
        None => {
            let (committed, id) = crate::share::ShareProfiles::add_direct_from_code_persisted(
                Some(default_home()),
                code,
                name,
            )?;
            profiles = committed;
            (id, true)
        }
    };
    let action = if created { "Saved" } else { "Updated" };
    if !request {
        return Ok(format!(
            "{action} peer contact {contact_id}; request=not_created (--no-request)"
        ));
    }
    let contact = profiles
        .direct_contacts
        .iter()
        .find(|contact| contact.id == contact_id)
        .ok_or_else(|| format!("saved peer contact disappeared: {contact_id}"))?;
    if contact.access_state == crate::share::DirectAccessState::Accepted {
        let (worker, error) = refresh_worker_state();
        return Ok(format!(
            "{action} peer contact {contact_id}; request=not_needed; authorization=active; connectivity={}; worker_refresh={}{}",
            share_status_code(&contact.status),
            worker,
            worker_error_suffix(error.as_deref()),
        ));
    }
    let identity = match identity.take() {
        Some(identity) => identity,
        None => load_identity().map_err(|error| {
                format!(
                    "saved peer contact {contact_id}, but signed access request was not created: {error}"
                )
            },
        )?,
    };
    let request_action = crate::share::queue_direct_request_for_contact(
        Some(default_home()),
        &identity,
        &contact_id,
        None,
    )
    .map_err(|error| {
        format!("saved peer contact {contact_id}, but signed access request failed: {error}")
    })?;
    let request_id = request_action.entry.record.request.request_id.clone();
    let (worker, error) = refresh_worker_state();
    let latest_profiles = crate::share::ShareProfiles::load_checked(Some(default_home())).ok();
    let latest = latest_profiles
        .as_ref()
        .and_then(|profiles| profiles.direct_request(&request_id).cloned())
        .unwrap_or_else(|| request_action.entry.clone());
    let entry = &latest;
    let authorization =
        if entry.record.decision.state == crate::share::DirectDecisionState::Accepted {
            "active"
        } else {
            "inactive"
        };
    let connectivity = latest_profiles
        .as_ref()
        .and_then(|profiles| {
            profiles
                .direct_contacts
                .iter()
                .find(|contact| contact.id == contact_id)
        })
        .map(|contact| share_status_code(&contact.status))
        .unwrap_or("waiting_for_access");
    let relay = entry
        .retries
        .request
        .relay_outcome
        .map(relay_outcome_code)
        .unwrap_or("unconfirmed");
    Ok(format!(
        "{action} peer contact {contact_id}; request_id={}; request_action={}; direction=outgoing; delivery={}; relay={}; peer_receipt={}; decision={}; decision_delivery={}; authorization={}; connectivity={}; worker_refresh={}{}",
        entry.record.request.request_id,
        if request_action.created { "created" } else { "reused" },
        entry.record.delivery.state.code(),
        relay,
        if entry.request_receipt.is_some() { "received" } else { "unconfirmed" },
        entry.record.decision.state.code(),
        entry.record.decision_delivery.state.code(),
        authorization,
        connectivity,
        worker,
        worker_error_suffix(error.as_deref()),
    ))
}

fn refresh_worker_state() -> (&'static str, Option<String>) {
    match crate::daemon::refresh_share_worker_checked() {
        Ok(true) => ("refreshed", None),
        Ok(false) => (
            "inactive",
            Some("Share server is not configured or Auto-Connect is off".to_string()),
        ),
        Err(error) => ("unavailable", Some(error)),
    }
}

fn relay_outcome_code(outcome: crate::share::DirectRelayOutcome) -> &'static str {
    match outcome {
        crate::share::DirectRelayOutcome::Forwarded => "forwarded",
        crate::share::DirectRelayOutcome::TargetOffline => "target_offline",
    }
}

fn share_status_code(status: &crate::share::ShareStatus) -> &'static str {
    match status {
        crate::share::ShareStatus::Offline => "offline",
        crate::share::ShareStatus::Waiting => "waiting",
        crate::share::ShareStatus::WaitingForAccess => "waiting_for_access",
        crate::share::ShareStatus::Available => "available",
        crate::share::ShareStatus::Connecting => "connecting",
        crate::share::ShareStatus::Connected => "connected",
        crate::share::ShareStatus::ConnectedDirect => "connected_direct",
        crate::share::ShareStatus::ConnectedRelay => "connected_relay",
        crate::share::ShareStatus::Failed(_) => "failed",
        crate::share::ShareStatus::IdentityConflict => "identity_conflict",
    }
}

fn worker_error_suffix(error: Option<&str>) -> String {
    error.map_or_else(String::new, |error| {
        format!("; worker_error={}", error.replace(['\t', '\r', '\n'], " "))
    })
}

fn default_home() -> String {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .to_string_lossy()
        .replace('\\', "/")
}

fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Smart Explorer CLI".to_string())
}

fn load_identity() -> Result<crate::share::ShareIdentity, String> {
    crate::share::ShareIdentity::load_or_create(default_device_name()).map_err(|error| {
        if error.contains("fehlt im sicheren Speicher") {
            format!("{error}; repair with `se share identity --repair`")
        } else {
            error
        }
    })
}
