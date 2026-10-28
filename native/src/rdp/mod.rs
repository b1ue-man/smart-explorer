//! Existing-system-client RDP adapter. The peer transport authenticates the
//! inventory; this module never imports a trusted pin from user JSON.
pub(crate) mod core;
mod os;
pub(crate) use self::core::{AuthenticatedProbe, CertificatePolicy, HostProbe, StartRequest};
pub(crate) use os::probe as probe_local_host;
pub(crate) use os::shared::clients::{
    choose_client, local_clients, watch_process_exit, Credentials, NEEDS_SDL,
};
/// Host-side RDP self-setup (design 3): triggers, state and system steps.
pub(crate) use os::shared::self_setup as host_setup;
pub(crate) use os::shared::session::{begin_owned_shutdown, resume_owned_sessions};
pub(crate) use os::shared::session::{
    close_owned_session, owned_sessions, shutdown_owned_sessions, start_owned,
};
pub(crate) use os::shared::session::{
    open_device, open_device_session, open_device_with, OpenOptions,
};
pub(crate) use os::shared::session::{RdpSession, SessionView, StartContext};

/// A host up to 0.5.188 answers no probe v2 and so has no SHA-1 thumbprint
/// for mstsc (design 2.7.2); RDP is not started then.
pub(crate) const OLD_HOST: &str =
    "Gegenstelle älter als 0.5.189: Remote Desktop braucht dort das Update";

/// Client and tunnel share one owner; retaining it is mandatory for GUI/jobs.
/// One probe serves one start: it is consumed here (design 2.1).
pub(crate) fn start(
    target: crate::share::PeerOpenTarget,
    probe: AuthenticatedProbe,
    request: &StartRequest,
    context: StartContext,
    pending: impl FnMut(),
) -> Result<RdpSession, String> {
    RdpSession::start(target, probe, request, context, pending)
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
/// after a crash the daemon closes the tunnel when the bound client ends.
pub(crate) fn own_in_background(session: RdpSession) -> Result<SessionView, String> {
    os::shared::session::adopt_owned(session)
}
