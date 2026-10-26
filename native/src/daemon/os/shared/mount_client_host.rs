//! The authenticated filesystem-host session, its streams and capability input.
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;

use crate::mount::os::shared::host_session::{LockAnswer, LockOrder, SpoolKeyAnswer};
use crate::mount::{
    BackendRoot, ChildRoot, DeviceRow, MountId, MountRecovery, MountStatus, PeerMountTarget,
};
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta, VfsResult};

use super::super::drive_modules::ipc_protocol_mount_host::MountHostRequest;
use super::super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
    MountBackendCapabilities, VaultIpcReply, VaultIpcRequest,
};
use super::super::mount_manager::lock_participant::HostLockWatch;
use super::super::{mount_process, mount_proxy};
use super::{devices, MountHostConfig};

const HOST_ATTACH_TIMEOUT: Duration = Duration::from_secs(10);

pub struct MountHostSession {
    pub config: MountHostConfig,
    pub backend: BackendHandle,
    pub cache_root: PathBuf,
    pub(super) id: MountId,
    pub(super) ipc_addr: SocketAddr,
    pub(super) session_token: String,
    stop: Mutex<mpsc::Receiver<()>>,
    /// DRV-M7: the drive serves a vault (`MountBackendScheme::Vault`).
    vault: bool,
    /// DRV-M7: the lock channel of a vault drive, started with the loop.
    lock_watch: OnceLock<Arc<HostLockWatch>>,
}

impl MountHostSession {
    pub(crate) fn report_capacity(
        &self,
        capacity: crate::mount::MountCapacityReport,
    ) -> Result<(), String> {
        let mut stream = host_stream(self.ipc_addr, Duration::from_secs(2))?;
        write_request(
            &mut stream,
            &IpcRequest::MountHostExt {
                id: self.id.clone(),
                session_token: self.session_token.clone(),
                req: MountHostRequest::Capacity { capacity },
            },
        )
        .map_err(|error| error.to_string())?;
        match read_response(&mut stream).map_err(|error| error.to_string())? {
            IpcResponse::Ok => Ok(()),
            IpcResponse::Err { msg, .. } => Err(msg),
            _ => Err("Unerwartete Kapazitätsantwort".into()),
        }
    }

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
            IpcResponse::Err { msg, .. } => Err(msg),
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

    /// DRV-M7: the spool key of this vault drive (`Vault::SpoolKey`, checked
    /// with the session token). The masterkey never reaches the host.
    pub(crate) fn request_spool_key(&self) -> SpoolKeyAnswer {
        let answer = host_stream(self.ipc_addr, HOST_ATTACH_TIMEOUT).and_then(|mut stream| {
            let request = IpcRequest::Vault {
                token: self.session_token.clone(),
                req: VaultIpcRequest::SpoolKey {
                    mount_id: self.id.as_str().to_string(),
                },
            };
            write_request(&mut stream, &request)
                .map_err(|error| format!("Spool-Schlüssel anfragen: {error}"))?;
            read_response(&mut stream).map_err(|error| format!("Spool-Schlüssel lesen: {error}"))
        });
        match answer {
            Ok(IpcResponse::Vault(VaultIpcReply::SpoolKey { key })) => {
                let decoded = URL_SAFE_NO_PAD
                    .decode(key.expose().as_bytes())
                    .map(zeroize::Zeroizing::new);
                match decoded
                    .ok()
                    .map(|bytes| crate::vault::SpoolKey::from_bytes(&bytes))
                {
                    Some(Ok(key)) => SpoolKeyAnswer::Key(key),
                    _ => SpoolKeyAnswer::Failed("Spool-Schlüssel ungültig".into()),
                }
            }
            Ok(IpcResponse::Err { msg, code })
                if code.as_deref() == Some(crate::vault::VAULT_LOCKED_CODE) =>
            {
                SpoolKeyAnswer::Locked(msg)
            }
            Ok(IpcResponse::Err { msg, .. }) => SpoolKeyAnswer::Failed(msg),
            Ok(_) => SpoolKeyAnswer::Failed("Unerwartete Antwort auf Vault::SpoolKey".into()),
            Err(error) => SpoolKeyAnswer::Failed(error),
        }
    }

    /// DRV-M6: a new connection to the shared folder `root` of a device
    /// through the daemon, and whether the device lets this contact write.
    pub(crate) fn child_backend(
        &self,
        target: &PeerMountTarget,
        root: &BackendRoot,
    ) -> Result<(BackendHandle, bool), String> {
        devices::child_backend(self, target, root)
    }

    /// DRV-M6: the shared folders and connections of a device.
    pub(crate) fn child_roots(&self, target: &PeerMountTarget) -> Result<Vec<ChildRoot>, String> {
        devices::child_roots(self, target)
    }

    /// DRV-M6: waits at most 30 s for a change of the device list since
    /// `since` and returns its revision with the list.
    pub(crate) fn device_watch(&self, since: u64) -> Result<(u64, Vec<DeviceRow>), String> {
        devices::device_watch(self, since)
    }

    /// DRV-M7: the next lock order of a vault drive; the first call starts
    /// the lock channel. Other drives never get one.
    pub(crate) fn next_lock_order(&self) -> Option<LockOrder> {
        if !self.vault {
            return None;
        }
        self.lock_watch
            .get_or_init(|| {
                HostLockWatch::start(self.ipc_addr, self.id.clone(), self.session_token.clone())
            })
            .take_order()
    }

    /// DRV-M7: the control loop's answer to the lock order it took.
    pub(crate) fn send_lock_answer(&self, answer: LockAnswer) {
        if let Some(watch) = self.lock_watch.get() {
            watch.answer(answer);
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
            IpcResponse::Err { msg, .. } => return Err(msg),
            _ => return Err("Unerwartete Laufwerk-Host-Antwort".into()),
        };
    if config.id != id {
        return Err("Laufwerk-Host erhielt eine fremde Konfiguration".into());
    }
    set_stream_timeout(&control, None);

    let scheme: Scheme = scheme.into();
    let vault = scheme == Scheme::Vault;
    let backend = connect_backend(ipc_addr, scheme, capabilities, &id, backend_token)?;
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
        vault,
        lock_watch: OnceLock::new(),
    })
}

fn connect_backend(
    ipc_addr: SocketAddr,
    scheme: Scheme,
    capabilities: MountBackendCapabilities,
    id: &MountId,
    backend_token: String,
) -> Result<BackendHandle, String> {
    let mut stream = host_stream(ipc_addr, HOST_ATTACH_TIMEOUT)?;
    mount_proxy::prepare_stream(&stream)
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
        IpcResponse::Err { msg, .. } => return Err(msg),
        _ => return Err("Unerwartete Laufwerk-Backend-Antwort".into()),
    }
    set_stream_timeout(&stream, None);
    let read = stream
        .try_clone()
        .map_err(|error| format!("Laufwerk-Backend-Stream: {error}"))?;
    let identity: BackendHandle = Arc::new(MountProxyIdentity { scheme });
    crate::agent::AgentBackend::from_streams(Box::new(read), Box::new(stream), identity)
        .map(|backend| mount_proxy::wrap(Arc::new(backend), capabilities))
        .map_err(|error| format!("Laufwerk-Backend-Protokoll: {error}"))
}

pub(super) fn host_stream(addr: SocketAddr, timeout: Duration) -> Result<TcpStream, String> {
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
    let token_value = std::env::var_os(mount_process::MOUNT_TOKEN_ENV);
    let addr_value = std::env::var_os(mount_process::MOUNT_IPC_ADDR_ENV);
    let cache_value = std::env::var_os(mount_process::MOUNT_CACHE_DIR_ENV);
    std::env::remove_var(mount_process::MOUNT_TOKEN_ENV);
    std::env::remove_var(mount_process::MOUNT_IPC_ADDR_ENV);
    std::env::remove_var(mount_process::MOUNT_CACHE_DIR_ENV);

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

pub(super) struct MountProxyIdentity {
    pub(super) scheme: Scheme,
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
