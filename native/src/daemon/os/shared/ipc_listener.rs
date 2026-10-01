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
            accept_loop(
                &listener,
                ACCEPT_WAIT,
                &loop_stopping,
                &|| stop_requested_for(&generation),
                |stream, peer| admit_client(stream, peer, &host, &token, &limiter),
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
) {
    loop {
        if stopping.load(Ordering::Acquire) || external_stop() {
            return;
        }
        match listener.accept() {
            Ok((stream, peer)) => {
                // The connection that woke a shutdown is never served.
                if stopping.load(Ordering::Acquire) {
                    return;
                }
                admit(stream, peer);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
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
    peer: SocketAddr,
    host: &ShareHost,
    token: &str,
    limiter: &Arc<PreAuthLimiter>,
) {
    if !peer.ip().is_loopback() {
        return;
    }
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
mod tests {
    use super::{prepare_ipc_client_stream, read_pre_auth_line, remaining_until, PreAuthLimiter};
    use std::io::{self, Read};
    use std::net::{TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn spawn_accept_loop(
        wait: Duration,
        external: std::sync::Arc<std::sync::atomic::AtomicBool>,
        checks: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> (
        super::IpcListener,
        std::sync::mpsc::Receiver<std::net::SocketAddr>,
        std::thread::JoinHandle<()>,
    ) {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let stopping = Arc::new(AtomicBool::new(false));
        let loop_stopping = Arc::clone(&stopping);
        let (sender, receiver) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let external_stop = || {
                checks.fetch_add(1, Ordering::SeqCst);
                external.load(Ordering::SeqCst)
            };
            super::accept_loop(
                &listener,
                wait,
                &loop_stopping,
                &external_stop,
                |stream, peer| {
                    drop(stream);
                    let _ = sender.send(peer);
                },
            );
        });
        (super::IpcListener { addr, stopping }, receiver, thread)
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn android_background_task_ipc_listener_blocks_and_stops_on_shutdown() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;

        let checks = Arc::new(AtomicUsize::new(0));
        let external = Arc::new(AtomicBool::new(false));
        let (handle, admitted, thread) =
            spawn_accept_loop(Duration::from_secs(5), external, Arc::clone(&checks));

        let client = TcpStream::connect(handle.addr).unwrap();
        let peer = admitted.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(peer, client.local_addr().unwrap());

        // Idle: the thread blocks in the wait instead of a 100 ms poll.
        std::thread::sleep(Duration::from_millis(100));
        let before = checks.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(700));
        let idle_checks = checks.load(Ordering::SeqCst) - before;
        assert!(
            idle_checks <= 1,
            "listener woke {idle_checks} times while idle"
        );

        // Shutdown wakes the blocking wait at once; the wake is not served.
        let started = Instant::now();
        drop(handle);
        thread.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(admitted.try_recv().is_err());
        drop(client);
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn android_background_task_ipc_listener_honours_stop_control_within_wait() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;

        let external = Arc::new(AtomicBool::new(false));
        let (handle, admitted, thread) = spawn_accept_loop(
            Duration::from_millis(200),
            Arc::clone(&external),
            Arc::new(AtomicUsize::new(0)),
        );
        // A stop control written by another process (or a handoff to a newer
        // generation) is seen at the next wake without any connection.
        let started = Instant::now();
        external.store(true, Ordering::SeqCst);
        let (done_sender, done) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            thread.join().unwrap();
            let _ = done_sender.send(());
        });
        done.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(admitted.try_recv().is_err());
        // The loop already ended; dropping the handle only wakes a closed port.
        drop(handle);
    }

    #[test]
    fn pre_auth_limit_releases_capacity_with_the_permit() {
        let limiter = PreAuthLimiter::new(2);
        let first = limiter.try_acquire().unwrap();
        let second = limiter.try_acquire().unwrap();
        assert!(limiter.try_acquire().is_none());

        drop(first);
        let replacement = limiter.try_acquire().unwrap();
        assert!(limiter.try_acquire().is_none());

        drop(second);
        drop(replacement);
        assert!(limiter.try_acquire().is_some());
    }

    #[test]
    fn absolute_deadline_never_refreshes_after_expiry() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(5);
        assert_eq!(
            remaining_until(deadline, now).unwrap(),
            Duration::from_secs(5)
        );
        assert_eq!(
            remaining_until(deadline, deadline).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(
            remaining_until(deadline, deadline + Duration::from_millis(1))
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn incomplete_pre_auth_read_times_out_and_releases_capacity() {
        let limiter = PreAuthLimiter::new(1);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server, _) = listener.accept().unwrap();

        let error = {
            let _permit = limiter.try_acquire().unwrap();
            assert!(limiter.try_acquire().is_none());
            let mut line = String::new();
            read_pre_auth_line(
                &mut server,
                &mut line,
                Instant::now() + Duration::from_millis(80),
            )
            .unwrap_err()
        };

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(limiter.try_acquire().is_some());
        drop(client);
    }

    #[test]
    fn accepted_ipc_stream_is_forced_back_to_blocking() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (mut server, _) = loop {
            match listener.accept() {
                Ok(pair) => break pair,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        };
        server.set_nonblocking(true).unwrap();
        prepare_ipc_client_stream(&server).unwrap();
        server
            .set_read_timeout(Some(Duration::from_millis(120)))
            .unwrap();

        let started = Instant::now();
        let mut one = [0u8; 1];
        let error = server.read(&mut one).unwrap_err();
        assert!(matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ));
        assert!(
            started.elapsed() >= Duration::from_millis(40),
            "read returned immediately; stream is still nonblocking"
        );
        drop(client);
    }
}
