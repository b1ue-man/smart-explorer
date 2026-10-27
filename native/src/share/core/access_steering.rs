//! What „Bildschirm steuern: erlauben“ includes (user rule 2026-10-27: „wenn
//! bildschirmfreigabe mit steuerung ist alles andere auch erlaubt … auch ohne
//! bestätigungen auf nem anderen gerät“). A device that may steer this screen
//! may also do what steering reaches anyway, without a separate grant and
//! without a confirmation on this host: watch the screen and list its windows,
//! start programs, use the machine login and reach this host's own RDP
//! service. Commands and the terminal stay separate rights; other TCP targets
//! keep their own grant.
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
                && own_rdp_port(request.port.get())
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
    fn steering_task_control_includes_view_and_own_rdp_only() {
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
