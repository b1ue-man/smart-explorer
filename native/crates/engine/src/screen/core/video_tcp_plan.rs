//! Which addresses a side names as TCP candidates of the direct picture link
//! (`video_tcp_wire.rs`) and how a connected path is classified.
//!
//! Candidates: every address of an interface that is up and not loopback,
//! with the listener's port of its family; loopback, unspecified, multicast
//! and broadcast addresses never, an IPv6 link-local address neither (it
//! needs the interface scope of the side that dials, which the peer does not
//! know). An IPv4 link-local address (169.254/16) is kept: it works between
//! two devices on the same link without a scope. Then the public address the
//! Share server's TCP reflector saw and the predicted one (the Iroh
//! endpoint's public IPv4 with the listener's port).
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use super::{CandidateKind, TcpCandidate, TcpPathKind, MAX_TCP_CANDIDATES};
use crate::net::InterfaceFacts;

/// Interface addresses one side names at most; the rest of
/// `MAX_TCP_CANDIDATES` stays for the reflected and predicted addresses.
pub const MAX_LOCAL_CANDIDATES: usize = 12;

fn push_unique(list: &mut Vec<SocketAddr>, addr: SocketAddr) {
    if !list.contains(&addr) {
        list.push(addr);
    }
}

fn link_local_v6(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}

fn shared_v4(ip: Ipv4Addr) -> bool {
    let [first, second, ..] = ip.octets();
    first == 100 && (64..128).contains(&second)
}

fn local_v4(ip: Ipv4Addr) -> bool {
    !(ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() || ip.is_broadcast())
}

fn local_v6(ip: Ipv6Addr) -> bool {
    !(ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() || link_local_v6(ip))
        && ip.to_ipv4_mapped().is_none()
}

/// The interface candidates: LAN IPv4 first (the common same-network
/// case), then other IPv4, then IPv6; at most `MAX_LOCAL_CANDIDATES`. A
/// family without a listener (`None` port) names none.
pub fn local_candidates(
    interfaces: &[InterfaceFacts],
    v4_port: Option<u16>,
    v6_port: Option<u16>,
) -> Vec<TcpCandidate> {
    let mut v4 = Vec::new();
    let mut v6 = Vec::new();
    for interface in interfaces.iter().filter(|face| face.up && !face.loopback) {
        for ip in &interface.addrs {
            match (ip.to_canonical(), v4_port, v6_port) {
                (IpAddr::V4(ip), Some(port), _) if local_v4(ip) => {
                    push_unique(&mut v4, SocketAddr::new(IpAddr::V4(ip), port));
                }
                (IpAddr::V6(ip), _, Some(port)) if local_v6(ip) => {
                    push_unique(&mut v6, SocketAddr::new(IpAddr::V6(ip), port));
                }
                _ => {}
            }
        }
    }
    v4.sort_by_key(|addr| !is_lan_ip(addr.ip()));
    v4.into_iter()
        .chain(v6)
        .take(MAX_LOCAL_CANDIDATES)
        .map(|addr| TcpCandidate {
            addr,
            kind: CandidateKind::Local,
        })
        .collect()
}

/// The predicted candidates: every public IPv4 among the Iroh endpoint's
/// direct addresses (`Endpoint::addr().ip_addrs()`, which carry what QAD
/// learned) with the IPv4 listener's port. Right when the NAT keeps the
/// source port; otherwise the reflected candidate covers it.
pub fn predicted_candidates(
    endpoint_addrs: &[SocketAddr],
    v4_port: Option<u16>,
) -> Vec<SocketAddr> {
    let Some(port) = v4_port else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for addr in endpoint_addrs {
        if let IpAddr::V4(ip) = addr.ip().to_canonical() {
            if is_public_v4(ip) {
                push_unique(&mut out, SocketAddr::new(IpAddr::V4(ip), port));
            }
        }
    }
    out
}

/// All candidates of one side in dial order: interface, reflected,
/// predicted; an address named twice keeps its first kind; at most
/// `MAX_TCP_CANDIDATES`.
pub fn merge_candidates(
    local: Vec<TcpCandidate>,
    reflected: Option<SocketAddr>,
    predicted: Vec<SocketAddr>,
) -> Vec<TcpCandidate> {
    let reflected = reflected.map(|addr| TcpCandidate {
        addr,
        kind: CandidateKind::Reflected,
    });
    let predicted = predicted.into_iter().map(|addr| TcpCandidate {
        addr,
        kind: CandidateKind::Predicted,
    });
    let mut out: Vec<TcpCandidate> = Vec::new();
    for candidate in local.into_iter().chain(reflected).chain(predicted) {
        if out.len() == MAX_TCP_CANDIDATES {
            break;
        }
        if !out.iter().any(|known| known.addr == candidate.addr) {
            out.push(candidate);
        }
    }
    out
}

/// A routable public IPv4 address: none of unspecified, `0/8`, loopback,
/// private (RFC 1918), shared (`100.64/10`, RFC 6598), link-local,
/// multicast, reserved (`240/4`) or broadcast.
pub fn is_public_v4(ip: Ipv4Addr) -> bool {
    let first = ip.octets()[0];
    !(ip.is_unspecified()
        || first == 0
        || ip.is_loopback()
        || ip.is_private()
        || shared_v4(ip)
        || ip.is_link_local()
        || ip.is_multicast()
        || first >= 240)
}

/// An address of the own network side: IPv4 private, shared, link-local or
/// loopback; IPv6 unique local (`fc00::/7`), link-local or loopback.
pub fn is_lan_ip(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => {
            ip.is_private() || shared_v4(ip) || ip.is_link_local() || ip.is_loopback()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00 || link_local_v6(ip)
        }
    }
}

/// The path of a connected link from this side's view: a LAN peer address
/// is `Lan`, a global IPv6 one `Ipv6`; a public IPv4 peer is `Public` when
/// this side's own address is public too (no NAT here), else `Punched`.
pub fn classify_path(local: SocketAddr, remote: SocketAddr) -> TcpPathKind {
    let remote = remote.ip().to_canonical();
    if is_lan_ip(remote) {
        return TcpPathKind::Lan;
    }
    match (remote, local.ip().to_canonical()) {
        (IpAddr::V6(_), _) => TcpPathKind::Ipv6,
        (IpAddr::V4(_), IpAddr::V4(own)) if is_public_v4(own) => TcpPathKind::Public,
        (IpAddr::V4(_), _) => TcpPathKind::Punched,
    }
}
