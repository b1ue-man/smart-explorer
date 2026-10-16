//! Host-key decisions of the SSH client.
//!
//! A server key is trusted when Smart Explorer pinned it before
//! (`known_hosts_sftp.txt`) or when the user's OpenSSH `known_hosts` lists it.
//! An unknown key is never stored silently: the connection ends with an
//! error that carries the key type and SHA-256 fingerprint in a fixed,
//! machine-readable form, and the CLI, the desktop app and the Android app
//! ask the person before they pin it (OpenSSH `StrictHostKeyChecking ask`).
//!
//! OpenSSH known_hosts format: sshd(8) "SSH_KNOWN_HOSTS FILE FORMAT"
//! (OpenSSH 10.x manual, read 2026-10-16): optional marker `@cert-authority`
//! or `@revoked`, comma-separated host patterns with `*`/`?` wildcards and
//! `!` negation, hashed names `|1|base64(salt)|base64(HMAC-SHA1(salt, host))`,
//! `[host]:port` for ports other than 22, then key type and base64 key.
use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha1::Sha1;

/// What the client decides about a presented host key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HostKeyVerdict {
    Trusted,
    Unknown,
    /// A different key of the same type is recorded in `source`.
    Changed {
        source: String,
    },
    /// `source` lists this key as `@revoked`.
    Revoked {
        source: String,
    },
}

/// The key a server presented, as shown to the person deciding about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownHostKey {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
}

const MARKER_START: &str = "[se-host-key-unknown ";
const MARKER_END: &str = "]";

impl UnknownHostKey {
    /// Error text with the fixed marker the UIs recognize, plus the command
    /// that confirms the key without a prompt.
    pub fn message(&self) -> String {
        format!(
            "SSH host key of {host}:{port} is not known yet ({algorithm} {fingerprint}); \
             nothing was saved. Compare the fingerprint with the server (ssh-keygen -lf on \
             the server, or the provider's documentation) and confirm it in Smart Explorer \
             (host-key prompt or dialog) or with \
             `se connections trust-host-key {host}:{port} --fingerprint {fingerprint}`. \
             {MARKER_START}host={host} port={port} type={algorithm} fp={fingerprint}{MARKER_END}",
            host = self.host,
            port = self.port,
            algorithm = self.algorithm,
            fingerprint = self.fingerprint,
        )
    }

    /// The key named by the marker inside `message`, if there is one.
    pub fn parse(message: &str) -> Option<Self> {
        let start = message.find(MARKER_START)? + MARKER_START.len();
        let end = start + message[start..].find(MARKER_END)?;
        let mut host = None;
        let mut port = None;
        let mut algorithm = None;
        let mut fingerprint = None;
        for field in message[start..end].split(' ') {
            let (name, value) = field.split_once('=')?;
            match name {
                "host" => host = Some(value.to_string()),
                "port" => port = value.parse().ok(),
                "type" => algorithm = Some(value.to_string()),
                "fp" => fingerprint = Some(value.to_string()),
                _ => {}
            }
        }
        Some(Self {
            host: host.filter(|host| !host.is_empty())?,
            port: port?,
            algorithm: algorithm?,
            fingerprint: fingerprint.filter(|fp| fp.starts_with("SHA256:"))?,
        })
    }
}

/// The name OpenSSH looks up: `host` on port 22, `[host]:port` otherwise.
pub(crate) fn openssh_host_name(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    }
}

/// One usable line of an OpenSSH known_hosts file.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct KnownHostsLine<'a> {
    pub(crate) revoked: bool,
    pub(crate) patterns: &'a str,
    pub(crate) key_base64: &'a str,
}

/// Parses one line; comments, blank lines, `@cert-authority` lines (host
/// certificates are not used) and malformed lines yield `None`.
pub(crate) fn parse_known_hosts_line(line: &str) -> Option<KnownHostsLine<'_>> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut fields = line.split_whitespace();
    let mut first = fields.next()?;
    let mut revoked = false;
    if first.starts_with('@') {
        match first {
            "@revoked" => revoked = true,
            _ => return None,
        }
        first = fields.next()?;
    }
    let _key_type = fields.next()?;
    let key_base64 = fields.next()?;
    Some(KnownHostsLine {
        revoked,
        patterns: first,
        key_base64,
    })
}

/// Whether the comma-separated `patterns` select `name`: at least one plain
/// pattern matches and no negated one does.
pub(crate) fn patterns_match(patterns: &str, name: &str) -> bool {
    let mut matched = false;
    for pattern in patterns.split(',') {
        if let Some(negated) = pattern.strip_prefix('!') {
            if pattern_matches(negated, name) {
                return false;
            }
        } else if pattern_matches(pattern, name) {
            matched = true;
        }
    }
    matched
}

fn pattern_matches(pattern: &str, name: &str) -> bool {
    if let Some(hashed) = pattern.strip_prefix("|1|") {
        return hashed_matches(hashed, name);
    }
    wildcard(
        pattern.to_ascii_lowercase().as_bytes(),
        name.to_ascii_lowercase().as_bytes(),
    )
}

fn hashed_matches(hashed: &str, name: &str) -> bool {
    let Some((salt, hash)) = hashed.split_once('|') else {
        return false;
    };
    let engine = base64::engine::general_purpose::STANDARD;
    let (Ok(salt), Ok(hash)) = (engine.decode(salt), engine.decode(hash)) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(&salt) else {
        return false;
    };
    // `ssh` and `ssh-keygen -H` hash the lower-case name.
    mac.update(name.to_ascii_lowercase().as_bytes());
    mac.verify_slice(&hash).is_ok()
}

/// `*` matches any run, `?` one character (OpenSSH `match_pattern`).
fn wildcard(pattern: &[u8], name: &[u8]) -> bool {
    match (pattern.first(), name.first()) {
        (None, None) => true,
        (Some(b'*'), _) => {
            wildcard(&pattern[1..], name) || (!name.is_empty() && wildcard(pattern, &name[1..]))
        }
        (Some(b'?'), Some(_)) => wildcard(&pattern[1..], &name[1..]),
        (Some(expected), Some(actual)) if expected == actual => wildcard(&pattern[1..], &name[1..]),
        _ => false,
    }
}

#[cfg(test)]
#[path = "host_key_tests.rs"]
mod tests;
