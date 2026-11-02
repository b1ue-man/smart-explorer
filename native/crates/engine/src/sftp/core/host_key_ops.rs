//! Host-key operations for the CLI, the desktop app and the Android app:
//! show a server's key, pin a key the person confirmed, forget a pin.
//!
//! Confirming always connects again and pins only when the server presents
//! exactly the fingerprint the person saw, so a key swapped between the
//! question and the answer is refused.
use std::future::Future;
use std::io;
use std::time::Duration;

use super::host_key::{HostKeyVerdict, UnknownHostKey};
use super::session::{host_key_async, HostKeyMode};

/// The SFTP connect deadline (`connection.rs`): resolve, TCP and key exchange.
const HOST_KEY_DEADLINE: Duration = Duration::from_secs(30);

/// A server's presented key and what the client knows about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentedHostKey {
    pub key: UnknownHostKey,
    /// Pinned by Smart Explorer or listed in OpenSSH's known_hosts.
    pub trusted: bool,
    /// Where a different key of the same type is recorded, if anywhere.
    pub changed_in: Option<String>,
}

/// Connects without login and reports the server's host key.
pub fn probe_host_key(host: &str, port: u16) -> io::Result<PresentedHostKey> {
    let outcome = run(host_key_async(host, port, HostKeyMode::Probe))?;
    let key = outcome.presented.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "the server did not present a host key",
        )
    })?;
    let (trusted, changed_in) = match outcome.verdict {
        Some(HostKeyVerdict::Trusted) => (true, None),
        Some(HostKeyVerdict::Changed { source }) => (false, Some(source)),
        Some(HostKeyVerdict::Revoked { source }) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "the host key of {host}:{port} is revoked in {source} ({} {})",
                    key.algorithm, key.fingerprint
                ),
            ))
        }
        Some(HostKeyVerdict::Unknown) | None => (false, None),
    };
    Ok(PresentedHostKey {
        key,
        trusted,
        changed_in,
    })
}

/// Pins the host key of `host:port` when the server presents `fingerprint`.
/// `replace` overwrites a different earlier pin (a key replaced on purpose).
pub fn trust_host_key(
    host: &str,
    port: u16,
    fingerprint: &str,
    replace: bool,
) -> io::Result<UnknownHostKey> {
    let outcome = run(host_key_async(
        host,
        port,
        HostKeyMode::Trust {
            expected: fingerprint.to_string(),
            replace,
        },
    ))?;
    outcome.presented.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "the server did not present a host key",
        )
    })
}

/// Removes Smart Explorer's pin of `host:port`; OpenSSH's files stay as they
/// are. `Ok(false)` when there was no pin.
pub fn forget_host_key(host: &str, port: u16) -> io::Result<bool> {
    super::known_hosts::forget(host, port)
}

/// The unknown host key named in an error message, if it names one.
pub fn unknown_host_key(message: &str) -> Option<UnknownHostKey> {
    UnknownHostKey::parse(message)
}

fn run<T>(future: impl Future<Output = io::Result<T>>) -> io::Result<T> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        tokio::time::timeout(HOST_KEY_DEADLINE, future)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "SSH host-key check timed out"))?
    })
}
