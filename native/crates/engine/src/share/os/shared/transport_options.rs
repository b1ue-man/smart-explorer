use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use iroh::RelayUrl;

use super::endpoint_routes::NodeTransportOptions;
use super::server_address::{migrate_stored, relay_url_acceptable, SignalServerConfig};

#[path = "relay_dns.rs"]
mod relay_dns;

/// How the Iroh endpoint resolves relay host names (`relay_dns.rs`).
pub(super) fn dns_resolver() -> iroh::dns::DnsResolver {
    relay_dns::resolver()
}

const RELAY_URL_ENV: &str = "SE_SHARE_RELAY_URL";
const RELAY_ONLY_ENV: &str = "SE_SHARE_RELAY_ONLY";
/// Larger server files are not ours to rewrite (the daemon refuses them).
const MAX_SERVER_FILE_BYTES: u64 = 16 * 1024;

/// Rewrites a stored server address in its canonical form (B21): a value
/// without scheme becomes `tcp://host:port`, which keeps its plaintext
/// meaning and shows it. Returns the new value when the file changed.
pub fn migrate_server_file(path: &Path) -> io::Result<Option<String>> {
    let Some(raw) = read_server_file(path)? else {
        return Ok(None);
    };
    let Some(canonical) = migrate_stored(&raw) else {
        return Ok(None);
    };
    write_server_file(path, &canonical)?;
    Ok(Some(canonical))
}

/// Switches a plaintext server address to the same server over TLS once the
/// server proves it there (TODO `DIREKT-INTERNET`, 2026-11-17): a device
/// with `tcp://host:port` gets an `http://` relay, and QUIC address
/// discovery runs only over TLS (`endpoint_routes.rs` `relay_mode`), so such
/// a device never learned its public address and every peer connection
/// across the internet fell back to the relay. The rule is
/// [`encrypted_upgrade`]; this writes its result. One try per process
/// (worker start).
pub fn upgrade_server_file(path: &Path) -> io::Result<Option<String>> {
    let Some(raw) = read_server_file(path)? else {
        return Ok(None);
    };
    let Some(encrypted) = encrypted_upgrade(&raw) else {
        return Ok(None);
    };
    write_server_file(path, &encrypted)?;
    Ok(Some(encrypted))
}

/// The encrypted address that replaces the stored plaintext address `raw`,
/// or `None`. The encrypted alternative
/// (`SignalServerConfig::encrypted_alternative`) is connected with the
/// normal certificate check; only a completed TLS and WebSocket handshake
/// yields it. A server without a valid certificate for its name keeps the
/// plaintext address. Blocks for that network attempt, so callers holding a
/// lock run it outside (Android: `share_server_upgrade.rs` of the mobile
/// crate). One network attempt per process for every caller together; an
/// address without plaintext form costs none.
pub fn encrypted_upgrade(raw: &str) -> Option<String> {
    let config = SignalServerConfig::parse_stored(raw).ok()?;
    let encrypted = config.encrypted_alternative()?;
    SignalServerConfig::parse_stored(&encrypted).ok()?;
    if UPGRADE_TRIED.swap(true, Ordering::AcqRel) {
        return None;
    }
    super::signal_connection::SignalConnection::connect(&encrypted).ok()?;
    Some(encrypted)
}

static UPGRADE_TRIED: AtomicBool = AtomicBool::new(false);

/// The worker's read of the server file: the canonical form (B21), then
/// the switch to TLS (`upgrade_server_file`); the new value when it changed.
pub fn refresh_server_file(path: &Path) -> io::Result<Option<String>> {
    let migrated = migrate_server_file(path)?;
    match upgrade_server_file(path) {
        Ok(Some(encrypted)) => {
            eprintln!("share: Share-Server-Adresse auf {encrypted} umgestellt (verschlüsselt, Adresserkennung für Direktwege)");
            Ok(Some(encrypted))
        }
        _ => Ok(migrated),
    }
}

/// The stored server address, `None` without a file or above the limit.
fn read_server_file(path: &Path) -> io::Result<Option<String>> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut raw = String::new();
    file.take(MAX_SERVER_FILE_BYTES + 1)
        .read_to_string(&mut raw)?;
    if raw.len() as u64 > MAX_SERVER_FILE_BYTES {
        return Ok(None);
    }
    Ok(Some(raw))
}

fn write_server_file(path: &Path, value: &str) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(value.as_bytes())?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Relay settings of the Iroh endpoint for a stored server address: the
/// relays of the active endpoints (HTTPS for TLS; HTTP only while the user
/// allowed a plaintext server), the server's certificate pins, and the
/// `SE_SHARE_RELAY_URL` override under the same plaintext rule (FC3).
pub(super) fn load(server: &str) -> NodeTransportOptions {
    let config = SignalServerConfig::parse_stored(server).unwrap_or_default();
    let plaintext = config.plaintext_permitted();
    let candidates = match std::env::var(RELAY_URL_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        Some(value) => override_candidates(&value),
        None => config.relay_urls(),
    };
    let relay_only = std::env::var(RELAY_ONLY_ENV)
        .map(|value| value.trim() == "1")
        .unwrap_or(false);
    NodeTransportOptions::new(relay_urls(candidates, plaintext), relay_only)
        .with_server_trust(config.pins(), plaintext)
}

/// Relay URLs are taken as given; other entries are read like a stored
/// server address and yield its relays, as before.
fn override_candidates(value: &str) -> Vec<String> {
    value
        .split([',', ';'])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .flat_map(|entry| {
            if entry.starts_with("https://") || entry.starts_with("http://") {
                vec![entry.to_string()]
            } else {
                SignalServerConfig::parse_stored(entry)
                    .map(|config| config.relay_urls())
                    .unwrap_or_default()
            }
        })
        .collect()
}

fn relay_urls(candidates: Vec<String>, plaintext: bool) -> Vec<RelayUrl> {
    let mut urls: Vec<RelayUrl> = Vec::new();
    for candidate in candidates {
        if !relay_url_acceptable(&candidate, plaintext) {
            continue;
        }
        let Ok(url) = candidate.parse::<RelayUrl>() else {
            continue;
        };
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    urls
}

#[cfg(test)]
mod review_task_tests {
    use super::{override_candidates, relay_urls};

    #[test]
    fn review_task_plain_relays_are_dropped_without_the_plaintext_permission() {
        let candidates = vec![
            "https://relay.example".to_string(),
            "http://relay.example:51821".to_string(),
        ];
        let secure = relay_urls(candidates.clone(), false);
        assert_eq!(secure.len(), 1);
        assert_eq!(secure[0].scheme(), "https");
        assert_eq!(relay_urls(candidates, true).len(), 2);
    }

    #[test]
    fn review_task_relay_override_keeps_urls_and_reads_addresses() {
        assert_eq!(
            override_candidates("http://127.0.0.1:51821"),
            ["http://127.0.0.1:51821"]
        );
        assert_eq!(
            override_candidates("tcp://127.0.0.1:51820"),
            ["http://127.0.0.1:51821"]
        );
    }
}
