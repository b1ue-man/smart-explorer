//! Share-server address (FC3): parsing, canonical form, transport security,
//! certificate pins and the Iroh relay URLs derived from it.
//!
//! There are two readings. What a user enters (`parse_input`) means TLS
//! (`wss://`) when it names no scheme; `tcp://`, `ws://` and `http://` need the
//! explicit plaintext permission. What is stored (`parse_stored`) may come
//! from an earlier version, where a value without a scheme meant plaintext
//! TCP; it keeps that meaning. Every writer stores the canonical form, which
//! always names its scheme, so a stored value without one is a legacy value
//! and `migrate_stored` rewrites it as `tcp://host:port`.
//!
//! Once a TLS endpoint is configured, plaintext endpoints are never used:
//! there is no fallback from TLS to plaintext, neither for signaling nor for
//! the relay.

use std::net::Ipv6Addr;

use super::core::{hex, hex_decode};

/// Signaling port of `se-share-server` and of addresses without a port.
pub const DEFAULT_SIGNAL_PORT: u16 = 51820;
const MAX_SERVER_CONFIG_BYTES: usize = 16 * 1024;
const PIN_OPTION: &str = "sha256=";
const PLAINTEXT_DENIED: &str = "ist unverschlüsselt: nur mit ausdrücklicher Erlaubnis \
(„Unverschlüsselt erlauben“ bzw. --allow-plaintext) oder mit wss:// verwenden";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalScheme {
    /// WebSocket through TLS (`wss://`, entered also as `https://`).
    Wss,
    /// WebSocket without TLS (`ws://`, entered also as `http://`).
    Ws,
    /// Newline-delimited JSON over plain TCP (`tcp://`, legacy values).
    Tcp,
}

impl SignalScheme {
    fn name(self) -> &'static str {
        match self {
            Self::Wss => "wss",
            Self::Ws => "ws",
            Self::Tcp => "tcp",
        }
    }

    pub fn is_encrypted(self) -> bool {
        self == Self::Wss
    }

    fn default_port(self) -> u16 {
        match self {
            Self::Wss => 443,
            Self::Ws => 80,
            Self::Tcp => DEFAULT_SIGNAL_PORT,
        }
    }
}

/// Effective transport security of a configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerSecurity {
    NotConfigured,
    Encrypted,
    Plaintext,
}

impl ServerSecurity {
    /// Short label for settings and status lines.
    pub fn label(self) -> &'static str {
        match self {
            Self::NotConfigured => "kein Share-Server",
            Self::Encrypted => "🔒 verschlüsselt",
            Self::Plaintext => "⚠ unverschlüsselt",
        }
    }

    /// Stable wire value (`encrypted`, `plaintext`, `none`) for JSON consumers.
    pub fn wire(self) -> &'static str {
        match self {
            Self::NotConfigured => "none",
            Self::Encrypted => "encrypted",
            Self::Plaintext => "plaintext",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalEndpoint {
    scheme: SignalScheme,
    /// Host name or IP address, IPv6 without brackets.
    host: String,
    port: Option<u16>,
    /// Empty or starting with `/` (WebSocket only).
    path: String,
    /// SHA-256 of the server certificate or of its public key (`wss` only).
    pin: Option<[u8; 32]>,
}

impl SignalEndpoint {
    pub fn scheme(&self) -> SignalScheme {
        self.scheme
    }

    pub fn is_encrypted(&self) -> bool {
        self.scheme.is_encrypted()
    }

    /// Host for name resolution and TLS server names (IPv6 without brackets).
    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port.unwrap_or_else(|| self.scheme.default_port())
    }

    pub fn pin(&self) -> Option<[u8; 32]> {
        self.pin
    }

    fn url_host(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        }
    }

    fn authority(&self) -> String {
        match self.port {
            Some(port) => format!("{}:{port}", self.url_host()),
            None => self.url_host(),
        }
    }

    /// The endpoint without options, for status lines and errors.
    pub fn label(&self) -> String {
        format!("{}://{}{}", self.scheme.name(), self.authority(), self.path)
    }

    /// The stored form: always with scheme, pins as `#sha256=<hex>`.
    pub fn canonical(&self) -> String {
        match &self.pin {
            Some(pin) => format!("{}#{PIN_OPTION}{}", self.label(), hex(pin)),
            None => self.label(),
        }
    }

    /// Request URL of a WebSocket endpoint (never with options).
    pub fn websocket_url(&self) -> Option<String> {
        if self.scheme == SignalScheme::Tcp {
            return None;
        }
        let path = if self.path.is_empty() {
            "/"
        } else {
            &self.path
        };
        Some(format!(
            "{}://{}{path}",
            self.scheme.name(),
            self.authority()
        ))
    }

    /// Relay URLs of this endpoint. Behind a path the relay shares the
    /// origin (reverse proxy); `se-share-server` itself serves it on the next
    /// port. A WebSocket endpoint without a path may be either, so both are
    /// offered; Iroh keeps the one that answers.
    fn relay_urls(&self) -> Vec<String> {
        let scheme = if self.is_encrypted() { "https" } else { "http" };
        let next_port = self.port().checked_add(1);
        let next = next_port.map(|port| format!("{scheme}://{}:{port}", self.url_host()));
        match self.scheme {
            SignalScheme::Tcp => next.into_iter().collect(),
            SignalScheme::Wss | SignalScheme::Ws => {
                let origin = format!("{scheme}://{}", self.authority());
                if self.path.is_empty() {
                    next.into_iter().chain(std::iter::once(origin)).collect()
                } else {
                    vec![origin]
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reading {
    Stored,
    Input { allow_plaintext: bool },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignalServerConfig {
    endpoints: Vec<SignalEndpoint>,
}

impl SignalServerConfig {
    /// A stored value: entries without a scheme are legacy plaintext TCP.
    pub fn parse_stored(raw: &str) -> Result<Self, String> {
        parse(raw, Reading::Stored)
    }

    /// User input: entries without a scheme mean `wss://host:51820`;
    /// plaintext entries need `allow_plaintext` and cannot be mixed with TLS.
    pub fn parse_input(raw: &str, allow_plaintext: bool) -> Result<Self, String> {
        let config = parse(raw, Reading::Input { allow_plaintext })?;
        if config.ignored_plaintext() > 0 {
            return Err(
                "Verschlüsselte und unverschlüsselte Adressen lassen sich nicht \
                        mischen: es gibt keinen Rückfall von TLS auf Klartext."
                    .into(),
            );
        }
        Ok(config)
    }

    pub fn is_empty(&self) -> bool {
        self.endpoints.is_empty()
    }

    pub fn endpoints(&self) -> &[SignalEndpoint] {
        &self.endpoints
    }

    /// The stored form of the whole configuration.
    pub fn canonical(&self) -> String {
        self.endpoints
            .iter()
            .map(SignalEndpoint::canonical)
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn has_tls(&self) -> bool {
        self.endpoints.iter().any(SignalEndpoint::is_encrypted)
    }

    /// Endpoints that are tried, in order: only TLS ones once any exists.
    pub fn active_endpoints(&self) -> Vec<&SignalEndpoint> {
        let tls = self.has_tls();
        self.endpoints
            .iter()
            .filter(|endpoint| !tls || endpoint.is_encrypted())
            .collect()
    }

    /// Plaintext entries that stay unused because a TLS entry exists.
    pub fn ignored_plaintext(&self) -> usize {
        if self.has_tls() {
            self.endpoints
                .iter()
                .filter(|endpoint| !endpoint.is_encrypted())
                .count()
        } else {
            0
        }
    }

    pub fn security(&self) -> ServerSecurity {
        if self.endpoints.is_empty() {
            ServerSecurity::NotConfigured
        } else if self.has_tls() {
            ServerSecurity::Encrypted
        } else {
            ServerSecurity::Plaintext
        }
    }

    /// The configuration runs in plaintext, which the user allowed; plain
    /// `http://` relays (own override, peer presences) are acceptable then.
    pub fn plaintext_permitted(&self) -> bool {
        self.security() == ServerSecurity::Plaintext
    }

    /// Relay URLs of the active endpoints, first occurrence first.
    pub fn relay_urls(&self) -> Vec<String> {
        let mut urls: Vec<String> = Vec::new();
        for url in self
            .active_endpoints()
            .into_iter()
            .flat_map(SignalEndpoint::relay_urls)
        {
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
        urls
    }

    /// Certificate pins of the active TLS endpoints.
    pub fn pins(&self) -> Vec<[u8; 32]> {
        let mut pins = Vec::new();
        for pin in self
            .active_endpoints()
            .into_iter()
            .filter_map(SignalEndpoint::pin)
        {
            if !pins.contains(&pin) {
                pins.push(pin);
            }
        }
        pins
    }

    /// One line for status and settings: security, active and ignored entries.
    pub fn summary(&self) -> String {
        let active: Vec<String> = self
            .active_endpoints()
            .into_iter()
            .map(SignalEndpoint::label)
            .collect();
        let mut summary = self.security().label().to_string();
        if !active.is_empty() {
            summary.push_str(&format!(" ({})", active.join(", ")));
        }
        let ignored = self.ignored_plaintext();
        if ignored > 0 {
            summary.push_str(&format!(
                "; {ignored} unverschlüsselte Adresse(n) ignoriert (kein Rückfall auf Klartext)"
            ));
        }
        summary
    }
}

/// The canonical form of a stored value when it differs (legacy values
/// without scheme, `http(s)://`, spacing); `None` when unchanged or invalid,
/// so an unreadable value is never overwritten.
pub fn migrate_stored(raw: &str) -> Option<String> {
    let config = SignalServerConfig::parse_stored(raw).ok()?;
    let canonical = config.canonical();
    (canonical != raw.trim()).then_some(canonical)
}

/// Whether a relay URL (own override or a peer's presence) may be dialed:
/// `https` always, plain `http` only with the plaintext permission.
pub fn relay_url_acceptable(url: &str, plaintext_permitted: bool) -> bool {
    let url = url.trim();
    url.starts_with("https://") || (plaintext_permitted && url.starts_with("http://"))
}

fn parse(raw: &str, reading: Reading) -> Result<SignalServerConfig, String> {
    let raw = raw.trim();
    if raw.len() > MAX_SERVER_CONFIG_BYTES || raw.chars().any(char::is_control) {
        return Err("Die Share-Server-Adresse ist ungültig oder zu lang.".into());
    }
    let mut endpoints: Vec<SignalEndpoint> = Vec::new();
    for token in raw
        .split([',', ';'])
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let endpoint = parse_endpoint(token, reading)?;
        if !endpoints.contains(&endpoint) {
            endpoints.push(endpoint);
        }
    }
    Ok(SignalServerConfig { endpoints })
}

fn parse_endpoint(token: &str, reading: Reading) -> Result<SignalEndpoint, String> {
    if token.chars().any(char::is_whitespace) || token.contains('@') {
        return Err(format!(
            "Die Share-Server-Adresse {token:?} enthält Leerzeichen oder Zugangsdaten."
        ));
    }
    let (address, options) = match token.split_once('#') {
        Some((address, options)) => (address, Some(options)),
        None => (token, None),
    };
    let (scheme, rest, explicit) = match address.split_once("://") {
        Some((scheme, rest)) => (scheme_named(scheme)?, rest, true),
        None => match reading {
            Reading::Stored => (SignalScheme::Tcp, address, false),
            Reading::Input { .. } => (SignalScheme::Wss, address, false),
        },
    };
    let (authority, path) = match rest.find('/') {
        Some(index) => rest.split_at(index),
        None => (rest, ""),
    };
    let (host, port) = parse_authority(authority)?;
    let port = match (scheme, port, explicit) {
        (_, Some(port), _) => Some(port),
        (SignalScheme::Tcp, None, _) | (_, None, false) => Some(DEFAULT_SIGNAL_PORT),
        (_, None, true) => None,
    };
    let path = match scheme {
        SignalScheme::Tcp if !path.trim_matches('/').is_empty() => {
            return Err(format!("tcp://-Adresse {token:?} darf keinen Pfad haben."));
        }
        SignalScheme::Tcp => String::new(),
        _ if path == "/" => String::new(),
        _ => path.to_string(),
    };
    let pin = parse_options(options, token)?;
    if pin.is_some() && scheme != SignalScheme::Wss {
        return Err("Ein Zertifikats-Fingerabdruck (#sha256=…) ist nur bei wss:// möglich.".into());
    }
    if let Reading::Input {
        allow_plaintext: false,
    } = reading
    {
        if !scheme.is_encrypted() {
            return Err(format!(
                "Die Share-Server-Adresse {token:?} {PLAINTEXT_DENIED}."
            ));
        }
    }
    Ok(SignalEndpoint {
        scheme,
        host,
        port,
        path,
        pin,
    })
}

fn scheme_named(scheme: &str) -> Result<SignalScheme, String> {
    match scheme.to_ascii_lowercase().as_str() {
        "wss" | "https" => Ok(SignalScheme::Wss),
        "ws" | "http" => Ok(SignalScheme::Ws),
        "tcp" => Ok(SignalScheme::Tcp),
        _ => Err(format!("Nicht unterstütztes Share-Server-Schema: {scheme}")),
    }
}

fn parse_options(options: Option<&str>, token: &str) -> Result<Option<[u8; 32]>, String> {
    let mut pin = None;
    for option in options
        .into_iter()
        .flat_map(|options| options.split('&'))
        .filter(|option| !option.is_empty())
    {
        let Some(value) = option.strip_prefix(PIN_OPTION) else {
            return Err(format!(
                "Unbekannte Option {option:?} in der Share-Server-Adresse {token:?}."
            ));
        };
        pin = Some(parse_pin(value)?);
    }
    Ok(pin)
}

/// 64 hex digits, optionally separated by `:` as `openssl x509 -fingerprint
/// -sha256` prints them.
pub fn parse_pin(value: &str) -> Result<[u8; 32], String> {
    let digits: String = value.chars().filter(|&c| c != ':').collect();
    let bytes = hex_decode(&digits).ok().filter(|bytes| bytes.len() == 32);
    bytes
        .and_then(|bytes| <[u8; 32]>::try_from(bytes.as_slice()).ok())
        .ok_or_else(|| {
            "Der Zertifikats-Fingerabdruck muss aus 64 Hex-Zeichen (SHA-256) bestehen.".to_string()
        })
}

fn parse_authority(authority: &str) -> Result<(String, Option<u16>), String> {
    if authority.is_empty() {
        return Err("Die Share-Server-Adresse nennt keinen Host.".into());
    }
    if let Some(rest) = authority.strip_prefix('[') {
        let (ip, after) = rest
            .split_once(']')
            .ok_or("IPv6-Adresse ohne schließende Klammer.")?;
        let ip: Ipv6Addr = ip
            .parse()
            .map_err(|_| format!("Ungültige IPv6-Adresse: {ip}"))?;
        let port = match after {
            "" => None,
            _ => Some(parse_port(
                after
                    .strip_prefix(':')
                    .ok_or("Nach der IPv6-Adresse muss :Port folgen.")?,
            )?),
        };
        return Ok((ip.to_string(), port));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(parse_port(port)?)),
        None => (authority, None),
    };
    if host.contains(':') {
        return Err("IPv6-Adressen in eckigen Klammern angeben, z. B. [::1]:51820.".into());
    }
    let valid = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '.' | '_'));
    if !valid {
        return Err(format!("Ungültiger Share-Server-Host: {host}"));
    }
    Ok((host.to_lowercase(), port))
}

fn parse_port(port: &str) -> Result<u16, String> {
    port.parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| format!("Ungültiger Share-Server-Port: {port}"))
}

#[cfg(test)]
#[path = "signal_connection_config_tests.rs"]
mod review_task_tests;
