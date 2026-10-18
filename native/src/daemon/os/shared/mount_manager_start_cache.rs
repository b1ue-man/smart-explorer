#[cfg(any(windows, target_os = "linux"))]
use std::path::{Path, PathBuf};

#[cfg(any(windows, target_os = "linux"))]
use crate::mount::MountId;
use crate::mount::{MountConfig, MountRecovery, MountStatus};

use super::{MountEntry, MountManager};

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
            .map(|config| match audit(&cache_root, &config.id) {
                Ok(recovery) => recovery,
                Err(error) => {
                    super::super::state::log(&format!(
                        "registered mount {} recovery audit failed: {error}",
                        config.id
                    ));
                    MountRecovery::Unknown
                }
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
) -> Result<(), String> {
    let mut state = manager.state_guard()?;
    if state.contains_key(key) {
        return Err("Diese Laufwerk-ID wird bereits verwaltet".into());
    }
    state.insert(
        key.to_string(),
        MountEntry {
            config,
            status: MountStatus::Mounting,
            backend: None,
            capabilities: None,
            launch_token: None,
            session_token: None,
            backend_token: None,
            control: None,
            child: None,
            backend_stream_active: false,
            pending_host_failure: None,
            host_terminal_failure: false,
            registry_recorded,
            recovery: MountRecovery::Unknown,
        },
    );
    Ok(())
}

#[cfg(any(windows, target_os = "linux"))]
pub(super) fn prepare(manager: &MountManager, key: &str, id: &MountId) -> Result<PathBuf, String> {
    let cache_root =
        crate::mount::prepare_spool_root(&crate::support_dirs::app_data_dir().join("mount-cache"))
            .map_err(|error| format!("Laufwerk-Cache absichern: {error}"))?;
    let recovery = match audit(&cache_root, id) {
        Ok(recovery) => recovery,
        Err(error) => {
            record(manager, key, MountRecovery::Unknown)?;
            return Err(format!("Laufwerk-Recovery lokal pruefen: {error}"));
        }
    };
    record(manager, key, recovery)?;
    Ok(cache_root)
}

/// The recovery state of a drive whose host does not run, read under the
/// cache lease of the platform (DRV-M2; Linux since DRV-M4).
#[cfg(windows)]
fn audit(cache_root: &Path, id: &MountId) -> std::io::Result<MountRecovery> {
    crate::mount::os::windows::audit_recovery(cache_root, id)
}

#[cfg(target_os = "linux")]
fn audit(cache_root: &Path, id: &MountId) -> std::io::Result<MountRecovery> {
    crate::mount::os::linux_os::audit_recovery(cache_root, id)
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
