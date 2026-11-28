//! Commands with administrator or root rights for the MCP `shell` tool, the
//! Share Exec host and the confirmation of mask list changes (plan
//! docs/plaene/2026-11-28-ersetzen-masken-shell, part C „Erhöhung lokal“,
//! contract 3; findings docs/refs/agenten-sperre-elevation-2026-11-28.md §4).
//!
//! - Already elevated (Windows token elevated, Linux effective uid 0): the
//!   command starts directly, no dialog.
//! - Windows: the se program starts itself elevated through
//!   `ShellExecuteExW` „runas“ (the UAC dialog) as the hidden helper
//!   `se elevated exec`; standard input, output and error travel over two
//!   named pipes this process creates with an explicit ACL (the user, the
//!   Administrators, NETWORK denied), `FILE_FLAG_FIRST_PIPE_INSTANCE` and
//!   `PIPE_REJECT_REMOTE_CLIENTS`. Both ends check the other's process id;
//!   the program is held open against changes from the request to the
//!   helper's start and the helper compares its own SHA-256 with the one
//!   requested.
//! - Linux: `pkexec --disable-internal-agent <se> elevated exec …` with the
//!   polkit agent of the session (126 dismissed, 127 not authorized); with
//!   no agent registered `sudo -n` instead; otherwise an error naming the
//!   cause. The helper talks over its standard streams, which pkexec and
//!   sudo hand on.
//!
//! Every elevated start asks once; a persistent shell that runs elevated is
//! one start. The command, its directory and its environment are plain
//! arguments of the helper, so the dialog's command line shows what runs.
//! A command started here runs in its own process group (Linux) or job
//! object (Windows): cancelling, and the end of its first process, end the
//! whole tree, as the Share Exec host does.
#[path = "core/helper_args.rs"]
pub mod helper_args;
// pkexec and sudo exist on Linux; the rules are tested on every system.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[path = "core/outcome.rs"]
mod outcome;
#[path = "core/types.rs"]
mod types;
#[path = "core/wire.rs"]
pub mod wire;

#[path = "os/shared/child.rs"]
mod child;
#[path = "os/shared/direct.rs"]
mod direct;
#[path = "os/shared/framed.rs"]
mod framed;
#[path = "os/shared/helper.rs"]
mod helper;

#[cfg(target_os = "linux")]
#[path = "os/linux_os.rs"]
mod platform;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;

pub use child::{ElevatedChild, ElevatedInput};
pub use helper::run_helper;
pub use types::{
    ElevateError, ElevatedCommand, ElevatedEvent, ElevatedExit, ElevatedSpec, Elevation,
};

/// Whether this process runs with administrator (Windows: elevated token)
/// or root rights (Linux: effective uid 0).
pub fn is_elevated() -> bool {
    platform::is_elevated()
}

/// Starts `spec` with administrator or root rights; asks the user through
/// the system dialog unless this process is elevated already. Blocks until
/// the user answered the dialog.
pub fn spawn_elevated(spec: ElevatedSpec) -> Result<ElevatedChild, ElevateError> {
    if is_elevated() {
        return direct::spawn(&spec, Elevation::Already);
    }
    platform::spawn_elevated(spec)
}

/// Starts `spec` with the rights of this process, in its own process group
/// or job object like an elevated start.
pub fn spawn_direct(spec: ElevatedSpec) -> Result<ElevatedChild, ElevateError> {
    direct::spawn(&spec, Elevation::Direct)
}

/// Asks the user to confirm `reason` through the system dialog of the
/// elevation (UAC, polkit): a change that weakens a protection (mask lists)
/// goes through only with the user at the device. A process that runs
/// elevated already holds every right the confirmation protects and gets no
/// dialog (decision P5: no dialog can prove more there). `sudo -n` is no
/// confirmation, because it asks nobody.
pub fn confirm_user_presence(reason: &str) -> Result<(), ElevateError> {
    if is_elevated() {
        return Ok(());
    }
    platform::confirm_user_presence(reason)
}

/// Windows: asks for a password in the system's credential dialog (CredUI);
/// `Ok(None)` when the user cancelled. Other systems answer `Failed`.
pub fn ask_password(
    caption: &str,
    message: &str,
) -> Result<Option<zeroize::Zeroizing<String>>, ElevateError> {
    platform::ask_password(caption, message)
}
