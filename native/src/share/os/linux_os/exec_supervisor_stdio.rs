//! Supervisor of an account unit (PRM-M5): `--share-exec-supervisor-stdio`.
//! systemd starts it as the job's account with the worker's connection as
//! standard input and output (`StandardInputFileDescriptor`,
//! `StandardOutputFileDescriptor`); standard error inherits it as well. The
//! mode takes the connection off the standard descriptors and points all
//! three at `/dev/null`, so no stray write of the process (a panic message)
//! enters the framed protocol, then serves the job like the supervisor of
//! the worker's own account. It never switches accounts: `exec_supervisor`
//! refuses a start whose account this process is not.
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram, UnixStream};
use std::path::Path;

use super::exec_supervisor::{self, SpawnPolicy};

pub(super) const STDIO_MODE: &str = "--share-exec-supervisor-stdio";

pub(super) fn run_if_requested(
    arguments: &[OsString],
    policy: &SpawnPolicy<'_>,
) -> Option<io::Result<()>> {
    (arguments.len() == 1 && arguments[0] == STDIO_MODE).then(|| {
        let stream = take_connection()?;
        notify_ready()?;
        exec_supervisor::run(stream, policy)
    })
}

/// Tells systemd (`Type=notify`) that the supervisor runs as the account.
/// The unit's start job ends with this message; a process started without
/// `NOTIFY_SOCKET` (by hand) has nobody to tell. The jobs' environment is
/// built from the start request, so they never see the variable.
fn notify_ready() -> io::Result<()> {
    let Some(address) = std::env::var_os("NOTIFY_SOCKET") else {
        return Ok(());
    };
    let bytes = address.as_bytes();
    let socket = UnixDatagram::unbound()?;
    match bytes.split_first() {
        // An abstract socket name starts with `@`.
        Some((b'@', name)) => {
            let address = SocketAddr::from_abstract_name(name)?;
            socket.send_to_addr(b"READY=1", &address)?;
        }
        Some(_) => {
            socket.send_to(b"READY=1", Path::new(&address))?;
        }
        None => {}
    }
    Ok(())
}

/// The connection on standard input, moved to a descriptor of its own.
fn take_connection() -> io::Result<UnixStream> {
    // SAFETY: `stat` is plain data that `fstat` fills for descriptor 0.
    let mut status: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(libc::STDIN_FILENO, &mut status) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if status.st_mode & libc::S_IFMT != libc::S_IFSOCK {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "standard input is not the connection of an account unit",
        ));
    }
    // SAFETY: `F_DUPFD_CLOEXEC` returns a new descriptor that only the
    // `OwnedFd` below owns.
    let duplicate = unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_DUPFD_CLOEXEC, 3) };
    if duplicate < 0 {
        return Err(io::Error::last_os_error());
    }
    let connection = unsafe { OwnedFd::from_raw_fd(duplicate) };
    let null = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")?;
    for target in [libc::STDIN_FILENO, libc::STDOUT_FILENO, libc::STDERR_FILENO] {
        // SAFETY: `dup2` replaces a standard descriptor with `/dev/null`;
        // the connection lives on in `connection`.
        if unsafe { libc::dup2(null.as_raw_fd(), target) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(UnixStream::from(connection))
}
