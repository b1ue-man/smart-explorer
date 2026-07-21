use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::ipc_protocol::ShareWorkerSnapshot;
use super::state::log;

#[path = "ipc_host_direct_event_persistence.rs"]
pub(super) mod direct_event_persistence;
#[path = "ipc_host_direct_event_queue.rs"]
pub(super) mod direct_event_queue;
#[path = "ipc_host_direct_event_schedule.rs"]
pub(super) mod direct_event_schedule;
#[path = "ipc_host_direct_events.rs"]
pub(super) mod direct_events;
#[path = "exec_grant_journal.rs"]
pub(super) mod exec_grant_journal;
#[path = "ipc_host_legacy_events.rs"]
pub(super) mod legacy_events;
#[path = "ipc_host_profile_merge.rs"]
pub(super) mod profile_merge;
#[path = "ipc_host_stop.rs"]
mod stop;
#[path = "ipc_host_ui_events.rs"]
pub(super) mod ui_events;

#[cfg(test)]
#[path = "ipc_host_direct_events_tests.rs"]
mod direct_events_tests;

const MAX_SHARE_SERVER_BYTES: u64 = 16 * 1024;

#[derive(Clone)]
pub(crate) struct ShareHost {
    pub(super) state: Arc<Mutex<ShareHostState>>,
    generation: Arc<str>,
    initialized: Arc<AtomicBool>,
    pub(super) exec_state: Arc<super::exec_state::ExecState>,
    exec_grant_lock: Arc<Mutex<()>>,
    pub(super) mounts: super::mount_manager::MountManager,
}

pub(super) struct ShareHostState {
    pub(super) service: Option<crate::share::ShareService>,
    pub(super) identity: Option<crate::share::ShareIdentity>,
    pub(super) identity_error: Option<String>,
    pub(super) profiles: crate::share::ShareProfiles,
    pub(super) profiles_error: Option<String>,
    /// Explicit Share stop barrier. Periodic reloads may refresh state while
    /// suspended, but only an IPC RefreshShare request may start service again.
    pub(super) suspended: bool,
    pub(super) server: String,
    pub(super) running_server: String,
    pub(super) signal_connected: bool,
    pub(super) signal_error: Option<String>,
    pub(super) last_reload: Instant,
    pub(super) ui_events: Vec<crate::share::ShareEvent>,
    /// Baseline for daemon-owned runtime mutations that could not yet be
    /// durably rebased. Keeping it makes consumed service events retryable.
    pub(super) pending_profiles_base: Option<crate::share::ShareProfiles>,
    pub(super) pending_direct_events: Vec<direct_event_queue::PendingDirectEvent>,
    pub(super) pending_legacy_events: Vec<(String, crate::share::PeerPresence)>,
    pub(super) exec_retry: Option<exec_grant_journal::ExecGrantPersistResult>,
}

impl ShareHost {
    pub(crate) fn new(generation: String) -> Self {
        // Keep construction free of credential and profile I/O so the daemon
        // can publish its authenticated Ping endpoint before a contended
        // identity transaction or slow Windows Credential Manager access.
        // The daemon calls reload_now immediately after listener publication.
        let state = ShareHostState {
            service: None,
            identity: None,
            identity_error: None,
            profiles: crate::share::ShareProfiles::default(),
            profiles_error: None,
            suspended: false,
            server: String::new(),
            running_server: String::new(),
            signal_connected: false,
            signal_error: None,
            last_reload: Instant::now() - Duration::from_secs(60),
            ui_events: Vec::new(),
            pending_profiles_base: None,
            pending_direct_events: Vec::new(),
            pending_legacy_events: Vec::new(),
            exec_retry: None,
        };
        ShareHost {
            state: Arc::new(Mutex::new(state)),
            generation: Arc::from(generation),
            initialized: Arc::new(AtomicBool::new(false)),
            exec_state: Arc::new(super::exec_state::ExecState::new()),
            exec_grant_lock: Arc::new(Mutex::new(())),
            mounts: super::mount_manager::MountManager::default(),
        }
    }

    pub(crate) fn generation(&self) -> &str {
        &self.generation
    }

    pub(crate) fn initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }

    pub(crate) fn mark_initialized(&self) {
        self.initialized.store(true, Ordering::Release);
    }

    pub(crate) fn tick(&self) {
        self.mounts.tick();
        self.drain_events();
        let should_reload = self
            .state
            .lock()
            .map(|state| state.last_reload.elapsed() >= Duration::from_secs(5))
            .unwrap_or(false);
        if should_reload {
            if let Err(error) = self.reload_now() {
                log(&format!("share worker reload failed: {error}"));
            }
        }
    }

    pub(crate) fn stop_mounts(&self) {
        self.mounts.stop_all();
    }

    pub(crate) fn reload_now(&self) -> Result<bool, String> {
        let _exclusive = self
            .exec_grant_lock
            .lock()
            .map_err(|_| "Exec-Grant mutation lock is poisoned".to_string())?;
        let running = self.reload_now_locked()?;
        drop(_exclusive);
        self.flush_legacy_answers();
        Ok(running)
    }

    pub(super) fn reload_now_locked(&self) -> Result<bool, String> {
        self.drain_events();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Share-Worker State ist gesperrt".to_string())?;
        if state.pending_profiles_base.is_some() {
            return Err(
                "Share-Status wartet nach einem Speicherfehler auf einen erneuten Commit".into(),
            );
        }
        state.last_reload = Instant::now();
        state.server = match load_share_server() {
            Ok(server) => server,
            Err(error) => {
                stop_service_locked(&mut state)?;
                return Err(format!("Share-Server-Konfiguration lesen: {error}"));
            }
        };
        match crate::share::ShareIdentity::load_or_create(default_device_name()) {
            Ok(identity) => {
                state.identity = Some(identity);
                state.identity_error = None;
            }
            Err(error) => {
                state.identity = None;
                state.identity_error = Some(error.clone());
                stop_service_locked(&mut state)?;
                return Err(format!("Share-Identitaet nicht verfuegbar: {error}"));
            }
        }
        let pending = exec_grant_journal::load_pending().map_err(|error| {
            exec_grant_journal::mask_all(&mut state.profiles);
            state.profiles_error = Some(error.clone());
            let _ = stop_service_locked(&mut state);
            format!("Exec-Grant Recovery nicht verfuegbar: {error}")
        })?;
        match crate::share::ShareProfiles::load_checked(Some(default_home())) {
            Ok(mut profiles) => {
                let identity = state
                    .identity
                    .clone()
                    .ok_or_else(|| "Share-Identitaet nicht verfuegbar".to_string())?;
                if !profiles.legacy_direct_requests.is_empty() {
                    profiles = crate::share::refresh_legacy_request_expiry(
                        Some(default_home()),
                        &identity,
                    )
                    .map_err(|error| {
                        state.profiles_error = Some(error.clone());
                        let _ = stop_service_locked(&mut state);
                        format!("Legacy-Anfragen konnten nicht authentifiziert werden: {error}")
                    })?;
                }
                if let Some(entry) = &pending {
                    exec_grant_journal::prepare_pending_runtime(&mut state, &mut profiles, entry)?;
                }
                state.profiles = profiles;
                state.profiles_error = None;
            }
            Err(error) => {
                state.profiles_error = Some(error.clone());
                stop_service_locked(&mut state)?;
                return Err(format!("Share-Profile nicht verfuegbar: {error}"));
            }
        }
        configure_or_restart_locked(&mut state)?;
        if let Some(entry) = &pending {
            exec_grant_journal::recover_and_record(&mut state, entry);
        } else {
            state.exec_retry = None;
        }
        Ok(state.service.is_some())
    }

    pub(crate) fn refresh_now(&self) -> Result<bool, String> {
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "Share-Worker State ist gesperrt".to_string())?;
            state.suspended = false;
        }
        self.reload_now()
    }

    pub(super) fn send_command(&self, cmd: crate::share::ShareCmd) -> Result<(), String> {
        if matches!(
            &cmd,
            crate::share::ShareCmd::EnableExec { .. }
                | crate::share::ShareCmd::DisableExec { .. }
                | crate::share::ShareCmd::ApplyExecGrant { .. }
        ) {
            return Err("Exec-Grant changes require the durable daemon mutation IPC".into());
        }
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "Share-Worker State ist gesperrt".to_string())?;
            if let crate::share::ShareCmd::Stop = &cmd {
                return stop::stop_locked(&mut state);
            }
        }
        self.reload_now()?;
        let service = {
            let state = self
                .state
                .lock()
                .map_err(|_| "Share-Worker State ist gesperrt".to_string())?;
            state.service.clone()
        }
        .ok_or_else(|| "Share-Worker ist nicht aktiv".to_string())?;
        service.cmd(cmd).map(|_| ())
    }

    pub(super) fn drain_for_ui(&self) -> ShareWorkerSnapshot {
        let should_reload = self
            .state
            .lock()
            .map(|state| state.last_reload.elapsed() >= Duration::from_secs(5))
            .unwrap_or(false);
        if should_reload {
            if let Err(error) = self.reload_now() {
                if let Ok(mut state) = self.state.lock() {
                    ui_events::push(&mut state.ui_events, crate::share::ShareEvent::Error(error));
                }
            }
        }
        self.drain_events();
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => return ShareWorkerSnapshot::default(),
        };
        let running = state.service.is_some();
        let (relay_url, candidates) = state
            .service
            .as_ref()
            .map(|service| (service.relay_url(), service.peer_candidates()))
            .unwrap_or_default();
        ShareWorkerSnapshot {
            events: std::mem::take(&mut state.ui_events),
            profiles: state.profiles.clone(),
            profile_revision: state.profiles.storage_revision.clone(),
            exec_grant_retry: state.exec_retry.clone(),
            pending_direct_requests: Vec::new(),
            running,
            connected: state.signal_connected,
            last_error: state.signal_error.clone(),
            relay_url,
            candidates,
        }
    }

    pub(crate) fn open_share(
        &self,
        target: crate::share::PeerOpenTarget,
    ) -> Result<(String, crate::vfs::BackendHandle, crate::share::ShareStatus), String> {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            self.reload_now()?;
            self.drain_events();
            let service = {
                let state = self
                    .state
                    .lock()
                    .map_err(|_| "Share-Worker gesperrt".to_string())?;
                state.service.clone()
            };
            let Some(service) = service else {
                return Err("Share-Server ist nicht konfiguriert oder Auto-Connect ist aus".into());
            };
            service.cmd(crate::share::ShareCmd::Refresh)?;
            match service.probe_backend_for_target(&target) {
                Ok(opened) => return Ok(opened),
                Err(error) => {
                    if Instant::now() >= deadline {
                        return Err(error);
                    }
                    std::thread::sleep(Duration::from_millis(750));
                }
            }
        }
    }

    pub(crate) fn exec_share(
        &self,
        target: crate::share::PeerOpenTarget,
        req: crate::share::ExecRequest,
    ) -> Result<crate::share::ExecResult, String> {
        self.reload_now()?;
        self.drain_events();
        let service = {
            let state = self
                .state
                .lock()
                .map_err(|_| "Share-Worker gesperrt".to_string())?;
            state.service.clone()
        };
        let Some(service) = service else {
            return Err("Share-Server ist nicht konfiguriert oder Auto-Connect ist aus".into());
        };
        service.cmd(crate::share::ShareCmd::Refresh)?;
        // Exec is intentionally at-most-once. Once handed to the Share service,
        // a transport error is ambiguous: the peer may already have started it.
        service.exec_for_target(&target, req)
    }
}

fn configure_or_restart_locked(state: &mut ShareHostState) -> Result<(), String> {
    if let Some(error) = &state.identity_error {
        return Err(format!("Share-Identitaet nicht verfuegbar: {error}"));
    }
    if let Some(error) = &state.profiles_error {
        return Err(format!("Share-Profile nicht verfuegbar: {error}"));
    }
    let identity = state
        .identity
        .clone()
        .ok_or_else(|| "Share-Identitaet nicht verfuegbar".to_string())?;
    if !share_service_requested(state.suspended, &state.server, state.profiles.auto_connect) {
        if let Some(service) = state.service.take() {
            service.cmd(crate::share::ShareCmd::Stop)?;
        }
        state.running_server.clear();
        state.signal_connected = false;
        state.signal_error = None;
        return Ok(());
    }
    let needs_restart = state
        .service
        .as_ref()
        .map(|service| {
            service.identity.node_id != identity.node_id
                || service.identity.device_id != identity.device_id
                || service.identity.device_name != identity.device_name
                || service.identity.direct_lookup_id != identity.direct_lookup_id
                || service.identity.direct_secret() != identity.direct_secret()
                || state.running_server != state.server
        })
        .unwrap_or(true);
    if needs_restart {
        if let Some(service) = state.service.take() {
            service.cmd(crate::share::ShareCmd::Stop)?;
        }
        state.running_server.clear();
        state.signal_connected = false;
        state.signal_error = None;
        match crate::share::ShareService::start(
            state.server.clone(),
            identity.clone(),
            state.profiles.clone(),
        ) {
            Ok(service) => {
                log("share worker started");
                configure_service(&service, &state.profiles)?;
                state.running_server = state.server.clone();
                state.service = Some(service);
            }
            Err(error) => return Err(format!("Share-Worker Start: {error}")),
        }
    } else if let Some(service) = &state.service {
        configure_service(service, &state.profiles)?;
    }
    Ok(())
}

fn share_service_requested(suspended: bool, server: &str, auto_connect: bool) -> bool {
    !suspended && !server.trim().is_empty() && auto_connect
}

fn stop_service_locked(state: &mut ShareHostState) -> Result<(), String> {
    if let Some(service) = state.service.take() {
        service.cmd(crate::share::ShareCmd::Stop)?;
    }
    state.running_server.clear();
    state.signal_connected = false;
    Ok(())
}

pub(super) fn configure_service(
    service: &crate::share::ShareService,
    profiles: &crate::share::ShareProfiles,
) -> Result<(), String> {
    service.cmd(crate::share::ShareCmd::Configure {
        direct: profiles.direct_contacts.clone(),
        direct_grants: profiles.direct_grants.clone(),
        rooms: profiles.rooms.clone(),
        default_direct_exports: profiles.default_direct_exports.clone(),
    })?;
    service
        .cmd(crate::share::ShareCmd::SyncDirectRequests {
            direct_requests: profiles.direct_requests.clone(),
            direct_request_tombstones: profiles.direct_request_tombstones.clone(),
        })
        .map(|_| ())
}

fn load_share_server() -> Result<String, String> {
    let path = crate::support_dirs::app_data_file("share_server.txt");
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.file_type().is_file() {
        return Err("Share server configuration is not a regular file".into());
    }
    if metadata.len() > MAX_SHARE_SERVER_BYTES {
        return Err("Share server configuration exceeds its 16 KiB limit".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    std::fs::File::open(path)
        .map_err(|error| error.to_string())?
        .take(MAX_SHARE_SERVER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_SHARE_SERVER_BYTES {
        return Err("Share server configuration exceeds its 16 KiB limit".into());
    }
    String::from_utf8(bytes)
        .map(|server| server.trim().to_string())
        .map_err(|_| "Share server configuration is not valid UTF-8".into())
}

fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Mein Geraet".to_string())
}

pub(super) fn default_home() -> String {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::share_service_requested;

    #[test]
    fn explicit_stop_barrier_blocks_periodic_auto_connect_reload() {
        assert!(share_service_requested(false, "127.0.0.1:9", true));
        assert!(!share_service_requested(true, "127.0.0.1:9", true));
        assert!(!share_service_requested(false, "", true));
        assert!(!share_service_requested(false, "127.0.0.1:9", false));
    }
}
