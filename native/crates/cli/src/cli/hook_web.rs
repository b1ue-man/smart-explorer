//! Web addresses the lockdown lets an agent fetch: `http`/`https` to hosts
//! that resolve only to public addresses. Loopback, private, link-local,
//! shared (CGNAT), unspecified and multicast addresses, names without a dot,
//! local suffixes and every other scheme (`file:` included) are refused, so a
//! fetch cannot reach a local server that serves protected files.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};

/// `Ok` when `url` may be fetched; otherwise the reason.
pub(super) fn check_url(url: &str) -> Result<(), String> {
    let (host, port) = host_and_port(url)?;
    if let Ok(ip) = host.parse::<IpAddr>() {
        return check_ip(ip);
    }
    check_name(&host)?;
    let addresses = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|error| format!("the host name cannot be resolved to check it: {error}"))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err("the host name resolves to no address".into());
    }
    for address in addresses {
        check_ip(address.ip())?;
    }
    Ok(())
}

/// Scheme, host and port of an absolute `http`/`https` URL.
pub(super) fn host_and_port(url: &str) -> Result<(String, u16), String> {
    let (scheme, rest) = url
        .trim()
        .split_once("://")
        .ok_or_else(|| "not an absolute http or https address".to_string())?;
    let port = match scheme.to_ascii_lowercase().as_str() {
        "http" => 80,
        "https" => 443,
        other => return Err(format!("the scheme `{other}:` is not a web address")),
    };
    let authority = rest.split(['/', '?', '#', '\\']).next().unwrap_or_default();
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let (host, explicit_port) = if let Some(inner) = authority.strip_prefix('[') {
        let (host, after) = inner
            .split_once(']')
            .ok_or_else(|| "malformed IPv6 address".to_string())?;
        (host.to_string(), after.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host.to_string(), Some(port)),
            None => (authority.to_string(), None),
        }
    };
    let port = match explicit_port {
        Some("") | None => port,
        Some(text) => text
            .parse::<u16>()
            .map_err(|_| "malformed port".to_string())?,
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return Err("the address names no host".into());
    }
    Ok((host, port))
}

fn check_name(host: &str) -> Result<(), String> {
    if !host.contains('.') {
        return Err("a host name without a dot is a local name".into());
    }
    for suffix in [
        ".localhost",
        ".local",
        ".internal",
        ".lan",
        ".home.arpa",
        ".intranet",
    ] {
        if host.ends_with(suffix) {
            return Err(format!("`{suffix}` names are local"));
        }
    }
    Ok(())
}

pub(super) fn check_ip(ip: IpAddr) -> Result<(), String> {
    let local = match ip {
        IpAddr::V4(v4) => local_v4(v4),
        IpAddr::V6(v6) => local_v6(v6),
    };
    if local {
        Err(format!("{ip} is a loopback, private or local address"))
    } else {
        Ok(())
    }
}

fn local_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || a == 0
        // Shared address space (RFC 6598), used by CGNAT and Tailscale.
        || (a == 100 && (64..=127).contains(&b))
        // Benchmarking (RFC 2544) and reserved (240/4).
        || (a == 198 && (b == 18 || b == 19))
        || a >= 240
}

fn local_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return local_v4(v4);
    }
    let segments = ip.segments();
    let octets = ip.octets();
    let embedded = Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
    // NAT64 (RFC 6052) carries an IPv4 address in the last 32 bits.
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return local_v4(embedded);
    }
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        // Unique local fc00::/7, link-local fe80::/10, site-local fec0::/10.
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] & 0xffc0) == 0xfec0
        // IPv4-compatible ::a.b.c.d (deprecated) of a local address.
        || (segments[..6] == [0; 6] && local_v4(embedded))
}
