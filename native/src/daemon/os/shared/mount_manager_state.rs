use crate::mount::{MountRecovery, MountSnapshot, MountStatus};

use super::restore::Retention;
use super::MountEntry;

const MAX_DAEMON_STATUS_DETAIL_BYTES: usize = 16 * 1024;
const UPDATE_STOP_PREFIX: &str = "Fuer ein Update beendet: ";

/// Ends the bookkeeping of an exited host. A host that ended on an update
/// stop (exit 75, contract V8) with a clean journal and without a failure
/// report ended like an unmount; otherwise it stays `Failed` and says why.
/// DRV-M7: the end of a vault drive reaches the vault side, and a drive the
/// lock of its vault ended stays registered for the next unlock.
pub(super) fn finish_exit(
    entry: &mut MountEntry,
    exit: super::super::mount_host_process::MountHostExit,
    retention: &mut Retention,
) {
    let end = super::termination::terminated(entry, exit.status.success());
    let lock_end = end
        .as_ref()
        .is_some_and(|end| end.reason == super::termination::MountEndReason::Lock);
    finish_host_exit(entry, exit, retention, lock_end);
    if let Some(end) = end {
        super::termination::after_exit(entry, end, super::termination::notify_vault);
    }
}

fn finish_host_exit(
    entry: &mut MountEntry,
    exit: super::super::mount_host_process::MountHostExit,
    retention: &mut Retention,
    lock_end: bool,
) {
    clear_host_runtime(entry);
    let update_stop = exit.status.code() == Some(super::super::update_stop::EXIT_UPDATE_STOP);
    let ended_cleanly = exit.status.success() || (update_stop && !entry.host_terminal_failure);
    if ended_cleanly && entry.recovery.is_clean() {
        let keep = lock_end
            || if update_stop {
                retention.keep_after_update_stop(entry)
            } else {
                retention.keeps(entry)
            };
        finish_clean(entry, keep);
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
        if update_stop {
            if let MountStatus::Failed { detail } = &mut entry.status {
                *detail = bounded_detail(format!("{UPDATE_STOP_PREFIX}{detail}"));
            }
        }
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
    let detail = exit_detail(entry.recovery, &exit.status, cause.as_deref());
    entry.status = MountStatus::Failed {
        detail: bounded_detail(if update_stop {
            format!("{UPDATE_STOP_PREFIX}{detail}")
        } else {
            detail
        }),
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
        MountRecovery::RequiresUnlock { pending } => {
            format!("Laufwerk-Host wurde beendet ({exit}){cause}; {pending} versiegelte Aenderung(en) werden nach dem Entsperren des Tresors gespeichert")
        }
    }
}

/// A clean end. `keep_registration` (DRV-M3, `Retention`) keeps the registry
/// record of a drive that restores at login.
pub(super) fn finish_clean(entry: &mut MountEntry, keep_registration: bool) {
    clear_host_runtime(entry);
    entry.recovery = MountRecovery::Clean;
    entry.pending_host_failure = None;
    entry.host_terminal_failure = false;
    if entry.backend_stream_active {
        entry.status = MountStatus::Unmounted;
        return;
    }
    clear_backend_runtime(entry);
    if entry.registry_recorded && !keep_registration {
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
                MountRecovery::Required | MountRecovery::RequiresUnlock { .. } => {
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

pub(super) fn terminate_after_observation_error(
    entry: &mut MountEntry,
    error: &std::io::Error,
    retention: &mut Retention,
) {
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
        Ok(Some(exit)) => finish_exit(entry, exit, retention),
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

#[cfg(test)]
mod update_stop_tests {
    use super::{finish_exit, MountEntry, Retention};
    use crate::mount::{
        BackendRoot, DriveSelection, MountConfig, MountId, MountMode, MountRecovery, MountSource,
        MountStatus,
    };

    fn exit_status(code: i32) -> std::process::ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(code << 8)
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(code as u32)
        }
    }

    fn exit(code: i32) -> super::super::super::mount_host_process::MountHostExit {
        super::super::super::mount_host_process::MountHostExit {
            status: exit_status(code),
            detail: None,
        }
    }

    /// A drive whose host reported `status` with `recovery` and exited.
    fn entry(
        id: &str,
        auto_remount: bool,
        status: MountStatus,
        recovery: MountRecovery,
        host_terminal_failure: bool,
    ) -> MountEntry {
        let config = MountConfig::new(
            MountId::parse(id).unwrap(),
            MountSource::SavedRemote {
                account: "sftp:prod".into(),
                root: BackendRoot::parse("/srv").unwrap(),
            },
            DriveSelection::Automatic,
            MountMode::ReadWrite,
            id,
        )
        .unwrap()
        .with_auto_remount(auto_remount);
        let mut entry = super::super::registered_entry(config, recovery, String::new());
        entry.status = status;
        // A record of an auto-remount drive stays; no other test entry
        // touches the registry of this machine.
        entry.registry_recorded = auto_remount;
        entry.host_terminal_failure = host_terminal_failure;
        entry
    }

    #[test]
    fn drive_task_update_stop_exit_clean_journal_counts_as_unmount() {
        let mut retention = Retention::default();
        // Clean journal, exit 75: unmounted like exit 0, not a failure.
        let mut plain = entry(
            "plain",
            false,
            MountStatus::Unmounted,
            MountRecovery::Clean,
            false,
        );
        finish_exit(&mut plain, exit(75), &mut retention);
        assert_eq!(plain.status, MountStatus::Unmounted);
        assert!(plain.recovery.is_clean());
        // Exit 0 keeps the behaviour of v0.5.182.
        let mut stopped = entry(
            "stopped",
            false,
            MountStatus::Unmounted,
            MountRecovery::Clean,
            false,
        );
        finish_exit(&mut stopped, exit(0), &mut retention);
        assert_eq!(stopped.status, MountStatus::Unmounted);
        // An auto-remount drive keeps its registry record for the next start.
        let mut login = entry(
            "login",
            true,
            MountStatus::Unmounted,
            MountRecovery::Clean,
            false,
        );
        finish_exit(&mut login, exit(75), &mut retention);
        assert_eq!(login.status, MountStatus::Unmounted);
        assert!(login.registry_recorded);
        assert!(retention.keeps(&login));
        // Changes left in the journal: `Failed` with the host's reason and
        // the update as cause; the recovery evidence stays.
        let mut pending = entry(
            "pending",
            false,
            MountStatus::Failed {
                detail: "1 change waits for Retry".into(),
            },
            MountRecovery::Required,
            true,
        );
        finish_exit(&mut pending, exit(75), &mut retention);
        match &pending.status {
            MountStatus::Failed { detail } => {
                assert!(detail.starts_with("Fuer ein Update beendet: "), "{detail}");
                assert!(detail.contains("1 change waits for Retry"), "{detail}");
            }
            other => panic!("unexpected status {other:?}"),
        }
        assert_eq!(pending.recovery, MountRecovery::Required);
        // A host that ended for the update before reporting its journal.
        let mut unknown = entry(
            "unknown",
            false,
            MountStatus::Mounting,
            MountRecovery::Unknown,
            false,
        );
        finish_exit(&mut unknown, exit(75), &mut retention);
        match &unknown.status {
            MountStatus::Failed { detail } => {
                assert!(detail.starts_with("Fuer ein Update beendet: "), "{detail}");
                assert!(detail.contains("Recovery-Status"), "{detail}");
            }
            other => panic!("unexpected status {other:?}"),
        }
        // A failure the host reported at the update stop (its shutdown
        // watchdog) stays visible although the journal is clean.
        let mut watchdog = entry(
            "watchdog",
            false,
            MountStatus::Failed {
                detail: "the mounted drive shutdown watchdog stopped".into(),
            },
            MountRecovery::Clean,
            true,
        );
        finish_exit(&mut watchdog, exit(75), &mut retention);
        match &watchdog.status {
            MountStatus::Failed { detail } => {
                assert!(detail.starts_with("Fuer ein Update beendet: "), "{detail}");
                assert!(detail.contains("watchdog"), "{detail}");
            }
            other => panic!("unexpected status {other:?}"),
        }
        // Any other non-zero exit stays the failure of v0.5.182.
        let mut crashed = entry(
            "crashed",
            false,
            MountStatus::Mounting,
            MountRecovery::Clean,
            false,
        );
        finish_exit(&mut crashed, exit(3), &mut retention);
        match &crashed.status {
            MountStatus::Failed { detail } => {
                assert!(!detail.contains("Update"), "{detail}");
            }
            other => panic!("unexpected status {other:?}"),
        }
    }
}
