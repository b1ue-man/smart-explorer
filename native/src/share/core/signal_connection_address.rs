//! Parsing of one signaling endpoint, including its authority and TLS pin.

use super::{
    Reading, SignalEndpoint, SignalScheme, DEFAULT_SIGNAL_PORT, PIN_OPTION, PLAINTEXT_DENIED,
};
use crate::share::core::hex_decode;
use std::net::Ipv6Addr;

pub(super) fn parse_endpoint(token: &str, reading: Reading) -> Result<SignalEndpoint, String> {
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
        if pin.is_some() {
            return Err("Nur einen Zertifikats-Fingerabdruck je Adresse angeben.".into());
        }
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
