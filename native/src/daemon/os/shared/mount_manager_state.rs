use std::process::ExitStatus;

use crate::mount::{MountRecovery, MountSnapshot, MountStatus};

use super::MountEntry;

pub(super) fn finish_exit(entry: &mut MountEntry, exit: ExitStatus) {
    clear_host_runtime(entry);
    if exit.success() && entry.recovery.is_clean() {
        finish_clean(entry);
        return;
    }
    clear_backend_runtime(entry);
    if !matches!(
        entry.status,
        MountStatus::Failed { .. }
            | MountStatus::RuntimeUnavailable { .. }
            | MountStatus::Conflict { .. }
    ) {
        entry.status = MountStatus::Failed {
            detail: exit_detail(entry.recovery, exit),
        };
    }
}

fn exit_detail(recovery: MountRecovery, exit: ExitStatus) -> String {
    match recovery {
        MountRecovery::Clean => {
            format!("Laufwerk-Host wurde vor der Laufwerk-Bereitschaft beendet ({exit}); der lokale Recovery-Cache ist sauber")
        }
        MountRecovery::Required => {
            format!("Laufwerk-Host wurde beendet ({exit}); lokale Aenderungen bleiben fuer Retry im Recovery-Cache")
        }
        MountRecovery::Unknown => {
            format!("Laufwerk-Host wurde beendet, bevor der lokale Recovery-Status verifiziert werden konnte ({exit}); der Cache bleibt bis zur erneuten Pruefung erhalten")
        }
    }
}

pub(super) fn finish_clean(entry: &mut MountEntry) {
    clear_host_runtime(entry);
    entry.recovery = MountRecovery::Clean;
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
