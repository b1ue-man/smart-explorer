use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse, ShareWorkerSnapshot,
};
use super::ipc_storage::{clear_ipc_addr, read_ipc_addr, read_token};
use super::state::{clear_heartbeat, clear_stop, request_stop};

static WORKER_RESTART_LOCK: Mutex<()> = Mutex::new(());

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
                                restart_worker_for_client(true)?;
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
                    restart_worker_for_client(false)?;
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
    ensure_worker_ready()?;
    let token = read_token().map_err(|error| format!("Background-Worker Token: {error}"))?;
    let addr = read_ipc_addr().ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1))
        .map_err(|error| format!("Background-Worker IPC: {error}"))?;
    set_stream_timeout(&stream, Some(Duration::from_millis(900)));
    write_request(&mut stream, &IpcRequest::DrainShareEvents { token })
        .map_err(|error| error.to_string())?;
    match read_response(&mut stream).map_err(|error| error.to_string())? {
        IpcResponse::ShareEvents { snapshot } => Ok(snapshot),
        IpcResponse::Err { msg } => Err(msg),
        _ => Err("Unerwartete Worker-Antwort".into()),
    }
}

pub fn ensure_worker_ready() -> Result<(), String> {
    match probe_worker(Duration::from_millis(700)) {
        WorkerProbe::Ready => Ok(()),
        WorkerProbe::Stale => restart_worker_for_client(true),
        WorkerProbe::Missing => restart_worker_for_client(false),
    }
}

fn restart_worker_for_client(stop_existing: bool) -> Result<(), String> {
    let _guard = WORKER_RESTART_LOCK
        .lock()
        .map_err(|_| "Background-Worker Neustart ist gesperrt".to_string())?;
    match probe_worker(Duration::from_millis(500)) {
        WorkerProbe::Ready => return Ok(()),
        WorkerProbe::Stale => {}
        WorkerProbe::Missing if stop_existing => {}
        WorkerProbe::Missing => {}
    }

    if stop_existing {
        request_stop().map_err(|error| format!("Background-Worker Stop anfordern: {error}"))?;
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            if matches!(
                probe_worker(Duration::from_millis(400)),
                WorkerProbe::Missing
            ) {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    clear_stop().map_err(|error| format!("Background-Worker Stopmarker entfernen: {error}"))?;
    clear_heartbeat();
    clear_ipc_addr();
    crate::autostart::spawn_daemon_now();

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if matches!(probe_worker(Duration::from_millis(700)), WorkerProbe::Ready) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err("Background-Worker wurde nach dem Neustart nicht bereit".into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerProbe {
    Ready,
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
        Ok(IpcResponse::Pong { version }) if worker_version_is_current(&version) => {
            WorkerProbe::Ready
        }
        Ok(IpcResponse::Pong { .. }) => WorkerProbe::Stale,
        Ok(_) | Err(_) => WorkerProbe::Missing,
    }
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
    use super::worker_version_is_current;

    #[test]
    fn worker_ping_requires_current_version() {
        assert!(worker_version_is_current(env!("CARGO_PKG_VERSION")));
        assert!(!worker_version_is_current(""));
        assert!(!worker_version_is_current("0.0.0"));
    }
}
