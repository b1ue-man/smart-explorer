use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};

use crate::mount::{DriveSelection, MountId, MountRecovery, MountSnapshot, MountStatus};
use crate::vfs::BackendHandle;

use super::{clear_backend_runtime, random_token, snapshot, MountEntry, MountManager};

pub(in crate::daemon) struct HostGrant {
    pub(in crate::daemon) config: super::super::ipc_protocol::MountHostConfig,
    pub(in crate::daemon) scheme: super::super::ipc_protocol::MountBackendScheme,
    pub(in crate::daemon) capabilities: super::super::ipc_protocol::MountBackendCapabilities,
    pub(in crate::daemon) session_token: String,
    pub(in crate::daemon) backend_token: String,
}

/// Keeps one daemon/backend generation alive until its serving thread and all
/// request workers have completely returned. Dropping this guard is the only
/// path that releases the generation fence.
pub(in crate::daemon) struct BackendStreamLease {
    manager: MountManager,
    id: MountId,
}

impl Drop for BackendStreamLease {
    fn drop(&mut self) {
        self.manager.backend_stream_finished(&self.id);
    }
}

impl MountManager {
    pub(in crate::daemon) fn grant_host(
        &self,
        id: &MountId,
        launch_token: &str,
    ) -> Result<HostGrant, String> {
        let mut state = self.state_guard()?;
        let entry = state
            .get_mut(id.as_str())
            .ok_or_else(|| "Unbekannter Laufwerk-Host".to_string())?;
        let expected = entry
            .launch_token
            .as_deref()
            .ok_or_else(|| "Laufwerk-Host wurde bereits gebunden".to_string())?;
        if !token_matches(expected, launch_token) {
            return Err("Laufwerk-Host-Token wurde abgelehnt".into());
        }
        let backend = entry
            .backend
            .as_ref()
            .ok_or_else(|| "Laufwerk-Backend ist nicht mehr verfuegbar".to_string())?;
        let scheme = backend.scheme().into();
        let capabilities = entry
            .capabilities
            .ok_or_else(|| "Laufwerk-Backend-Faehigkeiten fehlen".to_string())?;
        let session_token = random_token()?;
        let backend_token = random_token()?;
        entry.launch_token = None;
        entry.session_token = Some(session_token.clone());
        entry.backend_token = Some(backend_token.clone());
        Ok(HostGrant {
            config: (&entry.config).into(),
            scheme,
            capabilities,
            session_token,
            backend_token,
        })
    }

    pub(in crate::daemon) fn check_launch_token(
        &self,
        id: &MountId,
        launch_token: &str,
    ) -> Result<(), String> {
        let state = self.state_guard()?;
        let entry = state
            .get(id.as_str())
            .ok_or_else(|| "Unbekannter Laufwerk-Host".to_string())?;
        match entry.launch_token.as_deref() {
            Some(expected) if token_matches(expected, launch_token) => Ok(()),
            _ => Err("Laufwerk-Host-Token wurde abgelehnt".into()),
        }
    }

    pub(in crate::daemon) fn check_backend_token(
        &self,
        id: &MountId,
        backend_token: &str,
    ) -> Result<(), String> {
        let state = self.state_guard()?;
        let entry = state
            .get(id.as_str())
            .ok_or_else(|| "Unbekannter Laufwerk-Host".to_string())?;
        match entry.backend_token.as_deref() {
            Some(expected) if token_matches(expected, backend_token) => Ok(()),
            _ => Err("Laufwerk-Backend-Token wurde abgelehnt".into()),
        }
    }

    pub(in crate::daemon) fn check_session_token(
        &self,
        id: &MountId,
        session_token: &str,
    ) -> Result<(), String> {
        let mut state = self.state_guard()?;
        authenticated_entry(&mut state, id, session_token).map(|_| ())
    }

    pub(in crate::daemon) fn register_control(
        &self,
        id: &MountId,
        session_token: &str,
    ) -> Result<Receiver<()>, String> {
        let (send, receive) = mpsc::sync_channel(1);
        let mut state = self.state_guard()?;
        let entry = authenticated_entry(&mut state, id, session_token)?;
        if entry.control.is_some() {
            return Err("Laufwerk-Control ist bereits verbunden".into());
        }
        let stopping = matches!(
            entry.status,
            MountStatus::Unmounting
                | MountStatus::Unmounted
                | MountStatus::RuntimeUnavailable { .. }
        ) || entry.host_terminal_failure
            || entry.pending_host_failure.is_some();
        entry.control = Some(send.clone());
        if stopping {
            let _ = send.try_send(());
        }
        Ok(receive)
    }

    pub(in crate::daemon) fn take_backend(
        &self,
        id: &MountId,
        backend_token: &str,
    ) -> Result<(BackendHandle, BackendStreamLease), String> {
        let mut state = self.state_guard()?;
        let entry = state
            .get_mut(id.as_str())
            .ok_or_else(|| "Unbekannter Laufwerk-Host".to_string())?;
        let expected = entry
            .backend_token
            .as_deref()
            .ok_or_else(|| "Laufwerk-Backend wurde bereits verbunden".to_string())?;
        if !token_matches(expected, backend_token) {
            return Err("Laufwerk-Backend-Token wurde abgelehnt".into());
        }
        if entry.backend_stream_active {
            return Err("Laufwerk-Backend ist bereits verbunden".into());
        }
        let backend = entry
            .backend
            .clone()
            .ok_or_else(|| "Laufwerk-Backend ist nicht mehr verfuegbar".to_string())?;
        entry.backend_token = None;
        entry.backend_stream_active = true;
        Ok((
            backend,
            BackendStreamLease {
                manager: self.clone(),
                id: id.clone(),
            },
        ))
    }

    fn backend_stream_finished(&self, id: &MountId) {
        let control = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            let Some(entry) = state.get_mut(id.as_str()) else {
                return;
            };
            entry.backend_stream_active = false;
            clear_backend_runtime(entry);
            if matches!(
                entry.status,
                MountStatus::Unmounting | MountStatus::Unmounted
            ) {
                return;
            }
            let host_reported_terminal =
                matches!(entry.status, MountStatus::RuntimeUnavailable { .. })
                    || entry.host_terminal_failure;
            if host_reported_terminal {
                // The authenticated host already supplied the actionable
                // failure and, where applicable, an exact recovery boundary.
                // Ask it to stop without weakening that evidence.
                entry.control.clone()
            } else {
                entry.recovery = MountRecovery::Unknown;
                // The process stderr fallback usually arrives a few instructions
                // later. Keep this internal so polling cannot expose two failures
                // for one host generation.
                entry.pending_host_failure.get_or_insert_with(|| {
                    "Die private Laufwerk-Backend-Verbindung wurde unerwartet beendet".into()
                });
                entry.control.clone()
            }
        };
        if let Some(control) = control {
            let _ = control.try_send(());
        }
    }

    pub(in crate::daemon) fn update_status(
        &self,
        id: &MountId,
        session_token: &str,
        status: MountStatus,
        recovery: Option<MountRecovery>,
    ) -> Result<MountSnapshot, String> {
        validate_host_status(&status)?;
        if recovery.is_some()
            && !matches!(
                &status,
                MountStatus::Mounting | MountStatus::Failed { .. } | MountStatus::Unmounted
            )
        {
            return Err("Laufwerk-Host meldete Recovery-Status an einer ungueltigen Grenze".into());
        }
        let mut state = self.state_guard()?;
        let entry = authenticated_entry(&mut state, id, session_token)?;
        if !host_transition_allowed(&entry.status, &status) {
            return Err("Laufwerk-Host meldete einen ungueltigen Statuswechsel".into());
        }
        let reported_drive = match &status {
            MountStatus::Mounted { drive } | MountStatus::Conflict { drive, .. } => Some(drive),
            _ => None,
        };
        if let (Some(drive), DriveSelection::Letter(expected)) =
            (reported_drive, entry.config.drive)
        {
            if *drive != expected {
                return Err("Laufwerk-Host meldete einen anderen Laufwerksbuchstaben".into());
            }
        }
        if matches!(
            &status,
            MountStatus::Mounted { .. } | MountStatus::Conflict { .. }
        ) {
            // A running filesystem may have accepted a write immediately
            // after Dokany mounted it. Preserve its cache until a closed host
            // explicitly proves that no retryable state remains.
            entry.recovery = MountRecovery::Unknown;
        }
        if let Some(recovery) = recovery {
            entry.recovery = recovery;
        }
        let terminal_failure = host_status_is_terminal(&status, recovery.is_some());
        if terminal_failure {
            entry.pending_host_failure = None;
        }
        entry.host_terminal_failure = terminal_failure;
        entry.status = status;
        Ok(snapshot(entry))
    }
}

fn authenticated_entry<'a>(
    state: &'a mut HashMap<String, MountEntry>,
    id: &MountId,
    session_token: &str,
) -> Result<&'a mut MountEntry, String> {
    let entry = state
        .get_mut(id.as_str())
        .ok_or_else(|| "Unbekannter Laufwerk-Host".to_string())?;
    let expected = entry
        .session_token
        .as_deref()
        .ok_or_else(|| "Laufwerk-Host ist nicht authentifiziert".to_string())?;
    token_matches(expected, session_token)
        .then_some(entry)
        .ok_or_else(|| "Laufwerk-Session-Token wurde abgelehnt".to_string())
}

fn token_matches(expected: &str, actual: &str) -> bool {
    if expected.len() != actual.len() {
        return false;
    }
    expected
        .as_bytes()
        .iter()
        .zip(actual.as_bytes())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn validate_host_status(status: &MountStatus) -> Result<(), String> {
    const MAX_DETAIL_BYTES: usize = 16 * 1024;
    const MAX_PATH_BYTES: usize = 8 * 1024;
    match status {
        MountStatus::Mounted { .. } | MountStatus::Unmounted | MountStatus::Mounting => Ok(()),
        MountStatus::RuntimeUnavailable { detail } | MountStatus::Failed { detail }
            if detail.len() <= MAX_DETAIL_BYTES =>
        {
            Ok(())
        }
        MountStatus::Conflict { path, detail, .. }
            if path.len() <= MAX_PATH_BYTES && detail.len() <= MAX_DETAIL_BYTES =>
        {
            Ok(())
        }
        MountStatus::Unmounting => Err("Nur der Daemon darf einen Laufwerk-Stop einleiten".into()),
        _ => Err("Laufwerk-Hoststatus ueberschreitet das IPC-Limit".into()),
    }
}

fn host_transition_allowed(current: &MountStatus, next: &MountStatus) -> bool {
    match next {
        MountStatus::Unmounting => false,
        MountStatus::Mounting => matches!(current, MountStatus::Mounting),
        MountStatus::Mounted { .. } => matches!(
            current,
            MountStatus::Mounting | MountStatus::Mounted { .. } | MountStatus::Conflict { .. }
        ),
        MountStatus::Conflict { .. } => matches!(
            current,
            MountStatus::Mounting | MountStatus::Mounted { .. } | MountStatus::Conflict { .. }
        ),
        MountStatus::RuntimeUnavailable { .. } => {
            !matches!(current, MountStatus::Unmounting | MountStatus::Unmounted)
        }
        // A graceful Dokany close can discover a dirty/conflicted journal only
        // after the daemon has initiated Unmounting. Preserve that recovery
        // signal; only a terminal Unmounted state rejects further failures.
        MountStatus::Failed { .. } => !matches!(current, MountStatus::Unmounted),
        MountStatus::Unmounted => true,
    }
}

fn host_status_is_terminal(status: &MountStatus, has_recovery_boundary: bool) -> bool {
    matches!(status, MountStatus::RuntimeUnavailable { .. })
        || (matches!(status, MountStatus::Failed { .. }) && has_recovery_boundary)
}

#[cfg(test)]
#[path = "mount_manager_host_task_tests.rs"]
mod task_tests;
