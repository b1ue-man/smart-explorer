//! The worker's side of the account service on Linux (PRM-M6) and the frame
//! transport both sides share. A frame with a descriptor goes out in one
//! `sendmsg` with the descriptor attached to its first byte (`SCM_RIGHTS`);
//! the service reads every frame with `recvmsg`, takes at most one
//! descriptor per frame and closes it on exec. One connection serves one
//! job session, so the service ends the session's unit when the worker
//! closes the connection or dies. The worker talks only to a socket of root
//! in root's directory `/run/smart-explorer`.
use std::io::{self, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::share::access_catalog::{HelperState, HelperStatus};
use crate::share::exec_client_messages::{ACCOUNT_HELPER_UNAVAILABLE, ACCOUNT_NOT_ALLOWED};
use crate::share::exec_helper_protocol::{
    account_error_code, account_start_error, encode_frame, frame_length, read_frame, write_frame,
    HelperReply, HelperRequest, HELPER_PROTOCOL,
};

use super::exec_binary_copy::trusted_directories;
use super::exec_helper_setup::SOCKET_UNIT_FILE;
use super::exec_systemd::require_cgroup_v2;

pub(super) const HELPER_SOCKET: &str = "/run/smart-explorer/exec-helper.sock";
const HELPER_DIRECTORY: &str = "/run/smart-explorer";
/// A start waits up to 17 s for its unit; replies come well within this.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// Size of one descriptor in a control message.
const DESCRIPTOR_BYTES: libc::c_uint = std::mem::size_of::<libc::c_int>() as libc::c_uint;
/// Room for this many descriptors per read, so a frame with too many of
/// them is seen and all are closed.
const MAX_DESCRIPTORS: libc::c_uint = 4;

/// Sends `frame` with `descriptor` attached to its first byte.
pub(super) fn send_frame(
    stream: &UnixStream,
    frame: &[u8],
    descriptor: BorrowedFd<'_>,
) -> io::Result<()> {
    let mut iov = libc::iovec {
        iov_base: frame.as_ptr().cast_mut().cast(),
        iov_len: frame.len(),
    };
    let mut control = [0u64; 8];
    // SAFETY: `message` points at `iov` and `control`, which outlive the
    // call; the control buffer is 8-byte aligned and larger than one
    // descriptor's control message, which `CMSG_FIRSTHDR` places at its start.
    let sent = unsafe {
        let mut message: libc::msghdr = std::mem::zeroed();
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = libc::CMSG_SPACE(DESCRIPTOR_BYTES) as _;
        let header = libc::CMSG_FIRSTHDR(&message);
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(DESCRIPTOR_BYTES) as _;
        std::ptr::write_unaligned(
            libc::CMSG_DATA(header).cast::<libc::c_int>(),
            descriptor.as_raw_fd(),
        );
        loop {
            let sent = libc::sendmsg(stream.as_raw_fd(), &message, libc::MSG_NOSIGNAL);
            if sent >= 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                break sent;
            }
        }
    };
    let sent = usize::try_from(sent).map_err(|_| io::Error::last_os_error())?;
    // The rest of the frame, if the socket took only a part, follows plainly.
    let mut rest = stream;
    rest.write_all(&frame[sent.min(frame.len())..])
}

/// Receives one frame and a descriptor sent with it; `Ok(None)` when the
/// peer closed between frames. While `stop` is set, a wait that a signal or
/// the socket's read timeout interrupts ends with `Interrupted`.
pub(super) fn recv_frame(
    stream: &UnixStream,
    stop: Option<&AtomicBool>,
) -> io::Result<Option<(Vec<u8>, Option<OwnedFd>)>> {
    let mut descriptor = None;
    let mut header = [0u8; 4];
    let mut filled = 0;
    while filled < header.len() {
        let read = recv_some(stream, &mut header[filled..], &mut descriptor, stop)?;
        if read == 0 && filled == 0 {
            return Ok(None);
        }
        if read == 0 {
            return Err(early_end());
        }
        filled += read;
    }
    let mut payload = vec![0u8; frame_length(header)?];
    filled = 0;
    while filled < payload.len() {
        let read = recv_some(stream, &mut payload[filled..], &mut descriptor, stop)?;
        if read == 0 {
            return Err(early_end());
        }
        filled += read;
    }
    Ok(Some((payload, descriptor)))
}

/// One `recvmsg`. A descriptor that arrives goes to `descriptor`; a second
/// one in the same frame, or more than one at once, is an error, and every
/// descriptor of such a frame is closed.
fn recv_some(
    stream: &UnixStream,
    buffer: &mut [u8],
    descriptor: &mut Option<OwnedFd>,
    stop: Option<&AtomicBool>,
) -> io::Result<usize> {
    let mut iov = libc::iovec {
        iov_base: buffer.as_mut_ptr().cast(),
        iov_len: buffer.len(),
    };
    let mut control = [0u64; 8];
    loop {
        // SAFETY: as in `send_frame`; `recvmsg` writes at most
        // `msg_controllen` bytes of control data and `iov_len` bytes of data;
        // every descriptor of the control message becomes an `OwnedFd`.
        let (read, mut received, truncated) = unsafe {
            let mut message: libc::msghdr = std::mem::zeroed();
            message.msg_iov = &mut iov;
            message.msg_iovlen = 1;
            message.msg_control = control.as_mut_ptr().cast();
            message.msg_controllen = libc::CMSG_SPACE(MAX_DESCRIPTORS * DESCRIPTOR_BYTES) as _;
            let read = libc::recvmsg(stream.as_raw_fd(), &mut message, libc::MSG_CMSG_CLOEXEC);
            let mut received = Vec::new();
            let header = libc::CMSG_FIRSTHDR(&message);
            if read >= 0
                && !header.is_null()
                && (*header).cmsg_level == libc::SOL_SOCKET
                && (*header).cmsg_type == libc::SCM_RIGHTS
            {
                let length: usize = (*header).cmsg_len as _;
                let empty: usize = libc::CMSG_LEN(0) as _;
                let data = libc::CMSG_DATA(header).cast::<libc::c_int>();
                let count = length.saturating_sub(empty) / std::mem::size_of::<libc::c_int>();
                for index in 0..count {
                    let raw = std::ptr::read_unaligned(data.add(index));
                    received.push(OwnedFd::from_raw_fd(raw));
                }
            }
            (read, received, message.msg_flags & libc::MSG_CTRUNC != 0)
        };
        if let Ok(read) = usize::try_from(read) {
            if truncated || received.len() > 1 || (descriptor.is_some() && !received.is_empty()) {
                return Err(invalid("a frame carried more than one descriptor"));
            }
            if let Some(received) = received.pop() {
                *descriptor = Some(received);
            }
            return Ok(read);
        }
        let error = io::Error::last_os_error();
        let waiting = matches!(
            error.kind(),
            io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        );
        match stop {
            Some(stop) if waiting && stop.load(Ordering::SeqCst) => {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "stopping"));
            }
            Some(_) if waiting => {}
            None if error.kind() == io::ErrorKind::Interrupted => {}
            _ => return Err(error),
        }
    }
}

/// A connection to the account service after `hello`.
pub(super) struct HelperConnection {
    stream: UnixStream,
    /// Smart Explorer version of the service.
    version: String,
}

/// Why no connection came about.
pub(super) enum OpenFailure {
    /// No socket, or nobody answers it.
    NotRunning(io::Error),
    /// The service answered with an error or another protocol.
    Refused(io::Error),
}

impl OpenFailure {
    fn into_error(self) -> io::Error {
        match self {
            Self::NotRunning(error) => account_start_error(
                ACCOUNT_HELPER_UNAVAILABLE,
                format!("the account service does not answer at {HELPER_SOCKET} ({error})"),
            ),
            Self::Refused(error) => error,
        }
    }
}

impl HelperConnection {
    pub(super) fn open() -> Result<Self, OpenFailure> {
        let stream = connect().map_err(OpenFailure::NotRunning)?;
        let mut connection = Self {
            stream,
            version: String::new(),
        };
        match connection.request(&HelperRequest::hello(), None) {
            Ok(HelperReply::HelloOk {
                protocol,
                se_version,
            }) if protocol == HELPER_PROTOCOL => {
                connection.version = se_version;
                Ok(connection)
            }
            Ok(HelperReply::HelloOk {
                protocol,
                se_version,
            }) => Err(OpenFailure::Refused(account_start_error(
                ACCOUNT_HELPER_UNAVAILABLE,
                format!(
                    "the account service of Smart Explorer {se_version} speaks protocol \
                     {protocol}, this worker protocol {HELPER_PROTOCOL}"
                ),
            ))),
            Ok(other) => Err(OpenFailure::Refused(unexpected(&other))),
            Err(error) => Err(OpenFailure::Refused(error)),
        }
    }

    pub(super) fn version(&self) -> &str {
        &self.version
    }

    /// One request and its reply; an error reply becomes an error with the
    /// code the admission sends.
    fn request(
        &mut self,
        request: &HelperRequest,
        descriptor: Option<BorrowedFd<'_>>,
    ) -> io::Result<HelperReply> {
        match descriptor {
            Some(descriptor) => send_frame(&self.stream, &encode_frame(request)?, descriptor)?,
            None => write_frame(&mut &self.stream, request)?,
        }
        let reply = read_frame::<HelperReply>(&mut &self.stream)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the account service closed the connection",
            )
        })?;
        match reply {
            HelperReply::Error { code, detail } => {
                Err(account_start_error(code.wire_code(), detail))
            }
            reply => Ok(reply),
        }
    }
}

/// The session of one job: a connection whose account unit runs the job's
/// supervisor as another account.
pub(super) struct HelperSession {
    connection: HelperConnection,
    session: String,
}

impl HelperSession {
    /// Asks the service to run `supervisor` (the supervisor's end of the
    /// job's socket pair) as `account`.
    pub(super) fn start(
        account: &str,
        cwd: Option<&str>,
        supervisor: &UnixStream,
    ) -> io::Result<Self> {
        let mut connection = HelperConnection::open().map_err(OpenFailure::into_error)?;
        let request = HelperRequest::Start {
            account: account.into(),
            cwd: cwd.map(Into::into),
            pipe_in: None,
            pipe_out: None,
        };
        match connection.request(&request, Some(supervisor.as_fd()))? {
            HelperReply::Started { session } => Ok(Self {
                connection,
                session,
            }),
            other => Err(unexpected(&other)),
        }
    }

    /// The session's unit.
    pub(super) fn name(&self) -> &str {
        &self.session
    }

    pub(super) fn stop(&mut self) -> io::Result<()> {
        let request = HelperRequest::Stop {
            session: self.session.clone(),
        };
        match self.connection.request(&request, None)? {
            HelperReply::Stopped {} => Ok(()),
            other => Err(unexpected(&other)),
        }
    }

    /// `(active, empty)` of the session's unit.
    pub(super) fn status(&mut self) -> io::Result<(bool, bool)> {
        let request = HelperRequest::Status {
            session: self.session.clone(),
        };
        match self.connection.request(&request, None)? {
            HelperReply::Status { active, empty } => Ok((active, empty)),
            other => Err(unexpected(&other)),
        }
    }
}

/// State of the account service for a worker that is not root.
pub(super) fn helper_status() -> HelperStatus {
    if let Err(error) = require_cgroup_v2() {
        return status(HelperState::Unavailable, None, Some(error.to_string()));
    }
    let installed = std::fs::symlink_metadata(SOCKET_UNIT_FILE).is_ok();
    match HelperConnection::open() {
        Ok(connection) => status(
            HelperState::Installed,
            Some(connection.version().to_string()),
            None,
        ),
        Err(OpenFailure::NotRunning(error)) if installed => status(
            HelperState::Outdated,
            None,
            Some(format!("its socket does not answer ({error})")),
        ),
        Err(OpenFailure::NotRunning(_)) => status(HelperState::NotInstalled, None, None),
        Err(OpenFailure::Refused(error)) => match account_error_code(&error) {
            Some(ACCOUNT_NOT_ALLOWED) => status(
                HelperState::Installed,
                None,
                Some(format!("this user is not registered: {error}")),
            ),
            _ => status(HelperState::Outdated, None, Some(error.to_string())),
        },
    }
}

fn status(state: HelperState, version: Option<String>, detail: Option<String>) -> HelperStatus {
    HelperStatus {
        state,
        version,
        detail,
    }
}

/// Connects to the service's socket, which root created in root's
/// directory, and checks that root listens on it.
fn connect() -> io::Result<UnixStream> {
    trusted_directories(Path::new(HELPER_DIRECTORY))?;
    let socket = std::fs::symlink_metadata(HELPER_SOCKET)?;
    if !socket.file_type().is_socket() || socket.uid() != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{HELPER_SOCKET} is not root's socket"),
        ));
    }
    let stream = UnixStream::connect(HELPER_SOCKET)?;
    if peer_credentials(&stream)?.uid != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{HELPER_SOCKET} is not served by root"),
        ));
    }
    stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
    stream.set_write_timeout(Some(REPLY_TIMEOUT))?;
    Ok(stream)
}

/// Credentials of the process at the other end of `stream`, which the kernel
/// recorded when it connected (`SO_PEERCRED`).
pub(super) fn peer_credentials(stream: &UnixStream) -> io::Result<libc::ucred> {
    // SAFETY: `ucred` is plain data that `getsockopt` fills.
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut size,
        )
    };
    if result == 0 {
        Ok(credentials)
    } else {
        Err(io::Error::last_os_error())
    }
}

fn unexpected(reply: &HelperReply) -> io::Error {
    invalid_owned(format!(
        "unexpected reply of the account service: {reply:?}"
    ))
}

fn early_end() -> io::Error {
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "an account service frame ends early",
    )
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn invalid_owned(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
