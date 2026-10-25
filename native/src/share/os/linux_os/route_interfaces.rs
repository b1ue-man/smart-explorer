//! Kernel route lookup plus sysfs hardware proof. Unknown interfaces stay Direct.
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;

pub(super) fn physical_on_link(local: Option<IpAddr>, remote: SocketAddr) -> bool {
    if remote.ip().is_loopback() || remote.ip().is_unspecified() || remote.ip().is_multicast() {
        return false;
    }
    let Ok((index, source)) = lookup(local, remote) else { return false; };
    let Ok(interfaces) = if_addrs::get_if_addrs() else { return false; };
    interfaces.iter().any(|interface| {
        let base = Path::new("/sys/class/net").join(&interface.name);
        interface.index == Some(index) && !interface.is_loopback()
            && local.or(source).is_some_and(|source| interface.ip() == source)
            && std::fs::read_to_string(base.join("type")).is_ok_and(|kind| kind.trim() == "1")
            && base.join("device").exists()
            && std::fs::read_to_string(base.join("operstate")).is_ok_and(|state| state.trim() == "up")
    })
}

/// RTM_GETROUTE asks the kernel's actual policy routing decision, not a guessed subnet.
/// A gateway or missing output interface is deliberately not on-link evidence.
fn lookup(local: Option<IpAddr>, remote: SocketAddr) -> io::Result<(u32, Option<IpAddr>)> {
    // SAFETY: fixed AF_NETLINK/NETLINK_ROUTE constants, checked fd ownership.
    let raw = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW | libc::SOCK_CLOEXEC, libc::NETLINK_ROUTE) };
    if raw < 0 { return Err(io::Error::last_os_error()); }
    // SAFETY: socket returned a new descriptor, owned exactly once.
    let socket = unsafe { OwnedFd::from_raw_fd(raw) };
    let wait = libc::timeval { tv_sec: 0, tv_usec: 200_000 };
    // SAFETY: timeout pointer and length describe a live timeval.
    if unsafe { libc::setsockopt(socket.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVTIMEO,
        &wait as *const _ as *const libc::c_void, std::mem::size_of_val(&wait) as libc::socklen_t) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut request = vec![0u8; 28]; // nlmsghdr + rtmsg
    request[4..6].copy_from_slice(&26u16.to_ne_bytes()); // RTM_GETROUTE
    request[6..8].copy_from_slice(&1u16.to_ne_bytes()); // NLM_F_REQUEST
    request[8..12].copy_from_slice(&1u32.to_ne_bytes());
    request[16] = if remote.is_ipv4() { libc::AF_INET as u8 } else { libc::AF_INET6 as u8 };
    request[17] = if remote.is_ipv4() { 32 } else { 128 };
    attribute(&mut request, 1, &ip_bytes(remote.ip())); // RTA_DST
    // SAFETY: geteuid has no pointer or ownership preconditions.
    attribute(&mut request, 25, &unsafe { libc::geteuid() }.to_ne_bytes()); // RTA_UID
    attribute(&mut request, 27, &[libc::IPPROTO_UDP as u8]); // RTA_IP_PROTO
    attribute(&mut request, 29, &remote.port().to_be_bytes()); // RTA_DPORT
    if let Some(local) = local.filter(|ip| ip.is_ipv4() == remote.is_ipv4()) {
        request[18] = if local.is_ipv4() { 32 } else { 128 };
        attribute(&mut request, 2, &ip_bytes(local)); // RTA_SRC
    }
    if let SocketAddr::V6(remote) = remote {
        if remote.ip().is_unicast_link_local() {
            if remote.scope_id() == 0 { return Err(io::ErrorKind::AddrNotAvailable.into()); }
            attribute(&mut request, 4, &remote.scope_id().to_ne_bytes());
        }
    }
    let length = request.len() as u32;
    request[..4].copy_from_slice(&length.to_ne_bytes());
    // SAFETY: zero is a valid kernel sockaddr_nl; initialize its family.
    let mut destination: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    destination.nl_family = libc::AF_NETLINK as u16;
    // SAFETY: all pointers refer to the live buffers described by the lengths.
    let sent = unsafe { libc::sendto(socket.as_raw_fd(), request.as_ptr().cast(), request.len(), 0,
        &destination as *const _ as *const libc::sockaddr, std::mem::size_of_val(&destination) as libc::socklen_t) };
    if sent != request.len() as isize { return Err(io::Error::last_os_error()); }
    let mut response = [0u8; 8192];
    // SAFETY: response is writable for its full specified length.
    let mut sender: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    let mut sender_len = std::mem::size_of_val(&sender) as libc::socklen_t;
    let read = unsafe { libc::recvfrom(socket.as_raw_fd(), response.as_mut_ptr().cast(), response.len(), 0,
        &mut sender as *mut _ as *mut libc::sockaddr, &mut sender_len) };
    if read < 0 { return Err(io::Error::last_os_error()); }
    if sender.nl_pid != 0 { return Err(io::ErrorKind::PermissionDenied.into()); }
    parse_route(&response[..read as usize], remote)
}

fn attribute(message: &mut Vec<u8>, kind: u16, value: &[u8]) {
    let length = (value.len() + 4) as u16;
    message.extend(length.to_ne_bytes()); message.extend(kind.to_ne_bytes()); message.extend(value);
    while message.len() % 4 != 0 { message.push(0); }
}

fn ip_bytes(ip: IpAddr) -> Vec<u8> {
    match ip { IpAddr::V4(ip) => ip.octets().to_vec(), IpAddr::V6(ip) => ip.octets().to_vec() }
}

fn parse_route(message: &[u8], remote: SocketAddr) -> io::Result<(u32, Option<IpAddr>)> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "No physical on-link route proof");
    if message.len() < 28 || u16::from_ne_bytes([message[4], message[5]]) != 24
        || message[23] != 1 { return Err(invalid()); } // RTM_NEWROUTE, RTN_UNICAST
    let length = u32::from_ne_bytes(message[..4].try_into().map_err(|_| invalid())?) as usize;
    if length > message.len() || length < 28 { return Err(invalid()); }
    let mut offset = 28; let mut index = None; let mut source = None;
    while offset + 4 <= length {
        let size = u16::from_ne_bytes([message[offset], message[offset + 1]]) as usize;
        let kind = u16::from_ne_bytes([message[offset + 2], message[offset + 3]]) & 0x3fff;
        if size < 4 || offset + size > length { return Err(invalid()); }
        let value = &message[offset + 4..offset + size];
        match kind {
            4 if value.len() == 4 => index = Some(u32::from_ne_bytes(value.try_into().map_err(|_| invalid())?)),
            5 if value.iter().any(|byte| *byte != 0) => return Err(invalid()),
            18 | 21 | 22 => return Err(invalid()), // cross-family gateway or encapsulation
            7 if value.len() == 4 => source = Some(IpAddr::V4(std::net::Ipv4Addr::new(value[0], value[1], value[2], value[3]))),
            7 if value.len() == 16 => source = Some(IpAddr::V6(std::net::Ipv6Addr::from(<[u8;16]>::try_from(value).map_err(|_| invalid())?))),
            _ => {}
        }
        offset += (size + 3) & !3;
    }
    let index = index.filter(|index| *index > 0).ok_or_else(invalid)?;
    if let SocketAddr::V6(remote) = remote {
        if remote.ip().is_unicast_link_local() && remote.scope_id() != index { return Err(invalid()); }
    }
    Ok((index, source))
}
