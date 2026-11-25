//! The Share server's address services as this device last heard them in
//! `hello_ok`: the TCP address reflector (`tcp_reflect_v1`, server
//! `share-server/src/tcp_reflect.rs`) and the STUN responder (`stun_v1`,
//! server `share-server/src/stun.rs`). Both tell a device the public address
//! its NAT gives one local port: the reflector for TCP (the direct TCP
//! picture link of the screen stream, `screen/os/shared/video_tcp_link.rs`),
//! STUN (RFC 8489 Binding) for UDP.
//!
//! The device offers both capabilities in its hello; a server that runs a
//! service lists it in `hello_ok`, either bare (the default port: TCP 51823,
//! UDP 3478) or with its port (`tcp_reflect_v1:51900`). A device reaches the
//! service at the host of the Share endpoint it is connected to. The record
//! lives in this process, beside the signaling connection (like
//! `server_qad.rs`); the screen sessions read it with
//! [`server_address_services`].
use std::net::SocketAddr;
use std::sync::{Mutex, PoisonError};

use super::server_address::SignalServerConfig;

/// Hello capability of the TCP address reflector.
pub const TCP_REFLECT_CAPABILITY: &str = "tcp_reflect_v1";
/// Hello capability of the STUN Binding responder.
pub const STUN_CAPABILITY: &str = "stun_v1";
pub const DEFAULT_TCP_REFLECT_PORT: u16 = 51823;
pub const DEFAULT_STUN_PORT: u16 = 3478;
/// The reflector's answer: this prefix, the observed `ip:port` and `\n`.
pub const REFLECT_LINE_PREFIX: &str = "se-reflect 1 ";
/// The longest answer line: the prefix and a bracketed IPv6 address with
/// its port fit.
pub const MAX_REFLECT_LINE_BYTES: usize = 64;

/// The ports a server announced in `hello_ok`; `None`: the service is off
/// or the server predates it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AddressServicePorts {
    pub tcp_reflect: Option<u16>,
    pub stun: Option<u16>,
}

impl AddressServicePorts {
    /// The ports of the `hello_ok` capability list; an entry with a port
    /// that is not a number in 1..=65535 is ignored.
    pub fn from_capabilities(capabilities: &[String]) -> Self {
        let mut ports = Self::default();
        for capability in capabilities {
            if let Some(port) =
                announced_port(capability, TCP_REFLECT_CAPABILITY, DEFAULT_TCP_REFLECT_PORT)
            {
                ports.tcp_reflect = Some(port);
            }
            if let Some(port) = announced_port(capability, STUN_CAPABILITY, DEFAULT_STUN_PORT) {
                ports.stun = Some(port);
            }
        }
        ports
    }
}

fn announced_port(capability: &str, name: &str, default: u16) -> Option<u16> {
    let rest = capability.strip_prefix(name)?;
    if rest.is_empty() {
        return Some(default);
    }
    rest.strip_prefix(':')?
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
}

/// Where this device reaches the address services of its Share server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerAddressServices {
    /// Host of the connected Share endpoint: a DNS name or an IP address,
    /// IPv6 without brackets.
    pub host: String,
    pub tcp_reflect_port: Option<u16>,
    pub stun_port: Option<u16>,
}

static LAST: Mutex<Option<ServerAddressServices>> = Mutex::new(None);
/// The Iroh endpoint's own direct addresses as last published (QAD and the
/// interfaces), for the predicted TCP candidate of the screen's TCP link.
static OWN_ADDRS: Mutex<Vec<SocketAddr>> = Mutex::new(Vec::new());

/// Records the endpoint's own direct addresses each time they are published.
pub(crate) fn record_own_addrs(addrs: Vec<SocketAddr>) {
    *OWN_ADDRS.lock().unwrap_or_else(PoisonError::into_inner) = addrs;
}

/// The endpoint's own direct addresses as last published; empty before.
pub fn own_endpoint_addrs() -> Vec<SocketAddr> {
    OWN_ADDRS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Records what `hello_ok` announced for the endpoint `connection_label`
/// (`SignalEndpoint::label`) of the stored server address `server_config`.
pub(crate) fn record(server_config: &str, connection_label: &str, ports: AddressServicePorts) {
    *LAST.lock().unwrap_or_else(PoisonError::into_inner) =
        services_for(server_config, connection_label, ports);
}

fn services_for(
    server_config: &str,
    connection_label: &str,
    ports: AddressServicePorts,
) -> Option<ServerAddressServices> {
    let config = SignalServerConfig::parse_stored(server_config).ok()?;
    let endpoint = config
        .active_endpoints()
        .into_iter()
        .find(|endpoint| endpoint.label() == connection_label)?;
    Some(ServerAddressServices {
        host: endpoint.host().to_string(),
        tcp_reflect_port: ports.tcp_reflect,
        stun_port: ports.stun,
    })
}

/// The address services of the Share server this process last completed a
/// hello with; `None` before the first `hello_ok`. Both ports are `None`
/// with a server that runs neither service.
pub fn server_address_services() -> Option<ServerAddressServices> {
    LAST.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// The observed address of a reflector answer line (with its `\n`).
pub fn parse_reflect_line(line: &[u8]) -> Option<SocketAddr> {
    if line.len() > MAX_REFLECT_LINE_BYTES {
        return None;
    }
    std::str::from_utf8(line)
        .ok()?
        .strip_suffix('\n')?
        .strip_prefix(REFLECT_LINE_PREFIX)?
        .parse::<SocketAddr>()
        .ok()
        .filter(|addr| addr.port() != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// User 2026-11-25 („TCP und UDP, DIREKT“): the server names its TCP
    /// reflector and STUN responder in `hello_ok`, bare for the default
    /// port or with another one; the device keeps the host of the endpoint
    /// it is connected to and reads the reflector's line.
    #[test]
    fn address_services_task_ports_host_and_reflect_line() {
        let list =
            |items: &[&str]| -> Vec<String> { items.iter().map(|s| s.to_string()).collect() };
        let cases = [
            (
                "both default",
                list(&["tcp_reflect_v1", "stun_v1"]),
                Some(51823),
                Some(3478),
            ),
            (
                "other ports",
                list(&["tcp_reflect_v1:51900", "stun_v1:3479"]),
                Some(51900),
                Some(3479),
            ),
            ("none", list(&["tracked_direct_v1"]), None, None),
            (
                "bad ports",
                list(&["tcp_reflect_v1:0", "stun_v1:x", "stun_v10"]),
                None,
                None,
            ),
        ];
        for (case, capabilities, reflect, stun) in cases {
            let ports = AddressServicePorts::from_capabilities(&capabilities);
            assert_eq!((ports.tcp_reflect, ports.stun), (reflect, stun), "{case}");
        }

        let ports = AddressServicePorts {
            tcp_reflect: Some(51823),
            stun: None,
        };
        let config = "wss://share.example.org:51820, wss://[2001:db8::7]:51820";
        assert_eq!(
            services_for(config, "wss://[2001:db8::7]:51820", ports),
            Some(ServerAddressServices {
                host: "2001:db8::7".into(),
                tcp_reflect_port: Some(51823),
                stun_port: None,
            })
        );
        assert_eq!(
            services_for(config, "wss://share.example.org:51820", ports).map(|s| s.host),
            Some("share.example.org".into())
        );
        assert_eq!(services_for(config, "wss://other:51820", ports), None);

        let lines = [
            (
                "IPv4",
                "se-reflect 1 203.0.113.7:40123\n",
                Some("203.0.113.7:40123"),
            ),
            (
                "IPv6",
                "se-reflect 1 [2001:db8::9]:40123\n",
                Some("[2001:db8::9]:40123"),
            ),
            ("no newline", "se-reflect 1 203.0.113.7:40123", None),
            ("other version", "se-reflect 2 203.0.113.7:40123\n", None),
            ("port 0", "se-reflect 1 203.0.113.7:0\n", None),
            ("garbage", "se-reflect 1 nonsense\n", None),
        ];
        for (case, line, expected) in lines {
            let expected = expected.map(|text| text.parse::<SocketAddr>().unwrap());
            assert_eq!(parse_reflect_line(line.as_bytes()), expected, "{case}");
        }
        let long = format!("se-reflect 1 203.0.113.7:1{}\n", " ".repeat(64));
        assert_eq!(parse_reflect_line(long.as_bytes()), None, "too long");
        // The endpoint's published addresses reach the screen's TCP link.
        let own: Vec<SocketAddr> = vec!["203.0.113.7:40123".parse().unwrap()];
        record_own_addrs(own.clone());
        assert_eq!(own_endpoint_addrs(), own);
    }
}
