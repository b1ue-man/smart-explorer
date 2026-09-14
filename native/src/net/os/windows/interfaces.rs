//! Windows link facts from `GetAdaptersAddresses` (IP Helper): operational
//! state, unicast addresses, default gateways and the DHCPv4 lease server.
//! The adapter GUID is kept as `adapter_id` because Internet Connection
//! Sharing addresses connections by that GUID.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH, IP_ADAPTER_UNICAST_ADDRESS_LH,
};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows_sys::Win32::Networking::WinSock::SOCKET_ADDRESS;

use crate::net::InterfaceFacts;

const AF_UNSPEC: u32 = 0;
const AF_INET: u16 = 2;
const AF_INET6: u16 = 23;
const GAA_FLAG_SKIP_ANYCAST: u32 = 0x0002;
const GAA_FLAG_SKIP_MULTICAST: u32 = 0x0004;
const GAA_FLAG_SKIP_DNS_SERVER: u32 = 0x0008;
const GAA_FLAG_INCLUDE_GATEWAYS: u32 = 0x0080;
const IP_ADAPTER_DHCP_ENABLED: u32 = 0x0004;
const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
const ERROR_SUCCESS: u32 = 0;
const ERROR_BUFFER_OVERFLOW: u32 = 111;
const ERROR_NO_DATA: u32 = 232;
const MAX_ATTEMPTS: usize = 4;

pub(crate) fn gather_interface_facts() -> Result<Vec<InterfaceFacts>, String> {
    let flags = GAA_FLAG_SKIP_ANYCAST
        | GAA_FLAG_SKIP_MULTICAST
        | GAA_FLAG_SKIP_DNS_SERVER
        | GAA_FLAG_INCLUDE_GATEWAYS;
    let mut size: u32 = 16 * 1024;
    for _ in 0..MAX_ATTEMPTS {
        let mut buffer = vec![0u8; size as usize];
        // SAFETY: the buffer is sized by `size`, which the call updates when
        // more room is needed; the pointer stays valid for the whole call.
        let result = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC,
                flags,
                std::ptr::null(),
                buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
                &mut size,
            )
        };
        match result {
            ERROR_SUCCESS => {
                // SAFETY: on success the buffer holds a linked list of adapter
                // records written by the API within `size` bytes.
                return Ok(unsafe { collect(buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH) });
            }
            ERROR_NO_DATA => return Ok(Vec::new()),
            ERROR_BUFFER_OVERFLOW => continue,
            code => return Err(format!("GetAdaptersAddresses fehlgeschlagen (Code {code})")),
        }
    }
    Err("GetAdaptersAddresses: Puffer wuchs wiederholt".into())
}

unsafe fn collect(mut adapter: *const IP_ADAPTER_ADDRESSES_LH) -> Vec<InterfaceFacts> {
    let mut facts = Vec::new();
    while !adapter.is_null() {
        let record = &*adapter;
        let mut addrs = Vec::new();
        let mut unicast: *const IP_ADAPTER_UNICAST_ADDRESS_LH = record.FirstUnicastAddress;
        while !unicast.is_null() {
            if let Some(ip) = socket_address_ip(&(*unicast).Address) {
                addrs.push(ip);
            }
            unicast = (*unicast).Next;
        }
        let flags = record.Anonymous2.Flags;
        let dhcp_enabled = flags & IP_ADAPTER_DHCP_ENABLED != 0;
        let dhcp_lease = if dhcp_enabled {
            Some(socket_address_ip(&record.Dhcpv4Server).is_some())
        } else {
            Some(false)
        };
        facts.push(InterfaceFacts {
            name: wide_string(record.FriendlyName),
            adapter_id: ansi_string(record.AdapterName),
            index: record.Anonymous1.Anonymous.IfIndex,
            up: record.OperStatus == IfOperStatusUp,
            loopback: record.IfType == IF_TYPE_SOFTWARE_LOOPBACK,
            addrs,
            has_gateway: !record.FirstGatewayAddress.is_null(),
            dhcp_lease,
        });
        adapter = record.Next;
    }
    facts
}

/// Read the address out of a raw `sockaddr` without depending on the exact
/// `SOCKADDR_IN*` layouts: family at offset 0, IPv4 bytes at 4..8, IPv6
/// bytes at 8..24.
unsafe fn socket_address_ip(address: &SOCKET_ADDRESS) -> Option<IpAddr> {
    let base = address.lpSockaddr as *const u8;
    let length = address.iSockaddrLength.max(0) as usize;
    if base.is_null() || length < 8 {
        return None;
    }
    let family = u16::from_ne_bytes([*base, *base.add(1)]);
    match family {
        AF_INET if length >= 8 => {
            let octets = [*base.add(4), *base.add(5), *base.add(6), *base.add(7)];
            Some(IpAddr::V4(Ipv4Addr::from(octets)))
        }
        AF_INET6 if length >= 24 => {
            let mut octets = [0u8; 16];
            for (offset, slot) in octets.iter_mut().enumerate() {
                *slot = *base.add(8 + offset);
            }
            Some(IpAddr::V6(Ipv6Addr::from(octets)))
        }
        _ => None,
    }
}

unsafe fn wide_string(pointer: *const u16) -> String {
    if pointer.is_null() {
        return String::new();
    }
    let mut length = 0usize;
    while *pointer.add(length) != 0 {
        length += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length))
}

unsafe fn ansi_string(pointer: *const u8) -> String {
    if pointer.is_null() {
        return String::new();
    }
    std::ffi::CStr::from_ptr(pointer as *const std::ffi::c_char)
        .to_string_lossy()
        .into_owned()
}
