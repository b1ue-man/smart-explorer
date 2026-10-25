//! One dispatch for the requests a drive host makes on its extension channel
//! (`IpcRequest::MountHostExt`, plan W3; DRV-M6 moved it out of
//! `mount_lock_participant.rs` before `ChildBackend` and `DeviceWatch`
//! joined `PrepareLock`). The session token of the drive was checked before
//! (`ipc_host_ext.rs`, `authorize_extension`).
use std::io;
use std::net::TcpStream;

use crate::mount::{MountId, DEVICES_MOUNT_ID};

use super::super::ipc_host::ShareHost;
use super::super::ipc_protocol::{write_response, IpcResponse};
use super::ipc_protocol_mount_host::MountHostRequest;

/// Only the drive `se-devices` may ask for devices.
const NOT_THE_TREE: &str = "Nur der Gerätebaum darf Geräte abfragen";

pub(in crate::daemon) fn serve(
    host: &ShareHost,
    mut stream: TcpStream,
    id: &MountId,
    session_token: &str,
    req: MountHostRequest,
) -> io::Result<()> {
    match req {
        MountHostRequest::Capacity { capacity } => {
            let response = match host.mounts.update_capacity(id, session_token, capacity) {
                Ok(()) => IpcResponse::Ok,
                Err(msg) => IpcResponse::Err { msg, code: None },
            };
            write_response(&mut stream, &response)
        }
        MountHostRequest::PrepareLock { deadline_ms } => {
            host.mounts.serve_prepare_lock(stream, id, deadline_ms)
        }
        MountHostRequest::ChildBackend { target, root } => {
            if id.as_str() != DEVICES_MOUNT_ID {
                return refuse(&mut stream);
            }
            super::devices_host::serve_child(host, stream, target, root)
        }
        MountHostRequest::DeviceWatch { since } => {
            if id.as_str() != DEVICES_MOUNT_ID {
                return refuse(&mut stream);
            }
            super::devices_host::serve_watch(host, &mut stream, since)
        }
    }
}

fn refuse(stream: &mut TcpStream) -> io::Result<()> {
    write_response(
        stream,
        &IpcResponse::Err {
            msg: NOT_THE_TREE.into(),
            code: None,
        },
    )
}
