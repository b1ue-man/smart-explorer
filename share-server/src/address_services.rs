//! The address services beside signaling (user 2026-11-25: „TCP und UDP,
//! DIREKT“): the TCP address reflector (`tcp_reflect.rs`, TCP 51823) and the
//! STUN Binding responder (`stun.rs`, UDP 3478). Each tells a device the
//! public address its NAT gives one local port, so two devices connect
//! directly instead of over the relay: TCP by simultaneous open, UDP by hole
//! punching. Like QAD they improve the route and guard nothing: a service
//! that cannot bind stays off with a log line and the server runs on.
//!
//! A device offers `tcp_reflect_v1` and `stun_v1` in its hello; `hello_ok`
//! lists a running service bare on its default port or as `name:port`
//! (`announce`), and the device reaches it at the host of the Share address
//! it connected to.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::OnceLock;

use super::{stun, tcp_reflect};

/// Whether and where one address service listens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ServicePlan {
    Listen(SocketAddr),
    /// Off; the text says why.
    Off(String),
}

/// The plan from the value of `env` (`SE_SHARE_TCP_REFLECT_BIND`,
/// `SE_SHARE_STUN_BIND`): absent or empty, the signaling IP with
/// `default_port`, where a signaling bind on `0.0.0.0` becomes the
/// dual-stack `[::]` (as QAD's, `relay_qad.rs`); `off` turns the service
/// off; otherwise an `IP:PORT`. An invalid value leaves the service off with
/// its reason, as an invalid `SE_IROH_QAD_BIND` does.
pub(super) fn plan(
    env: &str,
    value: Option<&str>,
    signal: SocketAddr,
    default_port: u16,
) -> ServicePlan {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => {
            let ip = match signal.ip() {
                IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
                ip => ip,
            };
            ServicePlan::Listen(SocketAddr::new(ip, default_port))
        }
        Some(value) if value.eq_ignore_ascii_case("off") => {
            ServicePlan::Off(format!("disabled via {env}=off"))
        }
        Some(value) => match value.parse::<SocketAddr>() {
            Ok(address) => ServicePlan::Listen(address),
            Err(error) => ServicePlan::Off(format!("invalid {env} {value}: {error}")),
        },
    }
}

/// Binds `address`. For `[::]` it also binds `0.0.0.0` on the same port
/// where the OS allows that: Windows (and Linux with `bindv6only=1`) keeps
/// `[::]` to IPv6, while a dual-stack Linux `[::]` already takes IPv4 and
/// refuses the second bind. A host without IPv6 gets `0.0.0.0` alone. The
/// first socket is the one whose port is announced.
pub(super) fn bind_all<T>(
    address: SocketAddr,
    bind: impl Fn(SocketAddr) -> std::io::Result<T>,
) -> Result<Vec<T>, String> {
    if address.ip() != IpAddr::V6(Ipv6Addr::UNSPECIFIED) {
        return bind(address)
            .map(|bound| vec![bound])
            .map_err(|error| format!("cannot bind {address}: {error}"));
    }
    let ipv4 = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), address.port());
    match bind(address) {
        Ok(first) => {
            let mut bound = vec![first];
            bound.extend(bind(ipv4).ok());
            Ok(bound)
        }
        Err(error) => bind(ipv4)
            .map(|bound| vec![bound])
            .map_err(|fallback| format!("cannot bind {address} ({error}) or {ipv4}: {fallback}")),
    }
}

static REFLECT_PORT: OnceLock<u16> = OnceLock::new();
static STUN_PORT: OnceLock<u16> = OnceLock::new();

/// Starts both services as planned; what runs is announced from then on.
pub(super) fn start(reflect: &ServicePlan, stun_plan: &ServicePlan) {
    if let Some(port) = run(reflect, "TCP address reflector (TCP)", tcp_reflect::start) {
        let _ = REFLECT_PORT.set(port);
    }
    if let Some(port) = run(stun_plan, "STUN responder (UDP)", stun::start) {
        let _ = STUN_PORT.set(port);
    }
}

fn run(
    plan: &ServicePlan,
    name: &str,
    start: impl Fn(SocketAddr) -> Result<SocketAddr, String>,
) -> Option<u16> {
    match plan {
        ServicePlan::Off(reason) => {
            eprintln!("se-share-server: {name} off: {reason}");
            None
        }
        ServicePlan::Listen(address) => match start(*address) {
            Ok(bound) => {
                eprintln!("se-share-server {name} listening on {bound}");
                Some(bound.port())
            }
            Err(reason) => {
                eprintln!("se-share-server: {name} off: {reason}");
                None
            }
        },
    }
}

/// `capability` as `hello_ok` lists it; `None` for an address service
/// that does not run in this process.
pub(super) fn announce(capability: &str) -> Option<String> {
    announce_with(
        capability,
        REFLECT_PORT.get().copied(),
        STUN_PORT.get().copied(),
    )
}

pub(super) fn announce_with(
    capability: &str,
    reflect: Option<u16>,
    stun_port: Option<u16>,
) -> Option<String> {
    let (port, default) = match capability {
        tcp_reflect::CAPABILITY => (reflect, tcp_reflect::DEFAULT_PORT),
        stun::CAPABILITY => (stun_port, stun::DEFAULT_PORT),
        _ => return Some(capability.to_string()),
    };
    match port? {
        port if port == default => Some(capability.to_string()),
        port => Some(format!("{capability}:{port}")),
    }
}

#[cfg(test)]
#[path = "address_services_tests.rs"]
mod tests;
