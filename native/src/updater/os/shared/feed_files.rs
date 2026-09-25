//! Platform-neutral update-feed reads for hosts that install their own
//! package (the Android APK): the feed version, a `<name>.sha256` sidecar
//! and a streamed, cancelable download with progress. A feed is a folder or
//! an http(s) URL, classified exactly like the desktop updater's feed.
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::core::{parse_sha256_file, sha256_file};
use super::feed::{classify_feed, Feed};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(25);
/// Per read, so a large download on a slow link is not cut off as a whole.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_SIDECAR_BYTES: u64 = 64 * 1024;
const USER_AGENT: &str = "smart-explorer-updater";
const COPY_CHUNK: usize = 64 * 1024;

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
}

/// The feed's version (first line of `version.txt`), checked to look like
/// a version so a wrong feed fails with a clear message.
pub fn read_feed_version(feed: &str) -> Result<String, String> {
    let version = classify_feed(feed).read_version()?;
    let plausible = !version.is_empty()
        && version.len() <= 64
        && version.starts_with(|c: char| c.is_ascii_digit())
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if plausible {
        Ok(version)
    } else {
        Err(format!(
            "Update-Feed liefert keine gültige Version: „{version}“"
        ))
    }
}

/// The SHA-256 of `name` from its `<name>.sha256` sidecar (sha256sum format).
pub fn read_feed_sha256(feed: &str, name: &str) -> Result<String, String> {
    let hash_name = format!("{name}.sha256");
    let mut raw = String::new();
    open_feed_file(feed, &hash_name)?
        .0
        .take(MAX_SIDECAR_BYTES)
        .read_to_string(&mut raw)
        .map_err(|error| format!("Prüfsumme {hash_name} lesen: {error}"))?;
    parse_sha256_file(&raw, &hash_name)
}

/// Streams `name` to `dest` (through `dest` + `.part`), reporting
/// `(done, total)` bytes. A cancel or error removes the partial file.
pub fn download_feed_file(
    feed: &str,
    name: &str,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<(), String> {
    let (mut reader, total) = open_feed_file(feed, name)?;
    let partial = dest.with_file_name(format!(
        "{}.part",
        dest.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "download".to_string())
    ));
    let result = (|| {
        let mut file = std::fs::File::create(&partial)
            .map_err(|error| format!("Download-Datei {}: {error}", partial.display()))?;
        let mut buffer = vec![0u8; COPY_CHUNK];
        let mut done = 0u64;
        progress(0, total);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("Download abgebrochen".to_string());
            }
            let read = reader
                .read(&mut buffer)
                .map_err(|error| format!("Download {name}: {error}"))?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read])
                .map_err(|error| format!("Download schreiben: {error}"))?;
            done = done.saturating_add(read as u64);
            progress(done, total);
        }
        if total.is_some_and(|total| total != done) {
            return Err(format!("Download {name} unvollständig ({done} Bytes)"));
        }
        file.sync_all()
            .map_err(|error| format!("Download sichern: {error}"))?;
        drop(file);
        std::fs::rename(&partial, dest)
            .map_err(|error| format!("Download bereitstellen {}: {error}", dest.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

/// Lowercase hex SHA-256 of a local file.
pub fn file_sha256(path: &Path) -> Result<String, String> {
    sha256_file(path)
}

type FeedReader = Box<dyn Read + Send + Sync>;

fn open_feed_file(feed: &str, name: &str) -> Result<(FeedReader, Option<u64>), String> {
    match classify_feed(feed) {
        Feed::Local(dir) => {
            let path = dir.join(name);
            let file = std::fs::File::open(&path)
                .map_err(|error| format!("Update-Feed {}: {error}", path.display()))?;
            let size = file.metadata().map(|metadata| metadata.len()).ok();
            Ok((Box::new(file), size))
        }
        Feed::Http(base) => {
            let url = format!("{base}/{name}");
            let response = agent()
                .get(&url)
                .call()
                .map_err(|error| format!("HTTP {url}: {error}"))?;
            let size = response
                .header("Content-Length")
                .and_then(|value| value.trim().parse().ok());
            Ok((response.into_reader(), size))
        }
    }
}
