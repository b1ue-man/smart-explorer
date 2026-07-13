use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::handoff::{request_handoff, stop_requested_for};
use super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse, ShareWorkerSnapshot,
};
use super::ipc_storage::{read_ipc_addr, read_ipc_generation, read_token};

#[path = "ipc_exec_grant_client.rs"]
mod exec_grant_client;
pub(crate) use exec_grant_client::mutate_exec_grant;

static WORKER_RESTART_LOCK: Mutex<()> = Mutex::new(());
const WORKER_READY_TIMEOUT: Duration = Duration::from_secs(25);

pub fn open_share_backend(
    target: crate::share::PeerOpenTarget,
) -> Result<(String, crate::vfs::BackendHandle, crate::share::ShareStatus), String> {
    ensure_worker_ready()?;
    let token = read_token().map_err(|error| format!("Background-Worker Token: {error}"))?;
    let mut last = "Background-Worker nicht erreichbar".to_string();
    let mut restarted_after_agent_error = false;
    let mut restarted_after_missing_ipc = false;
    for _ in 0..8 {
        match read_ipc_addr()
            .and_then(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok())
        {
            Some(mut stream) => {
                set_stream_timeout(&stream, Some(Duration::from_secs(3)));
                let req = IpcRequest::OpenShare {
                    token: token.clone(),
                    target: target.clone(),
                };
                if let Err(error) = write_request(&mut stream, &req) {
                    last = error.to_string();
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                }
                let response = match read_response(&mut stream) {
                    Ok(response) => response,
                    Err(error) => {
                        last = error.to_string();
                        std::thread::sleep(Duration::from_millis(250));
                        continue;
                    }
                };
                match response {
                    IpcResponse::OpenOk { label, status } => {
                        set_stream_timeout(&stream, None);
                        let read = stream.try_clone().map_err(|error| error.to_string())?;
                        let inner: crate::vfs::BackendHandle = Arc::new(UnavailableBackend {
                            label: label.clone(),
                        });
                        let agent = match crate::agent::AgentBackend::from_streams(
                            Box::new(read),
                            Box::new(stream),
                            inner,
                        ) {
                            Ok(agent) => agent,
                            Err(error) => {
                                last = format!("Worker-Backend: {error}");
                                if restarted_after_agent_error {
                                    return Err(last);
                                }
                                restarted_after_agent_error = true;
                                restart_worker_for_client()?;
                                std::thread::sleep(Duration::from_millis(750));
                                continue;
                            }
                        };
                        return Ok((label, Arc::new(agent), status));
                    }
                    IpcResponse::Err { msg } => return Err(msg),
                    _ => return Err("Unerwartete Worker-Antwort".into()),
                }
            }
            None => {
                if !restarted_after_missing_ipc {
                    restarted_after_missing_ipc = true;
                    restart_worker_for_client()?;
                }
                std::thread::sleep(Duration::from_millis(750));
            }
        }
    }
    Err(last)
}

pub fn exec_share(
    target: crate::share::PeerOpenTarget,
    req: crate::share::ExecRequest,
) -> Result<crate::share::ExecResult, String> {
    ensure_worker_ready()?;
    let token = read_token().map_err(|error| format!("Background-Worker Token: {error}"))?;
    let addr = read_ipc_addr().ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|error| format!("Background-Worker IPC: {error}"))?;
    let socket_timeout = Duration::from_millis(req.timeout_ms.saturating_add(60_000).max(60_000));
    set_stream_timeout(&stream, Some(socket_timeout));
    write_request(&mut stream, &IpcRequest::ExecShare { token, target, req })
        .map_err(|error| error.to_string())?;
    match read_response(&mut stream).map_err(|error| error.to_string())? {
        IpcResponse::ExecResult { result } => Ok(result),
        IpcResponse::Err { msg } => Err(msg),
        _ => Err("Unerwartete Worker-Antwort".into()),
    }
}

pub fn refresh_share_worker_checked() -> Result<bool, String> {
    ensure_worker_ready()?;
    let token = read_token().map_err(|error| format!("Background-Worker Token: {error}"))?;
    let addr = read_ipc_addr().ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|error| format!("Background-Worker IPC: {error}"))?;
    set_stream_timeout(&stream, Some(Duration::from_secs(8)));
    write_request(&mut stream, &IpcRequest::RefreshShare { token })
        .map_err(|error| error.to_string())?;
    match read_response(&mut stream).map_err(|error| error.to_string())? {
        IpcResponse::RefreshOk { running } => Ok(running),
        IpcResponse::Ok => Ok(true),
        IpcResponse::Err { msg } => Err(msg),
        _ => Err("Unerwartete Worker-Antwort".into()),
    }
}

pub fn send_share_command(cmd: crate::share::ShareCmd) -> Result<(), String> {
    ensure_worker_ready()?;
    let token = read_token().map_err(|error| format!("Background-Worker Token: {error}"))?;
    let addr = read_ipc_addr().ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|error| format!("Background-Worker IPC: {error}"))?;
    set_stream_timeout(&stream, Some(Duration::from_secs(8)));
    write_request(&mut stream, &IpcRequest::ShareCommand { token, cmd })
        .map_err(|error| error.to_string())?;
    match read_response(&mut stream).map_err(|error| error.to_string())? {
        IpcResponse::Ok => Ok(()),
        IpcResponse::Err { msg } => Err(msg),
        _ => Err("Unerwartete Worker-Antwort".into()),
    }
}

pub fn drain_share_worker_events() -> Result<ShareWorkerSnapshot, String> {
    let mut last_error = "Background-Worker nicht erreichbar".to_string();
    for attempt in 0..3 {
        match drain_share_worker_events_once() {
            Ok(snapshot) => return Ok(snapshot),
            Err(error) => last_error = error,
        }
        if attempt < 2 {
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    Err(last_error)
}

fn drain_share_worker_events_once() -> Result<ShareWorkerSnapshot, String> {
    ensure_worker_ready()?;
    let token = read_token().map_err(|error| format!("Background-Worker Token: {error}"))?;
    let addr = read_ipc_addr().ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1))
        .map_err(|error| format!("Background-Worker IPC: {error}"))?;
    // A periodic refresh may briefly serialize credential/profile loading.
    // Polling runs off the UI thread, so allow the same bounded response window
    // as other control-plane commands instead of surfacing a transient EAGAIN.
    set_stream_timeout(&stream, Some(Duration::from_secs(8)));
    write_request(&mut stream, &IpcRequest::DrainShareEvents { token })
        .map_err(|error| error.to_string())?;
    match read_response(&mut stream).map_err(|error| error.to_string())? {
        IpcResponse::ShareEvents { snapshot } => Ok(*snapshot),
        IpcResponse::Err { msg } => Err(msg),
        _ => Err("Unerwartete Worker-Antwort".into()),
    }
}

pub fn ensure_worker_ready() -> Result<(), String> {
    match probe_worker(Duration::from_millis(700)) {
        WorkerProbe::Ready { .. } => Ok(()),
        WorkerProbe::Starting { .. } => {
            if wait_for_initializing_worker()? {
                Ok(())
            } else {
                restart_worker_for_client()
            }
        }
        WorkerProbe::Retiring => {
            if wait_for_replacement_worker()? {
                Ok(())
            } else {
                restart_worker_for_client()
            }
        }
        WorkerProbe::Stale | WorkerProbe::Missing => restart_worker_for_client(),
    }
}

/// Request a version-bound daemon handoff without waiting for Share readiness.
/// GUI update paths use this after the updated application is already live.
pub fn request_daemon_replacement() -> Result<(), String> {
    launch_replacement().map(|_| ())
}

fn restart_worker_for_client() -> Result<(), String> {
    let _guard = WORKER_RESTART_LOCK
        .lock()
        .map_err(|_| "Background-Worker Neustart ist gesperrt".to_string())?;
    match probe_worker(Duration::from_millis(500)) {
        WorkerProbe::Ready { .. } => return Ok(()),
        WorkerProbe::Starting { .. } if wait_for_initializing_worker()? => return Ok(()),
        WorkerProbe::Starting { .. } => {}
        WorkerProbe::Retiring if wait_for_replacement_worker()? => return Ok(()),
        WorkerProbe::Retiring => {}
        WorkerProbe::Stale | WorkerProbe::Missing => {}
    }

    let replacement = launch_replacement()?;

    let deadline = Instant::now() + WORKER_READY_TIMEOUT;
    while Instant::now() < deadline {
        if matches!(
            probe_worker(Duration::from_millis(700)),
            WorkerProbe::Ready { generation } if replacement.accepts(&generation)
        ) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err("Background-Worker wurde nach dem Neustart nicht bereit".into())
}

fn wait_for_initializing_worker() -> Result<bool, String> {
    let deadline = Instant::now() + WORKER_READY_TIMEOUT;
    while Instant::now() < deadline {
        match probe_worker(Duration::from_millis(700)) {
            WorkerProbe::Ready { .. } => return Ok(true),
            WorkerProbe::Starting { .. } => {
                std::thread::sleep(Duration::from_millis(250));
            }
            WorkerProbe::Retiring | WorkerProbe::Stale | WorkerProbe::Missing => {
                return Ok(false);
            }
        }
    }
    Err("Background-Worker Initialisierung hat das Zeitlimit ueberschritten".into())
}

fn wait_for_replacement_worker() -> Result<bool, String> {
    let deadline = Instant::now() + WORKER_READY_TIMEOUT;
    while Instant::now() < deadline {
        if matches!(
            probe_worker(Duration::from_millis(700)),
            WorkerProbe::Ready { .. }
        ) {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(false)
}

fn launch_replacement() -> Result<ReplacementLaunch, String> {
    let retiring_generation = read_ipc_generation();
    let generation = new_generation()?;
    // Launch first. The handoff child waits for its generation-specific
    // activation marker, so a failed CreateProcess/spawn cannot stop or strand
    // the currently healthy worker.
    crate::autostart::spawn_daemon_handoff_checked(&generation, retiring_generation.as_deref())
        .map_err(|error| format!("Background-Worker starten: {error}"))?;
    request_handoff(&generation)
        .map_err(|error| format!("Background-Worker Handoff aktivieren: {error}"))?;
    Ok(ReplacementLaunch {
        expected_generation: generation,
        retiring_generation,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplacementLaunch {
    expected_generation: String,
    retiring_generation: Option<String>,
}

impl ReplacementLaunch {
    fn accepts(&self, generation: &str) -> bool {
        generation == self.expected_generation
            || self
                .retiring_generation
                .as_deref()
                .is_none_or(|retiring| generation != retiring)
    }
}

fn new_generation() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| format!("Background-Worker Generation: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkerProbe {
    Ready { generation: String },
    Starting { generation: String },
    Retiring,
    Stale,
    Missing,
}

fn probe_worker(timeout: Duration) -> WorkerProbe {
    let Ok(token) = read_token() else {
        return WorkerProbe::Missing;
    };
    let Some(addr) = read_ipc_addr() else {
        return WorkerProbe::Missing;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, timeout) else {
        return WorkerProbe::Missing;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    if write_request(&mut stream, &IpcRequest::Ping { token }).is_err() {
        return WorkerProbe::Missing;
    }
    match read_response(&mut stream) {
        Ok(IpcResponse::Pong {
            version,
            generation,
            initialized,
        }) if worker_version_is_current(&version) && valid_generation(&generation) => {
            if initialized && stop_requested_for(&generation) {
                WorkerProbe::Retiring
            } else if initialized {
                WorkerProbe::Ready { generation }
            } else {
                WorkerProbe::Starting { generation }
            }
        }
        Ok(IpcResponse::Pong { .. }) => WorkerProbe::Stale,
        Ok(_) | Err(_) => WorkerProbe::Missing,
    }
}

fn valid_generation(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn worker_version_is_current(version: &str) -> bool {
    version == env!("CARGO_PKG_VERSION")
}

struct UnavailableBackend {
    label: String,
}

impl crate::vfs::Backend for UnavailableBackend {
    fn scheme(&self) -> crate::vfs::Scheme {
        crate::vfs::Scheme::Peer
    }

    fn root_display(&self) -> String {
        self.label.clone()
    }

    fn list_dir(&self, _path: &str) -> crate::vfs::VfsResult<Vec<crate::vfs::VfsMeta>> {
        unavailable()
    }

    fn stat(&self, _path: &str) -> crate::vfs::VfsResult<crate::vfs::VfsMeta> {
        unavailable()
    }

    fn open_read(&self, _path: &str) -> crate::vfs::VfsResult<Box<dyn Read + Send>> {
        unavailable()
    }

    fn open_write(&self, _path: &str) -> crate::vfs::VfsResult<Box<dyn Write + Send>> {
        unavailable()
    }

    fn rename(&self, _src: &str, _dst: &str) -> crate::vfs::VfsResult<()> {
        unavailable()
    }

    fn remove_file(&self, _path: &str) -> crate::vfs::VfsResult<()> {
        unavailable()
    }

    fn remove_dir(&self, _path: &str) -> crate::vfs::VfsResult<()> {
        unavailable()
    }

    fn mkdir_all(&self, _path: &str) -> crate::vfs::VfsResult<()> {
        unavailable()
    }
}

fn unavailable<T>() -> io::Result<T> {
    Err(io::Error::new(
        io::ErrorKind::NotConnected,
        "Background-Worker-Verbindung geschlossen",
    ))
}

#[cfg(test)]
mod tests {
    use super::{worker_version_is_current, ReplacementLaunch};

    #[test]
    fn worker_ping_requires_current_version() {
        assert!(worker_version_is_current(env!("CARGO_PKG_VERSION")));
        assert!(!worker_version_is_current(""));
        assert!(!worker_version_is_current("0.0.0"));
    }

    #[test]
    fn replacement_accepts_its_requested_generation() {
        let replacement = ReplacementLaunch {
            expected_generation: "11111111111111111111111111111111".into(),
            retiring_generation: Some("00000000000000000000000000000000".into()),
        };

        assert!(replacement.accepts("11111111111111111111111111111111"));
    }

    #[test]
    fn fresh_concurrent_launch_accepts_the_winning_generation() {
        let replacement = ReplacementLaunch {
            expected_generation: "11111111111111111111111111111111".into(),
            retiring_generation: None,
        };

        assert!(replacement.accepts("22222222222222222222222222222222"));
    }

    #[test]
    fn replacement_rejects_the_retiring_generation_but_accepts_another_successor() {
        let replacement = ReplacementLaunch {
            expected_generation: "11111111111111111111111111111111".into(),
            retiring_generation: Some("00000000000000000000000000000000".into()),
        };

        assert!(!replacement.accepts("00000000000000000000000000000000"));
        assert!(replacement.accepts("22222222222222222222222222222222"));
    }
}
