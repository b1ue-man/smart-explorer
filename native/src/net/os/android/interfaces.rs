//! Android link facts from `if-addrs` alone.
//!
//! App processes may not read `/proc/net/route` or the sysfs link state
//! (SELinux), and DHCP lease stores belong to the system, so the facts come
//! from the assigned addresses: Android removes addresses from a link that
//! goes down, so a listed interface is up, and a link holding a routable
//! (non-link-local) address was configured by a router, DHCP/SLAAC or the
//! carrier. A link with only link-local addresses stays router-less, which is
//! the direct-cable case the classification cares about. The DHCP verdict
//! stays `None` because no lease store is readable.
use std::collections::BTreeMap;
use std::net::IpAddr;

use crate::net::{is_link_local, InterfaceFacts};

pub(crate) fn gather_interface_facts() -> Result<Vec<InterfaceFacts>, String> {
    let interfaces = if_addrs::get_if_addrs()
        .map_err(|error| format!("Netzwerkschnittstellen lesen: {error}"))?;
    let mut facts: BTreeMap<String, InterfaceFacts> = BTreeMap::new();
    for iface in interfaces {
        let ip = iface.ip();
        let entry = facts
            .entry(iface.name.clone())
            .or_insert_with(|| InterfaceFacts {
                name: iface.name.clone(),
                adapter_id: iface.name.clone(),
                index: iface.index.unwrap_or(0),
                up: true,
                loopback: iface.is_loopback(),
                addrs: Vec::new(),
                has_gateway: false,
                dhcp_lease: None,
            });
        entry.has_gateway |= is_routable(&ip);
        if !entry.addrs.contains(&ip) {
            entry.addrs.push(ip);
        }
        if entry.index == 0 {
            entry.index = iface.index.unwrap_or(0);
        }
    }
    Ok(facts.into_values().collect())
}

fn is_routable(ip: &IpAddr) -> bool {
    !ip.is_loopback() && !ip.is_unspecified() && !is_link_local(ip)
}
