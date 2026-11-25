//! TCP address reflector (`tcp_reflect_v1`, `address_services.rs`): answers
//! every connection with the source address it observed as one line,
//! `se-reflect 1 <ip>:<port>\n`, and then holds the connection until the
//! device closes it or sends anything, `IDLE` at most. The device connects
//! from the port it listens on for the direct TCP picture link, so the line
//! names the public mapping of exactly that port, and the open connection
//! keeps the mapping alive while the two devices punch through.
//!
//! Limits, as for the other listeners: at most `MAX_CONNECTIONS` at once,
//! `MAX_CONNECTIONS_PER_SOURCE` per source (an IPv4 address or an IPv6 /64,
//! `limits.rs`; carrier-grade NAT puts many devices behind one IPv4) and the
//! accept rate of `rate_limits.rs` (16/s per source, 128/s in total, with
//! bursts). A connection beyond them is closed before any byte.
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use super::limits::{ConnectionLimiter, SourceClassifier};
use super::rate_limits::AcceptRateLimiter;

pub(super) const CAPABILITY: &str = "tcp_reflect_v1";
pub(super) const DEFAULT_PORT: u16 = 51823;
pub(super) const LINE_PREFIX: &str = "se-reflect 1 ";
/// The longest a reflector connection stays: the device's gathering
/// (reflector answer within 1.5 s), the offer and answer over the screen
/// control stream, the peer's own gathering (1.5 s) and the 5 s of the TCP
/// race, with room for slow round trips.
const IDLE: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONNECTIONS: usize = 512;
const MAX_CONNECTIONS_PER_SOURCE: usize = 32;
/// Pause after a failed accept (e.g. no descriptor left).
const ACCEPT_PAUSE: Duration = Duration::from_millis(20);

pub(super) struct ReflectLimits {
    pub(super) connections: ConnectionLimiter,
    pub(super) accepts: AcceptRateLimiter,
    pub(super) idle: Duration,
}

/// Binds the reflector at `address` (`address_services::bind_all`) and
/// serves it on its own thread; both listeners of `[::]` share the
/// connection limits.
pub(super) fn start(address: SocketAddr) -> Result<SocketAddr, String> {
    let listeners = super::address_services::bind_all(address, std::net::TcpListener::bind)?;
    for listener in &listeners {
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
    }
    let bound = listeners
        .iter()
        .find_map(|listener| listener.local_addr().ok())
        .ok_or_else(|| "no bound address".to_string())?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("cannot create its runtime: {error}"))?;
    let connections = ConnectionLimiter::new(MAX_CONNECTIONS, MAX_CONNECTIONS_PER_SOURCE);
    std::thread::Builder::new()
        .name("share-tcp-reflect".into())
        .spawn(move || {
            runtime.block_on(async move {
                let mut serving = Vec::new();
                for listener in listeners {
                    match TcpListener::from_std(listener) {
                        Ok(listener) => serving.push(tokio::spawn(serve(
                            listener,
                            ReflectLimits {
                                connections: connections.clone(),
                                accepts: AcceptRateLimiter::new(),
                                idle: IDLE,
                            },
                        ))),
                        Err(error) => eprintln!("se-share-server: TCP address reflector: {error}"),
                    }
                }
                for task in serving {
                    let _ = task.await;
                }
            })
        })
        .map_err(|error| format!("cannot start its thread: {error}"))?;
    Ok(bound)
}

/// Accepts forever within `limits`.
pub(super) async fn serve(listener: TcpListener, mut limits: ReflectLimits) {
    let classifier = SourceClassifier::default();
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(_) => {
                tokio::time::sleep(ACCEPT_PAUSE).await;
                continue;
            }
        };
        let source = classifier.classify(peer);
        let Some(permit) = limits.connections.try_acquire(source) else {
            continue;
        };
        if !limits.accepts.try_admit(source) {
            continue;
        }
        let idle = limits.idle;
        tokio::spawn(async move {
            let _permit = permit;
            answer(stream, peer, idle).await;
        });
    }
}

/// The answer line for the observed `peer` (an IPv4-mapped address as IPv4).
pub(super) fn line(peer: SocketAddr) -> String {
    let observed = SocketAddr::new(peer.ip().to_canonical(), peer.port());
    format!("{LINE_PREFIX}{observed}\n")
}

async fn answer(stream: TcpStream, peer: SocketAddr, idle: Duration) {
    let _ = stream.set_nodelay(true);
    let line = line(peer);
    match timeout(WRITE_TIMEOUT, write_all(&stream, line.as_bytes())).await {
        Ok(Ok(())) => {}
        _ => return,
    }
    let _ = timeout(idle, closed_or_spoke(&stream)).await;
}

async fn write_all(stream: &TcpStream, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        stream.writable().await?;
        match stream.try_write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Returns when the device closed the connection or sent anything (the
/// protocol has nothing for it to send).
async fn closed_or_spoke(stream: &TcpStream) {
    let mut byte = [0u8; 1];
    loop {
        if stream.readable().await.is_err() {
            return;
        }
        match stream.try_read(&mut byte) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            _ => return,
        }
    }
}

#[cfg(test)]
#[path = "tcp_reflect_tests.rs"]
mod tests;
