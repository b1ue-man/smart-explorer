use std::io::{self, Read, Write};
use std::path::Path;

use iroh::RelayUrl;

use super::endpoint_routes::NodeTransportOptions;
use super::server_address::{migrate_stored, relay_url_acceptable, SignalServerConfig};

const RELAY_URL_ENV: &str = "SE_SHARE_RELAY_URL";
const RELAY_ONLY_ENV: &str = "SE_SHARE_RELAY_ONLY";
/// Larger server files are not ours to rewrite (the daemon refuses them).
const MAX_SERVER_FILE_BYTES: u64 = 16 * 1024;

/// Rewrites a stored server address in its canonical form (B21): a value
/// without scheme becomes `tcp://host:port`, which keeps its plaintext
/// meaning and shows it. Returns the new value when the file changed.
pub(crate) fn migrate_server_file(path: &Path) -> io::Result<Option<String>> {
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
    let Some(canonical) = migrate_stored(&raw) else {
        return Ok(None);
    };
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(canonical.as_bytes())?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(Some(canonical))
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
