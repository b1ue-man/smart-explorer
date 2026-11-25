//! The packets of the direct UDP pair (`video_transport.rs`) and the STUN
//! Binding request that learns a socket's public mapping (RFC 8489).
//!
//! Every packet of the pair, big-endian: kind `0xB3`, type `u8` (1 picture
//! packet, 2 connectivity check, 3 its answer), session `u64` (from
//! `VIDEO_ANSWER`), counter `u64`; then the inner bytes sealed with
//! ChaCha20-Poly1305 under the session's key (RFC 8439; `ring`), the 18
//! head bytes as associated data and the nonce `[direction, 0, 0, 0,
//! counter]` (direction 1 host to viewer, 2 viewer to host), so no nonce
//! repeats under one key. Inner bytes: a picture packet
//! (`video_packet.rs`), or for a check and its answer the check's
//! transaction `u64`. A packet of another session, kind, direction or key
//! fails to open and is dropped.
//!
//! STUN (RFC 8489 5, 6, 14.2): Binding request `0x0001` with length 0, the
//! magic cookie `0x2112A442` and a 12-byte transaction id; the Binding
//! success response `0x0101` names the mapping in XOR-MAPPED-ADDRESS
//! (`0x0020`, port and address XOR the cookie, an IPv6 address XOR cookie
//! and transaction id) or, from an old server, MAPPED-ADDRESS (`0x0001`).
//! The first two bits of a STUN message are zero, so it is never confused
//! with a packet of the pair (RFC 7983).
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};

use super::super::wire::invalid;

pub const UDP_KIND: u8 = 0xB3;
pub const ENVELOPE_HEAD: usize = 18;
pub const TAG_BYTES: usize = 16;
/// Largest UDP payload of the pair: below the 1280-byte IPv6 minimum MTU
/// with the IP and UDP headers, so no path fragments it.
pub const MAX_UDP_PAYLOAD: usize = 1200;
/// Largest inner packet (a picture packet) the pair carries.
pub const MAX_INNER: usize = MAX_UDP_PAYLOAD - ENVELOPE_HEAD - TAG_BYTES;

/// What a packet of the pair carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UdpType {
    Picture = 1,
    Check = 2,
    Answer = 3,
}

impl UdpType {
    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Picture),
            2 => Some(Self::Check),
            3 => Some(Self::Answer),
            _ => None,
        }
    }
}

/// Which side seals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Host,
    Viewer,
}

impl Side {
    fn byte(self) -> u8 {
        match self {
            Self::Host => 1,
            Self::Viewer => 2,
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Host => Self::Viewer,
            Self::Viewer => Self::Host,
        }
    }
}

/// Seals and opens the packets of one side of the pair.
pub struct Envelope {
    key: LessSafeKey,
    session: u64,
    side: Side,
    counter: AtomicU64,
}

impl Envelope {
    pub fn new(key: &[u8; 32], session: u64, side: Side) -> io::Result<Self> {
        let unbound = UnboundKey::new(&CHACHA20_POLY1305, key)
            .map_err(|_| invalid("the pair's key was refused"))?;
        Ok(Self {
            key: LessSafeKey::new(unbound),
            session,
            side,
            counter: AtomicU64::new(0),
        })
    }

    fn nonce(side: Side, counter: u64) -> Nonce {
        let mut nonce = [0u8; 12];
        nonce[0] = side.byte();
        nonce[4..].copy_from_slice(&counter.to_be_bytes());
        Nonce::assume_unique_for_key(nonce)
    }

    fn head(kind: UdpType, session: u64, counter: u64) -> [u8; ENVELOPE_HEAD] {
        let mut head = [0u8; ENVELOPE_HEAD];
        head[0] = UDP_KIND;
        head[1] = kind as u8;
        head[2..10].copy_from_slice(&session.to_be_bytes());
        head[10..].copy_from_slice(&counter.to_be_bytes());
        head
    }

    /// One packet of this side.
    pub fn seal(&self, kind: UdpType, inner: &[u8]) -> io::Result<Vec<u8>> {
        if inner.len() > MAX_INNER {
            return Err(invalid("inner packet exceeds the pair's payload"));
        }
        let counter = self.counter.fetch_add(1, Ordering::Relaxed);
        if counter == u64::MAX {
            return Err(invalid("the pair's nonces are used up"));
        }
        let head = Self::head(kind, self.session, counter);
        let mut sealed = inner.to_vec();
        self.key
            .seal_in_place_append_tag(
                Self::nonce(self.side, counter),
                Aad::from(head),
                &mut sealed,
            )
            .map_err(|_| invalid("sealing a packet of the pair failed"))?;
        let mut packet = Vec::with_capacity(ENVELOPE_HEAD + sealed.len());
        packet.extend_from_slice(&head);
        packet.extend_from_slice(&sealed);
        Ok(packet)
    }

    /// A packet of the other side; `None` when it is none of this session.
    pub fn open(&self, packet: &[u8]) -> Option<(UdpType, Vec<u8>)> {
        if packet.len() < ENVELOPE_HEAD + TAG_BYTES || packet[0] != UDP_KIND {
            return None;
        }
        let kind = UdpType::from_byte(packet[1])?;
        let mut session = [0u8; 8];
        session.copy_from_slice(&packet[2..10]);
        if u64::from_be_bytes(session) != self.session {
            return None;
        }
        let mut counter = [0u8; 8];
        counter.copy_from_slice(&packet[10..ENVELOPE_HEAD]);
        let mut head = [0u8; ENVELOPE_HEAD];
        head.copy_from_slice(&packet[..ENVELOPE_HEAD]);
        let mut sealed = packet[ENVELOPE_HEAD..].to_vec();
        let nonce = Self::nonce(self.side.other(), u64::from_be_bytes(counter));
        let inner = self
            .key
            .open_in_place(nonce, Aad::from(head), &mut sealed)
            .ok()?;
        Some((kind, inner.to_vec()))
    }
}

pub const STUN_MAGIC: u32 = 0x2112_A442;
const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS: u16 = 0x0101;
const MAPPED_ADDRESS: u16 = 0x0001;
const XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// A Binding request with transaction id `transaction`.
pub fn stun_request(transaction: &[u8; 12]) -> [u8; 20] {
    let mut request = [0u8; 20];
    request[..2].copy_from_slice(&BINDING_REQUEST.to_be_bytes());
    request[4..8].copy_from_slice(&STUN_MAGIC.to_be_bytes());
    request[8..].copy_from_slice(transaction);
    request
}

/// Whether a datagram is a STUN message (first two bits zero, cookie).
pub fn is_stun(bytes: &[u8]) -> bool {
    bytes.len() >= 20 && bytes[0] & 0xC0 == 0 && bytes[4..8] == STUN_MAGIC.to_be_bytes()
}

/// The mapping a Binding success response for `transaction` names.
pub fn stun_mapping(response: &[u8], transaction: &[u8; 12]) -> Option<SocketAddr> {
    if !is_stun(response) || response[..2] != BINDING_SUCCESS.to_be_bytes() {
        return None;
    }
    if &response[8..20] != transaction {
        return None;
    }
    let length = usize::from(u16::from_be_bytes([response[2], response[3]]));
    let body = response.get(20..20 + length)?;
    let mut at = 0;
    let mut plain = None;
    while at + 4 <= body.len() {
        let kind = u16::from_be_bytes([body[at], body[at + 1]]);
        let len = usize::from(u16::from_be_bytes([body[at + 2], body[at + 3]]));
        let value = body.get(at + 4..at + 4 + len)?;
        match kind {
            XOR_MAPPED_ADDRESS => return address(value, Some(transaction)),
            MAPPED_ADDRESS => plain = address(value, None),
            _ => {}
        }
        // Attributes are padded to four bytes.
        at += 4 + len.div_ceil(4) * 4;
    }
    plain
}

/// An address attribute; with `transaction` its XOR form.
fn address(value: &[u8], transaction: Option<&[u8; 12]>) -> Option<SocketAddr> {
    if value.len() < 4 {
        return None;
    }
    let cookie = STUN_MAGIC.to_be_bytes();
    let mut port = u16::from_be_bytes([value[2], value[3]]);
    let mut mask = [0u8; 16];
    mask[..4].copy_from_slice(&cookie);
    if let Some(transaction) = transaction {
        port ^= (STUN_MAGIC >> 16) as u16;
        mask[4..].copy_from_slice(transaction);
    } else {
        mask = [0u8; 16];
    }
    let ip = match (value[1], value.len()) {
        (0x01, 8) => {
            let mut octets = [0u8; 4];
            for (index, octet) in octets.iter_mut().enumerate() {
                *octet = value[4 + index] ^ mask[index];
            }
            IpAddr::V4(Ipv4Addr::from(octets))
        }
        (0x02, 20) => {
            let mut octets = [0u8; 16];
            for (index, octet) in octets.iter_mut().enumerate() {
                *octet = value[4 + index] ^ mask[index];
            }
            IpAddr::V6(Ipv6Addr::from(octets))
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, port))
}

#[cfg(test)]
#[path = "udp_wire_tests.rs"]
mod tests;
