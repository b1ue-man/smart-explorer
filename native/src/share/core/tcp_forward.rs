//! Pure, bounded TCP forwarding contract. A tunnel exposes one exact service;
//! it never accepts a destination from the bytes of a local TCP connection.
use super::access_constraints::{TcpDirection, TcpHost, TcpPort};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

pub(crate) const TCP_LOCAL_ACCESS_NOTICE: &str = "Jeder gleichzeitig lokal laufende Prozess kann diesen Loopbackport erreichen. Der weitergeleitete Dienst muss seine Benutzer selbst authentifizieren; ohne eigene Authentifizierung erhält dieser Prozess Zugriff auf den Dienst.";
/// Protocol version 2 (0.5.189, design 4.6): `lifetime_secs = 0` means no
/// fixed lifetime; the tunnel lives as long as its QUIC connection, its
/// grant and (on the viewer) its bound client. Version 1 stays for hosts up
/// to 0.5.188, whose maximum lifetime is 86400 seconds.
pub(crate) const TCP_PROTOCOL_VERSION: u16 = 2;
pub(crate) const TCP_ALPN: &[u8] = b"smart-explorer/share-tcp/2";
pub(crate) const TCP_PROTOCOL_VERSION_V1: u16 = 1;
pub(crate) const TCP_ALPN_V1: &[u8] = b"smart-explorer/share-tcp/1";
/// Highest lifetime a version-1 host accepts; a viewer sends it to such a
/// host when it asked for no fixed lifetime.
pub(crate) const TCP_V1_MAX_LIFETIME_SECS: u32 = 24 * 60 * 60;

/// The protocol version an accepted connection's ALPN selects.
pub(crate) fn tcp_version_for_alpn(alpn: &[u8]) -> Option<u16> {
    match alpn {
        TCP_ALPN => Some(TCP_PROTOCOL_VERSION),
        TCP_ALPN_V1 => Some(TCP_PROTOCOL_VERSION_V1),
        _ => None,
    }
}
/// 64 sessions * 16 sockets * two 8-KiB pump buffers = 16 MiB of copy
/// buffers per node, in addition to the existing bounded QUIC buffers.
pub(crate) const MAX_TCP_TUNNELS: usize = 64;
pub(crate) const MAX_TCP_STREAMS: usize = 16;
pub(crate) const TCP_COPY_BYTES: usize = 8 * 1024;

pub(crate) fn shown_error(value: &str) -> String {
    value
        .chars()
        .take(1024)
        .map(|character| {
            if character.is_control()
                || matches!(character, '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}'
            | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
            {
                '?'
            } else {
                character
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct TunnelId(String);
impl TunnelId {
    pub(crate) fn generate() -> Result<Self, String> {
        super::core::random_hex_token::<16>().map(Self)
    }
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        if value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Self(value.to_ascii_lowercase()))
        } else {
            Err("TCP tunnel ID must contain exactly 32 hexadecimal characters".into())
        }
    }
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for TunnelId {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Self::parse(&value)
    }
}
impl From<TunnelId> for String {
    fn from(value: TunnelId) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TcpDestination {
    #[serde(default)]
    pub(crate) host: TcpHost,
    pub(crate) port: TcpPort,
}
impl TcpDestination {
    pub(crate) fn validate(&self) -> Result<(), String> {
        match &self.host {
            TcpHost::Loopback => Ok(()),
            TcpHost::Ip { address } if valid_ip(*address) => Ok(()),
            TcpHost::Dns { name, addresses }
                if valid_dns(name)
                    && addresses.len() <= 64
                    && addresses.iter().all(|ip| valid_ip(*ip)) =>
            {
                Ok(())
            }
            _ => {
                Err("TCP destination must be loopback, one unicast IP or one plain DNS name".into())
            }
        }
    }
    /// DNS addresses supplied by a requester are never used to resolve or
    /// authorize the destination; only the host's fresh lookup is evidence.
    pub(crate) fn from_host_port(host: &str, port: u16) -> Result<Self, String> {
        let host = match host {
            "loopback" | "localhost" => TcpHost::Loopback,
            value => match value.parse::<IpAddr>() {
                Ok(address) => TcpHost::Ip { address },
                Err(_) => TcpHost::Dns {
                    name: value.into(),
                    addresses: Vec::new(),
                },
            },
        };
        let value = Self {
            host,
            port: port.try_into()?,
        };
        value.validate()?;
        Ok(value)
    }
}
fn valid_ip(ip: IpAddr) -> bool {
    !ip.is_unspecified() && !ip.is_multicast()
}
fn valid_dns(name: &str) -> bool {
    name.len() <= 253
        && name.parse::<IpAddr>().is_err()
        && name.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TcpStart {
    pub(crate) destination: TcpDestination,
    /// Port zero asks the OS for an ephemeral loopback port. No LAN bind.
    #[serde(default)]
    pub(crate) local_port: u16,
    /// Version 1: the caller confirmed TCP_LOCAL_ACCESS_NOTICE. Version 2:
    /// the viewer's listener checks every local connection's origin by its
    /// `ListenerAdmission`; viewers always send `true` (old hosts check it).
    #[serde(default)]
    pub(crate) acknowledge_local_access: bool,
    /// 0 = no fixed lifetime (version 2 only); otherwise 1..=86400 seconds,
    /// an explicitly chosen limit (`--lifetime-secs`).
    #[serde(default)]
    pub(crate) lifetime_secs: u32,
}
impl TcpStart {
    /// Validation under the current protocol version.
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.validate_for(TCP_PROTOCOL_VERSION)
    }
    /// Validation under the protocol `version` of the connection.
    pub(crate) fn validate_for(&self, version: u16) -> Result<(), String> {
        self.destination.validate()?;
        if !self.acknowledge_local_access {
            return Err(TCP_LOCAL_ACCESS_NOTICE.into());
        }
        let lowest = if version >= TCP_PROTOCOL_VERSION {
            0
        } else {
            1
        };
        if !(lowest..=TCP_V1_MAX_LIFETIME_SECS).contains(&self.lifetime_secs) {
            return Err(if lowest == 0 {
                "TCP tunnel lifetime must be 0 (no fixed lifetime) or 1..86400 seconds".into()
            } else {
                "TCP tunnel lifetime must be 1..86400 seconds".into()
            });
        }
        Ok(())
    }
    /// The fixed lifetime, if one was chosen.
    pub(crate) fn fixed_lifetime(&self) -> Option<std::time::Duration> {
        (self.lifetime_secs > 0)
            .then(|| std::time::Duration::from_secs(u64::from(self.lifetime_secs)))
    }
    /// The Unix second at which a tunnel started at `now` ends, if fixed.
    pub(crate) fn expires_at(&self, now: i64) -> Option<i64> {
        (self.lifetime_secs > 0).then(|| now.saturating_add(i64::from(self.lifetime_secs)))
    }
    pub(crate) fn listener(&self) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.local_port)
    }
}

/// Which local connections a viewer's listener forwards (design 4.4, 9.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ListenerAdmission {
    /// Connections of processes of the background service's own account.
    #[default]
    SameAccount,
    /// RDP: only connections of the one client process bound later on the
    /// same IPC stream (PID and process start time).
    AwaitProcess,
    /// Android only: the OS discloses no connection owner (assumption A7).
    AnyLocal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TunnelRole {
    Listener,
    Target,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TunnelView {
    pub(crate) id: TunnelId,
    pub(crate) role: TunnelRole,
    pub(crate) remote_node: String,
    pub(crate) initiator_node: String,
    pub(crate) target_node: String,
    pub(crate) direction: TcpDirection,
    pub(crate) relation_kind: String,
    pub(crate) relation_id: String,
    pub(crate) destination: TcpDestination,
    pub(crate) bound_target: Option<SocketAddr>,
    pub(crate) listener: Option<SocketAddr>,
    pub(crate) started_at: i64,
    /// `None` = no fixed lifetime (protocol version 2). Written as
    /// `i64::MAX` (review C1): GUIs, scripts and the Android app of 0.5.188
    /// read a number here; this build reads `i64::MAX` and `null` as `None`.
    #[serde(with = "no_fixed_end")]
    pub(crate) expires_at: Option<i64>,
    /// Origin rule of a listener; `None` for a target tunnel.
    #[serde(default)]
    pub(crate) admission: Option<ListenerAdmission>,
    pub(crate) active_connections: usize,
    pub(crate) socket_read_bytes: u64,
    pub(crate) socket_written_bytes: u64,
    pub(crate) closing: bool,
    pub(crate) last_error: Option<String>,
}

/// `TunnelView.expires_at`: `None` travels as `i64::MAX`.
mod no_fixed_end {
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(value: &Option<i64>, out: S) -> Result<S::Ok, S::Error> {
        out.serialize_i64(value.unwrap_or(i64::MAX))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        input: D,
    ) -> Result<Option<i64>, D::Error> {
        let at = Option::<i64>::deserialize(input)?;
        Ok(at.filter(|at| *at != i64::MAX))
    }
}
