//! The channel of the worker IPC for one window of a followed session (E46,
//! S13). A worker without session variables (a Linux root service, E23)
//! starts the indicator and the consent windows as the session's account,
//! which cannot read the daemon token. For each such window the worker opens
//! a loopback listener of its own with a fresh token (256 bits from the
//! operating system); the window gets the address in
//! `SE_WORKER_IPC_CHANNEL` and the token as the first line of its standard
//! input, never in its environment or a file
//! (`consent_launch::spawn_window_with_channel`).
//!
//! The port is the window's own because the worker proves its token before
//! a client sends one (`ipc_proof.rs`): the proof is keyed by the token the
//! client holds, and only a listener of exactly one window knows which token
//! that is. A connection here waits in the prelude of the worker IPC
//! (`ipc_admission.rs`, `Authority::Window`), which answers the proof with
//! the window token and refuses a wrong token, so a connection without the
//! token holds neither a client thread nor one of the window's permits. An
//! admitted connection passes its one request only through
//! [`Channel::admit`], the one place that checks the window's scope:
//! - every window: `Ping`;
//! - the indicator: `Screen` `HostSessions`, `HostWatch`, `HostStop`;
//! - a consent window: `Access` `Approvals` (answered with the requests of
//!   its device only) and `Decide` for an open request of its device.
//!
//! Anything else, a wrong token, an ended window and a window of a session
//! that is no longer the active one get the answer of a wrong token. An
//! admitted request goes to the worker's own IPC as an ordinary client
//! request with the daemon token, so every check of that path applies. The
//! token ends when the window's process ended (the launcher drops the guard)
//! and when another session became the active one; then the window is ended
//! too, and its launcher starts the next one in the new session.
use std::io;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::daemon::ipc_proof;
use crate::daemon::ipc_protocol::screen::{ScreenIpcRequest, WATCH_WAIT_SECS};
use crate::daemon::ipc_protocol::{
    read_response, set_stream_timeout, write_request, write_response, AccessIpcReply,
    AccessIpcRequest, IpcRequest, IpcResponse,
};
use crate::share::access_approval::ApprovalDecision;
use crate::share::consent_launch::{WindowChannel, WindowChannelGuard, WindowScope, WindowSession};

#[cfg(all(test, target_os = "linux"))]
#[path = "bildschirm_kanal_task_tests.rs"]
mod tests;

/// Clients of one window served at the same time (list, watch, end, answer).
const MAX_CONNECTIONS: usize = 8;
/// Time for the rest of the request line of an admitted connection.
const PRE_AUTH: Duration = Duration::from_secs(5);
/// Longest blocking wait of the accept loop; dropping the guard wakes it.
const ACCEPT_WAIT: Duration = Duration::from_secs(5);
/// Connection to the worker's own IPC.
const CONNECT: Duration = Duration::from_secs(2);
/// Answer time of the worker for list, end, ping and answers.
const ANSWER: Duration = Duration::from_secs(20);

/// The worker's own IPC that admitted requests go to.
#[derive(Clone)]
struct Main {
    addr: SocketAddr,
    token: String,
}

static MAIN: Mutex<Option<Main>> = Mutex::new(None);

/// Called by `start_listener` once the worker IPC listens at `addr`.
pub(super) fn install(addr: SocketAddr, token: String) {
    *MAIN.lock().unwrap_or_else(PoisonError::into_inner) = Some(Main { addr, token });
    crate::share::consent_launch::install_window_channels(open);
}

fn open(scope: WindowScope, session: WindowSession) -> io::Result<WindowChannel> {
    let main = MAIN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "worker IPC not listening"))?;
    open_with(scope, session, main, device_of_open)
}

/// The device of the open request `id` of this worker.
fn device_of_open(id: &str) -> Option<String> {
    crate::share::consent_launch::approvals()
        .pending()
        .into_iter()
        .find(|view| view.id.as_str() == id)
        .map(|view| view.device_id)
}

fn open_with(
    scope: WindowScope,
    session: WindowSession,
    main: Main,
    device_of: fn(&str) -> Option<String>,
) -> io::Result<WindowChannel> {
    let token = new_token()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    let channel = Arc::new(Channel {
        token: token.clone(),
        scope,
        session,
        main,
        device_of,
        revoked: AtomicBool::new(false),
        switched: AtomicBool::new(false),
        window: Mutex::new(None),
    });
    let stopping = Arc::new(AtomicBool::new(false));
    let limiter = super::PreAuthLimiter::new(MAX_CONNECTIONS);
    let (served, loop_stopping) = (Arc::clone(&channel), Arc::clone(&stopping));
    std::thread::Builder::new()
        .name("worker-window-channel".into())
        .spawn(move || {
            let admissions = std::cell::RefCell::new(super::admission::AdmissionQueue::default());
            let window = super::admission::Authority::Window;
            super::accept_loop(
                &listener,
                ACCEPT_WAIT,
                &loop_stopping,
                &|| served.revoked.load(Ordering::Acquire),
                |stream, peer| {
                    admissions
                        .borrow_mut()
                        .accept(stream, peer, window, &served.token, |stream| {
                            client(&served, &limiter, stream)
                        })
                },
                || {
                    admissions
                        .borrow_mut()
                        .poll(window, &served.token, |stream| {
                            client(&served, &limiter, stream)
                        })
                },
            );
        })?;
    Ok(WindowChannel {
        addr,
        token,
        guard: Box::new(Guard {
            channel,
            _listener: super::IpcListener { addr, stopping },
        }),
    })
}

fn new_token() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Serves a connection the prelude admitted: it showed the window token.
fn client(channel: &Arc<Channel>, limiter: &Arc<super::PreAuthLimiter>, stream: TcpStream) {
    let Some(permit) = limiter.try_acquire() else {
        return;
    };
    let channel = Arc::clone(channel);
    let spawned = std::thread::Builder::new()
        .name("worker-window-client".into())
        .spawn(move || {
            let _permit = permit;
            if let Err(error) = channel.serve(stream) {
                super::log(&format!("window channel client: {error}"));
            }
        });
    if let Err(error) = spawned {
        super::log(&format!("window channel client not started: {error}"));
    }
}

/// What a window may have the worker do.
enum Admitted {
    Ping,
    Screen(ScreenIpcRequest),
    /// The open requests of `device` only.
    Approvals {
        device: String,
    },
    Decide {
        approval_id: String,
        decision: ApprovalDecision,
    },
}

/// The scope of a window (the only scope rule of the channel).
fn in_scope(
    scope: &WindowScope,
    request: IpcRequest,
    device_of: fn(&str) -> Option<String>,
) -> Option<Admitted> {
    match (scope, request) {
        (_, IpcRequest::Ping { .. }) => Some(Admitted::Ping),
        (WindowScope::Indicator, IpcRequest::Screen { req, .. }) => match req {
            ScreenIpcRequest::HostSessions
            | ScreenIpcRequest::HostWatch { .. }
            | ScreenIpcRequest::HostStop { .. } => Some(Admitted::Screen(req)),
            _ => None,
        },
        (WindowScope::Consent { device }, IpcRequest::Access { request, .. }) => match request {
            AccessIpcRequest::Approvals => Some(Admitted::Approvals {
                device: device.clone(),
            }),
            AccessIpcRequest::Decide {
                approval_id,
                decision,
            } if device_of(&approval_id).as_deref() == Some(device.as_str()) => {
                Some(Admitted::Decide {
                    approval_id,
                    decision,
                })
            }
            _ => None,
        },
        _ => None,
    }
}

struct Channel {
    token: String,
    scope: WindowScope,
    session: WindowSession,
    main: Main,
    device_of: fn(&str) -> Option<String>,
    revoked: AtomicBool,
    /// Another session became the active one (the reason of `revoked`).
    switched: AtomicBool,
    /// Ends the window (set once it runs).
    window: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl Channel {
    fn serve(&self, mut stream: TcpStream) -> io::Result<()> {
        super::prepare_ipc_client_stream(&stream)?;
        let deadline = Instant::now() + PRE_AUTH;
        let mut line = String::new();
        super::read_pre_auth_line(&mut stream, &mut line, deadline)?;
        super::clear_pre_auth_deadline(&stream)?;
        stream.set_write_timeout(Some(ANSWER))?;
        let admitted = serde_json::from_str::<IpcRequest>(line.trim_end())
            .ok()
            .and_then(|request| self.admit(request));
        match admitted {
            Some(admitted) => self.relay(admitted, &mut stream),
            None => refuse(&mut stream),
        }
    }

    /// The token and scope check of every request of this window.
    fn admit(&self, request: IpcRequest) -> Option<Admitted> {
        if self.revoked.load(Ordering::Acquire) {
            return None;
        }
        let token = request.daemon_token()?;
        if !super::admission::constant_time_eq(&self.token, token) || !self.session_current() {
            return None;
        }
        in_scope(&self.scope, request, self.device_of)
    }

    /// Another active session ends the token and the window; an unknown
    /// state only refuses this request.
    fn session_current(&self) -> bool {
        match (self.session.current)() {
            Some(active) if active == self.session.generation => true,
            Some(_) => {
                self.switched.store(true, Ordering::Release);
                self.revoke();
                let end = lock(&self.window).take();
                if let Some(end) = end {
                    end();
                }
                false
            }
            None => false,
        }
    }

    fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }

    /// One admitted request to the worker's own IPC with the daemon token.
    fn relay(&self, admitted: Admitted, window: &mut TcpStream) -> io::Result<()> {
        let token = self.main.token.clone();
        let mut only = None;
        let (request, wait) = match admitted {
            Admitted::Ping => (IpcRequest::Ping { token }, ANSWER),
            Admitted::Screen(req) => {
                let wait = match req {
                    ScreenIpcRequest::HostWatch { .. } => {
                        Duration::from_secs(WATCH_WAIT_SECS) + ANSWER
                    }
                    _ => ANSWER,
                };
                (IpcRequest::Screen { token, req }, wait)
            }
            Admitted::Approvals { device } => {
                only = Some(device);
                let request = AccessIpcRequest::Approvals;
                (IpcRequest::Access { token, request }, ANSWER)
            }
            Admitted::Decide {
                approval_id,
                decision,
            } => {
                let request = AccessIpcRequest::Decide {
                    approval_id,
                    decision,
                };
                (IpcRequest::Access { token, request }, ANSWER)
            }
        };
        let mut worker = ipc_proof::connect_token(&self.main.addr, CONNECT, &self.main.token)?;
        set_stream_timeout(&worker, Some(wait));
        write_request(&mut worker, &request)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let mut response =
            read_response(&mut worker).map_err(|error| io::Error::other(error.to_string()))?;
        if let (Some(device), IpcResponse::Access(AccessIpcReply::ApprovalList { pending })) =
            (only, &mut response)
        {
            pending.retain(|view| view.device_id == device);
        }
        write_response(window, &response)
    }
}

/// The answer of a wrong token (`ipc_admission.rs`), after the request line
/// was read completely.
fn refuse(stream: &mut TcpStream) -> io::Result<()> {
    write_response(
        stream,
        &IpcResponse::Err {
            msg: super::admission::REFUSED.into(),
            code: Some(super::super::ipc::AUTH_REJECTED_CODE.into()),
        },
    )?;
    stream.shutdown(Shutdown::Write)
}

fn lock<T>(slot: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    slot.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Held by the launcher while the window runs; dropping it ends the token
/// and the window's listener.
struct Guard {
    channel: Arc<Channel>,
    _listener: super::IpcListener,
}

impl WindowChannelGuard for Guard {
    fn window_started(&self, end: Box<dyn Fn() + Send + Sync>) {
        *lock(&self.channel.window) = Some(end);
    }

    fn session_switched(&self) -> bool {
        self.channel.switched.load(Ordering::Acquire)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.channel.revoke();
        *lock(&self.channel.window) = None;
    }
}
