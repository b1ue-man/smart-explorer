//! IP Helper's selected route and hardware-interface flags, never RFC1918 guesses.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use windows_sys::Win32::NetworkManagement::IpHelper::{GetBestRoute2, GetIfEntry2, MIB_IF_ROW2, MIB_IPFORWARD_ROW2};
use windows_sys::Win32::Networking::WinSock::SOCKADDR_INET;

pub(super) fn physical_on_link(local: Option<IpAddr>, remote: SocketAddr) -> bool {
    if remote.ip().is_loopback() || remote.ip().is_unspecified() || remote.ip().is_multicast() { return false; }
    let destination = sockaddr(remote);
    let source = local.map(|ip| sockaddr(SocketAddr::new(ip, 0)));
    // SAFETY: API output records are POD and zero-initialized before the call.
    let mut route: MIB_IPFORWARD_ROW2 = unsafe { std::mem::zeroed() };
    let mut best_source: SOCKADDR_INET = unsafe { std::mem::zeroed() };
    // SAFETY: initialized input and output records remain live for this call.
    let code = unsafe { GetBestRoute2(std::ptr::null(), 0, source.as_ref().map_or(std::ptr::null(), |source| source),
        &destination, 0, &mut route, &mut best_source) };
    if code != 0 || route.Loopback != 0 || route.InterfaceIndex == 0
        || !socket_ip(&route.NextHop).is_some_and(|ip| ip.is_unspecified()) { return false; }
    if local.is_some_and(|local| socket_ip(&best_source) != Some(local)) { return false; }
    if let SocketAddr::V6(remote) = remote {
        if remote.ip().is_unicast_link_local() && remote.scope_id() != route.InterfaceIndex { return false; }
    }
    // SAFETY: the record is zeroed POD, then its documented input index is set.
    let mut interface: MIB_IF_ROW2 = unsafe { std::mem::zeroed() };
    interface.InterfaceIndex = route.InterfaceIndex;
    // SAFETY: initialized in/out record lives through GetIfEntry2.
    if unsafe { GetIfEntry2(&mut interface) } != 0 { return false; }
    let flags = interface.InterfaceAndOperStatusFlags._bitfield;
    matches!(interface.Type, 6 | 71) && interface.OperStatus == 1
        && flags & 1 != 0 && flags & 2 == 0 // HardwareInterface, !FilterInterface
}

fn sockaddr(address: SocketAddr) -> SOCKADDR_INET {
    // SAFETY: SOCKADDR_INET is a C union with sufficient inline address storage.
    let mut result: SOCKADDR_INET = unsafe { std::mem::zeroed() };
    // Use the documented sockaddr byte layouts, avoiding ABI-dependent nested unions.
    let bytes = unsafe { std::slice::from_raw_parts_mut(&mut result as *mut _ as *mut u8, std::mem::size_of::<SOCKADDR_INET>()) };
    match address {
        SocketAddr::V4(address) => { bytes[..2].copy_from_slice(&2u16.to_ne_bytes()); bytes[4..8].copy_from_slice(&address.ip().octets()); }
        SocketAddr::V6(address) => { bytes[..2].copy_from_slice(&23u16.to_ne_bytes()); bytes[8..24].copy_from_slice(&address.ip().octets());
            bytes[24..28].copy_from_slice(&address.scope_id().to_ne_bytes()); }
    }
    result
}

fn socket_ip(address: &SOCKADDR_INET) -> Option<IpAddr> {
    // SAFETY: both union layouts have the family at offset zero; read only its active IP family.
    let bytes = unsafe { std::slice::from_raw_parts(address as *const _ as *const u8, std::mem::size_of::<SOCKADDR_INET>()) };
    match u16::from_ne_bytes([bytes[0], bytes[1]]) {
        2 => Some(IpAddr::V4(Ipv4Addr::new(bytes[4], bytes[5], bytes[6], bytes[7]))),
        23 => Some(IpAddr::V6(Ipv6Addr::from(<[u8;16]>::try_from(&bytes[8..24]).ok()?))),
        _ => None,
    }
}
