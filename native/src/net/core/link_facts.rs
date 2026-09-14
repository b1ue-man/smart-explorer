//! Platform-independent view of the host's network links.
//!
//! The OS adapters (`net/os/*/interfaces.rs`) only gather typed facts; the
//! classification here decides which link is a router-less direct connection
//! (cable or dumb switch, no gateway, no DHCP lease), which one carries the
//! internet, and which is an ordinary routed network.
use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InterfaceFacts {
    /// `eth0` on Linux, the friendly name on Windows.
    pub name: String,
    /// Stable OS handle: the interface name on Linux, the adapter GUID on
    /// Windows (what ICS and scope ids need).
    pub adapter_id: String,
    pub index: u32,
    pub up: bool,
    pub loopback: bool,
    pub addrs: Vec<IpAddr>,
    pub has_gateway: bool,
    /// `None` when the platform cannot tell whether a DHCP lease is active.
    pub dhcp_lease: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkClass {
    /// Up, no default gateway, no DHCP lease: a direct cable or dumb switch.
    RouterLess,
    /// Up with a default gateway that reaches the internet.
    Uplink,
    /// Up on an ordinary routed network without (known) internet.
    Routed,
    /// Down or loopback.
    Inactive,
}

impl LinkClass {
    pub fn label(self) -> &'static str {
        match self {
            LinkClass::RouterLess => "ohne Router (direkt)",
            LinkClass::Uplink => "Internetzugang",
            LinkClass::Routed => "Netzwerk mit Router",
            LinkClass::Inactive => "inaktiv",
        }
    }
}

pub fn is_link_local(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.octets()[0] == 169 && v4.octets()[1] == 254,
        IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
}

/// Decide every interface's class.
///
/// * `peer_ifaces`: interfaces where a paired Smart Explorer peer announced
///   itself; such a link never counts as uplink, so a device whose only
///   gateway is the sharing peer does not report "has internet" back.
/// * `internet_ifaces`: interfaces the platform confirmed as internet
///   connected; `None` when the platform offers no verdict, in which case a
///   default gateway alone counts.
/// * `shared_ifaces`: interfaces this host is currently sharing its uplink
///   on; they keep the `RouterLess` class although ICS/NetworkManager gave
///   them a static gateway address.
pub fn classify_links(
    facts: &[InterfaceFacts],
    peer_ifaces: &[u32],
    internet_ifaces: Option<&[u32]>,
    shared_ifaces: &[u32],
) -> Vec<(InterfaceFacts, LinkClass)> {
    facts
        .iter()
        .map(|iface| {
            let class = if !iface.up || iface.loopback {
                LinkClass::Inactive
            } else if shared_ifaces.contains(&iface.index) {
                LinkClass::RouterLess
            } else if !iface.has_gateway && iface.dhcp_lease != Some(true) {
                LinkClass::RouterLess
            } else if iface.has_gateway
                && !peer_ifaces.contains(&iface.index)
                && internet_ifaces.is_none_or(|set| set.contains(&iface.index))
            {
                LinkClass::Uplink
            } else {
                LinkClass::Routed
            };
            (iface.clone(), class)
        })
        .collect()
}

/// `"[fe80::1%3]:4433"` — a link-local IPv6 candidate bound to one local
/// interface. Global addresses use the plain `"[ip]:port"` form.
pub fn scoped_v6_candidate(ip: Ipv6Addr, port: u16, scope_id: u32) -> String {
    if (ip.segments()[0] & 0xffc0) == 0xfe80 && scope_id != 0 {
        format!("[{ip}%{scope_id}]:{port}")
    } else {
        format!("[{ip}]:{port}")
    }
}

/// Parse a candidate string, including the `%<scope>` form Rust's standard
/// parser rejects. Returns `None` for anything malformed.
pub fn parse_candidate(text: &str) -> Option<SocketAddr> {
    let text = text.trim();
    if let Ok(addr) = text.parse::<SocketAddr>() {
        return Some(addr);
    }
    let inner = text.strip_prefix('[')?;
    let (host, port) = inner.rsplit_once("]:")?;
    let port: u16 = port.parse().ok()?;
    let (ip, scope) = host.split_once('%')?;
    let ip: Ipv6Addr = ip.parse().ok()?;
    let scope: u32 = scope.parse().ok()?;
    Some(SocketAddr::V6(SocketAddrV6::new(ip, port, 0, scope)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn iface(index: u32, up: bool, addrs: &[&str], has_gateway: bool, dhcp: Option<bool>) -> InterfaceFacts {
        InterfaceFacts {
            name: format!("if{index}"),
            adapter_id: format!("id{index}"),
            index,
            up,
            loopback: false,
            addrs: addrs.iter().map(|ip| ip.parse().unwrap()).collect(),
            has_gateway,
            dhcp_lease: dhcp,
        }
    }

    #[test]
    fn apipa_without_gateway_is_router_less() {
        let facts = [iface(2, true, &["169.254.10.5", "fe80::1"], false, Some(false))];
        let classes = classify_links(&facts, &[], None, &[]);
        assert_eq!(classes[0].1, LinkClass::RouterLess);
    }

    #[test]
    fn static_addresses_without_gateway_are_router_less_too() {
        let facts = [iface(3, true, &["10.0.0.2"], false, None)];
        assert_eq!(classify_links(&facts, &[], None, &[])[0].1, LinkClass::RouterLess);
    }

    #[test]
    fn dhcp_lease_without_gateway_is_routed() {
        let facts = [iface(3, true, &["192.168.1.7"], false, Some(true))];
        assert_eq!(classify_links(&facts, &[], None, &[])[0].1, LinkClass::Routed);
    }

    #[test]
    fn gateway_is_uplink_unless_platform_denies_internet() {
        let facts = [iface(4, true, &["192.168.1.7"], true, Some(true))];
        assert_eq!(classify_links(&facts, &[], None, &[])[0].1, LinkClass::Uplink);
        assert_eq!(classify_links(&facts, &[], Some(&[4]), &[])[0].1, LinkClass::Uplink);
        assert_eq!(classify_links(&facts, &[], Some(&[]), &[])[0].1, LinkClass::Routed);
    }

    #[test]
    fn a_link_with_a_paired_peer_never_counts_as_uplink() {
        // The receiving side got DHCP from the sharing peer: its only gateway
        // is the peer, which must not read as "has own internet".
        let facts = [iface(5, true, &["192.168.137.20"], true, Some(true))];
        assert_eq!(classify_links(&facts, &[5], None, &[])[0].1, LinkClass::Routed);
    }

    #[test]
    fn shared_and_inactive_links() {
        let facts = [
            iface(6, true, &["192.168.137.1"], false, Some(false)),
            iface(7, false, &[], false, None),
            InterfaceFacts {
                loopback: true,
                up: true,
                ..iface(1, true, &["127.0.0.1"], false, None)
            },
        ];
        let classes = classify_links(&facts, &[], None, &[6]);
        assert_eq!(classes[0].1, LinkClass::RouterLess);
        assert_eq!(classes[1].1, LinkClass::Inactive);
        assert_eq!(classes[2].1, LinkClass::Inactive);
    }

    #[test]
    fn link_local_detection() {
        assert!(is_link_local(&IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1))));
        assert!(!is_link_local(&IpAddr::V4(Ipv4Addr::new(169, 253, 1, 1))));
        assert!(is_link_local(&"fe80::abcd".parse().unwrap()));
        assert!(!is_link_local(&"fd00::1".parse().unwrap()));
    }

    #[test]
    fn scoped_candidates_round_trip() {
        let text = scoped_v6_candidate("fe80::1".parse().unwrap(), 4433, 3);
        assert_eq!(text, "[fe80::1%3]:4433");
        let parsed = parse_candidate(&text).unwrap();
        match parsed {
            SocketAddr::V6(v6) => {
                assert_eq!(v6.scope_id(), 3);
                assert_eq!(v6.port(), 4433);
            }
            SocketAddr::V4(_) => panic!("expected v6"),
        }
        assert_eq!(scoped_v6_candidate("2001:db8::1".parse().unwrap(), 1, 3), "[2001:db8::1]:1");
        assert_eq!(parse_candidate("10.0.0.1:80").unwrap().port(), 80);
        assert!(parse_candidate("[fe80::1%x]:1").is_none());
        assert!(parse_candidate("nonsense").is_none());
    }
}
