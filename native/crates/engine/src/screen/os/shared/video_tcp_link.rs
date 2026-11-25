//! The direct TCP picture link of the screen stream („Übertragung: TCP“;
//! user 2026-11-25: „nicht über das relay, auch direkt. TCP und UDP,
//! DIREKT.“): one TLS-protected TCP connection straight between host and
//! viewer that carries the picture records in place of the QUIC uni
//! streams. Frames and candidate rules: `screen/core/video_tcp_wire.rs`.
//!
//! Each side opens one listener per address family on a port the OS picks,
//! with the port shared (`tcp_port_share.rs` of its OS), and names its
//! candidates: its interface addresses, the public address the Share
//! server's TCP reflector saw from that very port (`tcp_reflect_v1`; the
//! reflector stream stays open while punching, so the NAT keeps the
//! mapping), and the Iroh endpoint's public IPv4 with the port (right behind
//! a port-preserving NAT). With the peer's candidates both sides dial every
//! one of them from the listener's port while the listener accepts: two
//! SYNs that cross make one connection (TCP simultaneous open, RFC 9293
//! 3.5), which is how TCP passes two NATs with endpoint-independent
//! mapping. The first connection that passes TLS and the token wins, the
//! others are closed.
//!
//! Numbers, and why:
//! - `PUNCH_WINDOW` 3 s: SYN rounds go out this long after a side has the
//!   peer's candidates. The two sides start apart by one control-stream
//!   one-way delay (well below 0.5 s); a peer's first SYN that reached this
//!   side's NAT before its own mapping existed is retransmitted by the
//!   peer's kernel after its initial RTO of 1 s (RFC 6298 2.1); with the
//!   fresh rounds below, three seconds hold three rounds per candidate.
//! - `ATTEMPT_TIMEOUT` 1 s: an attempt without an answer is replaced by a
//!   fresh SYN from the same port, as often as the kernel's own first retry.
//! - `RETRY_PAUSE` 200 ms after a refused attempt: a NAT that answers an
//!   unsolicited SYN with RST refuses at once; the next SYN after 200 ms
//!   keeps the mapping fresh without flooding (at most 16 candidates).
//! - `ESTABLISH_TIMEOUT` 5 s: the window plus TLS 1.3 (one round trip) and
//!   the token (one round trip) of a connection that came up at its end,
//!   with up to 1 s per round trip.
//! - `REFLECT_TIMEOUT` 1.5 s: the reflector question while gathering; a
//!   missing answer only costs the reflected candidate.
//!
//! Buffers: the sockets keep the kernel's TCP buffer autotuning. Setting
//! `SO_SNDBUF`/`SO_RCVBUF` switches it off and is capped by
//! `net.core.wmem_max`/`rmem_max` (about 208 KiB by default on Linux and
//! Android), below the autotuning ceilings (`tcp_wmem`/`tcp_rmem`: 4 and
//! 6 MiB); Windows likewise turns its dynamic send buffering off for a
//! socket with an explicit `SO_SNDBUF`. `TCP_NODELAY` is on.
use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;

use crate::screen::core::video_tcp_wire::{
    TcpCandidate, TcpFailureReason, TcpPathKind, TcpToken, VideoTcpAnswer, VideoTcpOffer,
    VideoTcpResult, TOKEN_BYTES,
};

#[path = "video_tcp_establish.rs"]
mod establish;
#[cfg_attr(windows, path = "../windows/tcp_port_share.rs")]
#[cfg_attr(not(windows), path = "../linux_os/tcp_port_share.rs")]
mod port_share;
#[path = "video_tcp_records.rs"]
mod records;
#[path = "video_tcp_sockets.rs"]
mod sockets;
#[path = "video_tcp_tls.rs"]
mod tls;

pub use records::{TcpRecordReader, TcpRecordWriter};

pub const PUNCH_WINDOW: Duration = Duration::from_secs(3);
pub const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(1);
pub const RETRY_PAUSE: Duration = Duration::from_millis(200);
pub const ESTABLISH_TIMEOUT: Duration = Duration::from_secs(5);
pub const REFLECT_TIMEOUT: Duration = Duration::from_millis(1500);

/// The Share server's TCP reflector (`share::address_services`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReflectorTarget {
    /// DNS name or IP address, IPv6 with or without brackets.
    pub host: String,
    pub port: u16,
}

/// What a side gathers its candidates from besides its interfaces.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GatherPlan {
    /// `None`: the server runs no reflector (or none is connected).
    pub reflector: Option<ReflectorTarget>,
    /// The Iroh endpoint's direct addresses (`Endpoint::addr().ip_addrs()`),
    /// which carry the public address QAD learned.
    pub endpoint_addrs: Vec<SocketAddr>,
}

/// Why no link came up, for the status line and `VIDEO_TCP_RESULT`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TcpLinkFailure {
    pub reason: TcpFailureReason,
    /// What the side saw beyond the reason, e.g. the last dial error.
    pub detail: Option<String>,
}

impl TcpLinkFailure {
    pub(crate) fn new(reason: TcpFailureReason, detail: Option<String>) -> Self {
        Self { reason, detail }
    }

    /// „TCP direkt nicht möglich: <Grund> (<Einzelheit>)“.
    pub fn status_line(&self) -> String {
        match &self.detail {
            Some(detail) => format!(
                "TCP direkt nicht möglich: {} ({detail})",
                self.reason.text()
            ),
            None => format!("TCP direkt nicht möglich: {}", self.reason.text()),
        }
    }

    /// The `VIDEO_TCP_RESULT` that tells the peer.
    pub fn result(&self) -> VideoTcpResult {
        VideoTcpResult::Failed {
            reason: self.reason,
        }
    }
}

impl fmt::Display for TcpLinkFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.status_line())
    }
}

/// Where a link runs, for the statistics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TcpLinkInfo {
    pub local: SocketAddr,
    pub remote: SocketAddr,
    pub path: TcpPathKind,
    /// This side's SYN made the connection (else its listener accepted).
    pub dialed: bool,
}

/// An established link: records both ways in place of QUIC uni streams.
/// The host's picture writer sends each picture message as one record with
/// `writer.send(&[message])` (`video_tcp_carrier.rs`); the viewer's reader
/// takes them with `reader.recv()`, bounded by `DEFAULT_LINK_RECORD_BYTES`
/// until `set_max_record` names the frame contract's largest picture
/// message (`video_tcp_wire::link_record_limit`).
pub struct TcpPictureLink {
    pub info: TcpLinkInfo,
    pub reader: TcpRecordReader,
    pub writer: TcpRecordWriter,
}

impl TcpPictureLink {
    /// The `VIDEO_TCP_RESULT` that tells the peer.
    pub fn result(&self) -> VideoTcpResult {
        VideoTcpResult::Connected {
            remote: self.info.remote,
            path: self.info.path,
        }
    }
}

/// The host's side before `establish`: listeners, candidates, the fresh
/// TLS key and the session token.
pub struct HostTcpLocal {
    sockets: sockets::LocalSockets,
    key: tls::HostKey,
    token: TcpToken,
}

impl HostTcpLocal {
    /// Opens the listeners and gathers the candidates within `deadline`.
    pub async fn gather(
        plan: &GatherPlan,
        deadline: std::time::Instant,
    ) -> Result<Self, TcpLinkFailure> {
        Self::with_sockets(sockets::gather(plan, deadline).await?)
    }

    fn with_sockets(sockets: sockets::LocalSockets) -> Result<Self, TcpLinkFailure> {
        let local = |error: std::io::Error| {
            TcpLinkFailure::new(TcpFailureReason::LocalSocket, Some(error.to_string()))
        };
        let key = tls::HostKey::generate().map_err(local)?;
        let mut token = TcpToken([0; TOKEN_BYTES]);
        getrandom::getrandom(&mut token.0)
            .map_err(|error| local(std::io::Error::other(error.to_string())))?;
        Ok(Self {
            sockets,
            key,
            token,
        })
    }

    /// `VIDEO_TCP_ANSWER`.
    pub fn answer(&self) -> VideoTcpAnswer {
        VideoTcpAnswer {
            candidates: self.sockets.candidates.clone(),
            token: self.token.clone(),
            cert_sha256: self.key.cert_sha256,
        }
    }

    pub fn candidates(&self) -> &[TcpCandidate] {
        &self.sockets.candidates
    }

    /// Why the reflector gave no address, if it gave none.
    pub fn reflect_error(&self) -> Option<&str> {
        self.sockets.reflect_error.as_deref()
    }

    /// Dials the viewer's candidates and accepts until a connection passed
    /// TLS and the viewer's token, or until `deadline`.
    pub async fn establish(
        self,
        offer: &VideoTcpOffer,
        deadline: std::time::Instant,
    ) -> Result<TcpPictureLink, TcpLinkFailure> {
        let local = |error: std::io::Error| {
            TcpLinkFailure::new(TcpFailureReason::LocalSocket, Some(error.to_string()))
        };
        let role = establish::Role::Host {
            config: self.key.server_config().map_err(local)?,
            token: self.token,
        };
        establish::race(self.sockets, &offer.candidates, role, deadline).await
    }

    #[cfg(test)]
    pub(crate) fn loopback() -> std::io::Result<Self> {
        Self::with_sockets(sockets::LocalSockets::loopback()?)
            .map_err(|failure| std::io::Error::other(failure.status_line()))
    }
}

/// The viewer's side before `establish`: listeners and candidates.
pub struct ViewerTcpLocal {
    sockets: sockets::LocalSockets,
}

impl ViewerTcpLocal {
    /// Opens the listeners and gathers the candidates within `deadline`.
    pub async fn gather(
        plan: &GatherPlan,
        deadline: std::time::Instant,
    ) -> Result<Self, TcpLinkFailure> {
        Ok(Self {
            sockets: sockets::gather(plan, deadline).await?,
        })
    }

    /// `VIDEO_TCP_OFFER`.
    pub fn offer(&self) -> VideoTcpOffer {
        VideoTcpOffer {
            candidates: self.sockets.candidates.clone(),
        }
    }

    pub fn candidates(&self) -> &[TcpCandidate] {
        &self.sockets.candidates
    }

    /// Why the reflector gave no address, if it gave none.
    pub fn reflect_error(&self) -> Option<&str> {
        self.sockets.reflect_error.as_deref()
    }

    /// Dials the host's candidates and accepts until a connection passed
    /// TLS with the pinned host key and the host confirmed the token, or
    /// until `deadline`.
    pub async fn establish(
        self,
        answer: &VideoTcpAnswer,
        deadline: std::time::Instant,
    ) -> Result<TcpPictureLink, TcpLinkFailure> {
        let config = tls::client_config(answer.cert_sha256)
            .map_err(|error| TcpLinkFailure::new(TcpFailureReason::Tls, Some(error.to_string())))?;
        let role = establish::Role::Viewer {
            config,
            token: answer.token.clone(),
        };
        establish::race(self.sockets, &answer.candidates, role, deadline).await
    }

    #[cfg(test)]
    pub(crate) fn loopback() -> std::io::Result<Self> {
        Ok(Self {
            sockets: sockets::LocalSockets::loopback()?,
        })
    }
}

#[cfg(test)]
#[path = "video_tcp_link_tests.rs"]
mod tests;
