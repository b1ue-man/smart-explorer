//! The installed native RDP client of this OS (design 2.4): inventory, the
//! derived choice, trust and launch. Windows and Linux have adapters; the
//! remaining platforms (Android) have none, which every call reports.
use crate::rdp::core::{ClientCapability, HostService, StartRequest};
use crate::rdp::os::platform::clients as platform;
use crate::share::apps_wire::ManagedSecret;
use std::{
    net::SocketAddr,
    process::{Child, Command},
};

/// Shown where FreeRDP would need a terminal for its credential prompt
/// and no credentials were handed over (MCP, Android bridge; design 2.8).
pub(crate) const NEEDS_SDL: &str =
    "Für Remote Desktop ohne Terminal das Paket freerdp3-sdl installieren";

const NO_CLIENT: &str =
    "Auf diesem Gerät gibt es keinen RDP-Client; Bildschirm ansehen/steuern bleibt verfügbar";

pub(crate) struct ClientProcess {
    pub(crate) child: Child,
    // Platform-owned temporary profiles remain alive until the child exits.
    pub(crate) resources: Box<dyn Send>,
}

/// Credentials se hands to the client: the host's se-managed service
/// credentials (`RdpCredential`) or what the person typed into the desktop
/// GUI's own dialog (Linux without `sdl-freerdp3`, design 2.8). Never
/// account passwords from a store, never in argv, files or logs.
pub(crate) struct Credentials {
    pub(crate) username: String,
    pub(crate) domain: Option<String>,
    pub(crate) password: ManagedSecret,
}
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("domain", &self.domain)
            .finish_non_exhaustive()
    }
}

pub(crate) fn local_clients() -> Vec<ClientCapability> {
    platform::inventory()
}

/// Design 2.4: the first available client in the platform's order, or the
/// requested one (`--client` stays as an override).
pub(crate) fn choose_client(wanted: Option<&str>) -> Result<ClientCapability, String> {
    let clients = local_clients();
    if clients.is_empty() {
        return Err(NO_CLIENT.into());
    }
    let found = match wanted {
        Some(id) => clients.iter().find(|client| client.id == id),
        None => clients.iter().find(|client| client.available),
    };
    match found {
        Some(client) if client.available => Ok(client.clone()),
        Some(client) => Err(client.detail.clone()),
        None if wanted.is_some() => Err(
            "The selected RDP client is not installed or has no supported verification adapter"
                .into(),
        ),
        None => Err(clients
            .iter()
            .map(|client| format!("{}: {}", client.id, client.detail))
            .collect::<Vec<_>>()
            .join("; ")),
    }
}

pub(crate) fn check_client(request: &StartRequest) -> Result<ClientCapability, String> {
    let client = local_clients()
        .into_iter()
        .find(|client| client.id == request.client_id)
        .ok_or_else(|| {
            "The selected RDP client is not installed or has no supported verification adapter"
                .to_string()
        })?;
    if !client.available {
        return Err(client.detail.clone());
    }
    if client.certificate_policy != request.certificate_policy {
        return Err(
            "Selected RDP client does not implement the requested certificate policy".into(),
        );
    }
    if request.remote_app.is_some() && !client.remote_app {
        return Err("Selected native client has no implemented RemoteApp start adapter".into());
    }
    Ok(client)
}

pub(crate) fn launch(
    request: &StartRequest,
    service: &HostService,
    listener: SocketAddr,
    credentials: Option<&Credentials>,
) -> Result<ClientProcess, String> {
    check_client(request)?;
    if !listener.ip().is_loopback() || listener.port() == 0 {
        return Err("RDP client requires a loopback tunnel".into());
    }
    platform_launch(request, service, listener, credentials)
}

pub(crate) fn hide_probe_console(command: &mut Command) {
    platform::hide_probe_console(command);
}

#[cfg(any(windows, target_os = "linux"))]
fn platform_launch(
    request: &StartRequest,
    service: &HostService,
    listener: SocketAddr,
    credentials: Option<&Credentials>,
) -> Result<ClientProcess, String> {
    platform::launch(request, service, listener, credentials)
}
#[cfg(not(any(windows, target_os = "linux")))]
fn platform_launch(
    request: &StartRequest,
    service: &HostService,
    listener: SocketAddr,
    _: Option<&Credentials>,
) -> Result<ClientProcess, String> {
    platform::launch(request, service, listener)
}

/// Whether the client could not ask for account credentials itself
/// (FreeRDP without terminal; design 2.8).
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn needs_credential_input(client_id: &str) -> bool {
    platform::needs_credential_input(client_id)
}
#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn needs_credential_input(_: &str) -> bool {
    false
}

/// Trust the client needs for this listener (mstsc `CertHash`, session
/// credential); the guard lives as long as the client.
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn prepare_trust(
    listener: SocketAddr,
    service: &HostService,
    credentials: Option<&Credentials>,
) -> Result<Box<dyn Send>, String> {
    platform::prepare_trust(listener, service, credentials)
}
#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn prepare_trust(
    _: SocketAddr,
    _: &HostService,
    _: Option<&Credentials>,
) -> Result<Box<dyn Send>, String> {
    Err("No RDP client adapter on this platform".into())
}

/// Design 2.6: holds a handle of the bound client process and calls
/// `on_exit` when it ended (Linux pidfd, Windows process handle).
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn watch_process_exit(
    pid: u32,
    started: u64,
    on_exit: Box<dyn FnOnce() + Send>,
) -> Result<(), String> {
    platform::watch_process_exit(pid, started, on_exit)
}
#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn watch_process_exit(
    _: u32,
    _: u64,
    _: Box<dyn FnOnce() + Send>,
) -> Result<(), String> {
    Err("Prozessgebundene Weiterleitungen gibt es auf dieser Plattform nicht".into())
}
