use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::ipc_protocol::ShareWorkerSnapshot;
use super::state::log;

const MAX_SHARE_SERVER_BYTES: u64 = 16 * 1024;

#[derive(Clone)]
pub(crate) struct ShareHost {
    pub(super) state: Arc<Mutex<ShareHostState>>,
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
    pub(super) pending_direct_requests: Vec<crate::share::PeerPresence>,
}

impl ShareHost {
    pub(crate) fn new() -> Self {
        let (identity, identity_error) =
            match crate::share::ShareIdentity::load_or_create(default_device_name()) {
                Ok(identity) => (Some(identity), None),
                Err(error) => (None, Some(error)),
            };
        let (profiles, profiles_error) =
            match crate::share::ShareProfiles::load_checked(Some(default_home())) {
                Ok(profiles) => (profiles, None),
                Err(error) => (crate::share::ShareProfiles::default(), Some(error)),
            };
        let server = load_share_server().unwrap_or_else(|error| {
            log(&format!("share server configuration is invalid: {error}"));
            String::new()
        });
        let state = ShareHostState {
            service: None,
            identity,
            identity_error,
            profiles,
            profiles_error,
            suspended: false,
            server,
            running_server: String::new(),
            signal_connected: false,
            signal_error: None,
            last_reload: Instant::now() - Duration::from_secs(60),
            ui_events: Vec::new(),
            pending_direct_requests: Vec::new(),
        };
        ShareHost {
            state: Arc::new(Mutex::new(state)),
        }
    }

    pub(crate) fn tick(&self) {
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

    pub(crate) fn reload_now(&self) -> Result<bool, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Share-Worker State ist gesperrt".to_string())?;
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
        match crate::share::ShareProfiles::load_checked(Some(default_home())) {
            Ok(profiles) => {
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
        let answer_device = match &cmd {
            crate::share::ShareCmd::AnswerDirectRequest { presence, .. } => {
                Some(presence.device_id.clone())
            }
            _ => None,
        };
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "Share-Worker State ist gesperrt".to_string())?;
            if let crate::share::ShareCmd::Stop = &cmd {
                // Establish the barrier before asking the service to stop. If
                // delivery is ambiguous, periodic reloads still cannot race a
                // maintenance operation; an explicit RefreshShare releases it.
                state.suspended = true;
                let service = state.service.take();
                state.running_server.clear();
                state.signal_connected = false;
                state.signal_error = None;
                if let Some(service) = service {
                    service.cmd(crate::share::ShareCmd::Stop)?;
                }
                state.ui_events.push(crate::share::ShareEvent::Status(
                    "Share-Worker getrennt".to_string(),
                ));
                return Ok(());
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
        service.cmd(cmd)?;
        if let Some(device_id) = answer_device {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "Share-Worker State ist gesperrt".to_string())?;
            state
                .pending_direct_requests
                .retain(|pending| pending.device_id != device_id);
        }
        Ok(())
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
                    state.ui_events.push(crate::share::ShareEvent::Error(error));
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
            pending_direct_requests: state.pending_direct_requests.clone(),
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
    })
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

fn default_home() -> String {
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
