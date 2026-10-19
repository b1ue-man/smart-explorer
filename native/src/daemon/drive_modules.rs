//! Background-worker files of strand S2 (drives) of plan 2026-10-17
//! „Rechte, se Desktop, Tresore“. `daemon/mod.rs` includes this file once and
//! re-exports its modules (`pub(crate) use drive_modules::*`, R2); later S2
//! packages declare their worker files here instead of in the hub. Files
//! that belong to one existing module stay declared there: the registry of
//! the second file in `mount_registry.rs`, the start path and the restore in
//! `mount_manager.rs`.

// DRV-M3: the login entry that restores drives.
#[path = "os/shared/drive_autostart.rs"]
pub(crate) mod drive_autostart;
// DRV-M3: `IpcRequest::Drive { token, req }`.
#[path = "os/shared/ipc_protocol_drives.rs"]
pub(crate) mod ipc_protocol_drives;
// DRV-M7 (R5): `IpcRequest::MountHostExt { id, session_token, req }`.
#[path = "os/shared/ipc_protocol_mount_host.rs"]
pub(crate) mod ipc_protocol_mount_host;
// DRV-M6: the device tree drive `se-devices` (status/enable/disable, the
// host's child backends and watch, the placeholder backend of the worker).
#[path = "os/shared/devices_host.rs"]
mod devices_host;
#[path = "os/shared/devices_ipc.rs"]
pub(crate) mod devices_ipc;
#[path = "os/shared/devices_mount.rs"]
pub(crate) mod devices_mount;
// DRV-M6: serves `IpcRequest::MountHostExt` (lock preparation and the
// device requests of the tree).
#[path = "os/shared/mount_host_ext_serve.rs"]
pub(crate) mod mount_host_ext_serve;
// DRV-M3, decision S2 K7: why this worker started.
#[path = "os/shared/run_start_reason.rs"]
pub(crate) mod run_start_reason;

pub(crate) use super::mount_registry::registered_for_completion;
pub(crate) use ipc_protocol_drives::set_mount_options;
pub use run_start_reason::{is_drive_restore_invocation, note_daemon_start_args};
