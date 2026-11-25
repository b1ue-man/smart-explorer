//! The race of the direct TCP picture link (`video_tcp_link.rs`): every
//! listener accepts while one task per peer candidate dials it from the
//! listener's port in rounds (`PUNCH_WINDOW`, `ATTEMPT_TIMEOUT`,
//! `RETRY_PAUSE`). Every TCP connection, accepted or dialed, runs TLS in the
//! screen role (host = TLS server); the host also reads the viewer's token
//! there. The first connection that is ready wins: the host confirms the
//! token, the viewer presents it and waits for the confirmation. Everything
//! else is closed when the race ends.
//!
//! The viewer nominates one connection at a time: it writes the token only
//! on the connection it is ready with and closes the others, so both sides
//! end on the same connection. A host connection the viewer closed without
//! a token counts as nothing; a wrong token is reported.
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use rustls::{ClientConfig, ServerConfig};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinSet;
use tokio::time::{sleep, sleep_until, timeout_at, Instant};

use super::records::{self, LinkError, TlsLink};
use super::sockets::{dial_socket, LocalSockets};
use super::tls;
use super::{
    TcpLinkFailure, TcpLinkInfo, TcpPictureLink, ATTEMPT_TIMEOUT, PUNCH_WINDOW, RETRY_PAUSE,
};
use crate::screen::core::video_tcp_wire::{
    classify_path, TcpCandidate, TcpFailureReason, TcpToken, MAX_TCP_CANDIDATES,
};

/// Connections in TLS at the same time; a further one is closed at once (a
/// public listener takes connections from anyone during the race).
const MAX_PENDING_CONNECTIONS: usize = 32;
const EVENT_QUEUE: usize = 64;
/// Pause after a failed accept (e.g. no descriptor left).
const ACCEPT_PAUSE: Duration = Duration::from_millis(20);

pub(super) enum Role {
    Host {
        config: Arc<ServerConfig>,
        token: TcpToken,
    },
    Viewer {
        config: Arc<ClientConfig>,
        token: TcpToken,
    },
}

impl Role {
    fn connection(&self) -> io::Result<rustls::Connection> {
        match self {
            Self::Host { config, .. } => tls::server_connection(config),
            Self::Viewer { config, .. } => tls::client_connection(config),
        }
    }
}

enum Event {
    /// A TCP connection with a peer address came up.
    Up,
    /// Viewer: TLS passed; host: TLS and the token passed.
    Ready {
        link: TlsLink,
        dialed: bool,
    },
    /// TLS with a peer address failed.
    Tls(String),
    WrongToken,
    /// A candidate's dial rounds ended without a connection.
    DialGaveUp(String),
}

#[derive(Clone)]
struct Context {
    role: Arc<Role>,
    peer_ips: Arc<Vec<IpAddr>>,
    events: mpsc::Sender<Event>,
    gate: Arc<Semaphore>,
    deadline: Instant,
}

#[derive(Default)]
struct Tally {
    up: usize,
    tls: Option<String>,
    wrong_token: usize,
    refused: Option<String>,
    dial_errors: usize,
    last_dial_error: Option<String>,
}

impl Tally {
    /// The reason, most telling first: a wrong token, failed TLS, a refusal,
    /// a connection that did not finish in time, none at all.
    fn failure(self) -> TcpLinkFailure {
        let (reason, detail) = if self.wrong_token > 0 {
            let detail = format!("{} Verbindung(en)", self.wrong_token);
            (TcpFailureReason::WrongToken, Some(detail))
        } else if let Some(error) = self.tls {
            (TcpFailureReason::Tls, Some(error))
        } else if let Some(detail) = self.refused {
            (TcpFailureReason::Refused, Some(detail))
        } else if self.up > 0 {
            (TcpFailureReason::Timeout, None)
        } else {
            let last = self.last_dial_error.unwrap_or_else(|| "keine".to_string());
            let detail = format!("{} Adressen gewählt, zuletzt: {last}", self.dial_errors);
            (TcpFailureReason::Unreachable, Some(detail))
        };
        TcpLinkFailure::new(reason, detail)
    }
}

fn canonical(addr: SocketAddr) -> SocketAddr {
    SocketAddr::new(addr.ip().to_canonical(), addr.port())
}

/// Runs the race until a link is up or `deadline` passed.
pub(super) async fn race(
    sockets: LocalSockets,
    peer: &[TcpCandidate],
    role: Role,
    deadline: std::time::Instant,
) -> Result<TcpPictureLink, TcpLinkFailure> {
    let deadline = Instant::from_std(deadline);
    let dial_end = (Instant::now() + PUNCH_WINDOW).min(deadline);
    let LocalSockets {
        v4,
        v6,
        candidates: own,
        reflector,
        ..
    } = sockets;
    // Never dial an own candidate: on a shared address (a container bridge
    // on both devices) that would be a connection to this side itself.
    let targets: Vec<SocketAddr> = peer
        .iter()
        .take(MAX_TCP_CANDIDATES)
        .map(|candidate| canonical(candidate.addr))
        .filter(|addr| !own.iter().any(|candidate| candidate.addr == *addr))
        .collect();
    let (events, mut inbox) = mpsc::channel(EVENT_QUEUE);
    let context = Context {
        role: Arc::new(role),
        peer_ips: Arc::new(targets.iter().map(|addr| addr.ip()).collect()),
        events,
        gate: Arc::new(Semaphore::new(MAX_PENDING_CONNECTIONS)),
        deadline,
    };
    let mut tasks = JoinSet::new();
    let (mut from_v4, mut from_v6) = (None, None);
    for (bound, from) in [(v4, &mut from_v4), (v6, &mut from_v6)] {
        if let Some(bound) = bound {
            *from = Some(bound.addr);
            tasks.spawn(accept(bound.listener, context.clone()));
        }
    }
    for target in targets {
        let from = if target.is_ipv4() { from_v4 } else { from_v6 };
        if let Some(from) = from {
            tasks.spawn(dial(from, target, dial_end, context.clone()));
        }
    }
    let role = context.role.clone();
    drop(context);
    let mut tally = Tally::default();
    let outcome = loop {
        let event = match timeout_at(deadline, inbox.recv()).await {
            Ok(Some(event)) => event,
            Ok(None) | Err(_) => break Err(tally.failure()),
        };
        match event {
            Event::Up => tally.up += 1,
            Event::Tls(error) => {
                tally.tls.get_or_insert(error);
            }
            Event::WrongToken => tally.wrong_token += 1,
            Event::DialGaveUp(error) => {
                tally.dial_errors += 1;
                tally.last_dial_error = Some(error);
            }
            Event::Ready { mut link, dialed } => {
                let finished = finish(&role, &mut link, deadline).await;
                let refusal = match finished {
                    Ok(()) => break Ok(picture_link(link, dialed)),
                    // The connection came up (`Up`): a timeout stays one.
                    Err(LinkError::Timeout) => None,
                    Err(LinkError::Closed) => Some("ohne Bestätigung geschlossen".to_string()),
                    Err(LinkError::Io(error)) => Some(error.to_string()),
                    Err(LinkError::Tls(error)) => Some(error.to_string()),
                };
                // Only the viewer is refused; the host's confirmation that
                // did not arrive leaves it waiting for the viewer's next try.
                if matches!(*role, Role::Viewer { .. }) && refusal.is_some() {
                    tally.refused = refusal;
                }
            }
        }
    };
    tasks.abort_all();
    drop(reflector);
    outcome
}

/// The last step on the winning connection: the host confirms the token,
/// the viewer presents it.
async fn finish(role: &Role, link: &mut TlsLink, deadline: Instant) -> Result<(), LinkError> {
    match role {
        Role::Host { .. } => records::confirm(&mut link.writer, deadline).await,
        Role::Viewer { token, .. } => records::present_token(link, token, deadline).await,
    }
}

fn picture_link(link: TlsLink, dialed: bool) -> TcpPictureLink {
    let (local, remote) = (canonical(link.local), canonical(link.remote));
    TcpPictureLink {
        info: TcpLinkInfo {
            local,
            remote,
            path: classify_path(local, remote),
            dialed,
        },
        reader: link.reader,
        writer: link.writer,
    }
}

async fn accept(listener: TcpListener, context: Context) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    if let Ok(permit) = context.gate.clone().try_acquire_owned() {
                        let context = context.clone();
                        connections.spawn(async move {
                            let _permit = permit;
                            handle(stream, false, &context).await;
                        });
                    }
                }
                Err(_) => sleep(ACCEPT_PAUSE).await,
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}

async fn dial(from: SocketAddr, target: SocketAddr, dial_end: Instant, context: Context) {
    let mut last_error = format!("{target}: kein Versuch");
    while Instant::now() < dial_end {
        let socket = match dial_socket(from) {
            Ok(socket) => socket,
            Err(error) => {
                last_error = format!("{target}: {error}");
                break;
            }
        };
        let attempt_end = (Instant::now() + ATTEMPT_TIMEOUT).min(dial_end);
        match timeout_at(attempt_end, socket.connect(target)).await {
            Ok(Ok(stream)) => {
                if let Ok(_permit) = context.gate.clone().try_acquire_owned() {
                    handle(stream, true, &context).await;
                }
                return;
            }
            Ok(Err(error)) => {
                // The four-tuple is taken: the peer's SYN reached the
                // listener first, that connection runs already.
                if matches!(
                    error.kind(),
                    io::ErrorKind::AddrInUse | io::ErrorKind::AddrNotAvailable
                ) {
                    return;
                }
                last_error = format!("{target}: {error}");
                sleep_until((Instant::now() + RETRY_PAUSE).min(dial_end)).await;
            }
            Err(_) => last_error = format!("{target}: keine Antwort"),
        }
    }
    let _ = context.events.send(Event::DialGaveUp(last_error)).await;
}

async fn handle(stream: TcpStream, dialed: bool, context: &Context) {
    let _ = stream.set_nodelay(true);
    let Ok(remote) = stream.peer_addr() else {
        return;
    };
    let from_peer = context.peer_ips.contains(&remote.ip().to_canonical());
    if from_peer {
        let _ = context.events.send(Event::Up).await;
    }
    let report = |error: String| async move {
        if from_peer {
            let _ = context.events.send(Event::Tls(error)).await;
        }
    };
    let conn = match context.role.connection() {
        Ok(conn) => conn,
        Err(error) => return report(error.to_string()).await,
    };
    let mut link = match records::handshake(stream, conn, context.deadline).await {
        Ok(link) => link,
        Err(LinkError::Tls(error)) => return report(error.to_string()).await,
        Err(_) => return,
    };
    let event = match &*context.role {
        Role::Viewer { .. } => Event::Ready { link, dialed },
        Role::Host { token, .. } => {
            let received = records::read_token(&mut link.reader, context.deadline).await;
            match received {
                Ok(received) if tls::token_matches(token, &received) => {
                    Event::Ready { link, dialed }
                }
                Ok(_) => Event::WrongToken,
                // The viewer closed a connection it did not choose.
                Err(_) => return,
            }
        }
    };
    let _ = context.events.send(event).await;
}
