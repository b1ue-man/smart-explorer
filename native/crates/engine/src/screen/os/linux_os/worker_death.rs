//! Lifetime of the screen host process on Linux (SCR-M3): the process asks
//! the kernel for `SIGKILL` when the thread that started it ends
//! (`PR_SET_PDEATHSIG`; the daemon starts every host process from one thread
//! that lives as long as the daemon, `host_worker_client`) and then checks
//! that its parent is still the daemon that named itself in
//! `SE_SCREEN_WORKER_PARENT`; otherwise the daemon died before the signal
//! was armed. The end of the process's input is the second sign.
use std::io;
use std::process::{Child, Command};

use super::host_worker_client::PARENT_ENV;

/// Before the start: nothing to prepare.
pub(super) fn prepare(_command: &mut Command) {}

/// After the start: the process arms itself.
pub(super) fn bind(_child: &Child) -> io::Result<()> {
    Ok(())
}

/// In the host process, before anything else.
pub(super) fn arm_worker() -> io::Result<()> {
    let signal = libc::SIGKILL as libc::c_ulong;
    let unused: libc::c_ulong = 0;
    // SAFETY: PR_SET_PDEATHSIG takes the signal number as its only value;
    // no memory is passed, the other arguments are unused zeros.
    let armed = unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, signal, unused, unused, unused) };
    if armed != 0 {
        return Err(io::Error::last_os_error());
    }
    let expected = std::env::var(PARENT_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok());
    if let Some(expected) = expected {
        // SAFETY: getppid takes no arguments and cannot fail.
        let parent = unsafe { libc::getppid() };
        if u32::try_from(parent).ok() != Some(expected) {
            return Err(io::Error::other(
                "the daemon ended before the screen host process started",
            ));
        }
    }
    Ok(())
}
