//! Linux link facts from `if-addrs`, sysfs and the kernel routing tables.
//! Nothing here assumes NetworkManager; the DHCP verdict stays `None` when no
//! lease store is readable (NetworkManager fills it in when available).
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::Path;

use crate::net::InterfaceFacts;

pub(crate) fn gather_interface_facts() -> Result<Vec<InterfaceFacts>, String> {
    let interfaces = if_addrs::get_if_addrs()
        .map_err(|error| format!("Netzwerkschnittstellen lesen: {error}"))?;
    let gateways = gateway_interfaces();
    let mut facts: BTreeMap<String, InterfaceFacts> = BTreeMap::new();
    for iface in interfaces {
        let entry = facts.entry(iface.name.clone()).or_insert_with(|| InterfaceFacts {
            name: iface.name.clone(),
            adapter_id: iface.name.clone(),
            index: iface.index.unwrap_or(0),
            up: operstate_up(&iface.name),
            loopback: iface.is_loopback(),
            addrs: Vec::new(),
            has_gateway: gateways.iter().any(|gateway| gateway == &iface.name),
            dhcp_lease: dhcp_lease_hint(&iface.name, iface.index),
        });
        let ip = iface.ip();
        if !entry.addrs.contains(&ip) {
            entry.addrs.push(ip);
        }
        if entry.index == 0 {
            entry.index = iface.index.unwrap_or(0);
        }
    }
    Ok(facts.into_values().collect())
}

fn operstate_up(name: &str) -> bool {
    let operstate = std::fs::read_to_string(format!("/sys/class/net/{name}/operstate"))
        .map(|text| text.trim().to_string())
        .unwrap_or_default();
    match operstate.as_str() {
        "up" => true,
        // Loopback and some virtual links report "unknown" while carrying traffic.
        "unknown" => std::fs::read_to_string(format!("/sys/class/net/{name}/carrier"))
            .map(|text| text.trim() == "1")
            .unwrap_or(true),
        _ => false,
    }
}

/// Interface names that carry a default route (IPv4 `/proc/net/route` with
/// the gateway flag, IPv6 `/proc/net/ipv6_route` with an all-zero `::/0`
/// destination and the gateway flag).
fn gateway_interfaces() -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(text) = std::fs::read_to_string("/proc/net/route") {
        names.extend(parse_ipv4_routes(&text));
    }
    if let Ok(text) = std::fs::read_to_string("/proc/net/ipv6_route") {
        names.extend(parse_ipv6_routes(&text));
    }
    names.sort();
    names.dedup();
    names
}

const RTF_GATEWAY: u32 = 0x0002;

pub(crate) fn parse_ipv4_routes(text: &str) -> Vec<String> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let iface = fields.next()?;
            let destination = fields.next()?;
            let _gateway = fields.next()?;
            let flags = u32::from_str_radix(fields.next()?, 16).ok()?;
            (destination == "00000000" && flags & RTF_GATEWAY != 0).then(|| iface.to_string())
        })
        .collect()
}

pub(crate) fn parse_ipv6_routes(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 {
                return None;
            }
            let destination = fields[0];
            let prefix_len = fields[1];
            let flags = u32::from_str_radix(fields[8], 16).ok()?;
            let iface = fields[9];
            (destination.chars().all(|c| c == '0')
                && prefix_len == "00"
                && flags & RTF_GATEWAY != 0
                && iface != "lo")
                .then(|| iface.to_string())
        })
        .collect()
}

/// systemd-networkd keeps `/run/systemd/netif/leases/<ifindex>`; dhcpcd and
/// dhclient write per-interface lease files. Any readable, non-empty lease
/// file is a positive verdict; nothing readable stays `None`.
fn dhcp_lease_hint(name: &str, index: Option<u32>) -> Option<bool> {
    let mut candidates = Vec::new();
    if let Some(index) = index {
        candidates.push(format!("/run/systemd/netif/leases/{index}"));
    }
    candidates.push(format!("/var/lib/dhcpcd/{name}.lease"));
    candidates.push(format!("/var/lib/dhcp/dhclient.{name}.leases"));
    candidates.push(format!("/var/lib/dhclient/dhclient-{name}.leases"));
    for candidate in candidates {
        let path = Path::new(&candidate);
        match std::fs::metadata(path) {
            Ok(metadata) if metadata.is_file() && metadata.len() > 0 => return Some(true),
            Ok(_) => {}
            Err(_) => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_default_routes_are_detected_by_flag_and_destination() {
        let text = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                    eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
                    eth1\t0000FEA9\t00000000\t0001\t0\t0\t0\t0000FFFF\t0\t0\t0\n";
        assert_eq!(parse_ipv4_routes(text), ["eth0"]);
    }

    #[test]
    fn ipv6_default_routes_need_zero_prefix_and_gateway_flag() {
        let text = "00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe800000000000000000000000000001 00000400 00000001 00000000 00000003 wlan0\n\
                    fe800000000000000000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000001 00000000 00000001 eth1\n";
        assert_eq!(parse_ipv6_routes(text), ["wlan0"]);
    }
}
