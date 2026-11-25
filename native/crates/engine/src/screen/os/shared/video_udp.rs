//! The direct UDP pair of a screen session (`screen/core/video_transport.rs`,
//! „UDP“, user 2026-11-25: „auch DIREKT UDP“): the session's own sockets
//! (one IPv4, one IPv6, each on a port the system picks), its candidates and
//! the host's link task. Candidates are the addresses of the local
//! interfaces (no loopback, no IPv6 link-local: it needs a scope) with the
//! socket's port, and the public mapping a STUN Binding request
//! (`udp_wire::stun_request`, RFC 8489) to the Share server's responder
//! learns from the same socket (3 tries within 1.5 s). The checks
//! (`udp_checks.rs`) run on the host in a task of their own, which hands
//! the valid pair to the pipeline (`VideoRoute::set_direct`), and on the
//! viewer inside its picture task (`video_viewer.rs`, `ViewerLink`).
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::screen::core::gop_stream::now_us;
use crate::screen::core::video_transport::udp_checks::{Checks, Incoming};
use crate::screen::core::video_transport::udp_wire::{
    is_stun, stun_mapping, stun_request, Envelope, Side, UdpType, MAX_INNER,
};
use crate::screen::core::video_transport::video_route::{DirectSender, SendFuture, VideoRoute};
use crate::screen::core::video_transport::{StunServer, VideoAnswer, MAX_CANDIDATES, VIDEO_ANSWER};

/// Waits for a STUN answer, one per try.
const STUN_WAITS_MS: [u64; 3] = [250, 500, 750];
/// The longest lookup of the STUN responder's name.
const STUN_LOOKUP: Duration = Duration::from_secs(1);
/// Bytes read per datagram: above the pair's 1200-byte packets.
const RECV_BYTES: usize = 2048;
/// Receive errors in a row after which a socket pair is given up (Windows
/// reports an ICMP port-unreachable as an error of the next receive).
const MAX_ERRORS: u32 = 100;

fn random<const N: usize>() -> io::Result<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::getrandom(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes)
}

/// The two sockets of one side.
pub struct Sockets {
    v4: Option<UdpSocket>,
    v6: Option<UdpSocket>,
}

async fn recv_one(socket: Option<&UdpSocket>) -> io::Result<(Vec<u8>, SocketAddr)> {
    let Some(socket) = socket else {
        return std::future::pending().await;
    };
    let mut buffer = vec![0u8; RECV_BYTES];
    let (len, from) = socket.recv_from(&mut buffer).await?;
    buffer.truncate(len);
    Ok((buffer, from))
}

impl Sockets {
    pub async fn bind() -> io::Result<Self> {
        let v4 = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.ok();
        let v6 = UdpSocket::bind((Ipv6Addr::UNSPECIFIED, 0)).await.ok();
        if v4.is_none() && v6.is_none() {
            return Err(io::Error::other("no UDP socket for the direct pair"));
        }
        Ok(Self { v4, v6 })
    }

    fn for_addr(&self, to: &SocketAddr) -> Option<&UdpSocket> {
        match to {
            SocketAddr::V4(_) => self.v4.as_ref(),
            SocketAddr::V6(_) => self.v6.as_ref(),
        }
    }

    pub async fn send_to(&self, bytes: &[u8], to: SocketAddr) -> io::Result<()> {
        let socket = self
            .for_addr(&to)
            .ok_or_else(|| io::Error::other("no socket of the candidate's family"))?;
        socket.send_to(bytes, to).await.map(|_| ())
    }

    /// The next datagram of either socket and where it came from.
    pub async fn recv(&self) -> io::Result<(Vec<u8>, SocketAddr)> {
        tokio::select! {
            got = recv_one(self.v4.as_ref()) => got,
            got = recv_one(self.v6.as_ref()) => got,
        }
    }

    fn port(socket: Option<&UdpSocket>) -> Option<u16> {
        socket
            .and_then(|socket| socket.local_addr().ok())
            .map(|addr| addr.port())
    }

    /// The interface addresses with the sockets' ports.
    fn local_candidates(&self) -> Vec<SocketAddr> {
        let (v4, v6) = (Self::port(self.v4.as_ref()), Self::port(self.v6.as_ref()));
        let Ok(interfaces) = if_addrs::get_if_addrs() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for interface in interfaces {
            let ip = interface.ip();
            if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
                continue;
            }
            let port = match ip {
                IpAddr::V4(_) => v4,
                // Link-local IPv6 (fe80::/10) needs a scope id.
                IpAddr::V6(v6_ip) if v6_ip.segments()[0] & 0xffc0 == 0xfe80 => None,
                IpAddr::V6(_) => v6,
            };
            if let Some(port) = port {
                out.push(SocketAddr::new(ip, port));
            }
        }
        out
    }

    /// Local and STUN candidates, deduplicated, at most `MAX_CANDIDATES`.
    pub async fn candidates(&self, stun: Option<&StunServer>) -> Vec<SocketAddr> {
        let mut out = self.local_candidates();
        if let Some(stun) = stun {
            out.extend(self.stun_mappings(stun).await);
        }
        let mut seen = Vec::with_capacity(out.len());
        out.retain(|addr| {
            let new = !seen.contains(addr);
            seen.push(*addr);
            new
        });
        out.truncate(MAX_CANDIDATES);
        out
    }

    async fn stun_mappings(&self, stun: &StunServer) -> Vec<SocketAddr> {
        let lookup = tokio::net::lookup_host((stun.host.as_str(), stun.port));
        let Ok(Ok(addrs)) = tokio::time::timeout(STUN_LOOKUP, lookup).await else {
            return Vec::new();
        };
        let addrs: Vec<SocketAddr> = addrs.collect();
        let v4 = addrs.iter().copied().find(SocketAddr::is_ipv4);
        let v6 = addrs.iter().copied().find(SocketAddr::is_ipv6);
        let (a, b) = tokio::join!(
            stun_once(self.v4.as_ref(), v4),
            stun_once(self.v6.as_ref(), v6)
        );
        a.into_iter().chain(b).collect()
    }
}

/// One Binding request from `socket` to `server`, tried three times.
async fn stun_once(socket: Option<&UdpSocket>, server: Option<SocketAddr>) -> Option<SocketAddr> {
    let (socket, server) = (socket?, server?);
    let transaction: [u8; 12] = random().ok()?;
    let request = stun_request(&transaction);
    for wait in STUN_WAITS_MS {
        socket.send_to(&request, server).await.ok()?;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(wait);
        let mut buffer = [0u8; 512];
        // An error ends this try; the next one sends again.
        while let Ok(Ok((len, _))) =
            tokio::time::timeout_at(deadline, socket.recv_from(&mut buffer)).await
        {
            let answer = &buffer[..len];
            if is_stun(answer) {
                if let Some(mapping) = stun_mapping(answer, &transaction) {
                    return Some(mapping);
                }
            }
        }
    }
    None
}

/// The host's valid pair: pictures sealed and sent to the viewer's address.
struct DirectPair {
    sockets: Arc<Sockets>,
    envelope: Arc<Envelope>,
    remote: SocketAddr,
}

impl DirectSender for DirectPair {
    fn send<'a>(&'a self, packet: &'a [u8]) -> SendFuture<'a> {
        Box::pin(async move {
            let sealed = self.envelope.seal(UdpType::Picture, packet)?;
            self.sockets.send_to(&sealed, self.remote).await
        })
    }

    fn max_packet(&self) -> usize {
        MAX_INNER
    }
}

/// Before a session's own sockets open (the UDP pair, the TCP link): the
/// firewall admits se's program (`share::ensure_peer_firewall`: on Windows
/// the program rules for TCP and UDP in every profile, with the UAC
/// confirmation when one is missing; on Linux the active firewall, with the
/// polkit dialog), in a thread of its own that the session never waits
/// for; its status line goes to the screen log.
pub fn ensure_firewall_in_background(side: &'static str) {
    // Tests change no firewall of the machine they run on.
    if cfg!(any(test, feature = "test-support")) {
        return;
    }
    let started = std::thread::Builder::new()
        .name("screen-firewall".into())
        .spawn(move || {
            let line = match crate::share::ensure_peer_firewall() {
                Ok(status) => status,
                Err(error) => format!("Firewall nicht eingerichtet: {error}"),
            };
            log(&format!("firewall ({side}): {line}"));
        });
    if let Err(error) = started {
        log(&format!("firewall ({side}): not checked: {error}"));
    }
}

/// One line in the screen log of this process.
fn log(line: &str) {
    let line = format!("screen udp: {line}");
    // The unit tests write no file of the account they run under.
    #[cfg(not(test))]
    crate::screen::os::host_worker_client::host_log::note(None, &line);
    #[cfg(test)]
    let _ = line;
}

/// The host's side: binds, gathers, queues `VIDEO_ANSWER` for the session
/// (`out`) and runs the checks until it is aborted (the session's end).
pub fn spawn_host_link(
    viewer: Vec<SocketAddr>,
    route: Arc<VideoRoute>,
    stun: Option<StunServer>,
    out: mpsc::Sender<(u8, Vec<u8>)>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(error) = host_link(viewer, &route, stun, out).await {
            route.note(&format!("direct UDP pair ends: {error}"));
        }
        route.set_direct(None);
    })
}

async fn host_link(
    viewer: Vec<SocketAddr>,
    route: &Arc<VideoRoute>,
    stun: Option<StunServer>,
    out: mpsc::Sender<(u8, Vec<u8>)>,
) -> io::Result<()> {
    let sockets = Arc::new(Sockets::bind().await?);
    let candidates = sockets.candidates(stun.as_ref()).await;
    let key: [u8; 32] = random()?;
    let session = u64::from_be_bytes(random()?);
    let seed = u64::from_be_bytes(random()?);
    let envelope = Arc::new(Envelope::new(&key, session, Side::Host)?);
    let payload = serde_json::to_vec(&VideoAnswer::new(session, &key, &candidates))
        .map_err(io::Error::other)?;
    if out.send((VIDEO_ANSWER, payload)).await.is_err() {
        return Ok(());
    }
    route.note(&format!(
        "direct UDP pair: {} own and {} viewer candidates",
        candidates.len(),
        viewer.len()
    ));
    let mut checks = Checks::new(envelope.clone(), viewer, seed, now_us());
    let mut errors = 0u32;
    loop {
        let wait = checks.due_us().saturating_sub(now_us());
        tokio::select! {
            got = sockets.recv() => match got {
                Ok((bytes, from)) => {
                    errors = 0;
                    if let Incoming::Reply { packet, to } = checks.on_packet(&bytes, from, now_us()) {
                        let _ = sockets.send_to(&packet, to).await;
                    }
                }
                Err(error) => {
                    errors += 1;
                    if errors > MAX_ERRORS {
                        return Err(error);
                    }
                }
            },
            () = tokio::time::sleep(Duration::from_micros(wait)) => {
                for (packet, to) in checks.tick(now_us()) {
                    let _ = sockets.send_to(&packet, to).await;
                }
            }
        }
        if let Some(change) = checks.take_change() {
            route.note(&match change {
                Some(remote) => format!("direct UDP pair valid to {remote}"),
                None => "direct UDP pair lost its consent: QUIC datagrams".to_string(),
            });
            let pair = change.map(|remote| {
                Arc::new(DirectPair {
                    sockets: sockets.clone(),
                    envelope: envelope.clone(),
                    remote,
                }) as Arc<dyn DirectSender>
            });
            route.set_direct(pair);
        }
    }
}

/// The viewer's side, driven by its picture task (`video_viewer.rs`).
pub struct ViewerLink {
    sockets: Arc<Sockets>,
    candidates: Vec<SocketAddr>,
    checks: Option<Checks>,
    errors: u32,
}

impl ViewerLink {
    pub async fn gather(stun: Option<StunServer>) -> io::Result<Self> {
        let sockets = Sockets::bind().await?;
        let candidates = sockets.candidates(stun.as_ref()).await;
        Ok(Self {
            sockets: Arc::new(sockets),
            candidates,
            checks: None,
            errors: 0,
        })
    }

    pub fn candidates(&self) -> Vec<String> {
        self.candidates.iter().map(SocketAddr::to_string).collect()
    }

    pub fn sockets(&self) -> Arc<Sockets> {
        self.sockets.clone()
    }

    /// The host's `VIDEO_ANSWER`: its key and candidates start the checks.
    pub fn answer(&mut self, payload: &[u8], now_us: u64) -> io::Result<()> {
        if self.checks.is_some() {
            return Ok(());
        }
        let answer: VideoAnswer = serde_json::from_slice(payload).map_err(io::Error::other)?;
        answer.validate()?;
        let envelope = Envelope::new(&answer.key_bytes()?, answer.session, Side::Viewer)?;
        let seed = u64::from_be_bytes(random()?);
        self.checks = Some(Checks::new(
            Arc::new(envelope),
            answer.candidates(),
            seed,
            now_us,
        ));
        Ok(())
    }

    /// When `tick` has checks to send.
    pub fn due_us(&self) -> Option<u64> {
        self.checks.as_ref().map(Checks::due_us)
    }

    /// Whether a pair is valid now.
    pub fn direct(&self) -> bool {
        self.checks
            .as_ref()
            .is_some_and(|checks| checks.selected().is_some())
    }

    pub async fn tick(&mut self, now_us: u64) {
        if let Some(checks) = self.checks.as_mut() {
            for (packet, to) in checks.tick(now_us) {
                let _ = self.sockets.send_to(&packet, to).await;
            }
        }
    }

    /// A datagram of the sockets: a picture packet comes back, a check is
    /// answered here. `false` in the second place: the sockets keep
    /// failing and the pair is given up.
    pub async fn on_datagram(
        &mut self,
        got: io::Result<(Vec<u8>, SocketAddr)>,
        now_us: u64,
    ) -> (Option<Vec<u8>>, bool) {
        let (bytes, from) = match got {
            Ok(got) => got,
            Err(_) => {
                self.errors += 1;
                return (None, self.errors <= MAX_ERRORS);
            }
        };
        self.errors = 0;
        let Some(checks) = self.checks.as_mut() else {
            return (None, true);
        };
        match checks.on_packet(&bytes, from, now_us) {
            Incoming::Picture(inner) => (Some(inner), true),
            Incoming::Reply { packet, to } => {
                let _ = self.sockets.send_to(&packet, to).await;
                (None, true)
            }
            Incoming::Nothing => (None, true),
        }
    }
}
