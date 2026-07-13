#[derive(Debug, serde::Serialize)]
pub(crate) struct PeerAddOutput {
    action: &'static str,
    contact_id: String,
    selector: String,
    endpoint: String,
    request_action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorization: Option<AuthorizationOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    connectivity: Option<StateOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    worker_refresh: Option<WorkerRefreshOutput>,
    #[serde(skip)]
    text_lifecycle: Option<TextLifecycle>,
}

#[derive(Debug, serde::Serialize)]
struct StateOutput {
    state: &'static str,
}

#[derive(Debug, serde::Serialize)]
struct AuthorizationOutput {
    state: &'static str,
    active: bool,
}

#[derive(Debug, serde::Serialize)]
struct WorkerRefreshOutput {
    state: &'static str,
    error: Option<String>,
}

#[derive(Debug)]
struct TextLifecycle {
    direction: &'static str,
    delivery: &'static str,
    relay: &'static str,
    peer_receipt: &'static str,
    decision: &'static str,
    decision_delivery: &'static str,
    authorization: &'static str,
    connectivity: &'static str,
}

impl PeerAddOutput {
    fn new(action: &'static str, contact_id: String, request_action: &'static str) -> Self {
        let endpoint = crate::share::PeerOpenTarget::Direct {
            contact_id: contact_id.clone(),
        }
        .endpoint_prefix();
        Self {
            action,
            selector: contact_id.clone(),
            contact_id,
            endpoint,
            request_action,
            request_id: None,
            request: None,
            authorization: None,
            connectivity: None,
            worker_refresh: None,
            text_lifecycle: None,
        }
    }

    pub(crate) fn render(&self, json: bool) -> Result<String, String> {
        if json {
            return serde_json::to_string_pretty(self).map_err(|error| error.to_string());
        }
        let action = match self.action {
            "saved" => "Saved",
            _ => "Updated",
        };
        let prefix = format!(
            "{action} peer contact {}; selector={}; endpoint={}",
            self.contact_id, self.selector, self.endpoint
        );
        match self.request_action {
            "not_created" => Ok(format!("{prefix}; request_action=not_created; reason=--no-request")),
            "not_needed" => Ok(format!(
                "{prefix}; request_action=not_needed; authorization={}; connectivity={}; worker_refresh={}{}",
                self.authorization.as_ref().map(|value| value.state).unwrap_or("inactive"),
                self.connectivity.as_ref().map(|value| value.state).unwrap_or("unknown"),
                self.worker_refresh.as_ref().map(|value| value.state).unwrap_or("unknown"),
                worker_error_suffix(self.worker_refresh.as_ref().and_then(|value| value.error.as_deref())),
            )),
            request_action => {
                let lifecycle = self.text_lifecycle.as_ref();
                Ok(format!(
                    "{prefix}; request_id={}; request_action={request_action}; direction={}; delivery={}; relay={}; peer_receipt={}; decision={}; decision_delivery={}; authorization={}; connectivity={}; worker_refresh={}{}",
                    self.request_id.as_deref().unwrap_or("-"),
                    lifecycle.map(|value| value.direction).unwrap_or("outgoing"),
                    lifecycle.map(|value| value.delivery).unwrap_or("unconfirmed"),
                    lifecycle.map(|value| value.relay).unwrap_or("unconfirmed"),
                    lifecycle.map(|value| value.peer_receipt).unwrap_or("unconfirmed"),
                    lifecycle.map(|value| value.decision).unwrap_or("pending"),
                    lifecycle.map(|value| value.decision_delivery).unwrap_or("not_started"),
                    lifecycle.map(|value| value.authorization).unwrap_or("inactive"),
                    lifecycle.map(|value| value.connectivity).unwrap_or("waiting_for_access"),
                    self.worker_refresh.as_ref().map(|value| value.state).unwrap_or("unknown"),
                    worker_error_suffix(self.worker_refresh.as_ref().and_then(|value| value.error.as_deref())),
                ))
            }
        }
    }
}

pub(crate) fn add_peer(code: &str, name: &str, request: bool) -> Result<PeerAddOutput, String> {
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
    let action = if created { "saved" } else { "updated" };
    if !request {
        return Ok(PeerAddOutput::new(action, contact_id, "not_created"));
    }
    let contact = profiles
        .direct_contacts
        .iter()
        .find(|contact| contact.id == contact_id)
        .ok_or_else(|| format!("saved peer contact disappeared: {contact_id}"))?;
    if contact.access_state == crate::share::DirectAccessState::Accepted {
        let (worker, error) = refresh_worker_state();
        let mut output = PeerAddOutput::new(action, contact_id, "not_needed");
        output.authorization = Some(AuthorizationOutput {
            state: "active",
            active: true,
        });
        output.connectivity = Some(StateOutput {
            state: share_status_code(&contact.status),
        });
        output.worker_refresh = Some(WorkerRefreshOutput {
            state: worker,
            error,
        });
        return Ok(output);
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
    let mut output = PeerAddOutput::new(
        action,
        contact_id,
        if request_action.created {
            "created"
        } else {
            "reused"
        },
    );
    output.request_id = Some(entry.record.request.request_id.to_string());
    output.request = Some(crate::cli::share::lifecycle_output::request_value(
        entry,
        latest_profiles.as_ref().unwrap_or(&profiles),
    ));
    output.worker_refresh = Some(WorkerRefreshOutput {
        state: worker,
        error,
    });
    output.text_lifecycle = Some(TextLifecycle {
        direction: "outgoing",
        delivery: entry.record.delivery.state.code(),
        relay,
        peer_receipt: if entry.request_receipt.is_some() {
            "received"
        } else {
            "unconfirmed"
        },
        decision: entry.record.decision.state.code(),
        decision_delivery: entry.record.decision_delivery.state.code(),
        authorization,
        connectivity,
    });
    Ok(output)
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
        crate::share::DirectRelayOutcome::LegacyForwarded => "legacy_forwarded",
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

#[cfg(test)]
mod tests {
    use super::{PeerAddOutput, TextLifecycle, WorkerRefreshOutput};

    #[test]
    fn add_output_exposes_copyable_selector_endpoint_and_request_id() {
        let mut output = PeerAddOutput::new("saved", "contact-a".into(), "created");
        output.request_id = Some("request-a".into());
        output.request = Some(serde_json::json!({
            "request_id": "request-a",
            "direction": "outgoing",
            "delivery": {"state": "queued"},
            "relay": {"outcome": null},
            "peer_receipt": {"request": {"state": "unconfirmed"}},
            "decision": {"state": "pending"},
            "authorization": {"state": "inactive", "active": false},
        }));
        output.worker_refresh = Some(WorkerRefreshOutput {
            state: "refreshed",
            error: None,
        });
        output.text_lifecycle = Some(TextLifecycle {
            direction: "outgoing",
            delivery: "queued",
            relay: "unconfirmed",
            peer_receipt: "unconfirmed",
            decision: "pending",
            decision_delivery: "not_started",
            authorization: "inactive",
            connectivity: "waiting_for_access",
        });

        let text = output.render(false).unwrap();
        assert!(text.contains("selector=contact-a"));
        assert!(text.contains("endpoint=share://direct/contact-a"));
        assert!(text.contains("request_id=request-a"));

        let json: serde_json::Value = serde_json::from_str(&output.render(true).unwrap()).unwrap();
        assert_eq!(json["selector"], "contact-a");
        assert_eq!(json["endpoint"], "share://direct/contact-a");
        assert_eq!(json["request_id"], "request-a");
        assert_eq!(json["request"]["delivery"]["state"], "queued");
        assert_eq!(json["request"]["decision"]["state"], "pending");
        assert_eq!(json["request"]["authorization"]["active"], false);
        assert_eq!(json["worker_refresh"]["state"], "refreshed");
    }
}
