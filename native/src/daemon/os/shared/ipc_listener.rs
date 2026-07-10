use std::io::{self, Read};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::ipc_host::ShareHost;
use super::ipc_storage::{clear_ipc_addr, load_or_create_token, write_ipc_addr};
use super::line::MAX_IPC_LINE;
use super::state::{log, stop_requested};

const PRE_AUTH_READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PRE_AUTH_CONNECTIONS: usize = 16;

pub(crate) fn start_listener(host: ShareHost) -> io::Result<()> {
    let token = load_or_create_token()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    write_ipc_addr(addr)?;
    let limiter = PreAuthLimiter::new(MAX_PRE_AUTH_CONNECTIONS);

    let spawned = std::thread::Builder::new()
        .name("daemon-ipc".into())
        .spawn(move || {
            log(&format!("background worker IPC listening on {addr}"));
            loop {
                if stop_requested() {
                    clear_ipc_addr();
                    return;
                }
                match listener.accept() {
                    Ok((stream, peer)) => {
                        if !peer.ip().is_loopback() {
                            continue;
                        }
                        let Some(permit) = limiter.try_acquire() else {
                            continue;
                        };
                        let deadline = Instant::now() + PRE_AUTH_READ_TIMEOUT;
                        if let Err(error) = prepare_ipc_client_stream(&stream) {
                            log(&format!("daemon IPC client setup failed: {error}"));
                            continue;
                        }
                        let host = host.clone();
                        let token = token.clone();
                        let spawned = std::thread::Builder::new()
                            .name("daemon-ipc-client".into())
                            .spawn(move || {
                                if let Err(error) = super::ipc::handle_client(
                                    stream, host, &token, permit, deadline,
                                ) {
                                    log(&format!("daemon IPC client error: {error}"));
                                }
                            });
                        if let Err(error) = spawned {
                            log(&format!("daemon IPC client spawn failed: {error}"));
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(error) => {
                        log(&format!("daemon IPC accept failed: {error}"));
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
            }
        });

    match spawned {
        Ok(_) => Ok(()),
        Err(error) => {
            clear_ipc_addr();
            Err(error)
        }
    }
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
