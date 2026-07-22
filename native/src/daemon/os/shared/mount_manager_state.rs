use crate::mount::{MountRecovery, MountSnapshot, MountStatus};

use super::MountEntry;

const MAX_DAEMON_STATUS_DETAIL_BYTES: usize = 16 * 1024;

pub(super) fn finish_exit(
    entry: &mut MountEntry,
    exit: super::super::mount_host_process::MountHostExit,
) {
    clear_host_runtime(entry);
    if exit.status.success() && entry.recovery.is_clean() {
        finish_clean(entry);
        return;
    }
    clear_backend_runtime(entry);
    // Failed/RuntimeUnavailable can only be published by the authenticated
    // host while its child is alive. Keep that richer IPC detail and ignore
    // the launcher's duplicate stderr line. A Conflict describes remote data,
    // not a live filesystem once the host has exited, so preserve its context
    // inside the actionable failure below.
    if matches!(entry.status, MountStatus::RuntimeUnavailable { .. }) || entry.host_terminal_failure
    {
        entry.pending_host_failure = None;
        return;
    }
    let pending = entry.pending_host_failure.take();
    let process_cause = exit.detail.or(pending);
    let status_cause = match &entry.status {
        MountStatus::Conflict { path, detail, .. } => {
            Some(format!("Remote-Konflikt bei {path}: {detail}"))
        }
        MountStatus::Failed { detail } => Some(format!("Vorheriger Dateifehler: {detail}")),
        _ => None,
    };
    let cause = match (status_cause, process_cause) {
        (Some(status), Some(process)) => Some(format!("{status}; {process}")),
        (Some(status), None) => Some(status),
        (None, process) => process,
    };
    entry.status = MountStatus::Failed {
        detail: bounded_detail(exit_detail(entry.recovery, &exit.status, cause.as_deref())),
    };
}

fn exit_detail(
    recovery: MountRecovery,
    exit: &std::process::ExitStatus,
    cause: Option<&str>,
) -> String {
    let cause = cause
        .filter(|detail| !detail.trim().is_empty())
        .map(|detail| format!(": {}", detail.trim()))
        .unwrap_or_default();
    match recovery {
        MountRecovery::Clean => {
            format!("Laufwerk-Host wurde vor der Laufwerk-Bereitschaft beendet ({exit}){cause}; der lokale Recovery-Cache ist sauber")
        }
        MountRecovery::Required => {
            format!("Laufwerk-Host wurde beendet ({exit}){cause}; lokale Aenderungen bleiben fuer Retry im Recovery-Cache")
        }
        MountRecovery::Unknown => {
            format!("Laufwerk-Host wurde beendet, bevor der lokale Recovery-Status verifiziert werden konnte ({exit}){cause}; der Cache bleibt bis zur erneuten Pruefung erhalten")
        }
    }
}

pub(super) fn finish_clean(entry: &mut MountEntry) {
    clear_host_runtime(entry);
    entry.recovery = MountRecovery::Clean;
    entry.pending_host_failure = None;
    entry.host_terminal_failure = false;
    if entry.backend_stream_active {
        entry.status = MountStatus::Unmounted;
        return;
    }
    clear_backend_runtime(entry);
    if entry.registry_recorded {
        if let Err(error) = super::super::mount_registry::remove(&entry.config.id) {
            entry.status = MountStatus::Failed {
                detail: format!("Laufwerk-Registry bereinigen: {error}"),
            };
            return;
        }
        entry.registry_recorded = false;
    }
    entry.status = MountStatus::Unmounted;
}

pub(super) fn fail_process_observation(
    entry: &mut MountEntry,
    error: &std::io::Error,
    kill_error: Option<&std::io::Error>,
) {
    let mut cause = format!("Laufwerk-Hoststatus lesen: {error}");
    if let Some(kill_error) = kill_error {
        cause.push_str(&format!("; Host beenden: {kill_error}"));
    }
    entry
        .pending_host_failure
        .get_or_insert_with(|| cause.clone());
    if entry.recovery.is_clean() {
        entry.recovery = MountRecovery::Unknown;
    }
    if !matches!(entry.status, MountStatus::RuntimeUnavailable { .. })
        && !entry.host_terminal_failure
    {
        if let MountStatus::Conflict { path, detail, .. } = &entry.status {
            cause = format!("Remote-Konflikt bei {path}: {detail}; {cause}");
        } else if let MountStatus::Failed { detail } = &entry.status {
            cause = format!("Vorheriger Dateifehler: {detail}; {cause}");
        }
        entry.status = MountStatus::Failed {
            detail: bounded_detail(match entry.recovery {
                MountRecovery::Required => {
                    format!("{cause}; lokale Aenderungen bleiben fuer Retry im Recovery-Cache")
                }
                MountRecovery::Clean => cause,
                MountRecovery::Unknown => {
                    format!("{cause}; der lokale Recovery-Status ist noch nicht verifiziert")
                }
            }),
        };
    }
    entry.host_terminal_failure = true;
    if let Some(control) = &entry.control {
        let _ = control.try_send(());
    }
}

pub(super) fn fail_process_reap(entry: &mut MountEntry, error: &std::io::Error) {
    let existing = match &entry.status {
        MountStatus::Failed { detail } | MountStatus::RuntimeUnavailable { detail } => {
            detail.as_str()
        }
        _ => "Laufwerk-Host konnte nicht abschliessend beobachtet werden",
    };
    entry.status = MountStatus::Failed {
        detail: bounded_detail(format!("{existing}; Laufwerk-Host abwarten: {error}")),
    };
    entry.host_terminal_failure = true;
}

pub(super) fn terminate_after_observation_error(entry: &mut MountEntry, error: &std::io::Error) {
    let Some(mut child) = entry.child.take() else {
        fail_process_observation(entry, error, None);
        return;
    };
    let kill_error = child.kill().err();
    fail_process_observation(entry, error, kill_error.as_ref());
    let exit = if kill_error.is_none() {
        child.wait().map(Some)
    } else {
        child.try_wait()
    };
    match exit {
        Ok(Some(exit)) => finish_exit(entry, exit),
        Ok(None) => entry.child = Some(child),
        Err(wait_error) => {
            fail_process_reap(entry, &wait_error);
            entry.child = Some(child);
        }
    }
}

fn bounded_detail(mut detail: String) -> String {
    if detail.len() <= MAX_DAEMON_STATUS_DETAIL_BYTES {
        return detail;
    }
    let marker = "[Fehlerdetail gekuerzt] ";
    let content_limit = MAX_DAEMON_STATUS_DETAIL_BYTES.saturating_sub(marker.len());
    let mut boundary = detail.len().saturating_sub(content_limit);
    while boundary < detail.len() && !detail.is_char_boundary(boundary) {
        boundary += 1;
    }
    detail.drain(..boundary);
    detail.insert_str(0, marker);
    detail
}

fn clear_host_runtime(entry: &mut MountEntry) {
    entry.launch_token = None;
    entry.session_token = None;
    entry.backend_token = None;
    entry.control = None;
    entry.child = None;
}

pub(super) fn clear_backend_runtime(entry: &mut MountEntry) {
    if entry.backend_stream_active {
        return;
    }
    entry.backend = None;
    entry.capabilities = None;
}

pub(super) fn removable(entry: &MountEntry) -> bool {
    entry.child.is_none()
        && !entry.backend_stream_active
        && !entry.registry_recorded
        && matches!(entry.status, MountStatus::Unmounted)
}

pub(super) fn snapshot(entry: &MountEntry) -> MountSnapshot {
    MountSnapshot {
        config: entry.config.clone(),
        status: entry.status.clone(),
        recovery: entry.recovery,
        recovery_required_compat: entry.recovery.requires_retention(),
    }
}

pub(super) fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| format!("Laufwerk-Token: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
