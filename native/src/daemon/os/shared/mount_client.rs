use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::mount::{MountConfig, MountId, MountRecovery, MountSnapshot, MountStatus};
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta, VfsResult};

use super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
};
use super::ipc_storage::{read_ipc_addr, read_token};

pub use super::ipc_protocol::MountHostConfig;

// Stop/Retry may legitimately wait for one five-minute Dokany callback plus
// manager cleanup. Keep a margin above the manager's fail-safe stop grace.
const CONTROL_TIMEOUT: Duration =
    Duration::from_secs(super::mount_manager::STOP_GRACE.as_secs() + 30);
const HOST_ATTACH_TIMEOUT: Duration = Duration::from_secs(10);

pub fn start_mount(config: MountConfig) -> Result<MountSnapshot, String> {
    mount_request(|token| IpcRequest::StartMount { token, config })
}

pub fn stop_mount(id: MountId) -> Result<MountSnapshot, String> {
    mount_request(|token| IpcRequest::StopMount { token, id })
}

pub fn retry_mount(id: MountId) -> Result<MountSnapshot, String> {
    mount_request(|token| IpcRequest::RetryMount { token, id })
}

pub fn list_mounts() -> Result<Vec<MountSnapshot>, String> {
    super::ipc_client::ensure_worker_ready()?;
    let (mut stream, token) = authenticated_stream(CONTROL_TIMEOUT)?;
    write_request(&mut stream, &IpcRequest::ListMounts { token })
        .map_err(|error| format!("Laufwerk-IPC senden: {error}"))?;
    match read_response(&mut stream).map_err(|error| format!("Laufwerk-IPC lesen: {error}"))? {
        IpcResponse::Mounts { mounts } => Ok(mounts),
        IpcResponse::Err { msg } => Err(msg),
        _ => Err("Unerwartete Laufwerk-IPC-Antwort".into()),
    }
}

fn mount_request(request: impl FnOnce(String) -> IpcRequest) -> Result<MountSnapshot, String> {
    super::ipc_client::ensure_worker_ready()?;
    let (mut stream, token) = authenticated_stream(CONTROL_TIMEOUT)?;
    write_request(&mut stream, &request(token))
        .map_err(|error| format!("Laufwerk-IPC senden: {error}"))?;
    match read_response(&mut stream).map_err(|error| format!("Laufwerk-IPC lesen: {error}"))? {
        IpcResponse::Mount { mount } => Ok(mount),
        IpcResponse::Err { msg } => Err(msg),
        _ => Err("Unerwartete Laufwerk-IPC-Antwort".into()),
    }
}

pub struct MountHostSession {
    pub config: MountHostConfig,
    pub backend: BackendHandle,
    pub cache_root: PathBuf,
    id: MountId,
    ipc_addr: SocketAddr,
    session_token: String,
    stop: Mutex<mpsc::Receiver<()>>,
}

impl MountHostSession {
    pub fn report_status(&self, status: MountStatus) -> Result<(), String> {
        self.report_status_with_recovery(status, None)
    }

    pub(crate) fn report_status_with_recovery(
        &self,
        status: MountStatus,
        recovery: Option<MountRecovery>,
    ) -> Result<(), String> {
        let mut stream = host_stream(self.ipc_addr, HOST_ATTACH_TIMEOUT)?;
        write_request(
            &mut stream,
            &IpcRequest::MountHostStatus {
                id: self.id.clone(),
                session_token: self.session_token.clone(),
                status,
                recovery,
                recovery_required: recovery.map(MountRecovery::requires_retention),
            },
        )
        .map_err(|error| format!("Laufwerk-Status senden: {error}"))?;
        match read_response(&mut stream)
            .map_err(|error| format!("Laufwerk-Status lesen: {error}"))?
        {
            IpcResponse::Mount { .. } | IpcResponse::Ok => Ok(()),
            IpcResponse::Err { msg } => Err(msg),
            _ => Err("Unerwartete Laufwerk-Statusantwort".into()),
        }
    }

    /// Returns true for an explicit Stop and for control EOF. Losing the
    /// daemon must always unmount rather than leave an orphaned drive behind.
    pub fn wait_for_stop_timeout(&self, timeout: Duration) -> bool {
        let Ok(stop) = self.stop.lock() else {
            return true;
        };
        match stop.recv_timeout(timeout) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => true,
            Err(RecvTimeoutError::Timeout) => false,
        }
    }
}

pub fn connect_mount_host(id: MountId) -> Result<MountHostSession, String> {
    let HostEnvironment {
        launch_token,
        ipc_addr,
        cache_root,
    } = take_host_environment()?;
    let mut control = host_stream(ipc_addr, HOST_ATTACH_TIMEOUT)?;
    write_request(
        &mut control,
        &IpcRequest::MountHostAttach {
            id: id.clone(),
            launch_token,
        },
    )
    .map_err(|error| format!("Laufwerk-Host anmelden: {error}"))?;
    let (config, scheme, capabilities, session_token, backend_token) =
        match read_response(&mut control)
            .map_err(|error| format!("Laufwerk-Host-Antwort lesen: {error}"))?
        {
            IpcResponse::MountHostReady {
                config,
                scheme,
                capabilities,
                session_token,
                backend_token,
            } => (config, scheme, capabilities, session_token, backend_token),
            IpcResponse::Err { msg } => return Err(msg),
            _ => return Err("Unerwartete Laufwerk-Host-Antwort".into()),
        };
    if config.id != id {
        return Err("Laufwerk-Host erhielt eine fremde Konfiguration".into());
    }
    set_stream_timeout(&control, None);

    let backend = connect_backend(ipc_addr, scheme.into(), capabilities, &id, backend_token)?;
    let (stop_send, stop_receive) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name(format!("mount-control-{}", id.as_str()))
        .spawn(move || {
            // Any response other than the authenticated Stop, read failure, or
            // EOF is fail-closed and asks the host loop to unmount.
            let _ = read_response(&mut control);
            let _ = stop_send.send(());
        })
        .map_err(|error| format!("Laufwerk-Control starten: {error}"))?;

    Ok(MountHostSession {
        config,
        backend,
        cache_root,
        id,
        ipc_addr,
        session_token,
        stop: Mutex::new(stop_receive),
    })
}

fn connect_backend(
    ipc_addr: SocketAddr,
    scheme: Scheme,
    capabilities: super::ipc_protocol::MountBackendCapabilities,
    id: &MountId,
    backend_token: String,
) -> Result<BackendHandle, String> {
    let mut stream = host_stream(ipc_addr, HOST_ATTACH_TIMEOUT)?;
    super::mount_proxy::prepare_stream(&stream)
        .map_err(|error| format!("Laufwerk-Backend-Transport: {error}"))?;
    write_request(
        &mut stream,
        &IpcRequest::MountHostBackend {
            id: id.clone(),
            backend_token,
        },
    )
    .map_err(|error| format!("Laufwerk-Backend anmelden: {error}"))?;
    match read_response(&mut stream)
        .map_err(|error| format!("Laufwerk-Backend-Antwort lesen: {error}"))?
    {
        IpcResponse::Ok => {}
        IpcResponse::Err { msg } => return Err(msg),
        _ => return Err("Unerwartete Laufwerk-Backend-Antwort".into()),
    }
    set_stream_timeout(&stream, None);
    let read = stream
        .try_clone()
        .map_err(|error| format!("Laufwerk-Backend-Stream: {error}"))?;
    let identity: BackendHandle = Arc::new(MountProxyIdentity { scheme });
    crate::agent::AgentBackend::from_streams(Box::new(read), Box::new(stream), identity)
        .map(|backend| super::mount_proxy::wrap(Arc::new(backend), capabilities))
        .map_err(|error| format!("Laufwerk-Backend-Protokoll: {error}"))
}

fn authenticated_stream(timeout: Duration) -> Result<(TcpStream, String), String> {
    let token = read_token().map_err(|error| format!("Background-Worker Token: {error}"))?;
    let addr = read_ipc_addr().ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|error| format!("Background-Worker IPC: {error}"))?;
    set_stream_timeout(&stream, Some(timeout));
    Ok((stream, token))
}

fn host_stream(addr: SocketAddr, timeout: Duration) -> Result<TcpStream, String> {
    if !addr.ip().is_loopback() {
        return Err("Laufwerk-IPC-Adresse ist nicht lokal".into());
    }
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|error| format!("Laufwerk-IPC verbinden: {error}"))?;
    set_stream_timeout(&stream, Some(timeout));
    Ok(stream)
}

struct HostEnvironment {
    launch_token: String,
    ipc_addr: SocketAddr,
    cache_root: PathBuf,
}

fn take_host_environment() -> Result<HostEnvironment, String> {
    let token_value = std::env::var_os(super::mount_process::MOUNT_TOKEN_ENV);
    let addr_value = std::env::var_os(super::mount_process::MOUNT_IPC_ADDR_ENV);
    let cache_value = std::env::var_os(super::mount_process::MOUNT_CACHE_DIR_ENV);
    std::env::remove_var(super::mount_process::MOUNT_TOKEN_ENV);
    std::env::remove_var(super::mount_process::MOUNT_IPC_ADDR_ENV);
    std::env::remove_var(super::mount_process::MOUNT_CACHE_DIR_ENV);

    let launch_token = token_value
        .and_then(|value| value.into_string().ok())
        .ok_or_else(|| "Laufwerk-Host-Token fehlt".to_string())?;
    if launch_token.len() != 64 || !launch_token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Laufwerk-Host-Token ist ungueltig".into());
    }
    let addr = addr_value
        .and_then(|value| value.into_string().ok())
        .ok_or_else(|| "Laufwerk-IPC-Adresse fehlt".to_string())?;
    if addr.len() > 128 {
        return Err("Laufwerk-IPC-Adresse ist zu lang".into());
    }
    let ipc_addr: SocketAddr = addr
        .parse()
        .map_err(|_| "Laufwerk-IPC-Adresse ist ungueltig".to_string())?;
    if !ipc_addr.ip().is_loopback() {
        return Err("Laufwerk-IPC-Adresse ist nicht lokal".into());
    }
    let cache_root =
        PathBuf::from(cache_value.ok_or_else(|| "Laufwerk-Cachepfad fehlt".to_string())?);
    if cache_root.as_os_str().to_string_lossy().len() > 32_768 {
        return Err("Laufwerk-Cachepfad ist zu lang".into());
    }
    let cache_root = crate::mount::prepare_spool_root(&cache_root)
        .map_err(|error| format!("Laufwerk-Cachepfad absichern: {error}"))?;
    Ok(HostEnvironment {
        launch_token,
        ipc_addr,
        cache_root,
    })
}

struct MountProxyIdentity {
    scheme: Scheme,
}

impl MountProxyIdentity {
    fn unavailable<T>() -> io::Result<T> {
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "Laufwerk-Backend-Proxystream ist nicht verfuegbar",
        ))
    }
}

impl Backend for MountProxyIdentity {
    fn scheme(&self) -> Scheme {
        self.scheme
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn state_identity(&self) -> String {
        "mount-host-proxy".into()
    }

    fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
        Self::unavailable()
    }

    fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
        Self::unavailable()
    }

    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Self::unavailable()
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Self::unavailable()
    }

    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Self::unavailable()
    }

    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Self::unavailable()
    }

    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Self::unavailable()
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Self::unavailable()
    }
}
