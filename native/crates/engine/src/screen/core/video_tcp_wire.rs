//! The direct TCP picture link of the screen stream („Übertragung: TCP“; user
//! 2026-11-25: „nicht über das relay, auch direkt. TCP und UDP, DIREKT.“): a
//! TCP connection straight between the two devices, never through the relay
//! (the relay is HTTPS/WebSocket over TCP through the server, exactly what the
//! user rejects). This module holds its control frames on the authenticated
//! screen control stream and the candidate rules; the sockets, TLS and the
//! simultaneous open live in `os/shared/video_tcp_link.rs`.
//!
//! Flow: a host announces `video_tcp` (`VIDEO_TCP_V1`) in `SERVER_HELLO`. A
//! viewer that wants TCP opens its listener, gathers its candidates and
//! sends `VIDEO_TCP_OFFER` (240); the frame is its opt-in, so the client
//! hello needs no field (as `SUBSCRIBE_FEED`). The host gathers its own and
//! answers `VIDEO_TCP_ANSWER` (241) with its candidates, a fresh 32-byte
//! session token and the SHA-256 of its fresh TLS key; then both sides dial
//! every peer candidate from their listener's port while listening on it (TCP
//! simultaneous open). Each side ends with `VIDEO_TCP_RESULT` (242): the
//! connected address and its path, or the reason none connected.
//!
//! Tags 240 to 242 are taken from the high end of the tag space; tags 1 to
//! 60 belong to the older frames, 61 to 64 to the UDP picture path
//! (`FRAME_LOSS`, `VIDEO_OFFER`, `VIDEO_ANSWER`, `VIDEO_REPORT`).
//! All three payloads are JSON within the default control budget
//! (`MAX_CONTROL_BYTES`), so `wire::tag_limit` needs no entry.
use std::fmt;
use std::io;
use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};

use super::wire::{from_json, invalid, to_json};
use super::wire_binary::MAX_RECORD_BYTES;

#[path = "video_tcp_plan.rs"]
mod plan;
pub use plan::{
    classify_path, is_lan_ip, is_public_v4, local_candidates, merge_candidates,
    predicted_candidates, MAX_LOCAL_CANDIDATES,
};

/// `ScreenServerHello::video_tcp`: the host answers `VIDEO_TCP_OFFER`.
pub const VIDEO_TCP_V1: u16 = 1;
/// Viewer to host: the viewer's candidates (its opt-in).
pub const VIDEO_TCP_OFFER: u8 = 240;
/// Host to viewer: the host's candidates, the token and the key's SHA-256.
pub const VIDEO_TCP_ANSWER: u8 = 241;
/// Either direction: which candidate connected, or why none did.
pub const VIDEO_TCP_RESULT: u8 = 242;
/// Candidates one side names at most: its interface addresses
/// (`MAX_LOCAL_CANDIDATES`), the reflected and the predicted public address.
pub const MAX_TCP_CANDIDATES: usize = 16;
/// The session token the viewer writes as its first TLS bytes.
pub const TOKEN_BYTES: usize = 32;
/// The host's answer to a valid token, its first TLS bytes.
pub const CONFIRM: [u8; 4] = *b"SETC";
/// ALPN of the TLS session on the TCP link; any other TLS peer fails.
pub const TCP_ALPN: &[u8] = b"smart-explorer/screen-tcp/1";
/// Bytes a link record may carry beyond one picture record payload: the
/// picture group head with its contract extension and the record head.
pub const LINK_RECORD_OVERHEAD: usize = 4096;
/// Largest record of the link (`u32` length + bytes): one picture record
/// (`wire_binary::MAX_RECORD_BYTES`) with its heads.
pub const MAX_LINK_RECORD_BYTES: usize = MAX_RECORD_BYTES + LINK_RECORD_OVERHEAD;

/// How a side learned a candidate (statistics; every kind is dialed).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    /// An address of one of the device's own interfaces.
    Local,
    /// The public address the Share server's TCP reflector saw.
    Reflected,
    /// The public IP the Iroh endpoint learned (QAD) with the listener's
    /// port: right behind a port-preserving NAT.
    Predicted,
    /// A kind of a newer peer.
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TcpCandidate {
    pub addr: SocketAddr,
    pub kind: CandidateKind,
}

/// The 32-byte session token; never printed, wiped on drop, compared only in
/// constant time (`os/shared/video_tcp_tls.rs`), so it has no `PartialEq`.
#[derive(Clone)]
pub struct TcpToken(pub [u8; TOKEN_BYTES]);

impl fmt::Debug for TcpToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TcpToken(..)")
    }
}

impl Drop for TcpToken {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

/// `VIDEO_TCP_OFFER`: `{"candidates":[{"addr":"192.168.1.5:40000","kind":"local"}]}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoTcpOffer {
    pub candidates: Vec<TcpCandidate>,
}

/// `VIDEO_TCP_ANSWER`: the host's candidates, `token` and `cert_sha256` as
/// 64 lower-case hex digits. `cert_sha256` is the SHA-256 of the host's TLS
/// certificate entry, a raw public key (RFC 7250: its SubjectPublicKeyInfo
/// DER), which the viewer pins.
#[derive(Clone, Debug)]
pub struct VideoTcpAnswer {
    pub candidates: Vec<TcpCandidate>,
    pub token: TcpToken,
    pub cert_sha256: [u8; 32],
}

#[derive(Serialize, Deserialize)]
struct AnswerWire {
    candidates: Vec<TcpCandidate>,
    token: String,
    cert_sha256: String,
}

/// The path a link took (statistics and the status line).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TcpPathKind {
    /// Private, link-local, shared (100.64/10) or loopback address.
    Lan,
    /// A global IPv6 address.
    Ipv6,
    /// A public IPv4 address that is this side's own interface address.
    Public,
    /// A public IPv4 address reached through this side's NAT.
    Punched,
    #[serde(other)]
    Other,
}

impl TcpPathKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Lan => "LAN",
            Self::Ipv6 => "IPv6",
            Self::Public => "öffentlich",
            Self::Punched => "durch NAT gelocht",
            Self::Other => "unbekannt",
        }
    }
}

/// Why no TCP link came up; the wire key is the variant in snake case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TcpFailureReason {
    /// No TCP connection to any candidate came up.
    Unreachable,
    /// The host closed the connection without confirming the token.
    Refused,
    /// A connection brought a wrong token (host side).
    WrongToken,
    /// The TLS handshake with the peer failed (key pin, protocol).
    Tls,
    /// A connection came up, but TLS or the token did not finish in time.
    Timeout,
    /// This device could not open its TCP port.
    LocalSocket,
    #[serde(other)]
    Other,
}

impl TcpFailureReason {
    /// The German reason for the status line.
    pub fn text(self) -> &'static str {
        match self {
            Self::Unreachable => {
                "keine TCP-Adresse der Gegenstelle erreichbar (beide Seiten wählten direkt; \
                 ein NAT oder eine Firewall lässt TCP nicht durch)"
            }
            Self::Refused => {
                "die Gegenstelle hat die TCP-Verbindung abgelehnt (Sitzungsschlüssel nicht \
                 bestätigt)"
            }
            Self::WrongToken => {
                "über TCP kam ein falscher Sitzungsschlüssel an; die Verbindung wurde verworfen"
            }
            Self::Tls => "TLS-Aushandlung über TCP fehlgeschlagen",
            Self::Timeout => "Zeitüberschreitung beim Aufbau der TCP-Verbindung",
            Self::LocalSocket => "TCP-Port konnte auf diesem Gerät nicht geöffnet werden",
            Self::Other => "TCP-Verbindung fehlgeschlagen (Grund einer neueren Version)",
        }
    }
}

/// `VIDEO_TCP_RESULT`: `{"outcome":"connected","remote":"…","path":"punched"}`
/// or `{"outcome":"failed","reason":"unreachable"}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum VideoTcpResult {
    Connected {
        remote: SocketAddr,
        path: TcpPathKind,
    },
    Failed {
        reason: TcpFailureReason,
    },
}

/// Checks and canonicalizes the candidates a peer named: at most
/// `MAX_TCP_CANDIDATES`, no port 0, no unspecified, multicast, broadcast or
/// loopback address and no IPv6 link-local one (its scope is the peer's);
/// an IPv4-mapped IPv6 address becomes IPv4, duplicates are dropped.
fn checked(candidates: Vec<TcpCandidate>) -> io::Result<Vec<TcpCandidate>> {
    if candidates.len() > MAX_TCP_CANDIDATES {
        return Err(invalid("TCP offer names more than 16 candidates"));
    }
    let mut out: Vec<TcpCandidate> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let addr = SocketAddr::new(candidate.addr.ip().to_canonical(), candidate.addr.port());
        if addr.port() == 0 || !dialable(addr.ip()) {
            return Err(invalid("TCP offer names an address that cannot be dialed"));
        }
        if !out.iter().any(|known| known.addr == addr) {
            out.push(TcpCandidate {
                addr,
                kind: candidate.kind,
            });
        }
    }
    Ok(out)
}

fn dialable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !(ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() || ip.is_broadcast())
        }
        IpAddr::V6(ip) => {
            !(ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_multicast()
                || (ip.segments()[0] & 0xffc0) == 0xfe80)
        }
    }
}

fn hex32(text: &str) -> io::Result<[u8; 32]> {
    let bytes = data_encoding::HEXLOWER_PERMISSIVE
        .decode(text.as_bytes())
        .map_err(|_| invalid("TCP answer field is not hex"))?;
    bytes
        .try_into()
        .map_err(|_| invalid("TCP answer field is not 32 bytes"))
}

impl VideoTcpOffer {
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        checked(self.candidates.clone())?;
        to_json(self)
    }

    pub fn decode(payload: &[u8]) -> io::Result<Self> {
        let offer: Self = from_json(payload)?;
        Ok(Self {
            candidates: checked(offer.candidates)?,
        })
    }
}

impl VideoTcpAnswer {
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        checked(self.candidates.clone())?;
        to_json(&AnswerWire {
            candidates: self.candidates.clone(),
            token: data_encoding::HEXLOWER.encode(&self.token.0),
            cert_sha256: data_encoding::HEXLOWER.encode(&self.cert_sha256),
        })
    }

    pub fn decode(payload: &[u8]) -> io::Result<Self> {
        let wire: AnswerWire = from_json(payload)?;
        Ok(Self {
            candidates: checked(wire.candidates)?,
            token: TcpToken(hex32(&wire.token)?),
            cert_sha256: hex32(&wire.cert_sha256)?,
        })
    }
}

impl VideoTcpResult {
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        to_json(self)
    }

    pub fn decode(payload: &[u8]) -> io::Result<Self> {
        from_json(payload)
    }
}

#[cfg(test)]
#[path = "video_tcp_wire_tests.rs"]
mod tests;
