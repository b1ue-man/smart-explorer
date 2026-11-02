//! The requests of the device-tree host to its daemon (DRV-M6, U17-19):
//! one new connection per child backend, per list of shared folders and per
//! long poll of the device list. Each runs on a connection of its own, so a
//! slow device never blocks the control, status or backend streams of the
//! drive.
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use crate::mount::{
    list_peer_roots, peer_root_choices, BackendRoot, ChildRoot, DeviceRow, PeerMountTarget,
};
use crate::vfs::{BackendHandle, Scheme};

use super::super::drive_modules::ipc_protocol_mount_host::{MountHostReply, MountHostRequest};
use super::super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
    MountBackendCapabilities,
};
use super::{host_stream, MountHostSession, MountProxyIdentity};

/// `ShareHost::open_share` waits up to 45 s for a device; the answer to
/// `ChildBackend` may take that long plus a margin.
const OPEN_TIMEOUT: Duration = Duration::from_secs(50);
/// The daemon answers a `DeviceWatch` after 30 s at the latest.
const WATCH_TIMEOUT: Duration = Duration::from_secs(35);

/// A connection to the shared folder `root` of a device, once the daemon
/// opened it: the stream, what the device's backend can, and whether the
/// device lets this contact write.
fn open_child(
    session: &MountHostSession,
    target: &PeerMountTarget,
    root: &str,
) -> Result<(TcpStream, MountBackendCapabilities, bool), String> {
    let mut stream = host_stream(session.ipc_addr, OPEN_TIMEOUT)?;
    super::super::mount_proxy::prepare_stream(&stream)
        .map_err(|error| format!("Geräte-Backend-Transport: {error}"))?;
    let request = IpcRequest::MountHostExt {
        id: session.id.clone(),
        session_token: session.session_token.clone(),
        req: MountHostRequest::ChildBackend {
            target: target.clone(),
            root: root.to_string(),
        },
    };
    write_request(&mut stream, &request).map_err(|error| format!("Gerät anfragen: {error}"))?;
    match read_response(&mut stream).map_err(|error| format!("Geräte-Antwort lesen: {error}"))? {
        IpcResponse::MountHostExt(MountHostReply::ChildReady {
            capabilities,
            writable,
        }) => {
            set_stream_timeout(&stream, None);
            Ok((stream, capabilities, writable))
        }
        IpcResponse::Err { msg, .. } => Err(msg),
        _ => Err("Unerwartete Antwort auf ChildBackend".into()),
    }
}

/// The stream as the backend of the drive host, wrapped like the one of a
/// single-peer drive.
fn backend_over(
    stream: TcpStream,
    capabilities: MountBackendCapabilities,
) -> Result<BackendHandle, String> {
    let read = stream
        .try_clone()
        .map_err(|error| format!("Geräte-Backend-Stream: {error}"))?;
    let identity: BackendHandle = Arc::new(MountProxyIdentity {
        scheme: Scheme::Peer,
    });
    crate::agent::AgentBackend::from_streams(Box::new(read), Box::new(stream), identity)
        .map(|backend| super::super::mount_proxy::wrap(Arc::new(backend), capabilities))
        .map_err(|error| format!("Geräte-Backend-Protokoll: {error}"))
}

pub(super) fn child_backend(
    session: &MountHostSession,
    target: &PeerMountTarget,
    root: &BackendRoot,
) -> Result<(BackendHandle, bool), String> {
    let (stream, capabilities, writable) = open_child(session, target, root.as_str())?;
    Ok((backend_over(stream, capabilities)?, writable))
}

/// The shared folders and connections of a device (U17-19): the same
/// reading as the drive window, without the root „Alle freigegebenen
/// Bereiche“. The stream ends when the listing is read.
pub(super) fn child_roots(
    session: &MountHostSession,
    target: &PeerMountTarget,
) -> Result<Vec<ChildRoot>, String> {
    let (stream, capabilities, _) = open_child(session, target, "/")?;
    let backend = backend_over(stream, capabilities)?;
    let listings = list_peer_roots(&*backend).map_err(|error| error.to_string())?;
    Ok(peer_root_choices(listings)
        .into_iter()
        .filter(|choice| choice.root.as_str() != "/")
        .map(|choice| ChildRoot {
            name: choice.root.as_str().trim_start_matches('/').to_string(),
            root: choice.root,
        })
        .collect())
}

pub(super) fn device_watch(
    session: &MountHostSession,
    since: u64,
) -> Result<(u64, Vec<DeviceRow>), String> {
    let mut stream = host_stream(session.ipc_addr, WATCH_TIMEOUT)?;
    let request = IpcRequest::MountHostExt {
        id: session.id.clone(),
        session_token: session.session_token.clone(),
        req: MountHostRequest::DeviceWatch { since },
    };
    write_request(&mut stream, &request)
        .map_err(|error| format!("Geräteliste anfragen: {error}"))?;
    match read_response(&mut stream).map_err(|error| format!("Geräteliste lesen: {error}"))? {
        IpcResponse::MountHostExt(MountHostReply::Devices { revision, devices }) => {
            Ok((revision, devices))
        }
        IpcResponse::Err { msg, .. } => Err(msg),
        _ => Err("Unerwartete Antwort auf DeviceWatch".into()),
    }
}
