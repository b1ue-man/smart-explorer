use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::{mpsc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const DNS_QUEUE_CAPACITY: usize = 16;

fn io_err(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

fn timed_out(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, message.into())
}

struct DnsRequest {
    host: String,
    port: u16,
    expires: Instant,
    max_addresses: usize,
    reply: mpsc::SyncSender<io::Result<Vec<SocketAddr>>>,
}

struct DnsService {
    requests: mpsc::SyncSender<DnsRequest>,
}

static DNS_SERVICE: OnceLock<Result<DnsService, String>> = OnceLock::new();

impl DnsService {
    fn start(startup_timeout: Duration) -> Result<Self, String> {
        let (requests, incoming) = mpsc::sync_channel(DNS_QUEUE_CAPACITY);
        let (startup, ready) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("ftp-dns".to_string())
            .spawn(move || {
                let _ = startup.send(Ok::<(), String>(()));
                while let Ok(request) = incoming.recv() {
                    let result = resolve_request(&request);
                    let _ = request.reply.send(result);
                }
            })
            .map_err(|error| error.to_string())?;
        ready
            .recv_timeout(startup_timeout)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => "FTP DNS worker startup timed out".to_string(),
                mpsc::RecvTimeoutError::Disconnected => {
                    "FTP DNS worker exited during startup".to_string()
                }
            })??;
        Ok(Self { requests })
    }
}

fn service(startup_timeout: Duration) -> io::Result<&'static DnsService> {
    match DNS_SERVICE.get_or_init(|| DnsService::start(startup_timeout)) {
        Ok(service) => Ok(service),
        Err(error) => Err(io_err(error)),
    }
}

fn resolve_request(request: &DnsRequest) -> io::Result<Vec<SocketAddr>> {
    request
        .expires
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| timed_out("FTP DNS resolution timed out in queue"))?;
    // getaddrinfo uses the OS's mDNS/LLMNR/VPN policy. There is exactly one
    // worker and a bounded queue: an uninterruptible OS call can block that
    // worker, but callers still time out and cannot spawn unbounded workers.
    let lookup = (request.host.as_str(), request.port).to_socket_addrs()?;
    let mut addresses = Vec::new();
    for address in lookup {
        if !addresses.contains(&address) {
            addresses.push(address);
        }
        if addresses.len() == request.max_addresses {
            break;
        }
    }
    if addresses.is_empty() {
        Err(io_err("FTP DNS returned no address"))
    } else {
        if Instant::now() >= request.expires { return Err(timed_out("FTP OS resolver exceeded its deadline")) }
        Ok(addresses)
    }
}

pub(super) fn resolve_host(
    host: &str,
    port: u16,
    expires: Instant,
    max_addresses: usize,
) -> io::Result<Vec<SocketAddr>> {
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let remaining = expires
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| timed_out("FTP DNS resolution deadline expired"))?;
    let (reply, response) = mpsc::sync_channel(1);
    let request = DnsRequest {
        host: host.to_string(),
        port,
        expires,
        max_addresses,
        reply,
    };
    let service = service(remaining)?;
    expires
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| timed_out("FTP DNS resolution deadline expired"))?;
    service
        .requests
        .try_send(request)
        .map_err(|error| match error {
            mpsc::TrySendError::Full(_) => {
                io::Error::new(io::ErrorKind::WouldBlock, "FTP DNS resolver queue is full")
            }
            mpsc::TrySendError::Disconnected(_) => io_err("FTP DNS resolver worker stopped"),
        })?;
    let remaining = expires
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| timed_out("FTP DNS resolution deadline expired"))?;
    response
        .recv_timeout(remaining)
        .map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => timed_out("FTP DNS resolution timed out"),
            mpsc::RecvTimeoutError::Disconnected => io_err("FTP DNS resolver worker stopped"),
        })?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_ip_bypasses_dns_service_even_after_deadline() {
        let address = resolve_host(
            "127.0.0.1",
            2121,
            Instant::now() - Duration::from_secs(1),
            8,
        )
        .unwrap();
        assert_eq!(address, vec![SocketAddr::from(([127, 0, 0, 1], 2121))]);
    }

    #[test]
    fn hostname_resolution_is_safe_inside_an_existing_tokio_runtime() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let addresses = runtime
            .block_on(async {
                resolve_host(
                    "localhost",
                    2121,
                    Instant::now() + Duration::from_secs(2),
                    8,
                )
            })
            .unwrap();
        assert!(addresses
            .iter()
            .all(|address| address.ip().is_loopback() && address.port() == 2121));
    }
}
