//! Runs a job's root process as its grant's run-as account (Linux hosts with
//! a root worker). The ids come resolved from the worker (`exec_account.rs`);
//! between `fork` and `exec` the child only sets the supplementary groups,
//! the group and the user, in that order, checks that root cannot be
//! regained and then enters its working directory with the new rights, so it
//! never starts in a directory the account could not open itself. Any
//! failure of these steps ends the start: no process runs. A missing home
//! directory falls back to `/`, as sshd does; a working directory the device
//! asked for does not. The terminal of a terminal job is handed to the
//! account with mode 0620, as `sshd` does for a session's PTY.
use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::Command;

use crate::share::exec_supervisor_protocol::RunAsAccount;

/// Group of terminal devices; the account's own group where it is missing.
const TTY_GROUP: &std::ffi::CStr = c"tty";
const TERMINAL_MODE: libc::mode_t = 0o620;

pub(super) fn switch_to(
    command: &mut Command,
    account: &RunAsAccount,
    cwd: &str,
    home_fallback: bool,
) -> io::Result<()> {
    let groups: Vec<libc::gid_t> = account
        .groups
        .iter()
        .map(|group| *group as libc::gid_t)
        .collect();
    let uid = account.uid as libc::uid_t;
    let gid = account.gid as libc::gid_t;
    let cwd = CString::new(cwd).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "working directory contains NUL",
        )
    })?;
    // SAFETY: the hook runs between fork and exec, only makes
    // async-signal-safe system calls on data prepared before the fork and
    // allocates nothing.
    unsafe {
        command.pre_exec(move || {
            if libc::setgroups(groups.len() as _, groups.as_ptr()) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::setgid(gid) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::setuid(uid) != 0 {
                return Err(io::Error::last_os_error());
            }
            // As `sshd` does after dropping root: a process that can get
            // root or its group back would run with more than the account.
            if (uid != 0 && libc::setuid(0) == 0) || (gid != 0 && libc::setgid(0) == 0) {
                return Err(io::Error::from_raw_os_error(libc::EPERM));
            }
            if libc::chdir(cwd.as_ptr()) != 0 && (!home_fallback || libc::chdir(c"/".as_ptr()) != 0)
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

pub(super) fn hand_terminal(slave: &OwnedFd, account: &RunAsAccount) -> io::Result<()> {
    let group = tty_group().unwrap_or(account.gid as libc::gid_t);
    // SAFETY: both calls only use the descriptor `slave` owns.
    if unsafe { libc::fchown(slave.as_raw_fd(), account.uid as libc::uid_t, group) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: as above.
    if unsafe { libc::fchmod(slave.as_raw_fd(), TERMINAL_MODE) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn tty_group() -> Option<libc::gid_t> {
    let mut buffer = vec![0u8; 16 * 1024];
    // SAFETY: `group` is plain data; `getgrnam_r` fills it with pointers into
    // `buffer`, of which only the numeric id is read.
    let mut group: libc::group = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::group = std::ptr::null_mut();
    let status = unsafe {
        libc::getgrnam_r(
            TTY_GROUP.as_ptr(),
            &mut group,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    (status == 0 && !result.is_null()).then_some(group.gr_gid)
}
