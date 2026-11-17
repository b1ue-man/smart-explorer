//! Existing-system-client RDP adapter. The peer transport authenticates the
//! inventory; this module never imports a trusted pin from user JSON.
pub mod core;
mod os;
pub use self::core::{AuthenticatedProbe, HostProbe};
pub use os::probe as probe_local_host;
#[cfg(all(target_os = "linux", any(test, feature = "test-support")))]
pub use os::shared::clients::choose_client;
pub use os::shared::clients::{local_clients, watch_process_exit, Credentials, NEEDS_SDL};
/// Sign-in with the grant's account, host side (plan 2026-11-17 P4).
pub use os::shared::host_login;
/// Host-side RDP self-setup (design 3): triggers, state and system steps.
pub use os::shared::self_setup as host_setup;
/// The Android bridge's open (`mobile/.../rdp.rs`).
#[cfg(any(target_os = "android", all(unix, any(test, feature = "test-support"))))]
pub use os::shared::session::open_device;
pub use os::shared::session::SessionView;
pub use os::shared::session::{begin_owned_shutdown, resume_owned_sessions};
pub use os::shared::session::{close_owned_session, owned_sessions, shutdown_owned_sessions};
pub use os::shared::session::{open_device_session, open_device_with, OpenOptions};
/// Sign-in with the device's account, viewer side: state, storing the
/// password on the device (GUI, `se rdp password`; never MCP).
pub use os::shared::viewer_login;

/// A host up to 0.5.188 answers no probe v2 and so has no SHA-1 thumbprint
/// for mstsc (design 2.7.2); RDP is not started then.
pub const OLD_HOST: &str = "Gegenstelle älter als 0.5.189: Remote Desktop braucht dort das Update";

/// The PAM helper of the xrdp sign-in (P4): `se --rdp-xrdp-login`, started
/// by `pam_exec` of xrdp-sesman as root, exits with this code (0 accepts the
/// login with a se token, 1 hands it on to the password check). `None` for
/// every other invocation.
pub fn run_xrdp_login_if_requested(arguments: &[std::ffi::OsString]) -> Option<i32> {
    let first = arguments.first()?.to_str()?;
    if first != self::core::xrdp_login::PAM_SWITCH {
        return None;
    }
    Some(if arguments.len() == 1 {
        os::xrdp_login_check()
    } else {
        1
    })
}

/// Close a locally owned native session synchronously. A separate CLI process
/// can still close the shared daemon lease; its native owner observes that
/// revocation. No path kills an unowned PID.
pub fn close(id: crate::share::tcp_forward::TunnelId) -> Result<bool, String> {
    if close_owned_session(&id)? {
        return Ok(true);
    }
    os::shared::session::close_transport(id)
}
