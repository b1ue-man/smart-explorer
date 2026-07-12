use std::io;
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use suppaftp::types::FileType;
use suppaftp::{FtpError, RustlsConnector, RustlsFtpStream};

use super::resolver::resolve_host;

fn io_err<E: std::fmt::Display>(error: E) -> io::Error {
    io::Error::other(error.to_string())
}

fn ftp_err(error: FtpError) -> io::Error {
    match error {
        FtpError::ConnectionError(error) => error,
        error => io_err(error),
    }
}

#[derive(Clone)]
pub(super) struct FtpUrl {
    pub(super) secure: bool,
    pub(super) user: String,
    pub(super) password: String,
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) root: String,
}

fn decode_userinfo(value: &str) -> io::Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return Err(io_err(
                "unvollstaendige Prozentkodierung in FTP-Zugangsdaten",
            ));
        }
        let high = hex_nibble(bytes[index + 1])
            .ok_or_else(|| io_err("ungueltige Prozentkodierung in FTP-Zugangsdaten"))?;
        let low = hex_nibble(bytes[index + 2])
            .ok_or_else(|| io_err("ungueltige Prozentkodierung in FTP-Zugangsdaten"))?;
        decoded.push((high << 4) | low);
        index += 3;
    }
    String::from_utf8(decoded).map_err(|_| io_err("FTP-Zugangsdaten sind nicht gueltiges UTF-8"))
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(super) fn parse_ftp_url(url: &str) -> io::Result<FtpUrl> {
    let url = url.trim();
    let (secure, rest) = if let Some(rest) = url.strip_prefix("ftps://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("ftp://") {
        (false, rest)
    } else {
        return Err(io_err("kein ftp(s)://-URL"));
    };
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    let root = if path.is_empty() { "/" } else { path }.to_string();
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(index) => (Some(&authority[..index]), &authority[index + 1..]),
        None => (None, authority),
    };
    let (user, password) = match userinfo {
        Some(userinfo) => match userinfo.find(':') {
            Some(index) => (
                decode_userinfo(&userinfo[..index])?,
                decode_userinfo(&userinfo[index + 1..])?,
            ),
            None => (decode_userinfo(userinfo)?, String::new()),
        },
        None => ("anonymous".to_string(), "anonymous@example.com".to_string()),
    };
    let (host, port) = match hostport.rfind(':') {
        Some(index) => {
            let port = hostport[index + 1..]
                .parse::<u16>()
                .map_err(|_| io_err("ungültiger FTP-Port"))?;
            (hostport[..index].to_string(), port)
        }
        None => (hostport.to_string(), 21),
    };
    if host.is_empty() {
        return Err(io_err("FTP-Host fehlt"));
    }
    Ok(FtpUrl {
        secure,
        user,
        password,
        host,
        port,
        root,
    })
}

fn rustls_client_config() -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("ring provider supports default protocol versions")
    .with_root_certificates(roots)
    .with_no_client_auth();
    Arc::new(config)
}

const FTP_SETUP_TIMEOUT: Duration = Duration::from_secs(10);
const FTP_CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(3);
const FTP_DATA_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const FTP_IO_TIMEOUT: Duration = Duration::from_secs(60);
const FTP_MAX_CONNECT_ADDRESSES: usize = 8;

#[derive(Clone, Copy)]
struct FtpTiming {
    setup: Duration,
    connect_attempt: Duration,
    data_connect: Duration,
    io: Duration,
}

impl FtpTiming {
    const PRODUCTION: Self = Self {
        setup: FTP_SETUP_TIMEOUT,
        connect_attempt: FTP_CONNECT_ATTEMPT_TIMEOUT,
        data_connect: FTP_DATA_CONNECT_TIMEOUT,
        io: FTP_IO_TIMEOUT,
    };
}

struct SetupDeadline {
    expires: Instant,
}

impl SetupDeadline {
    fn new(timeout: Duration) -> Self {
        Self {
            expires: Instant::now() + timeout,
        }
    }

    fn remaining(&self, stage: &str) -> io::Result<Duration> {
        self.expires
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("FTP setup timed out during {stage}"),
                )
            })
    }

    fn map_error(&self, stage: &str, error: io::Error) -> io::Error {
        if self.expires <= Instant::now() {
            io::Error::new(
                io::ErrorKind::TimedOut,
                format!("FTP setup timed out during {stage}: {error}"),
            )
        } else {
            error
        }
    }
}

/// Shuts down the setup socket at the absolute deadline. The helper thread is
/// always cancelled and joined before `connect_stream_with_timing` returns, so
/// repeated failed setups cannot accumulate detached timeout workers.
struct SetupWatchdog {
    cancel: Option<mpsc::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl SetupWatchdog {
    fn arm(stream: &TcpStream, deadline: &SetupDeadline) -> io::Result<Self> {
        let timeout = deadline.remaining("setup watchdog")?;
        let watched = stream.try_clone()?;
        let (cancel, cancelled) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("ftp-setup-deadline".to_string())
            .spawn(move || {
                if cancelled.recv_timeout(timeout).is_err() {
                    let _ = watched.shutdown(Shutdown::Both);
                }
            })?;
        Ok(Self {
            cancel: Some(cancel),
            worker: Some(worker),
        })
    }
}

impl Drop for SetupWatchdog {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn connect_addresses_with(
    addresses: &[SocketAddr],
    deadline: &SetupDeadline,
    per_attempt: Duration,
    mut connect: impl FnMut(SocketAddr, Duration) -> io::Result<TcpStream>,
) -> io::Result<TcpStream> {
    let mut last_error = None;
    for &address in addresses.iter().take(FTP_MAX_CONNECT_ADDRESSES) {
        let timeout = deadline.remaining("TCP connect")?.min(per_attempt);
        match connect(address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| io_err("FTP host has no reachable address")))
}

fn set_setup_timeouts(stream: &TcpStream, deadline: &SetupDeadline, stage: &str) -> io::Result<()> {
    let timeout = deadline.remaining(stage)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))
}

fn connect_data_stream(
    address: SocketAddr,
    connect_timeout: Duration,
    io_timeout: Duration,
) -> suppaftp::FtpResult<TcpStream> {
    let stream =
        TcpStream::connect_timeout(&address, connect_timeout).map_err(FtpError::ConnectionError)?;
    stream
        .set_read_timeout(Some(io_timeout))
        .and_then(|()| stream.set_write_timeout(Some(io_timeout)))
        .map_err(FtpError::ConnectionError)?;
    Ok(stream)
}

pub(super) fn connect_stream(config: &FtpUrl) -> io::Result<RustlsFtpStream> {
    connect_stream_with_timing(config, FtpTiming::PRODUCTION, rustls_client_config())
}

fn connect_stream_with_timing(
    config: &FtpUrl,
    timing: FtpTiming,
    tls_config: Arc<rustls::ClientConfig>,
) -> io::Result<RustlsFtpStream> {
    let deadline = SetupDeadline::new(timing.setup);
    let addresses = resolve_host(
        &config.host,
        config.port,
        deadline.expires,
        FTP_MAX_CONNECT_ADDRESSES,
    )?;
    let stream = connect_addresses_with(
        &addresses,
        &deadline,
        timing.connect_attempt,
        |address, timeout| TcpStream::connect_timeout(&address, timeout),
    )?;
    set_setup_timeouts(&stream, &deadline, "server greeting")?;
    let watchdog = SetupWatchdog::arm(&stream, &deadline)?;
    let mut ftp = RustlsFtpStream::connect_with_stream(stream)
        .map_err(ftp_err)
        .map_err(|error| deadline.map_error("server greeting", error))?;
    ftp = ftp.passive_stream_builder(move |address| {
        connect_data_stream(address, timing.data_connect, timing.io)
    });
    if config.secure {
        set_setup_timeouts(ftp.get_ref(), &deadline, "AUTH TLS")?;
        let connector = RustlsConnector::from(tls_config);
        ftp = ftp
            .into_secure(connector, &config.host)
            .map_err(ftp_err)
            .map_err(|error| deadline.map_error("AUTH TLS", error))?;
    }
    set_setup_timeouts(ftp.get_ref(), &deadline, "login")?;
    ftp.login(&config.user, &config.password)
        .map_err(ftp_err)
        .map_err(|error| deadline.map_error("login", error))?;
    set_setup_timeouts(ftp.get_ref(), &deadline, "binary mode")?;
    ftp.transfer_type(FileType::Binary)
        .map_err(ftp_err)
        .map_err(|error| deadline.map_error("binary mode", error))?;
    // Keep every control operation bounded. The keepalive thread cannot probe
    // while a command owns this stream, so a blackholed command must time out
    // itself; callers then retire/reconnect the channel without replaying a
    // mutation. Passive data sockets use the same inactivity deadline.
    ftp.get_ref().set_read_timeout(Some(timing.io))?;
    ftp.get_ref().set_write_timeout(Some(timing.io))?;
    drop(watchdog);
    Ok(ftp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn rustls_config_builds_with_ring() {
        let _ = rustls_client_config();
    }

    #[test]
    fn initial_connect_attempts_are_capped_and_share_one_deadline() {
        let addresses: Vec<_> = (1..=20)
            .map(|last| SocketAddr::from(([192, 0, 2, last], 21)))
            .collect();
        let attempts = AtomicUsize::new(0);
        let deadline = SetupDeadline::new(Duration::from_secs(1));
        let error = connect_addresses_with(
            &addresses,
            &deadline,
            Duration::from_millis(100),
            |_, timeout| {
                assert!(timeout <= Duration::from_millis(100));
                attempts.fetch_add(1, Ordering::SeqCst);
                Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    "fixture refusal",
                ))
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
        assert_eq!(attempts.load(Ordering::SeqCst), FTP_MAX_CONNECT_ADDRESSES);
    }
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod connection_tests;
