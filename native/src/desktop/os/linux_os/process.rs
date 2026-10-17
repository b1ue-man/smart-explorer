//! Processes of se Desktop on Linux (DSK-M3, C5.5, C5.7): the shell starts
//! in its own session (`setsid`) with null standard streams, a surface starts
//! with pipes for its channel, and a process is ended by PID only after
//! `/proc/<pid>/exe` named the expected program. The signal goes through a
//! pidfd opened before the check, so a reused PID is never hit.
use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Starts `exe args` detached from the caller (own session, null streams).
pub(crate) fn spawn_detached(exe: &Path, args: &[&str]) -> io::Result<Child> {
    let mut command = Command::new(exe);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // `CommandExt::setsid` is unstable; `setsid(2)` is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    command.spawn()
}

/// Starts a surface with stdin/stdout as its channel to the shell.
pub(crate) fn spawn_piped(exe: &Path, args: &[String]) -> io::Result<Child> {
    Command::new(exe)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
}

/// `/proc/<pid>/exe`; a program replaced while it runs reads
/// `<path> (deleted)`.
fn image(pid: u32) -> Option<PathBuf> {
    if pid == 0 {
        return None;
    }
    std::fs::read_link(format!("/proc/{pid}/exe")).ok()
}

/// Whether the image `found` is the program `expected`, also after an
/// installation replaced the file (`install-linux.sh` renames atomically).
pub(crate) fn same_image(found: &Path, expected: &Path) -> bool {
    let found = found.to_string_lossy();
    let found = found.strip_suffix(" (deleted)").unwrap_or(&found);
    let canonical = std::fs::canonicalize(expected).ok();
    std::iter::once(expected)
        .chain(canonical.as_deref())
        .any(|candidate| found == candidate.to_string_lossy())
}

/// The process `pid` runs and its image is `expected`.
pub(crate) fn alive_with_image(pid: u32, expected: &Path) -> bool {
    image(pid).is_some_and(|found| same_image(&found, expected))
}

struct PidFd(libc::c_int);

impl PidFd {
    fn open(pid: libc::pid_t) -> Option<PidFd> {
        // Variadic arguments go as `long`, as `syscall(2)` reads them.
        let fd = unsafe {
            libc::syscall(
                libc::SYS_pidfd_open,
                libc::c_long::from(pid),
                0 as libc::c_long,
            )
        };
        (fd >= 0).then(|| PidFd(fd as libc::c_int))
    }

    fn kill(&self) -> io::Result<()> {
        let sent = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                libc::c_long::from(self.0),
                libc::c_long::from(libc::SIGKILL),
                std::ptr::null::<libc::siginfo_t>(),
                0 as libc::c_long,
            )
        };
        if sent == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl Drop for PidFd {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.0);
        }
    }
}

/// Ends `pid` with `SIGKILL` only when its image is `expected`; `Ok(false)`
/// when the process is gone or runs another program.
pub(crate) fn terminate_checked(pid: u32, expected: &Path) -> io::Result<bool> {
    let Ok(raw) = libc::pid_t::try_from(pid) else {
        return Ok(false);
    };
    if raw <= 0 {
        return Ok(false);
    }
    // The pidfd pins the process before its image is read (Linux ≥ 5.3);
    // older kernels fall back to `kill` right after the check.
    let pidfd = PidFd::open(raw);
    if !alive_with_image(pid, expected) {
        return Ok(false);
    }
    let result = match &pidfd {
        Some(pidfd) => pidfd.kill(),
        None => {
            if unsafe { libc::kill(raw, libc::SIGKILL) } == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    };
    match result {
        Ok(()) => Ok(true),
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(false),
        Err(error) => Err(error),
    }
}
