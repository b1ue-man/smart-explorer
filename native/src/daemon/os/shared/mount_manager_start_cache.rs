#[cfg(any(windows, target_os = "linux"))]
use std::path::{Path, PathBuf};

#[cfg(any(windows, target_os = "linux"))]
use crate::mount::MountId;
use crate::mount::{MountConfig, MountRecovery, MountStatus};

use super::{MountEntry, MountManager};

#[derive(Clone, Copy)]
pub(super) enum StartEntry {
    New,
    Retry,
}

pub(super) fn ensure_retryable(entry: &MountEntry) -> Result<(), String> {
    if entry.child.is_some() || entry.backend_stream_active {
        return Err(
            "Die vorherige Laufwerk-Generation beendet noch einen Remote-Aufruf; Retry ist erst danach sicher"
                .into(),
        );
    }
    Ok(())
}

pub(super) fn audit_registered(configs: &[MountConfig]) -> Vec<MountRecovery> {
    #[cfg(any(windows, target_os = "linux"))]
    {
        let cache_root = match crate::mount::prepare_spool_root(
            &crate::support_dirs::app_data_dir().join("mount-cache"),
        ) {
            Ok(path) => path,
            Err(error) => {
                super::super::state::log(&format!(
                    "registered mount recovery root audit failed: {error}"
                ));
                return vec![MountRecovery::Unknown; configs.len()];
            }
        };
        return configs
            .iter()
            .map(|config| match config.source {
                // An offline registry load cannot verify today's account.
                // Do not open/compact/clean this cache before its start gate.
                crate::mount::MountSource::GoogleDrive { .. } => MountRecovery::Unknown,
                _ => match audit(&cache_root, &config.id) {
                Ok(pending) => recovery_of(pending),
                Err(error) => {
                    super::super::state::log(&format!(
                        "registered mount {} recovery audit failed: {error}",
                        config.id
                    ));
                    MountRecovery::Unknown
                }
                },
            })
            .collect();
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    vec![MountRecovery::Unknown; configs.len()]
}

pub(super) fn insert(
    manager: &MountManager,
    key: &str,
    config: MountConfig,
    registry_recorded: bool,
    startup_lease: Option<std::sync::Arc<dyn Send+Sync>>,
) -> Result<(), String> {
    insert_generation(manager, key, config, registry_recorded, startup_lease, StartEntry::New)
}

pub(super) fn restart(
    manager: &MountManager,
    key: &str,
    config: MountConfig,
    registry_recorded: bool,
    startup_lease: Option<std::sync::Arc<dyn Send+Sync>>,
) -> Result<(), String> {
    insert_generation(manager, key, config, registry_recorded, startup_lease, StartEntry::Retry)
}

fn insert_generation(
    manager: &MountManager,
    key: &str,
    config: MountConfig,
    registry_recorded: bool,
    startup_lease: Option<std::sync::Arc<dyn Send+Sync>>,
    admission: StartEntry,
) -> Result<(), String> {
    let mut state = manager.state_guard()?;
    // A retry keeps its previous entry visible through every fallible start
    // gate. Only this locked commit may replace that idle generation. Recheck
    // stream ownership here because host IPC does not take the lifecycle lock.
    let recovery = match (admission, state.get(key)) {
        (StartEntry::New, None) => MountRecovery::Unknown,
        (StartEntry::New, Some(_)) => {
            return Err("Diese Laufwerk-ID wird bereits verwaltet".into());
        }
        (StartEntry::Retry, None) => return Err("Das Laufwerk wird nicht verwaltet".into()),
        (StartEntry::Retry, Some(entry)) => {
            ensure_retryable(entry)?;
            entry.recovery
        }
    };
    state.insert(
        key.to_string(),
        MountEntry {
            capacity: None,
            config,
            status: MountStatus::Mounting,
            backend: None,
            capabilities: None,
            launch_token: None,
            session_token: None,
            backend_token: None,
            control: None,
            child: None,
            startup_lease:startup_lease.map(super::start_path::StartupLease::new),
            backend_stream_active: false,
            pending_host_failure: None,
            host_terminal_failure: false,
            registry_recorded,
            recovery,
            lock_end: false,
            lock_pending: None,
            stop_requested: false,
        },
    );
    Ok(())
}

/// The cache root and the journal records the drive still owns (DRV-M7:
/// the count of a vault drive whose vault is locked).
#[cfg(any(windows, target_os = "linux"))]
pub(super) fn prepare(
    manager: &MountManager,
    key: &str,
    id: &MountId,
) -> Result<(PathBuf, Option<usize>), String> {
    let cache_root =
        crate::mount::prepare_spool_root(&crate::support_dirs::app_data_dir().join("mount-cache"))
            .map_err(|error| format!("Laufwerk-Cache absichern: {error}"))?;
    let config = manager.state_guard()?.get(key)
        .ok_or_else(|| "Laufwerk-Start wurde abgebrochen".to_string())?.config.clone();
    if matches!(config.source, crate::mount::MountSource::GoogleDrive { .. }) {
        record(manager, key, MountRecovery::Unknown)?;
        crate::mount::preflight_mount_recovery_identity(&cache_root, &config)
            .map_err(|error| error.to_string())?;
        return Ok((cache_root, None));
    }
    let pending = match audit(&cache_root, id) {
        Ok(pending) => pending,
        Err(error) => {
            record(manager, key, MountRecovery::Unknown)?;
            // A missing offline lease or an unreadable count grants no Clean
            // state. The authorized host may still open/repair with its key.
            super::super::state::log(&format!("mount {id} offline inventory unknown: {error}"));
            None
        }
    };
    record(manager, key, recovery_of(pending))?;
    Ok((cache_root, pending))
}

#[cfg(any(windows, target_os = "linux"))]
fn recovery_of(pending: Option<usize>) -> MountRecovery {
    match pending {
        Some(0) => MountRecovery::Clean,
        Some(_) => MountRecovery::Required,
        None => MountRecovery::Unknown,
    }
}

/// The journal records of a drive whose host does not run, read under the
/// cache lease of the platform (DRV-M2; Linux since DRV-M4; counted since
/// DRV-M7).
#[cfg(windows)]
fn audit(cache_root: &Path, id: &MountId) -> std::io::Result<Option<usize>> {
    crate::mount::os::windows::audit_pending_state(cache_root, id)
}

#[cfg(target_os = "linux")]
fn audit(cache_root: &Path, id: &MountId) -> std::io::Result<Option<usize>> {
    crate::mount::os::linux_os::audit_pending_state(cache_root, id)
}

#[cfg(any(windows, target_os = "linux"))]
fn record(manager: &MountManager, key: &str, recovery: MountRecovery) -> Result<(), String> {
    let mut state = manager.state_guard()?;
    let entry = state
        .get_mut(key)
        .ok_or_else(|| "Laufwerk-Start wurde abgebrochen".to_string())?;
    entry.recovery = recovery;
    Ok(())
}
