//! The daemon side of the device-tree host (DRV-M6, U17-21): the device
//! list from the Share profiles, the boards that give every change a
//! revision, `DeviceWatch` (the host learns about devices) and
//! `ChildBackend` (the host opens a shared folder of a device).
//!
//! Only the drive `se-devices` may ask for devices; the session token of
//! that drive authorizes the connection (`ipc_host_ext.rs`).
use std::io;
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::mount::{
    device_key_of, open_target_of, BackendRoot, DeviceKind, DeviceRow, MountMode,
    MountRootSecurity, PeerMountTarget, RoomRef,
};
use crate::share::{device_view, ShareStatus};

use super::super::backend_server::serve_backend;
use super::super::ipc_host::ShareHost;
use super::super::ipc_protocol::{write_response, IpcResponse, MountBackendCapabilities};
use super::devices_ipc::{host_dialect, DevicesStatus};
use super::ipc_protocol_mount_host::{MountHostReply, MAX_WATCH_MS};

/// How often a waiting request looks again (the beat of the host's loop).
pub(super) const POLL: Duration = Duration::from_millis(250);
/// The answer to a refused `ChildBackend` of a restricted worker.
pub(super) const DEFERRED_CODE: &str = "share_deferred";
const DEFERRED_TEXT: &str = "Wartet auf Smart Explorer Share";

/// Gives each different observation the next revision; the first one is 1.
pub(super) struct Board<T> {
    state: Mutex<(u64, Option<T>)>,
}

impl<T: Clone + PartialEq> Board<T> {
    pub(super) const fn new() -> Self {
        Self {
            state: Mutex::new((0, None)),
        }
    }

    /// The revision of `value`: unchanged while it equals the last one.
    pub(super) fn observe(&self, value: &T) -> u64 {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.1.as_ref() != Some(value) {
            state.0 += 1;
            state.1 = Some(value.clone());
        }
        state.0
    }
}

static DEVICES: Board<Vec<DeviceRow>> = Board::new();
static STATUS: Board<DevicesStatus> = Board::new();

/// `status` with its revision; the revision itself is no part of the
/// comparison.
pub(super) fn observe_status(mut status: DevicesStatus) -> DevicesStatus {
    status.revision = 0;
    status.revision = STATUS.observe(&status);
    status
}

/// Polls `observe` until its revision differs from `since` or `max` has
/// passed, and returns the last observation.
pub(super) fn wait_for_change<T>(
    since: u64,
    max: Duration,
    poll: Duration,
    mut observe: impl FnMut() -> (u64, T),
) -> T {
    let deadline = Instant::now() + max;
    loop {
        let (revision, value) = observe();
        if revision != since || Instant::now() >= deadline {
            return value;
        }
        std::thread::sleep(poll.min(deadline.saturating_duration_since(Instant::now())));
    }
}

fn longest_wait() -> Duration {
    Duration::from_millis(MAX_WATCH_MS)
}

/// `DriveIpcRequest::DevicesWatch`: the status after a change of `since`,
/// or after 30 s unchanged (V4).
pub(super) fn watch_status(host: &ShareHost, since: u64) -> DevicesStatus {
    wait_for_change(since, longest_wait(), POLL, || {
        let status = super::devices_mount::status(host);
        (status.revision, status)
    })
}

/// The devices of the Share profiles: every Direct contact, every member of
/// a room except this device. A contact whose identity changed is left out
/// (no access, like `TileCode::opens_files`).
pub(super) fn projection(host: &ShareHost) -> Vec<DeviceRow> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or_default();
    let Ok(state) = host.state.lock() else {
        return Vec::new();
    };
    let own = state
        .identity
        .as_ref()
        .map(|identity| identity.device_id.clone());
    let mut rows = Vec::new();
    for contact in &state.profiles.direct_contacts {
        if contact.status == ShareStatus::IdentityConflict {
            continue;
        }
        let target = PeerMountTarget::Direct {
            contact_id: contact.id.clone(),
        };
        rows.push(DeviceRow {
            key: device_key_of(&target),
            name: contact.display_name.clone(),
            kind: DeviceKind::Direct,
            room: None,
            online: device_view::online(contact, now),
            target,
        });
    }
    for room in &state.profiles.rooms {
        for member in &room.members {
            if own.as_deref() == Some(member.device_id.as_str()) {
                continue;
            }
            let target = PeerMountTarget::RoomDevice {
                room_id: room.id.clone(),
                device_id: member.device_id.clone(),
            };
            rows.push(DeviceRow {
                key: device_key_of(&target),
                name: member.device_name.clone(),
                kind: DeviceKind::Room,
                room: Some(RoomRef {
                    id: room.id.clone(),
                    name: room.name.clone(),
                }),
                online: device_view::reachable(&member.status),
                target,
            });
        }
    }
    rows.sort_by(|left, right| left.key.cmp(&right.key));
    rows
}

/// `MountHostRequest::DeviceWatch`: the device list for the host.
pub(super) fn serve_watch(host: &ShareHost, stream: &mut TcpStream, since: u64) -> io::Result<()> {
    let (revision, devices) = wait_for_change(since, longest_wait(), POLL, || {
        let devices = projection(host);
        let revision = DEVICES.observe(&devices);
        (revision, (revision, devices))
    });
    write_response(
        stream,
        &IpcResponse::MountHostExt(MountHostReply::Devices { revision, devices }),
    )
}

/// The refusal of a worker that was started only for the drive restore: its
/// Share host is off, so no device can be opened (V11).
pub(super) fn child_gate(share_allowed: bool) -> Option<IpcResponse> {
    (!share_allowed).then(|| IpcResponse::Err {
        msg: DEFERRED_TEXT.into(),
        code: Some(DEFERRED_CODE.into()),
    })
}

/// Whether the device lets this contact write: a Direct device that offers
/// a backend with all mounted-write guarantees and has not refused the
/// contact. An unknown grant counts as allowed; the device refuses then.
pub(super) fn writable_for(
    host: &ShareHost,
    target: &PeerMountTarget,
    capabilities: MountBackendCapabilities,
) -> bool {
    let PeerMountTarget::Direct { contact_id } = target else {
        return false;
    };
    if !capabilities.staged_write().supports_mounted_writes() {
        return false;
    }
    let Ok(state) = host.state.lock() else {
        return false;
    };
    state
        .profiles
        .direct_contacts
        .iter()
        .find(|contact| contact.id == *contact_id)
        .is_none_or(|contact| device_view::granted_write(&state.profiles, contact) != Some(false))
}

/// `MountHostRequest::ChildBackend`: opens `root` of the device, answers
/// `ChildReady` and then serves the backend protocol on the stream.
pub(super) fn serve_child(
    host: &ShareHost,
    mut stream: TcpStream,
    target: PeerMountTarget,
    root: String,
) -> io::Result<()> {
    if let Some(refusal) = child_gate(super::run_start_reason::share_allowed()) {
        return write_response(&mut stream, &refusal);
    }
    let opened = target
        .validate()
        .map_err(|error| error.to_string())
        .and_then(|()| BackendRoot::parse(&root).map_err(|error| error.to_string()))
        .and_then(|root| {
            let (_, backend, _) = host.open_share(open_target_of(&target))?;
            super::super::rooted_backend::RootedBackend::new_in(
                backend,
                &root,
                MountMode::ReadWrite,
                MountRootSecurity::Enforced,
                host_dialect(),
            )
            .map_err(|error| format!("Geräte-Wurzel absichern: {error}"))
        });
    let backend = match opened {
        Ok(backend) => backend,
        Err(msg) => return write_response(&mut stream, &IpcResponse::Err { msg, code: None }),
    };
    let capabilities = MountBackendCapabilities::from_backend(&backend);
    let writable = writable_for(host, &target, capabilities);
    write_response(
        &mut stream,
        &IpcResponse::MountHostExt(MountHostReply::ChildReady {
            capabilities,
            writable,
        }),
    )?;
    super::super::mount_proxy::prepare_stream(&stream)?;
    let read = stream.try_clone()?;
    serve_backend(read, stream, backend)
}
