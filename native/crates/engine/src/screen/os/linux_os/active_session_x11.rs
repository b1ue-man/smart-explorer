//! X11 connections of a worker without session variables (E23, S3): the
//! daemon of a root service lists monitors and windows, probes the server
//! and reads window icons of the active X11 session it follows. A process
//! with its own session variables connects as before through `DISPLAY` and
//! `XAUTHORITY` of its environment (`RustConnection::connect(None)`).
//! Otherwise the connection goes to the display logind names for the
//! followed session, through its socket in `/tmp/.X11-unix` (never the
//! abstract socket, which any local process could hold), only to a server
//! of root or of the session's account (`SO_PEERCRED`), with the cookie of
//! the verified `XAUTHORITY` of that session (or `~/.Xauthority` of its
//! account), read only from a regular file of that account. A process
//! without a followed X11 session gets no connection, never a guessed one.
use std::fs::OpenOptions;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;

use x11rb::connection::Connection;
use x11rb::rust_connection::{DefaultStream, RustConnection};

use super::{ActiveSession, SessionKind};

/// The one authorization x11rb speaks.
pub const COOKIE_NAME: &[u8] = b"MIT-MAGIC-COOKIE-1";
/// Xauthority family of a local (Unix socket) server, keyed by host name.
pub const FAMILY_LOCAL: u16 = 256;
/// Xauthority family that matches every address.
pub const FAMILY_WILD: u16 = 65535;
/// Largest Xauthority file read.
const MAX_XAUTHORITY: u64 = 64 * 1024;
/// Directory of the local X server sockets.
const SOCKET_DIR: &str = "/tmp/.X11-unix";

/// A local display `:N` or `:N.S`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalDisplay {
    pub number: u32,
    pub screen: usize,
}

/// The local display of `value`; `None` for a remote or malformed name.
pub fn parse_display(value: &str) -> Option<LocalDisplay> {
    if !super::env::local_display(value) {
        return None;
    }
    let rest = value.strip_prefix(':')?;
    let (number, screen) = rest.split_once('.').unwrap_or((rest, "0"));
    if number.is_empty() || screen.is_empty() || screen.contains('.') {
        return None;
    }
    Some(LocalDisplay {
        number: number.parse().ok()?,
        screen: screen.parse().ok()?,
    })
}

/// How a process reaches its X server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// Through its own `DISPLAY` and `XAUTHORITY` (unchanged path).
    Own,
    /// To the followed X11 session.
    Followed,
    /// Not at all; the reason for the error.
    Refused(&'static str),
}

/// The route of a process: `own_session` when it has session variables,
/// `followed` the kind of the session it follows, if any.
pub fn route(own_session: bool, followed: Option<SessionKind>) -> Route {
    match (own_session, followed) {
        (true, _) => Route::Own,
        (false, Some(SessionKind::X11)) => Route::Followed,
        (false, Some(_)) => Route::Refused("the followed session is not X11"),
        (false, None) => Route::Refused("no followed X11 session"),
    }
}

/// Whether an X server of `peer_uid` may get the cookie of a session of
/// `session_uid`: one of root (display manager) or of that account
/// (rootless Xorg, Xwayland).
pub fn server_allowed(peer_uid: u32, session_uid: u32) -> bool {
    peer_uid == 0 || peer_uid == session_uid
}

fn word(file: &[u8], at: &mut usize) -> Option<u16> {
    let bytes = file.get(*at..at.checked_add(2)?)?;
    *at += 2;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn field<'a>(file: &'a [u8], at: &mut usize) -> Option<&'a [u8]> {
    let length = usize::from(word(file, at)?);
    let value = file.get(*at..at.checked_add(length)?)?;
    *at += length;
    Some(value)
}

/// The cookie for display `number` of the local host `host` in an
/// Xauthority file (the rule of Xlib: the first entry of family `Local`
/// with this host, or `Wild`, whose number is empty or this one); `None`
/// without one or for a malformed file.
pub fn pick_cookie(file: &[u8], host: &[u8], number: u32) -> Option<Vec<u8>> {
    let wanted = number.to_string();
    let mut at = 0;
    while at < file.len() {
        let family = word(file, &mut at)?;
        let address = field(file, &mut at)?;
        let display = field(file, &mut at)?;
        let name = field(file, &mut at)?;
        let data = field(file, &mut at)?;
        let host_matches = family == FAMILY_WILD || (family == FAMILY_LOCAL && address == host);
        let number_matches = display.is_empty() || display == wanted.as_bytes();
        if host_matches && number_matches && name == COOKIE_NAME && !data.is_empty() {
            return Some(data.to_vec());
        }
    }
    None
}

/// A connection to the X server of this process's session.
pub fn connect() -> io::Result<(RustConnection, usize)> {
    let own = super::spawn::has_own_session(&|name| std::env::var(name).ok());
    let followed = if own { None } else { super::current().ok() };
    match route(own, followed.as_ref().map(|session| session.kind)) {
        Route::Own => RustConnection::connect(None).map_err(io::Error::other),
        Route::Followed => match &followed {
            Some(session) => connect_session(session),
            None => Err(io::Error::new(io::ErrorKind::NotConnected, "no session")),
        },
        Route::Refused(reason) => Err(io::Error::new(io::ErrorKind::NotConnected, reason)),
    }
}

fn session_value<'a>(session: &'a ActiveSession, name: &str) -> Option<&'a str> {
    session
        .env
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// Connects to the X server of the followed X11 session.
fn connect_session(session: &ActiveSession) -> io::Result<(RustConnection, usize)> {
    let refused = |reason: &str| io::Error::new(io::ErrorKind::PermissionDenied, reason);
    let display = session_value(session, "DISPLAY")
        .and_then(parse_display)
        .ok_or_else(|| refused("the followed session has no local display"))?;
    let uid = session.account.uid;
    let socket = format!("{SOCKET_DIR}/X{}", display.number);
    // The socket directory is writable by every account: a path that is no
    // socket of root or of the session's account (a link to another root
    // service, for instance) is refused before this process connects.
    let entry = std::fs::symlink_metadata(&socket)?;
    if !entry.file_type().is_socket() || !server_allowed(entry.uid(), uid) {
        return Err(refused(
            "the X socket is no socket of root or of the session's account",
        ));
    }
    let stream = UnixStream::connect(&socket)?;
    if !server_allowed(peer_uid(&stream)?, uid) {
        return Err(refused("the X server belongs to another account"));
    }
    let named = session_value(session, "XAUTHORITY");
    let cookie = match named {
        Some(path) => Some(read_xauthority(path, uid)?),
        None if session.account.home.starts_with('/') => {
            let path = format!("{}/.Xauthority", session.account.home.trim_end_matches('/'));
            match read_xauthority(&path, uid) {
                Ok(file) => Some(file),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            }
        }
        None => None,
    }
    .and_then(|file| pick_cookie(&file, &host_name(), display.number));
    let (stream, _) = DefaultStream::from_unix_stream(stream)?;
    let connection = match cookie {
        Some(data) => RustConnection::connect_to_stream_with_auth_info(
            stream,
            display.screen,
            COOKIE_NAME.to_vec(),
            data,
        ),
        None => RustConnection::connect_to_stream(stream, display.screen),
    }
    .map_err(io::Error::other)?;
    if display.screen >= connection.setup().roots.len() {
        return Err(refused("the X server has no such screen"));
    }
    Ok((connection, display.screen))
}

/// The Xauthority file at `path` when it is a regular file of `uid` (the
/// last component is not followed; the open file itself is checked).
fn read_xauthority(path: &str, uid: u32) -> io::Result<Vec<u8>> {
    if !path.starts_with('/') {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "XAUTHORITY is not absolute",
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "XAUTHORITY is not a file of the session's account",
        ));
    }
    let mut content = Vec::new();
    file.take(MAX_XAUTHORITY + 1).read_to_end(&mut content)?;
    if content.len() as u64 > MAX_XAUTHORITY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "XAUTHORITY is too large",
        ));
    }
    Ok(content)
}

/// The UID of the process at the other end of `stream`.
fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: credentials and length live for the call and length is the
    // real size of the buffer getsockopt fills.
    let code = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    if code != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(credentials.uid)
}

/// The host name Xauthority entries of family `Local` carry.
fn host_name() -> Vec<u8> {
    let mut buffer = [0u8; 256];
    // SAFETY: buffer lives for the call and its length is passed.
    let code = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    if code != 0 {
        return Vec::new();
    }
    let end = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());
    buffer[..end].to_vec()
}
