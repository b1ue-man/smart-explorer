//! Pseudo-terminal of a terminal exec on Linux and Android hosts. The POSIX
//! pair comes from `posix_openpt`, `grantpt`, `unlockpt` and `ptsname_r`, not
//! from the one-call helper that glibc before 2.34 keeps in libutil (the
//! desktop app is built against glibc 2.17). `exec_supervisor.rs` gives the
//! slave to the job's root as stdin, stdout and stderr and as its controlling
//! terminal; the master carries the output, the input and the window size.
//!
//! Android: SELinux lets an app create and use PTYs (`create_pty` in the
//! `untrusted_app_all` domain); the Share service can serve terminals only
//! while the app process runs, like every other exec of that host.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::{Command, Stdio};
use std::sync::mpsc;

use crate::share::exec_terminal::{ExecTerminalRequest, ExecTerminalSize};

/// Chunks of input (each at most 64 KiB) the master's writer may be behind
/// before the session ends instead of growing without bound.
const INPUT_QUEUE: usize = 64;
/// Room for the slave path (`/dev/pts/<n>`) that `ptsname_r` writes.
const SLAVE_NAME_BYTES: usize = 128;

/// Both ends of a new pseudo-terminal. Every descriptor is close-on-exec.
pub(super) struct Pty {
    pub(super) master: OwnedFd,
    pub(super) slave: OwnedFd,
}

/// True when this host can open a pseudo-terminal: opens and closes a pair.
pub(super) fn terminal_supported() -> bool {
    open_pair(ExecTerminalSize { cols: 80, rows: 24 }).is_ok()
}

/// Opens a pair whose window already has `size`, with UTF-8 input editing
/// enabled like the terminal modes of an SSH session.
pub(super) fn open_pair(size: ExecTerminalSize) -> io::Result<Pty> {
    open_pair_raw(size).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("cannot open a pseudo-terminal: {error}"),
        )
    })
}

fn open_pair_raw(size: ExecTerminalSize) -> io::Result<Pty> {
    // SAFETY: `posix_openpt` returns a new descriptor or -1.
    let master =
        owned(unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) })?;
    // SAFETY: both calls only use the descriptor `master` owns.
    if unsafe { libc::grantpt(master.as_raw_fd()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: as above.
    if unsafe { libc::unlockpt(master.as_raw_fd()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut name: [libc::c_char; SLAVE_NAME_BYTES] = [0; SLAVE_NAME_BYTES];
    // SAFETY: `name` has the room `ptsname_r` is told about and receives a
    // NUL-terminated path on success; the return value is the error number.
    let code = unsafe { libc::ptsname_r(master.as_raw_fd(), name.as_mut_ptr(), name.len()) };
    if code != 0 {
        return Err(io::Error::from_raw_os_error(code));
    }
    // The window size belongs to the pair, so the first program already sees it.
    set_size(&master, size)?;
    // SAFETY: `name` holds the NUL-terminated path `ptsname_r` wrote.
    let slave = owned(unsafe {
        libc::open(
            name.as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        )
    })?;
    enable_utf8_input(&slave)?;
    Ok(Pty { master, slave })
}

fn owned(raw: libc::c_int) -> io::Result<OwnedFd> {
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a descriptor that a successful call just returned and
    // that nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

/// `TIOCSWINSZ` on the master; the kernel sends `SIGWINCH` to the foreground
/// process group of the terminal when the size changed.
pub(super) fn set_size(master: &impl AsRawFd, size: ExecTerminalSize) -> io::Result<()> {
    let window = libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: `TIOCSWINSZ` reads one `winsize` from the pointer.
    if unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &window) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// `IUTF8`: the line discipline erases whole UTF-8 characters, as `ssh` asks.
fn enable_utf8_input(slave: &OwnedFd) -> io::Result<()> {
    // SAFETY: `tcgetattr` fills the zeroed `termios` before it is changed and
    // handed to `tcsetattr`; both only use the descriptor `slave` owns.
    unsafe {
        let mut attributes: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(slave.as_raw_fd(), &mut attributes) != 0 {
            return Err(io::Error::last_os_error());
        }
        attributes.c_iflag |= libc::IUTF8;
        if libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &attributes) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Gives `command` the slave as stdin, stdout and stderr and the terminal's
/// environment: `TERM` from the request, no stale `COLUMNS` and `LINES`
/// (programs read the real size from the terminal). The caller's copies of
/// the slave close when `command` is dropped, which must follow the spawn:
/// the output ends only after the last slave descriptor is closed.
pub(super) fn attach_slave(
    command: &mut Command,
    slave: OwnedFd,
    request: &ExecTerminalRequest,
) -> io::Result<()> {
    command
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave))
        .env("TERM", &request.term)
        .env_remove("COLUMNS")
        .env_remove("LINES");
    Ok(())
}

/// Runs in the forked child between `fork` and `exec` (std's `pre_exec`):
/// only async-signal-safe system calls, no allocation, no locks. The child
/// leads a new session, and the slave on stdin (std redirected it already)
/// becomes the session's controlling terminal. `setsid` fails for a process
/// group leader, so no `setpgid` may precede it.
pub(super) fn become_session_leader() -> io::Result<()> {
    // SAFETY: `setsid` and `ioctl` on descriptor 0 are system calls that touch
    // no memory of this process but the local argument.
    unsafe {
        if libc::setsid() < 0 {
            return Err(io::Error::last_os_error());
        }
        let steal: libc::c_int = 0;
        if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY, steal) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Reads the master. Linux reports the close of every slave descriptor as
/// `EIO`: that is the end of the output.
pub(super) struct MasterReader(File);

impl Read for MasterReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.0.read(buffer) {
                Err(error) if error.raw_os_error() == Some(libc::EIO) => return Ok(0),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                other => return other,
            }
        }
    }
}

/// Input side of a terminal job. A writer thread owns the blocking writes to
/// the master, so a full terminal input buffer never stops the supervisor's
/// event loop (cancel, window size and the root's exit stay serviced).
pub(super) struct TerminalInput {
    queue: mpsc::SyncSender<Vec<u8>>,
    master: OwnedFd,
}

/// Starts the writer thread of `master` and returns the input side and the
/// reader of the output.
pub(super) fn start_io(master: OwnedFd) -> io::Result<(TerminalInput, MasterReader)> {
    let reader = MasterReader(File::from(master.try_clone()?));
    let mut writer = File::from(master.try_clone()?);
    let (queue, input) = mpsc::sync_channel::<Vec<u8>>(INPUT_QUEUE);
    std::thread::Builder::new()
        .name("exec-terminal-input".into())
        .spawn(move || {
            while let Ok(chunk) = input.recv() {
                if writer.write_all(&chunk).is_err() {
                    break;
                }
            }
        })?;
    Ok((TerminalInput { queue, master }, reader))
}

impl TerminalInput {
    /// Queues `data` for the master. Once the writer has ended the terminal is
    /// closed and the root's exit follows, so late input is dropped; a
    /// backlog of `INPUT_QUEUE` chunks means nothing reads the terminal any
    /// more and is an error.
    pub(super) fn write(&self, data: Vec<u8>) -> io::Result<()> {
        match self.queue.try_send(data) {
            Ok(()) | Err(mpsc::TrySendError::Disconnected(_)) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "terminal input is not being read",
            )),
        }
    }

    pub(super) fn resize(&self, size: ExecTerminalSize) -> io::Result<()> {
        set_size(&self.master, size)
    }
}
