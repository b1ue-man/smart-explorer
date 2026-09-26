//! This phone as exec host (api.md §5): the provider and the grant targets
//! of `share.status`, `share.setExec` through the daemon's journal-backed
//! grant mutation (like the desktop), and the job list with cancel
//! (`share.execJobs`, `share.cancelExecJob`). Commands starting or ending on
//! this phone wake the Share poller, which then sends `share`.
use std::sync::OnceLock;

use serde_json::{json, Value};

use super::args::{bool_arg, invalid, str_arg};
use super::share_state::{committed, default_home, wake};
use crate::daemon::{ExecCancelTarget, ExecJobDirection, ExecJobsSnapshot};
use crate::mobile::ApiError;
use crate::share::{
    exec_target_views, resolve_exec_target, ExecId, ExecJobView, ExecProviderStatus,
    ExecTargetRelation, ExecTargetView, ShareProfiles,
};

/// The exec provider of this device; its first use also ends command trees
/// a crashed app process left behind, so callers fetch it outside locks.
pub(super) fn provider() -> &'static ExecProviderStatus {
    static PROVIDER: OnceLock<ExecProviderStatus> = OnceLock::new();
    PROVIDER.get_or_init(crate::share::exec_provider_status)
}

/// `execProvider` of `share.status`.
pub(super) fn provider_json(status: &ExecProviderStatus) -> Value {
    json!({
        "available": status.available,
        "provider": status.provider,
        "detail": status.detail,
    })
}

/// `execTargets` of `share.status`: every direct grant and room member.
pub(super) fn targets_json(profiles: &ShareProfiles) -> Vec<Value> {
    exec_target_views(profiles)
        .iter()
        .map(target_json)
        .collect()
}

fn target_json(view: &ExecTargetView) -> Value {
    json!({
        "targetKey": view.target_key,
        "relation": match view.relation {
            ExecTargetRelation::Direct => "direct",
            ExecTargetRelation::Room => "room",
        },
        "roomId": view.room_id,
        "roomName": view.room_name,
        "deviceId": view.device_id,
        "name": view.device_name,
        "fingerprint": view.fingerprint,
        "enabled": view.enabled,
        "baseAuthorized": view.base_authorized,
        "policyRevision": view.policy_revision,
    })
}

/// `share.setExec {targetKey, enabled}` → `{revision}`. The key is resolved
/// against the stored profiles, so a replaced identity is `not_found`.
pub(super) fn set_exec(args: &Value) -> Result<Value, ApiError> {
    let key = str_arg(args, "targetKey")?;
    let enabled = bool_arg(args, "enabled")?;
    let profiles = ShareProfiles::load_checked(default_home())
        .map_err(|error| ApiError::internal(format!("Share-Profile nicht lesbar: {error}")))?;
    let view = resolve_exec_target(&profiles, key).ok_or_else(|| {
        ApiError::not_found(
            "Gerät nicht gefunden oder seine Identität hat sich geändert – bitte neu laden.",
        )
    })?;
    if enabled {
        let provider = provider();
        if !provider.available {
            return Err(ApiError::unsupported(format!(
                "Befehle auf diesem Telefon sind nicht verfügbar: {}",
                provider.detail
            )));
        }
        // Like the desktop: a grant never waits on a relation that is not
        // active (it would apply the moment the relation came back).
        if !view.base_authorized {
            return Err(ApiError::new(
                "conflict",
                "Die Freigabe für dieses Gerät ist nicht aktiv; Befehle lassen sich nicht erlauben.",
            ));
        }
    }
    let result = crate::daemon::mutate_exec_grant(view.target, enabled)
        .map_err(|error| ApiError::internal(format!("Befehlsfreigabe nicht geändert: {error}")))?;
    let answer = grant_answer(
        result.persisted,
        result.applied,
        result.error.as_deref(),
        result.revision,
    )?;
    // The journal stored and applied the change: show it before the next
    // worker snapshot, which carries the same profile revision.
    if let Ok(stored) = ShareProfiles::load_checked(default_home()) {
        committed(stored);
    }
    wake();
    Ok(answer)
}

/// Only a change that is both stored and applied counts (desktop
/// `apply_exec_grant`); anything else names what is missing.
pub(super) fn grant_answer(
    persisted: bool,
    applied: bool,
    error: Option<&str>,
    revision: u64,
) -> Result<Value, ApiError> {
    if persisted && applied && error.is_none() {
        return Ok(json!({ "revision": revision }));
    }
    let yes_no = |value: bool| if value { "ja" } else { "nein" };
    Err(ApiError::internal(format!(
        "Befehlsfreigabe nicht vollständig geändert (gespeichert: {}, angewendet: {}): {}",
        yes_no(persisted),
        yes_no(applied),
        error.unwrap_or("die Anwendung im Hintergrund-Dienst steht noch aus"),
    )))
}

/// `share.execJobs {}` → `{active, history}`.
pub(super) fn exec_jobs() -> Result<Value, ApiError> {
    let snapshot = crate::daemon::exec_jobs()
        .map_err(|error| ApiError::internal(format!("Befehlsliste nicht verfügbar: {error}")))?;
    Ok(jobs_json(&snapshot))
}

/// Incoming (commands on this phone) first, then outgoing ones.
pub(super) fn jobs_json(snapshot: &ExecJobsSnapshot) -> Value {
    json!({
        "active": job_list(&snapshot.incoming_active, &snapshot.outgoing_active),
        "history": job_list(&snapshot.incoming_history, &snapshot.outgoing_history),
    })
}

fn job_list(incoming: &[ExecJobView], outgoing: &[ExecJobView]) -> Vec<Value> {
    let incoming = incoming.iter().map(|view| job_json("incoming", view));
    let outgoing = outgoing.iter().map(|view| job_json("outgoing", view));
    incoming.chain(outgoing).collect()
}

fn job_json(direction: &str, view: &ExecJobView) -> Value {
    let terminal = view.terminal.as_ref();
    json!({
        "direction": direction,
        "execId": view.exec_id.as_str(),
        "peerDeviceId": view.peer_device_id,
        "peerName": view.peer_device_name,
        "program": view.program,
        // The lifecycle's own snake_case names (`timed_out`, …).
        "state": serde_json::to_value(&view.state).unwrap_or(Value::Null),
        "startedAt": view.started_at,
        "finishedAt": view.finished_at,
        "exitCode": terminal.and_then(|terminal| terminal.exit_code),
        "message": terminal.and_then(|terminal| terminal.message.clone()),
    })
}

/// `share.cancelExecJob {direction, execId, peerDeviceId}` → `{}`.
pub(super) fn cancel_exec_job(args: &Value) -> Result<Value, ApiError> {
    let target = cancel_target(args)?;
    let found = crate::daemon::cancel_exec(target)
        .map_err(|error| ApiError::internal(format!("Befehl nicht gestoppt: {error}")))?;
    if !found {
        return Err(ApiError::not_found("Der Befehl läuft nicht mehr."));
    }
    wake();
    Ok(json!({}))
}

pub(super) fn cancel_target(args: &Value) -> Result<ExecCancelTarget, ApiError> {
    let direction = match str_arg(args, "direction")? {
        "incoming" => ExecJobDirection::Incoming,
        "outgoing" => ExecJobDirection::Outgoing,
        other => return Err(invalid(format!("Unbekannte Richtung: {other}"))),
    };
    let exec_id =
        ExecId::parse(str_arg(args, "execId")?).map_err(|_| invalid("Ungültige Befehls-ID."))?;
    let peer_device_id = str_arg(args, "peerDeviceId")?.to_string();
    Ok(ExecCancelTarget {
        direction,
        exec_id,
        peer_device_id,
    })
}

/// Changes whenever a command of another device starts or ends here.
#[cfg(target_os = "android")]
pub(super) fn host_activity() -> u64 {
    crate::share::exec_host_activity()
}

/// The Linux host tests run no exec host.
#[cfg(not(target_os = "android"))]
pub(super) fn host_activity() -> u64 {
    0
}

/// Lets a starting or ending command wake the Share poller at once, so the
/// app's notification follows within one `share.execJobs` round trip.
#[cfg(target_os = "android")]
pub(super) fn watch_host_activity() {
    crate::share::set_exec_host_listener(wake);
}

#[cfg(not(target_os = "android"))]
pub(super) fn watch_host_activity() {}
