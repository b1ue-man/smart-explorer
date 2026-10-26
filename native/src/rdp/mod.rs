//! Existing-system-client RDP adapter. The peer transport authenticates the
//! inventory; this module never imports a trusted pin from user JSON.
pub(crate) mod core;
mod os;
pub(crate) use self::core::{AuthenticatedProbe, CertificatePolicy, HostProbe, StartRequest};
pub(crate) use os::probe as probe_local_host;
pub(crate) use os::shared::clients::local_clients;
pub(crate) use os::shared::session::{begin_owned_shutdown, resume_owned_sessions};
pub(crate) use os::shared::session::{
    close_owned_session, owned_sessions, shutdown_owned_sessions, start_owned,
};
pub(crate) use os::shared::session::{RdpSession, SessionView};

/// Client and tunnel share one owner; retaining it is mandatory for GUI/jobs.
pub(crate) fn start(
    target: crate::share::PeerOpenTarget,
    probe: &AuthenticatedProbe,
    request: &StartRequest,
    wait: std::time::Duration,
    pending: impl FnMut(),
) -> Result<RdpSession, String> {
    RdpSession::start(target, probe, request, wait, pending)
}

/// Close a locally owned native session synchronously. A separate CLI process
/// can still close the shared daemon lease; its native owner observes that
/// revocation. No path kills an unowned PID.
pub(crate) fn close(id: crate::share::tcp_forward::TunnelId) -> Result<bool, String> {
    if close_owned_session(&id)? {
        return Ok(true);
    }
    os::shared::session::close_transport(id)
}

/// MCP/mobile may transfer the already admitted session to an owned worker.
/// The surrounding server must close its sessions during its normal shutdown;
/// the daemon TCP lease independently bounds service exposure after a crash.
pub(crate) fn own_in_background(session: RdpSession) -> Result<SessionView, String> {
    os::shared::session::adopt_owned(session)
}
