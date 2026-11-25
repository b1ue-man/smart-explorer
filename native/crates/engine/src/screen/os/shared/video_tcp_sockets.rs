//! Sockets of the direct TCP picture link (`video_tcp_link.rs`): one
//! listener per address family on a port the OS picks, dial sockets bound to
//! that same port, and the gathering of the candidates (interfaces, the Share
//! server's TCP reflector asked from that port, the Iroh endpoint's public
//! IPv4).
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tokio::time::Instant;

use super::port_share::share_port;
use super::{GatherPlan, TcpLinkFailure, REFLECT_TIMEOUT};
use crate::screen::core::video_tcp_wire::{
    local_candidates, merge_candidates, predicted_candidates, TcpCandidate, TcpFailureReason,
};
use crate::share::tcp_reflect_client;

/// Pending connections a listener queues.
const BACKLOG: u32 = 32;

/// One family's listener and the address it is bound to (unspecified IP,
/// the shared port).
pub(super) struct Bound {
    pub(super) listener: TcpListener,
    pub(super) addr: SocketAddr,
}

/// A side's sockets and candidates between gathering and `establish`.
pub(super) struct LocalSockets {
    pub(super) v4: Option<Bound>,
    pub(super) v6: Option<Bound>,
    pub(super) candidates: Vec<TcpCandidate>,
    pub(super) reflect_error: Option<String>,
    /// The reflector stream, open until the link is up (NAT mapping).
    pub(super) reflector: Option<TcpStream>,
}

fn new_socket(ip: IpAddr) -> io::Result<TcpSocket> {
    let socket = match ip {
        IpAddr::V4(_) => TcpSocket::new_v4()?,
        IpAddr::V6(_) => TcpSocket::new_v6()?,
    };
    share_port(&socket)?;
    socket.set_nodelay(true)?;
    Ok(socket)
}

fn listen(ip: IpAddr) -> io::Result<Bound> {
    let socket = new_socket(ip)?;
    socket.bind(SocketAddr::new(ip, 0))?;
    let addr = socket.local_addr()?;
    Ok(Bound {
        listener: socket.listen(BACKLOG)?,
        addr,
    })
}

/// A socket bound to a listener's address, for one dial attempt.
pub(super) fn dial_socket(bound: SocketAddr) -> io::Result<TcpSocket> {
    let socket = new_socket(bound.ip())?;
    socket.bind(bound)?;
    Ok(socket)
}

fn local_failure(detail: String) -> TcpLinkFailure {
    TcpLinkFailure::new(TcpFailureReason::LocalSocket, Some(detail))
}

/// Opens the listeners and gathers the candidates. Only both listeners
/// failing fails; a missing reflector answer or interface list costs only
/// those candidates.
pub(super) async fn gather(
    plan: &GatherPlan,
    deadline: std::time::Instant,
) -> Result<LocalSockets, TcpLinkFailure> {
    let deadline = Instant::from_std(deadline);
    let v4 = listen(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    let v6 = listen(IpAddr::V6(Ipv6Addr::UNSPECIFIED));
    let (v4, v6) = match (v4, v6) {
        (Err(error), Err(_)) => return Err(local_failure(error.to_string())),
        (v4, v6) => (v4.ok(), v6.ok()),
    };
    let v4_port = v4.as_ref().map(|bound| bound.addr.port());
    let v6_port = v6.as_ref().map(|bound| bound.addr.port());
    // The OS adapters read system tables (Windows: GetAdaptersAddresses).
    let interfaces = tokio::task::spawn_blocking(crate::net::gather_interface_facts)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let local = local_candidates(&interfaces, v4_port, v6_port);
    let (reflected, reflector, reflect_error) = match (&plan.reflector, &v4) {
        (Some(target), Some(bound)) => {
            let end = (Instant::now() + REFLECT_TIMEOUT).min(deadline);
            match ask_reflector(&target.host, target.port, bound.addr, end).await {
                Ok((observed, stream)) => (Some(observed), Some(stream), None),
                Err(error) => (None, None, Some(error.to_string())),
            }
        }
        (Some(_), None) => (None, None, Some("kein IPv4-Port offen".to_string())),
        (None, _) => (None, None, None),
    };
    let predicted = predicted_candidates(&plan.endpoint_addrs, v4_port);
    Ok(LocalSockets {
        v4,
        v6,
        candidates: merge_candidates(local, reflected, predicted),
        reflect_error,
        reflector,
    })
}

/// The public address the reflector sees for `bound`'s port, asked from
/// that port over IPv4 (an IPv6 address is the device's own already).
async fn ask_reflector(
    host: &str,
    port: u16,
    bound: SocketAddr,
    deadline: Instant,
) -> io::Result<(SocketAddr, TcpStream)> {
    let server = tcp_reflect_client::resolve(host, port, deadline)
        .await?
        .into_iter()
        .find(|addr| addr.ip().to_canonical().is_ipv4())
        .map(|addr| SocketAddr::new(addr.ip().to_canonical(), addr.port()))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Share-Server ohne IPv4-Adresse"))?;
    let (observed, stream) =
        tcp_reflect_client::reflect(dial_socket(bound)?, server, deadline).await?;
    let observed = SocketAddr::new(observed.ip().to_canonical(), observed.port());
    Ok((observed, stream))
}

impl LocalSockets {
    /// Test sockets: an IPv4 listener on 127.0.0.1, its one candidate.
    #[cfg(test)]
    pub(super) fn loopback() -> io::Result<Self> {
        let bound = listen(IpAddr::V4(Ipv4Addr::LOCALHOST))?;
        let candidates = vec![TcpCandidate {
            addr: bound.addr,
            kind: crate::screen::core::video_tcp_wire::CandidateKind::Local,
        }];
        Ok(Self {
            v4: Some(bound),
            v6: None,
            candidates,
            reflect_error: None,
            reflector: None,
        })
    }
}
