use std::net::TcpStream;
use std::time::Duration;

use super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
};
use super::ipc_storage::{read_ipc_addr, read_token};

/// Probe mount guarantees through the daemon that owns the live Share
/// service. Using the GUI's loopback AgentBackend here would only describe the
/// proxy and could incorrectly advertise writable support for the peer.
pub fn probe_share_mount_capabilities(
    target: crate::share::PeerOpenTarget,
    root: &crate::mount::BackendRoot,
) -> Result<crate::vfs::MountPathCapabilities, String> {
    super::ipc_client::ensure_worker_ready()?;
    let token = read_token().map_err(|error| format!("Background-Worker Token: {error}"))?;
    let addr = read_ipc_addr().ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|error| format!("Background-Worker IPC: {error}"))?;
    // The daemon's peer probe includes its bounded 45-second transport/fallback
    // window. Keep a small IPC margin without leaving an unbounded UI worker.
    set_stream_timeout(&stream, Some(Duration::from_secs(50)));
    write_request(
        &mut stream,
        &IpcRequest::ProbeShareMount {
            token,
            target,
            root: root.as_str().to_string(),
        },
    )
    .map_err(|error| format!("Peer-Laufwerkspruefung senden: {error}"))?;
    match read_response(&mut stream)
        .map_err(|error| format!("Peer-Laufwerkspruefung lesen: {error}"))?
    {
        IpcResponse::MountPathCapabilities { capabilities } => Ok(capabilities.into()),
        IpcResponse::Err { msg } => Err(msg),
        _ => Err("Unerwartete Antwort auf Peer-Laufwerkspruefung".into()),
    }
}
