//! STUN Binding responder (`stun_v1`, `address_services.rs`; RFC 8489): a
//! device's UDP socket learns the public address its NAT gives it, for the
//! direct UDP picture path of the screen stream. A Binding request (class
//! request, method Binding, the magic cookie, a length that matches the
//! datagram) gets a Binding success response with XOR-MAPPED-ADDRESS
//! (section 14.2) and the request's transaction ID; everything else is
//! ignored without an answer, as are the request's own attributes (the
//! responder authenticates nothing and needs none of them).
//!
//! Limits: the accept rate of `rate_limits.rs` per source (an IPv4 address
//! or an IPv6 /64: 16 answers/s with a burst of 32) and in total (128/s,
//! burst 256). An answer (32 bytes for IPv4, 44 for IPv6) is at most 2.2
//! times a minimal request (20 bytes), so the responder amplifies little.
use std::net::{IpAddr, SocketAddr, UdpSocket};

use super::limits::SourceClassifier;
use super::rate_limits::AcceptRateLimiter;

pub(super) const CAPABILITY: &str = "stun_v1";
pub(super) const DEFAULT_PORT: u16 = 3478;
pub(super) const MAGIC_COOKIE: u32 = 0x2112_A442;
pub(super) const BINDING_REQUEST: u16 = 0x0001;
pub(super) const BINDING_SUCCESS: u16 = 0x0101;
pub(super) const XOR_MAPPED_ADDRESS: u16 = 0x0020;
pub(super) const HEADER_BYTES: usize = 20;
/// The largest datagram read; a longer one arrives cut and fails the
/// length check.
const MAX_DATAGRAM: usize = 1500;

/// The Binding success response to `request` from `source`; `None` for
/// anything that is not a well-formed Binding request.
pub(super) fn answer(request: &[u8], source: SocketAddr) -> Option<Vec<u8>> {
    if request.len() < HEADER_BYTES || request[0] & 0xC0 != 0 || source.port() == 0 {
        return None;
    }
    let message_type = u16::from_be_bytes([request[0], request[1]]);
    let length = usize::from(u16::from_be_bytes([request[2], request[3]]));
    let cookie = u32::from_be_bytes([request[4], request[5], request[6], request[7]]);
    if message_type != BINDING_REQUEST
        || cookie != MAGIC_COOKIE
        || length % 4 != 0
        || HEADER_BYTES + length != request.len()
    {
        return None;
    }
    let transaction = &request[8..HEADER_BYTES];
    let x_port = source.port() ^ (MAGIC_COOKIE >> 16) as u16;
    let (family, x_address): (u8, Vec<u8>) = match source.ip().to_canonical() {
        IpAddr::V4(ip) => (0x01, (u32::from(ip) ^ MAGIC_COOKIE).to_be_bytes().to_vec()),
        IpAddr::V6(ip) => {
            let mut key = [0u8; 16];
            key[..4].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
            key[4..].copy_from_slice(transaction);
            let octets = ip.octets();
            (0x02, octets.iter().zip(key).map(|(a, k)| a ^ k).collect())
        }
    };
    let value_length = 4 + x_address.len();
    let mut response = Vec::with_capacity(HEADER_BYTES + 4 + value_length);
    response.extend_from_slice(&BINDING_SUCCESS.to_be_bytes());
    response.extend_from_slice(&((4 + value_length) as u16).to_be_bytes());
    response.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    response.extend_from_slice(transaction);
    response.extend_from_slice(&XOR_MAPPED_ADDRESS.to_be_bytes());
    response.extend_from_slice(&(value_length as u16).to_be_bytes());
    response.extend_from_slice(&[0, family]);
    response.extend_from_slice(&x_port.to_be_bytes());
    response.extend_from_slice(&x_address);
    Some(response)
}

/// Binds the responder at `address` (`address_services::bind_all`) and
/// serves each socket on its own thread.
pub(super) fn start(address: SocketAddr) -> Result<SocketAddr, String> {
    let sockets = super::address_services::bind_all(address, UdpSocket::bind)?;
    let bound = sockets
        .iter()
        .find_map(|socket| socket.local_addr().ok())
        .ok_or_else(|| "no bound address".to_string())?;
    for socket in sockets {
        std::thread::Builder::new()
            .name("share-stun".into())
            .spawn(move || serve(&socket))
            .map_err(|error| format!("cannot start its thread: {error}"))?;
    }
    Ok(bound)
}

fn serve(socket: &UdpSocket) {
    let classifier = SourceClassifier::default();
    let mut limiter = AcceptRateLimiter::new();
    let mut buffer = [0u8; MAX_DATAGRAM];
    loop {
        let Ok((length, source)) = socket.recv_from(&mut buffer) else {
            continue;
        };
        handle(socket, &buffer[..length], source, &mut |source| {
            limiter.try_admit(classifier.classify(source))
        });
    }
}

/// Answers one datagram when it is a Binding request and `admit` lets its
/// source through; `true` when an answer went out. A datagram without an
/// answer costs its source nothing.
pub(super) fn handle(
    socket: &UdpSocket,
    datagram: &[u8],
    source: SocketAddr,
    admit: &mut impl FnMut(SocketAddr) -> bool,
) -> bool {
    let Some(response) = answer(datagram, source) else {
        return false;
    };
    admit(source) && socket.send_to(&response, source).is_ok()
}

#[cfg(test)]
#[path = "stun_tests.rs"]
mod tests;
