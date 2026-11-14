//! The processes a worker without session variables starts in the followed
//! session (E23, S3), across the account boundary between a root daemon
//! and the session's account:
//! - The screen host process carries `SE_SCREEN_FOLLOWED_SESSION` with the
//!   kind it was started for (`x11`, `wayland`, or `none` without a
//!   session). A host with that mark never follows a session by itself
//!   (`active_session::current` refuses), so a host started as root without
//!   a session stays without display (`no_display`) instead of reaching a
//!   desktop as root.
//! - Its pipes to the daemon are the only way in; an X11 host makes itself
//!   not dumpable, so no other process of the account opens them through
//!   `/proc/<pid>/fd` or attaches to it. A Wayland host stays dumpable:
//!   xdg-desktop-portal opens `/proc/<pid>/root` of its caller and refuses a
//!   process it may not inspect.
//! - The indicator window is held by a pidfd, so the window of a session
//!   that is no longer active is ended without the risk of a reused PID.
//!
//! A worker with its own session variables marks nothing and its processes
//! start as before.
use std::io;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::process::{Child, Command};

use super::{ActiveSession, SessionKind};

/// Mark of a screen host started by a worker without session variables.
pub const HOST_ENV: &str = "SE_SCREEN_FOLLOWED_SESSION";

/// The mark a host gets: `None` from a worker with own session variables
/// (unchanged environment), else the kind of the followed session.
pub fn marker(own_session: bool, followed: Option<SessionKind>) -> Option<&'static str> {
    if own_session {
        return None;
    }
    Some(match followed {
        Some(SessionKind::X11) => "x11",
        Some(SessionKind::Wayland) => "wayland",
        Some(SessionKind::None) | None => "none",
    })
}

/// Marks the host `command` for `followed`; call it after `spawn::apply`.
pub fn mark_host(followed: Option<&ActiveSession>, command: &mut Command) {
    let own = super::spawn::has_own_session(&|name| std::env::var(name).ok());
    if let Some(mark) = marker(own, followed.map(|session| session.kind)) {
        command.env(HOST_ENV, mark);
    }
}

/// Whether a process with mark `mark` is a host started by a worker
/// without session variables (it never follows a session itself).
pub fn is_started_host(mark: Option<&str>) -> bool {
    mark.is_some()
}

/// Whether a host with mark `mark` makes itself not dumpable.
pub fn seals(mark: Option<&str>) -> bool {
    mark == Some("x11")
}

/// In the host process, before it answers its first request.
pub fn seal_host() -> io::Result<()> {
    if !seals(std::env::var(HOST_ENV).ok().as_deref()) {
        return Ok(());
    }
    let off: libc::c_ulong = 0;
    // SAFETY: PR_SET_DUMPABLE takes the flag as its only value; no memory is
    // passed.
    if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, off, off, off, off) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Whether the indicator window started for the session `running` must
/// give way to one in the active session `active`.
pub fn window_moved(running: Option<&str>, active: Option<&str>) -> bool {
    active.is_some() && running != active
}

/// Whether the indicator window of a worker that follows the session
/// (`follows`) was closed by the person: it ended normally (`code` 0). That
/// window runs as the session's account and may not reach this worker to
/// end the sessions itself, so its close is „Alle beenden“ (C7): no session
/// runs on after the person closed the window. An update stop (75), a
/// crash or a signal keep their own handling.
pub fn window_closed_by_person(follows: bool, code: Option<i32>) -> bool {
    follows && code == Some(0)
}

/// The indicator window, held by a pidfd.
pub struct WindowHandle(OwnedFd);

impl WindowHandle {
    /// The handle of `child`; call it before the child is waited for.
    pub fn of(child: &Child) -> Option<Self> {
        let pid = libc::pid_t::try_from(child.id()).ok()?;
        let flags: libc::c_uint = 0;
        // SAFETY: pidfd_open takes a PID and flags and returns a new file
        // descriptor or -1; the child is not reaped yet, so its PID is its.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, flags) };
        let fd = RawFd::try_from(fd).ok().filter(|fd| *fd >= 0)?;
        // SAFETY: fd is a new descriptor owned by nobody else.
        Some(Self(unsafe { OwnedFd::from_raw_fd(fd) }))
    }

    /// Asks the window to end (`SIGTERM`); a window that ended already is
    /// not touched.
    pub fn terminate(&self) {
        use std::os::fd::AsRawFd;
        let flags: libc::c_uint = 0;
        // SAFETY: the pidfd is open for the call; no siginfo is passed.
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.0.as_raw_fd(),
                libc::SIGTERM,
                std::ptr::null::<libc::siginfo_t>(),
                flags,
            );
        }
    }
}
