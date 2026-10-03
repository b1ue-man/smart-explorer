use std::io::{self, Read};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::handoff::stop_requested_for;
use super::ipc_host::ShareHost;
use super::ipc_storage::{
    clear_ipc_addr, clear_ipc_generation, load_or_create_token, write_ipc_addr,
    write_ipc_generation,
};
use super::line::MAX_IPC_LINE;
use super::platform;
use super::state::log;

#[path = "ipc_admission.rs"]
mod admission;

const PRE_AUTH_READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PRE_AUTH_CONNECTIONS: usize = 16;
/// Longest blocking wait for a client before the stop control is read again
/// (Linux/Android adapters; the Windows adapter keeps its 100 ms poll). An
/// in-process stop wakes the wait at once through `IpcListener::shutdown`.
const ACCEPT_WAIT: Duration = Duration::from_secs(5);

/// The accept loop of one worker generation. `shutdown` (also on drop) ends
/// it immediately instead of at the next wait timeout, and it never depends
/// on a stop control another start may already have consumed.
pub(crate) struct IpcListener {
    addr: SocketAddr,
    stopping: Arc<AtomicBool>,
}

impl IpcListener {
    pub(crate) fn shutdown(&self) {
        if !self.stopping.swap(true, Ordering::AcqRel) {
            platform::wake_ipc_listener(self.addr);
        }
    }
}

impl Drop for IpcListener {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(crate) fn start_listener(host: ShareHost) -> io::Result<IpcListener> {
    // The singleton is held before this function is called, so any address
    // left by a crashed/older instance is now safe for the new owner to clear.
    clear_publication();
    let token = load_or_create_token()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    write_ipc_generation(host.generation())?;
    if let Err(error) = write_ipc_addr(addr) {
        clear_ipc_generation();
        return Err(error);
    }
    let limiter = PreAuthLimiter::new(MAX_PRE_AUTH_CONNECTIONS);
    let stopping = Arc::new(AtomicBool::new(false));
    let loop_stopping = Arc::clone(&stopping);

    let spawned = std::thread::Builder::new()
        .name("daemon-ipc".into())
        .spawn(move || {
            log(&format!("background worker IPC listening on {addr}"));
            // A stop leaves this generation's publication in place. The next
            // singleton owner clears it before publishing, while a delayed
            // retiring listener must never erase a successor's address or
            // generation sidecar.
            let generation = host.generation().to_string();
            let admissions = std::cell::RefCell::new(admission::AdmissionQueue::default());
            accept_loop(
                &listener,
                ACCEPT_WAIT,
                &loop_stopping,
                &|| stop_requested_for(&generation),
                |stream, peer| admissions.borrow_mut().accept(stream, peer, &host, &token,
                    |stream| admit_client(stream, &host, &token, &limiter)),
                || admissions.borrow_mut().poll(&host, &token,
                    |stream| admit_client(stream, &host, &token, &limiter)),
            );
        });

    match spawned {
        Ok(_) => Ok(IpcListener { addr, stopping }),
        Err(error) => {
            clear_publication();
            Err(error)
        }
    }
}

/// Accept clients until `stopping` is set or `external_stop` (the stop
/// control) asks for it. Between clients the thread blocks in the platform
/// wait instead of polling.
fn accept_loop(
    listener: &TcpListener,
    wait: Duration,
    stopping: &AtomicBool,
    external_stop: &dyn Fn() -> bool,
    mut admit: impl FnMut(TcpStream, SocketAddr),
    mut poll_admission: impl FnMut() -> bool,
) {
    loop {
        if stopping.load(Ordering::Acquire) || external_stop() {
            return;
        }
        let pending = poll_admission();
        match listener.accept() {
            Ok((stream, peer)) => {
                // The connection that woke a shutdown is never served.
                if stopping.load(Ordering::Acquire) {
                    return;
                }
                admit(stream, peer);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                let wait = if pending { wait.min(admission::PRELUDE_WAIT) } else { wait };
                if let Err(error) = platform::wait_for_ipc_client(listener, wait) {
                    log(&format!("daemon IPC wait failed: {error}"));
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
            Err(error) => {
                log(&format!("daemon IPC accept failed: {error}"));
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

fn admit_client(
    stream: TcpStream,
    host: &ShareHost,
    token: &str,
    limiter: &Arc<PreAuthLimiter>,
) {
    let Some(permit) = limiter.try_acquire() else {
        return;
    };
    let deadline = Instant::now() + PRE_AUTH_READ_TIMEOUT;
    if let Err(error) = prepare_ipc_client_stream(&stream) {
        log(&format!("daemon IPC client setup failed: {error}"));
        return;
    }
    let host = host.clone();
    let token = token.to_string();
    let spawned = std::thread::Builder::new()
        .name("daemon-ipc-client".into())
        .spawn(move || {
            if let Err(error) = super::ipc::handle_client(stream, host, &token, permit, deadline) {
                log(&format!("daemon IPC client error: {error}"));
            }
        });
    if let Err(error) = spawned {
        log(&format!("daemon IPC client spawn failed: {error}"));
    }
}

fn clear_publication() {
    clear_ipc_addr();
    clear_ipc_generation();
}

pub(super) fn read_pre_auth_line(
    stream: &mut TcpStream,
    line: &mut String,
    deadline: Instant,
) -> io::Result<usize> {
    line.clear();
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let remaining = remaining_until(deadline, Instant::now())?;
        stream.set_read_timeout(Some(remaining))?;
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(n) => {
                if bytes.len().saturating_add(n) > MAX_IPC_LINE {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "ipc line too large",
                    ));
                }
                bytes.extend_from_slice(&byte[..n]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(deadline_elapsed());
            }
            Err(error) => return Err(error),
        }
    }
    let count = bytes.len();
    *line = String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "ipc line invalid utf8"))?;
    Ok(count)
}

pub(super) fn clear_pre_auth_deadline(stream: &TcpStream) -> io::Result<()> {
    stream.set_read_timeout(None)
}

fn prepare_ipc_client_stream(stream: &TcpStream) -> io::Result<()> {
    stream.set_nonblocking(false)
}

fn remaining_until(deadline: Instant, now: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(now)
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(deadline_elapsed)
}

fn deadline_elapsed() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "daemon IPC authentication deadline elapsed",
    )
}

struct PreAuthLimiter {
    active: AtomicUsize,
    limit: usize,
}

impl PreAuthLimiter {
    fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            active: AtomicUsize::new(0),
            limit,
        })
    }

    fn try_acquire(self: &Arc<Self>) -> Option<PreAuthPermit> {
        let mut active = self.active.load(Ordering::Acquire);
        loop {
            if active >= self.limit {
                return None;
            }
            match self.active.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Some(PreAuthPermit {
                        limiter: Arc::clone(self),
                    });
                }
                Err(observed) => active = observed,
            }
        }
    }
}

pub(super) struct PreAuthPermit {
    limiter: Arc<PreAuthLimiter>,
}

impl Drop for PreAuthPermit {
    fn drop(&mut self) {
        self.limiter.active.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
#[path = "ipc_listener_tests.rs"]
mod tests;
