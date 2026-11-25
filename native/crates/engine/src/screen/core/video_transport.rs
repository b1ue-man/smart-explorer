//! Pictures as datagrams under „UDP“ (`Delivery::Realtime`; user
//! 2026-11-25: „NUTZ das“, „TCP und UDP, DIREKT“, „auch DIREKT UDP“), as
//! Moonlight and Sunshine stream video (plan 2026-11-14 `bildkette.md` 2.1):
//! each encoded picture goes as UDP packets with Reed-Solomon parity
//! (`fec.rs`, 20 %), nothing is sent again, a picture the viewer cannot
//! complete is reported (`FRAME_LOSS`) and the host continues from the
//! newest picture the viewer confirmed (B-36) or with a key frame.
//!
//! Carriers, in this order of preference:
//! - `video_udp_v1`: the screen session's own UDP sockets on a direct pair
//!   of the two devices (`screen/os/shared/video_udp.rs`): candidates are
//!   the local interface addresses and the public mapping a STUN Binding
//!   request (RFC 8489) to the Share server learns from the same socket;
//!   both sides send connectivity checks to every candidate of the other
//!   (as ICE, RFC 8445) and the host takes the first pair whose check was
//!   answered. Packets are authenticated and encrypted (ChaCha20-Poly1305)
//!   with a key of the session that travels on the QUIC control stream
//!   (`udp_wire.rs`). No congestion control, no retransmission.
//! - `video_datagrams_v1`: the same packets as QUIC DATAGRAM frames of the
//!   session's connection, only while no direct pair is validated; QUIC
//!   still controls their congestion and on a relay path they ride the
//!   relay. `STATS` names which carrier the pictures took (`udp_path`).
//!
//! Negotiation (additive, an older peer keeps its behaviour): the host names
//! the carriers in `SERVER_HELLO.video_carriers`; a viewer that reads them
//! sends `VIDEO_OFFER` (tag 62, its carriers and UDP candidates) once after
//! `HELLO_OK`; the host answers `VIDEO_ANSWER` (tag 63, session id, key, its
//! candidates) when it reads `video_udp_v1`. The viewer reports
//! `FRAME_LOSS` (tag 61, 16 bytes: group, frame) once per broken chain and
//! `VIDEO_REPORT` (tag 64, JSON counters) once a second while they change.
//! The host names the last picture it sent as datagrams with `VIDEO_TAIL`
//! (tag 65, 16 bytes: group, frame) once no newer one followed for
//! `TAIL_AFTER` (a still screen): a viewer that lacks it reports the loss,
//! and the capture delivers the unchanged screen again for the continuation
//! or key frame that answers it, as Sunshine repeats frames.
//! Without both announcements „UDP“ keeps the picture streams and their
//! stale reset (`pacing.rs`).
//!
//! „TCP“ (`Delivery::Reliable`): the pictures go as one message per record
//! on the direct TCP link (`video_tcp_wire.rs`, `screen/os/shared/
//! video_tcp_link.rs`, `video_tcp_carrier.rs`), which both sides set up at
//! the session's start when the host names `video_tcp`; without it the QUIC
//! picture streams carry them, and `STATS` says why (`tcp_failure`).
//! Only `video` pictures of H.264, HEVC and AV1 without hybrid regions
//! travel as messages; tiles and previews keep the streams.
use std::io;
use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use super::wire::invalid;
use super::wire_hello::{ScreenServerHello, SessionMode};

#[path = "udp_checks.rs"]
pub mod udp_checks;
#[path = "udp_wire.rs"]
pub mod udp_wire;
#[path = "video_assembly.rs"]
pub mod video_assembly;
#[path = "video_chain.rs"]
pub mod video_chain;
#[path = "video_delivery.rs"]
pub mod video_delivery;
#[path = "video_host.rs"]
pub mod video_host;
#[path = "video_message.rs"]
pub mod video_message;
#[path = "video_packet.rs"]
pub mod video_packet;
#[path = "video_route.rs"]
pub mod video_route;
#[path = "video_sink.rs"]
pub mod video_sink;
#[path = "video_viewer.rs"]
pub mod video_viewer;

/// The QUIC DATAGRAM carrier.
pub const VIDEO_DATAGRAMS_V1: &str = "video_datagrams_v1";
/// The direct UDP carrier.
pub const VIDEO_UDP_V1: &str = "video_udp_v1";
/// Carriers of this build, in its order of preference.
pub const CARRIERS: [&str; 2] = [VIDEO_UDP_V1, VIDEO_DATAGRAMS_V1];

/// Viewer to host: the picture the viewer gave up (`FrameLoss`, 16 bytes).
pub const FRAME_LOSS: u8 = 61;
/// Viewer to host: its carriers and UDP candidates (`VideoOffer`, JSON).
pub const VIDEO_OFFER: u8 = 62;
/// Host to viewer: the direct pair's key and candidates (`VideoAnswer`,
/// JSON); only to a viewer that sent `VIDEO_OFFER`. No `HostFrame`: the
/// viewer's control reader takes it before the host frames are decoded.
pub const VIDEO_ANSWER: u8 = 63;
/// Viewer to host: its counters (`VideoReport`, JSON).
pub const VIDEO_REPORT: u8 = 64;
/// Host to viewer: the last picture sent as datagrams before a still
/// screen (16 bytes, as `FRAME_LOSS`); no `HostFrame`, as `VIDEO_ANSWER`.
pub const VIDEO_TAIL: u8 = 65;
pub const FRAME_LOSS_BYTES: usize = 16;
pub const VIDEO_OFFER_BYTES: usize = 2048;
pub const VIDEO_ANSWER_BYTES: usize = 2048;
pub const VIDEO_REPORT_BYTES: usize = 512;
/// Candidates one side names at most.
pub const MAX_CANDIDATES: usize = 16;

/// `FRAME_LOSS`: the viewer gave up `frame_id` of group `gop_id` and skips
/// that group's chain until a key frame or a newer group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameLoss {
    pub gop_id: u64,
    pub frame_id: u64,
}

impl FrameLoss {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(FRAME_LOSS_BYTES);
        out.extend_from_slice(&self.gop_id.to_be_bytes());
        out.extend_from_slice(&self.frame_id.to_be_bytes());
        out
    }

    pub fn decode(payload: &[u8]) -> io::Result<Self> {
        let bytes: [u8; FRAME_LOSS_BYTES] = payload
            .try_into()
            .map_err(|_| invalid("FRAME_LOSS carries 16 bytes"))?;
        let mut gop = [0u8; 8];
        let mut frame = [0u8; 8];
        gop.copy_from_slice(&bytes[..8]);
        frame.copy_from_slice(&bytes[8..]);
        Ok(Self {
            gop_id: u64::from_be_bytes(gop),
            frame_id: u64::from_be_bytes(frame),
        })
    }
}

/// `VIDEO_OFFER`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoOffer {
    /// The carriers this viewer reads (`CARRIERS`); unknown names are kept
    /// and ignored.
    pub carriers: Vec<String>,
    /// Its UDP candidates (`ip:port`) for `video_udp_v1`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<String>,
}

/// `VIDEO_ANSWER`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoAnswer {
    /// Names the session's packets (`udp_wire::Envelope`).
    pub session: u64,
    /// The 32-byte key in hex.
    pub key: String,
    /// The host's UDP candidates (`ip:port`).
    pub candidates: Vec<String>,
}

/// `VIDEO_REPORT`: counters of the viewer since its session started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoReport {
    /// Pictures handed to the decoder.
    pub pictures: u64,
    /// Pictures given up incomplete.
    pub lost_pictures: u64,
    /// Complete pictures skipped because their chain broke.
    #[serde(default)]
    pub skipped_pictures: u64,
    /// Pictures completed with rebuilt packets.
    pub recovered_pictures: u64,
    /// Data packets that did not arrive (rebuilt or not).
    pub lost_packets: u64,
    /// Data packets rebuilt from parity.
    pub recovered_packets: u64,
    /// Packets of pictures already finished.
    pub late_packets: u64,
    /// Packets that came on the direct UDP pair.
    #[serde(default)]
    pub direct_packets: u64,
}

/// The carrier of the pictures, as `STATS` names it (`udp_path`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoPath {
    /// The direct UDP pair.
    Direct,
    /// QUIC DATAGRAM frames on a direct QUIC path.
    QuicDirect,
    /// QUIC DATAGRAM frames through the relay (HTTPS, not UDP).
    Relay,
}

impl VideoPath {
    pub fn key(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::QuicDirect => "quic_direct",
            Self::Relay => "relay",
        }
    }
}

/// The carriers both sides read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Carriers {
    pub datagrams: bool,
    pub udp: bool,
}

impl Carriers {
    pub fn of(names: &[String]) -> Self {
        let has = |name: &str| names.iter().any(|known| known == name);
        Self {
            datagrams: has(VIDEO_DATAGRAMS_V1),
            udp: has(VIDEO_UDP_V1),
        }
    }

    pub fn any(self) -> bool {
        self.datagrams || self.udp
    }

    /// Both sides read it.
    pub fn and(self, other: Self) -> Self {
        Self {
            datagrams: self.datagrams && other.datagrams,
            udp: self.udp && other.udp,
        }
    }

    pub fn names(self) -> Vec<String> {
        let mut names = Vec::new();
        if self.udp {
            names.push(VIDEO_UDP_V1.to_string());
        }
        if self.datagrams {
            names.push(VIDEO_DATAGRAMS_V1.to_string());
        }
        names
    }
}

/// What this build announces in `SERVER_HELLO.video_carriers`.
pub fn host_carriers() -> Vec<String> {
    CARRIERS.iter().map(|name| name.to_string()).collect()
}

/// The carriers a viewer offers for a session of `mode` to a host that
/// announced `server`: none for a probe or a metadata channel.
pub fn viewer_carriers(server: &ScreenServerHello, mode: SessionMode, metadata: bool) -> Carriers {
    if metadata || mode == SessionMode::Probe {
        return Carriers::default();
    }
    let all = Carriers {
        datagrams: true,
        udp: true,
    };
    Carriers::of(&server.video_carriers).and(all)
}

/// Whether a viewer sets up the direct TCP link (`VIDEO_TCP_OFFER`) for a
/// session of `mode` with a host that announced `server`.
pub fn viewer_tcp(server: &ScreenServerHello, mode: SessionMode, metadata: bool) -> bool {
    !metadata
        && mode != SessionMode::Probe
        && server.video_tcp == Some(super::video_tcp_wire::VIDEO_TCP_V1)
}

fn parse_candidates(names: &[String]) -> io::Result<Vec<SocketAddr>> {
    if names.len() > MAX_CANDIDATES {
        return Err(invalid("too many UDP candidates"));
    }
    names
        .iter()
        .map(|name| {
            name.parse()
                .map_err(|_| invalid("UDP candidate is no ip:port"))
        })
        .collect()
}

impl VideoOffer {
    pub fn validate(&self) -> io::Result<()> {
        if self.carriers.len() > 8 || self.carriers.iter().any(|name| name.len() > 32) {
            return Err(invalid("VIDEO_OFFER names too many carriers"));
        }
        parse_candidates(&self.candidates).map(|_| ())
    }

    pub fn candidates(&self) -> Vec<SocketAddr> {
        parse_candidates(&self.candidates).unwrap_or_default()
    }
}

impl VideoAnswer {
    pub fn new(session: u64, key: &[u8; 32], candidates: &[SocketAddr]) -> Self {
        Self {
            session,
            key: key.iter().map(|byte| format!("{byte:02x}")).collect(),
            candidates: candidates.iter().map(SocketAddr::to_string).collect(),
        }
    }

    pub fn validate(&self) -> io::Result<()> {
        self.key_bytes()?;
        parse_candidates(&self.candidates).map(|_| ())
    }

    pub fn key_bytes(&self) -> io::Result<[u8; 32]> {
        let text = self.key.as_bytes();
        if text.len() != 64 {
            return Err(invalid("VIDEO_ANSWER key is 64 hex digits"));
        }
        let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        let mut key = [0u8; 32];
        for (out, pair) in key.iter_mut().zip(text.chunks(2)) {
            match (digit(pair[0]), digit(pair[1])) {
                (Some(high), Some(low)) => *out = (high << 4) | low,
                _ => return Err(invalid("VIDEO_ANSWER key is 64 hex digits")),
            }
        }
        Ok(key)
    }

    pub fn candidates(&self) -> Vec<SocketAddr> {
        parse_candidates(&self.candidates).unwrap_or_default()
    }
}

/// The Share server's STUN responder (UDP, `stun_v1`, port 3478).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StunServer {
    pub host: String,
    pub port: u16,
}

/// The STUN responder of the Share server this process last completed a
/// hello with (`share::address_services`, `stun_v1` in its `hello_ok`);
/// `None`: local candidates only (LAN, public addresses and the
/// peer-reflexive mappings the checks learn).
pub fn stun_server() -> Option<StunServer> {
    let services = crate::share::address_services::server_address_services()?;
    Some(StunServer {
        host: services.host,
        port: services.stun_port?,
    })
}

#[cfg(test)]
#[path = "video_transport_tests.rs"]
mod tests;
