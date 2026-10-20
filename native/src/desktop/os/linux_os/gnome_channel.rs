//! The socket the GNOME surfaces connect to (U34-13): the extension starts
//! the surfaces, so they are no child processes of the shell and reach it
//! over `desktop.sock` instead of standard input and output. A connection is
//! accepted only from the same user (`SO_PEERCRED`) with a first frame that is
//! the greeting `{protocol 1, monitor "gnome-<index>", token}` and carries the
//! content of `desktop.token` (compared in constant time). Anything else is
//! closed, as is a connection that sends no greeting within five seconds (a
//! program that only connects must not hold a thread).
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use super::gnome_control::RuntimeFiles;
use crate::desktop::core::protocol::{check_hello, read_frame, ShellMessage, SurfaceMessage};
use crate::desktop::os::surface_link::{surface_streams, FrameSender};

/// How long a connection has for its greeting.
pub(crate) const HELLO_WAIT: Duration = Duration::from_secs(5);

/// A surface that passed the greeting.
#[derive(Debug)]
pub(crate) struct Accepted {
    pub(crate) stream: UnixStream,
    /// The process behind the socket (`SO_PEERCRED`).
    pub(crate) pid: u32,
    /// The monitor index of `gnome-<index>`.
    pub(crate) index: u32,
}

#[derive(Debug)]
pub(crate) enum ListenerEvent {
    Accepted(Accepted),
    /// A connection that was closed, with the reason (`surface_hello_rejected`).
    Rejected(String),
}

/// `(uid, pid)` of the process at the other end.
pub(crate) type PeerFn = dyn Fn(&UnixStream) -> io::Result<(u32, u32)> + Send + Sync;

/// The credentials of the peer of a Unix socket (`SO_PEERCRED`).
pub(crate) fn peer_credentials(stream: &UnixStream) -> io::Result<(u32, u32)> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `credentials` and `length` are valid for the call and `length`
    // is the size of the structure the kernel fills in.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::addr_of_mut!(credentials).cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((credentials.uid, u32::try_from(credentials.pid).unwrap_or(0)))
}

/// The listener of the shell.
pub(crate) struct GnomeListener {
    events: Receiver<ListenerEvent>,
    socket: PathBuf,
}

impl GnomeListener {
    /// Listens on `desktop.sock` in the runtime folder `files`.
    pub(crate) fn bind(files: &RuntimeFiles) -> io::Result<GnomeListener> {
        GnomeListener::bind_with(files, Arc::new(peer_credentials), HELLO_WAIT)
    }

    /// Like [`GnomeListener::bind`] with the peer check and the wait for the
    /// greeting given (the suite's stand-ins).
    pub(crate) fn bind_with(
        files: &RuntimeFiles,
        peer: Arc<PeerFn>,
        hello_wait: Duration,
    ) -> io::Result<GnomeListener> {
        let socket = files.socket_path();
        // A socket of an earlier shell is no longer served.
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket)?;
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        }
        let (sender, events) = mpsc::channel();
        let files = files.clone();
        std::thread::Builder::new()
            .name("gnome-accept".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let (files, peer, sender) = (files.clone(), peer.clone(), sender.clone());
                    let spawned = std::thread::Builder::new()
                        .name("gnome-hello".into())
                        .spawn(move || greet(stream, &files, &*peer, hello_wait, &sender));
                    if spawned.is_err() {
                        return;
                    }
                }
            })?;
        Ok(GnomeListener { events, socket })
    }

    pub(crate) fn socket(&self) -> &Path {
        &self.socket
    }

    /// The connections and refusals since the last call.
    pub(crate) fn drain(&self) -> Vec<ListenerEvent> {
        self.events.try_iter().collect()
    }
}

fn own_uid() -> u32 {
    // SAFETY: `getuid` has no arguments and cannot fail.
    unsafe { libc::getuid() }
}

/// The index of the greeting's monitor `gnome-<index>`.
fn index_of(monitor: &str) -> Option<u32> {
    monitor.strip_prefix("gnome-")?.parse().ok()
}

fn greet(
    mut stream: UnixStream,
    files: &RuntimeFiles,
    peer: &PeerFn,
    hello_wait: Duration,
    events: &Sender<ListenerEvent>,
) {
    let reject = |why: String| {
        let _ = events.send(ListenerEvent::Rejected(why));
    };
    let (uid, pid) = match peer(&stream) {
        Ok(credentials) => credentials,
        Err(error) => return reject(format!("Gegenstelle unbekannt: {error}")),
    };
    if uid != own_uid() {
        return reject(format!("fremder Benutzer {uid}"));
    }
    let Some(token) = files.read_token() else {
        return reject("desktop.token fehlt".into());
    };
    if stream.set_read_timeout(Some(hello_wait)).is_err() {
        return reject("Lesefrist nicht setzbar".into());
    }
    let hello = match read_frame::<_, SurfaceMessage>(&mut stream) {
        Ok(Some(hello)) => hello,
        Ok(None) => return reject("Verbindung ohne Begrüßung beendet".into()),
        Err(error) => return reject(format!("keine Begrüßung: {error}")),
    };
    if let Err(error) = check_hello(&hello, None, Some(&token)) {
        return reject(error.to_string());
    }
    let SurfaceMessage::Hello { monitor, .. } = &hello else {
        return reject("erste Nachricht ist keine Begrüßung".into());
    };
    let Some(index) = index_of(monitor) else {
        return reject(format!("Monitor {monitor:?} ist kein GNOME-Monitor"));
    };
    if stream.set_read_timeout(None).is_err() {
        return reject("Lesefrist nicht löschbar".into());
    }
    let _ = events.send(ListenerEvent::Accepted(Accepted { stream, pid, index }));
}

/// The surface's end (U34-13): connects to `desktop.sock` of the shell. The
/// greeting with the token is the surface's first frame, sent by the caller;
/// `stop` and the end of the connection end the process like the standard
/// streams do (`surface_link::surface_streams`).
pub(crate) fn surface_socket<W>(
    path: &Path,
    token: &str,
    wake: W,
) -> io::Result<(FrameSender<SurfaceMessage>, Receiver<ShellMessage>)>
where
    W: Fn() + Send + 'static,
{
    if token.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "kein Token für den Wächter",
        ));
    }
    let stream = UnixStream::connect(path)?;
    let reader = stream.try_clone()?;
    surface_streams(reader, stream, wake)
}
