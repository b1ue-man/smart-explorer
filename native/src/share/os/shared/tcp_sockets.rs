//! Host-facing socket boundary: fresh DNS lookup, numeric socket binding,
//! loopback-only listeners. No target hostname reaches TcpStream::connect.
use crate::share::access_constraints::TcpHost;
use crate::share::access_revocation::AccessHandle;
use crate::share::loopback_services::LoopbackService;
use crate::share::tcp_forward::{TcpDestination, TcpStart};
use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::net::{TcpListener, TcpStream};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(test)]
#[path = "batches_tcp_task_tests.rs"]
mod batches_tcp_task_tests;

pub(crate) async fn resolve(destination: &TcpDestination) -> io::Result<Vec<SocketAddr>> {
    destination.validate().map_err(io::Error::other)?;
    let port = destination.port.get();
    let mut addresses: Vec<_> = match &destination.host {
        TcpHost::Loopback => vec![
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port),
        ],
        TcpHost::Ip { address } => vec![SocketAddr::new(*address, port)],
        TcpHost::Dns { name, .. } => tokio::time::timeout(
            CONNECT_TIMEOUT,
            tokio::net::lookup_host((name.as_str(), port)),
        )
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "TCP destination DNS lookup timed out",
            )
        })??
        .take(64)
        .collect(),
    };
    addresses.retain(|address| !address.ip().is_unspecified() && !address.ip().is_multicast());
    addresses.sort();
    addresses.dedup();
    if addresses.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "TCP destination has no unicast address",
        ));
    }
    Ok(addresses)
}
/// The session's admitted IP is immutable. A changed DNS response must not
/// transfer an Ask approval to a different IP, even inside the same rule.
pub(crate) struct AuthorizedSocket {
    // Fields drop in declaration order: the real socket closes before the
    // final authority reference can unregister its lease.
    socket: TcpStream,
    admitted: SocketAddr,
    _access: Arc<AccessHandle>,
}
impl AuthorizedSocket {
    pub(crate) fn stream(&mut self) -> &mut TcpStream {
        &mut self.socket
    }
}
pub(crate) async fn connect_bound(
    destination: &TcpDestination,
    admitted: SocketAddr,
    access: Arc<AccessHandle>,
) -> io::Result<AuthorizedSocket> {
    connect_bound_reusing(destination, admitted, access, None).await
}

/// The first successful authorized connection is not discarded as a probe.
/// Reuse preserves one-shot services and any already queued server greeting.
/// Later streams always connect to the same admitted numeric endpoint.
pub(crate) async fn connect_bound_reusing(
    destination: &TcpDestination,
    admitted: SocketAddr,
    access: Arc<AccessHandle>,
    initial: Option<AuthorizedSocket>,
) -> io::Result<AuthorizedSocket> {
    let mut revoked = access.revoked();
    let connect = async move {
        let current = resolve(destination).await?;
        if !current.contains(&admitted) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "TCP destination DNS changed; reopen the tunnel to authorize the new address",
            ));
        }
        if access.reason().is_some() {
            return Err(access_revoked());
        }
        let retained = match initial {
            Some(initial) => {
                if !Arc::ptr_eq(&access, &initial._access) || initial.admitted != admitted {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "TCP retained socket authority changed",
                    ));
                }
                // A queued error/reset requires a fresh connection to this IP.
                // A read FIN alone is a valid half-close: do not discard the
                // remaining write direction or consume a greeting to test it.
                if initial.socket.take_error()?.is_some() || initial.socket.peer_addr().is_err() {
                    None
                } else {
                    Some(initial.socket)
                }
            }
            None => None,
        };
        verify_rdp_listener(destination, admitted).await?;
        let stream = match retained {
            Some(stream) => stream,
            None => tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(admitted))
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "TCP destination connect timed out")
                })??,
        };
        if access.reason().is_some() {
            return Err(access_revoked());
        }
        if stream.peer_addr()? != admitted {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "TCP destination socket identity changed",
            ));
        }
        stream.set_nodelay(true)?;
        Ok(AuthorizedSocket {
            socket: stream,
            admitted,
            _access: access,
        })
    };
    tokio::select! {
        biased;
        () = revoked_signal(&mut revoked) => Err(access_revoked()),
        result = connect => result,
    }
}

/// Design 2.7 point 4: a loopback connection to one of this host's RDP
/// ports is forwarded only while the RDP service process owns that listener.
/// The owner noted by the last probe is checked first; on a mismatch (the
/// service restarted with a new PID) the host probes once more and checks
/// the current owner, so restarts of TermService/xrdp keep RDP and manual
/// TCP forwards to that port working. A foreign process that took the port
/// is still refused (threat T-R4).
async fn verify_rdp_listener(destination: &TcpDestination, admitted: SocketAddr) -> io::Result<()> {
    if destination.host != TcpHost::Loopback || !admitted.ip().is_loopback() {
        return Ok(());
    }
    let port = admitted.port();
    if !crate::share::access_steering::own_rdp_ports().contains(&port) {
        return Ok(());
    }
    if owned_by_noted_listener(port).await? {
        return Ok(());
    }
    // One fresh probe for all connections that found the same mismatch.
    static REPROBE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _turn = REPROBE.lock().await;
    if owned_by_noted_listener(port).await? {
        return Ok(());
    }
    tokio::task::spawn_blocking(crate::rdp::host_setup::refresh_probe)
        .await
        .map_err(io::Error::other)?;
    if owned_by_noted_listener(port).await? {
        return Ok(());
    }
    let owner = crate::share::access_steering::own_rdp_listener(port)
        .map_or_else(|| "kein RDP-Dienst".to_string(), |pid| format!("PID {pid}"));
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("RDP-Port {port} gehört nicht dem RDP-Dienst ({owner}); Verbindung abgewiesen"),
    ))
}

/// Whether the RDP listener the last probe noted for `port` owns it now.
async fn owned_by_noted_listener(port: u16) -> io::Result<bool> {
    use crate::share::os_sockets;
    let Some(pid) = crate::share::access_steering::own_rdp_listener(port) else {
        return Ok(false);
    };
    tokio::task::spawn_blocking(move || os_sockets::listener_owned_by(port, pid))
        .await
        .map_err(io::Error::other)?
}

/// The local services this host currently exposes under steering (design
/// 4.1/4.2) and the refreshed exposed-port set of `access_steering`. An OS
/// without a readable socket table (Android, assumption A7) reports why.
pub(crate) fn local_services() -> Result<Vec<LoopbackService>, String> {
    use crate::share::{access_steering, loopback_services, os_sockets};
    let rows = match os_sockets::listeners() {
        Ok(rows) => rows,
        Err(error) => {
            // An unknown listing exposes nothing: the old set never stays in
            // force for new tunnels (review S5).
            access_steering::note_exposed_ports([]);
            return Err(format!(
                "Lauschende Dienste dieses Geräts sind nicht ermittelbar: {error}"
            ));
        }
    };
    let own_exe = std::env::current_exe().ok();
    let skip = |pid: u32| se_process(pid, own_exe.as_deref());
    let services = loopback_services::services_from(&rows, skip, os_sockets::process_name);
    let mut excluded = access_steering::excluded_ports();
    excluded.extend(os_sockets::internal_ports());
    access_steering::note_exposed_ports(loopback_services::exposed_ports(&services, &excluded));
    Ok(services
        .into_iter()
        .filter(|service| loopback_services::exposable(service, &excluded))
        .collect())
}

/// se's own listeners (threat T-L3): this process, or any se program whose
/// image this account can read, wherever it is installed.
fn se_process(pid: u32, own_exe: Option<&std::path::Path>) -> bool {
    use crate::share::{loopback_services, os_sockets};
    if pid == std::process::id() {
        return true;
    }
    let image = os_sockets::process_identity(pid)
        .ok()
        .and_then(|identity| identity.image);
    match (image, own_exe) {
        (Some(image), Some(own_exe)) => loopback_services::is_se_image(&image, own_exe),
        _ => false,
    }
}

/// Why `address:port` could not be bound for an automatic mapping. The
/// device address follows from public identity data, so another account of
/// this computer can take the port first (review S6): then the stable
/// address names that account's program, not the device, and says so.
pub(crate) fn taken_note(address: Ipv4Addr, port: u16) -> &'static str {
    use crate::share::loopback_services::OwnerClass;
    use crate::share::os_sockets;
    let (Ok(own), Ok(rows)) = (os_sockets::own_account(), os_sockets::listeners()) else {
        return "Port belegt";
    };
    let mine = |owner: &OwnerClass| match owner {
        OwnerClass::NormalUser { uid_or_sid } => *uid_or_sid == own,
        OwnerClass::System => own == "0",
        OwnerClass::Unknown => false,
    };
    let foreign = rows
        .iter()
        .filter(|row| row.local.port() == port)
        .filter(|row| row.local.ip() == IpAddr::V4(address) || row.local.ip().is_unspecified())
        .any(|row| !mine(&row.owner));
    if foreign {
        TAKEN_BY_OTHER_ACCOUNT
    } else {
        "Port belegt"
    }
}

const TAKEN_BY_OTHER_ACCOUNT: &str =
    "Port von einem anderen Konto dieses Computers belegt: dort antwortet nicht das Gerät";

fn access_revoked() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "TCP access revoked before socket use",
    )
}
async fn revoked_signal(
    revoked: &mut tokio::sync::watch::Receiver<
        Option<crate::share::access_revocation::AccessRevocation>,
    >,
) {
    loop {
        if revoked.borrow().is_some() {
            return;
        }
        if revoked.changed().await.is_err() {
            return;
        }
    }
}
pub(crate) async fn listen(request: &TcpStart) -> io::Result<TcpListener> {
    request.validate().map_err(io::Error::other)?;
    TcpListener::bind(request.listener()).await
}
