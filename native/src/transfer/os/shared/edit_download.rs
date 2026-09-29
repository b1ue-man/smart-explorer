//! Whole-file recovery for read-only editor downloads. No remote mutation is
//! replayed, and no bytes from a failed stream reach the published editor copy.
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::vfs::{Backend, Scheme, VfsMeta};

use super::local_stage::{cleanup_partial, create_download_part, ensure_local_space};

const MAX_ATTEMPTS: usize = 3;
const RECONNECT_WINDOW: Duration = Duration::from_secs(45);

#[cfg(test)]
#[path = "edit_download_task_tests.rs"]
mod task_tests;

enum DownloadFailure {
    Remote(io::Error),
    Local(String),
}

impl DownloadFailure {
    fn message(&self) -> String {
        match self {
            Self::Remote(error) => error.to_string(),
            Self::Local(message) => message.clone(),
        }
    }

    fn retryable(&self) -> bool {
        matches!(self, Self::Remote(error) if matches!(error.kind(),
            io::ErrorKind::NotConnected | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted | io::ErrorKind::ConnectionRefused
                | io::ErrorKind::TimedOut | io::ErrorKind::BrokenPipe
                | io::ErrorKind::UnexpectedEof))
    }
}

/// Return the local path and the remote revision observed by the successful
/// attempt. The editor must not adopt an unrelated stat performed afterwards.
pub(crate) fn download_for_edit(
    backend: &dyn Backend,
    path: &str,
    id: Option<&str>,
    dest: &Path,
) -> Result<(String, i64), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut retry_deadline = None;
    for attempt in 0..MAX_ATTEMPTS {
        match download_once(backend, path, id, dest) {
            Ok(result) => return Ok(result),
            Err(failure) => {
                if backend.scheme() != Scheme::Peer
                    || !failure.retryable()
                    || attempt + 1 == MAX_ATTEMPTS
                {
                    return Err(failure.message());
                }
                // The window starts on failure, not at download start: a large
                // healthy file may take longer than the reconnection window.
                let deadline = *retry_deadline
                    .get_or_insert_with(|| Instant::now() + RECONNECT_WINDOW);
                let delay = Duration::from_secs((attempt + 1) as u64);
                if Instant::now() + delay >= deadline {
                    return Err(failure.message());
                }
                std::thread::sleep(delay);
                // A later PeerBackend read re-resolves routes/authorization and
                // replaces only the failed session generation. Never use ReadAt
                // without a revision token to bind old and new byte ranges.
            }
        }
    }
    Err("Remote-Download ohne Ergebnis beendet".into())
}

fn download_once(
    backend: &dyn Backend,
    path: &str,
    id: Option<&str>,
    dest: &Path,
) -> Result<(String, i64), DownloadFailure> {
    let peer = backend.scheme() == Scheme::Peer;
    let metadata = match backend.stat(path) {
        Ok(meta) if !meta.is_dir => Some(meta),
        Ok(_) => None,
        Err(error) if peer => return Err(DownloadFailure::Remote(error)),
        // Preserve ID-based/provider opens whose path-only stat is unavailable.
        Err(_) => None,
    };
    // A duplicate-named Drive item may have different metadata from path stat.
    // Do not validate its bytes or conflict baseline against a different ID.
    let metadata = metadata.filter(|meta| {
        peer || id.is_none() || meta.id.as_deref() == id
    });
    let expected = metadata.as_ref().map(|meta| meta.size).unwrap_or(0);
    ensure_local_space(dest, expected).map_err(DownloadFailure::Local)?;
    let read_size = metadata
        .as_ref()
        .map(|meta| backend.read_size(path, meta.size))
        .transpose()
        .map_err(DownloadFailure::Remote)?
        .flatten();
    let mut reader = backend
        .open_read_id(path, id)
        .map_err(DownloadFailure::Remote)?;
    let (part, mut output) = create_download_part(dest)
        .map_err(|error| DownloadFailure::Local(error.to_string()))?;
    let result = copy_checked(&mut *reader, &mut output, read_size).and_then(|()| {
        output.flush().and_then(|()| output.sync_all())
            .map_err(|error| DownloadFailure::Local(error.to_string()))?;
        if peer {
            let current = backend.stat(path).map_err(DownloadFailure::Remote)?;
            if !metadata.as_ref().is_some_and(|before| same_revision(before, &current)) {
                return Err(DownloadFailure::Local(
                    "Remote-Datei wurde während des Öffnens geändert; bitte erneut öffnen".into(),
                ));
            }
        }
        Ok(())
    });
    drop(reader);
    // Windows cannot reliably remove/replace a stage with its output still open.
    drop(output);
    if let Err(error) = result {
        cleanup_partial(&part);
        return Err(error);
    }
    if let Err(error) = super::platform::replace_file_atomic(&part, dest) {
        cleanup_partial(&part);
        return Err(DownloadFailure::Local(error.to_string()));
    }
    Ok((dest.to_string_lossy().into_owned(),
        metadata.map(|meta| meta.mtime_ms).unwrap_or(0)))
}

fn same_revision(before: &VfsMeta, after: &VfsMeta) -> bool {
    !after.is_dir && before.size == after.size && before.mtime_ms == after.mtime_ms
        && before.id == after.id
}

fn copy_checked(
    reader: &mut dyn Read,
    output: &mut dyn Write,
    expected: Option<u64>,
) -> Result<(), DownloadFailure> {
    let mut copied = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let limit = expected.map(|size| {
            size.saturating_sub(copied).saturating_add(1).min(buffer.len() as u64) as usize
        }).unwrap_or(buffer.len());
        let count = match reader.read(&mut buffer[..limit]) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(DownloadFailure::Remote(error)),
        };
        if count == 0 {
            break;
        }
        if expected.is_some_and(|size| count as u64 > size.saturating_sub(copied)) {
            return Err(DownloadFailure::Local("Download-Quelle ist während der Übertragung gewachsen".into()));
        }
        output.write_all(&buffer[..count])
            .map_err(|error| DownloadFailure::Local(error.to_string()))?;
        copied = copied.saturating_add(count as u64);
    }
    if let Some(expected) = expected {
        if copied != expected {
            return Err(DownloadFailure::Local(format!(
                "Download unvollstaendig: {copied} von {expected} Bytes")));
        }
    }
    Ok(())
}
