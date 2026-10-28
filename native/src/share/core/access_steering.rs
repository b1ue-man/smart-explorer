//! What „Bildschirm steuern: erlauben“ includes (user rule 2026-10-27: „wenn
//! bildschirmfreigabe mit steuerung ist alles andere auch erlaubt … auch ohne
//! bestätigungen auf nem anderen gerät“). A device that may steer this screen
//! may also do what steering reaches anyway, without a separate grant and
//! without a confirmation on this host: watch the screen and list its windows,
//! start programs, use the machine login and reach this host's own RDP
//! service. Commands and the terminal stay separate rights; other TCP targets
//! keep their own grant.
//!
//! Since 0.5.189 (design 4.2, user rule r5 „wenn auf localhost was offen ist
//! wird das frei gegeben für andere geräte. per default“) steering also
//! reaches every locally listening service this host lists as exposable:
//! loopback-only listeners of any account, wildcard listeners of a person's
//! account, never se's own ports. A steering device already operates the
//! session that reaches these ports with every program; the tunnel adds no
//! reach the one grant does not have.
use std::collections::BTreeSet;
use std::sync::RwLock;

use crate::share::access_constraints::{AccessRequest, TcpDirection, TcpHost};
use crate::share::access_grant::{AccessGrant, Capability, CapabilityDecision};

/// Listener ports of this host's own RDP services, as its last RDP probe
/// proved them (`rdp::probe_local_host`).
static OWN_RDP_PORTS: RwLock<Vec<u16>> = RwLock::new(Vec::new());

/// Records the RDP listener ports a probe of this host found.
pub(crate) fn note_own_rdp_ports(ports: impl IntoIterator<Item = u16>) {
    if let Ok(mut own) = OWN_RDP_PORTS.write() {
        *own = ports.into_iter().collect();
    }
}

fn own_rdp_port(port: u16) -> bool {
    OWN_RDP_PORTS.read().is_ok_and(|own| own.contains(&port))
}

/// The RDP listener ports this host knows (excluded from the automatic
/// service list: they run through „Remote Desktop“).
pub(crate) fn own_rdp_ports() -> BTreeSet<u16> {
    OWN_RDP_PORTS
        .read()
        .map(|own| own.iter().copied().collect())
        .unwrap_or_default()
}

/// Port → listener process of the host's RDP services at their last probe
/// (design 2.7 point 4: the host checks this ownership per connection).
static OWN_RDP_LISTENERS: RwLock<Vec<(u16, u32)>> = RwLock::new(Vec::new());

/// Records the listener PIDs a probe proved for the RDP ports.
pub(crate) fn note_own_rdp_listeners(listeners: impl IntoIterator<Item = (u16, u32)>) {
    if let Ok(mut own) = OWN_RDP_LISTENERS.write() {
        *own = listeners.into_iter().collect();
    }
}

/// The process that owned `port` when the RDP probe proved it; `None` when
/// `port` is no RDP port of this host.
pub(crate) fn own_rdp_listener(port: u16) -> Option<u32> {
    OWN_RDP_LISTENERS
        .read()
        .ok()?
        .iter()
        .find(|(own, _)| *own == port)
        .map(|(_, pid)| *pid)
}

/// Ports of locally listening services that steering exposes, as the host's
/// last service listing found them (design 4.2).
static EXPOSED_PORTS: RwLock<BTreeSet<u16>> = RwLock::new(BTreeSet::new());

/// Replaces the exposed service ports after a fresh listing.
pub(crate) fn note_exposed_ports(ports: impl IntoIterator<Item = u16>) {
    if let Ok(mut exposed) = EXPOSED_PORTS.write() {
        *exposed = ports.into_iter().collect();
    }
}

fn exposed_loopback_port(port: u16) -> bool {
    EXPOSED_PORTS
        .read()
        .is_ok_and(|exposed| exposed.contains(&port))
}

/// The loopback port of the background service's own IPC (T-L3); the daemon
/// notes it when it binds. Excluded from every service list.
static IPC_PORTS: RwLock<BTreeSet<u16>> = RwLock::new(BTreeSet::new());

/// Called by the daemon's IPC bind (`daemon::ipc_listener::start_listener`).
pub(crate) fn note_ipc_port(port: u16) {
    if let Ok(mut ports) = IPC_PORTS.write() {
        ports.insert(port);
    }
}

/// Ports never exposed automatically: RDP (own action) and se's IPC.
pub(crate) fn excluded_ports() -> BTreeSet<u16> {
    let mut excluded = own_rdp_ports();
    if let Ok(ports) = IPC_PORTS.read() {
        excluded.extend(ports.iter().copied());
    }
    excluded
}

/// Devices (device ID, public key) that currently may steer this screen in
/// at least one active relation; T3 of the RDP self-setup asks here.
static STEERING_DEVICES: RwLock<BTreeSet<(String, String)>> = RwLock::new(BTreeSet::new());

pub(crate) fn note_steering_devices(devices: BTreeSet<(String, String)>) {
    if let Ok(mut current) = STEERING_DEVICES.write() {
        *current = devices;
    }
}

pub(crate) fn steering_device(device_id: &str, public_key: &str) -> bool {
    let key = (device_id.to_string(), public_key.to_string());
    STEERING_DEVICES
        .read()
        .is_ok_and(|devices| devices.contains(&key))
}

/// Whether `grant` allows steering the screen at `now`.
pub(crate) fn steers(grant: &AccessGrant, now: i64) -> bool {
    !grant.expired_at(now) && grant.decision(Capability::ScreenControl) == CapabilityDecision::Allow
}

/// Whether steering includes `capability` without a further context.
pub(crate) fn includes(capability: Capability) -> bool {
    capability == Capability::ScreenView
}

/// Whether steering includes the concrete `request`.
pub(crate) fn includes_request(request: &AccessRequest) -> bool {
    match request {
        AccessRequest::AppLaunch(_) | AccessRequest::LoginAccess(_) => true,
        AccessRequest::TcpForward(request) => {
            request.direction == TcpDirection::PeerToHost
                && request.host == TcpHost::Loopback
                && request.resolved_ip.is_loopback()
                && (own_rdp_port(request.port.get()) || exposed_loopback_port(request.port.get()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::access_constraints::{NodeId, TcpForwardRequest, TcpPort};

    fn rdp(port: u16, host: TcpHost, direction: TcpDirection) -> AccessRequest {
        let node = |byte: char| NodeId::parse(&byte.to_string().repeat(64)).expect("node id");
        AccessRequest::TcpForward(TcpForwardRequest {
            initiator: node('a'),
            target: node('b'),
            direction,
            host,
            resolved_ip: std::net::IpAddr::from([127, 0, 0, 1]),
            port: TcpPort::try_from(port).expect("port"),
        })
    }

    #[test]
    fn screen_metadata_runtime_task_steering_includes_view_and_own_rdp() {
        let mut grant = AccessGrant::default();
        assert!(!steers(&grant, 0));
        grant.set(Capability::ScreenControl, CapabilityDecision::Allow);
        assert!(steers(&grant, 0));
        assert_eq!(
            grant.decision_at(Capability::ScreenView, 0),
            CapabilityDecision::Allow
        );
        assert_eq!(
            grant.decision_at(Capability::ExecCommands, 0),
            CapabilityDecision::Off
        );
        note_own_rdp_ports([3390]);
        let own = rdp(3390, TcpHost::Loopback, TcpDirection::PeerToHost);
        assert_eq!(grant.decision_for(&own, 0), CapabilityDecision::Allow);
        for other in [
            rdp(22, TcpHost::Loopback, TcpDirection::PeerToHost),
            rdp(3390, TcpHost::Loopback, TcpDirection::HostToPeer),
        ] {
            assert_eq!(grant.decision_for(&other, 0), CapabilityDecision::Off);
        }
        grant.set(Capability::ScreenControl, CapabilityDecision::Ask);
        assert_eq!(grant.decision_for(&own, 0), CapabilityDecision::Off);
    }
}
