use crate::app::app_models::{TransferMsg, TransferProgress};
use crate::app::transfer_helpers::{cleanup_partial, create_download_part, ensure_local_space};
use std::path::Path;
use std::sync::atomic::AtomicBool;

// Keep the transfer boundary explicit: the backend/path inputs and mutable
// progress/cancellation state have separate ownership and lifetime semantics.
#[allow(clippy::too_many_arguments)]
pub(super) fn download_file_progress(
    be: &dyn crate::vfs::Backend,
    src: &str,
    dest: &Path,
    expected: u64,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    progress: &mut TransferProgress,
    last: &mut std::time::Instant,
    cancel: Option<&AtomicBool>,
) -> Result<String, String> {
    use std::io::{Read, Write};

    super::cancel::check_optional(cancel)?;
    let read_size = be.read_size(src, expected).map_err(|error| error.to_string())?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        super::cancel::check_optional(cancel)?;
    }
    ensure_local_space(dest, expected)?;
    let mut reader = be.open_read(src).map_err(|e| e.to_string())?;
    super::cancel::check_optional(cancel)?;
    let (part, mut output) = create_download_part(dest).map_err(|e| e.to_string())?;
    let mut copied = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        if let Err(error) = super::cancel::check_optional(cancel) {
            drop(output);
            cleanup_partial(&part);
            return Err(error);
        }
        let limit = read_size.map(|length| length.saturating_sub(copied).saturating_add(1)
            .min(buf.len() as u64) as usize).unwrap_or(buf.len());
        let n = match reader.read(&mut buf[..limit]) {
            Ok(n) => n,
            Err(error) => {
                drop(output);
                cleanup_partial(&part);
                return Err(error.to_string());
            }
        };
        if let Err(error) = super::cancel::check_optional(cancel) {
            drop(output);
            cleanup_partial(&part);
            return Err(error);
        }
        if n == 0 {
            break;
        }
        if read_size.is_some_and(|length| n as u64 > length.saturating_sub(copied)) {
            drop(output);
            cleanup_partial(&part);
            return Err("Download-Quelle ist während der Übertragung gewachsen".to_string());
        }
        if let Err(error) = output.write_all(&buf[..n]) {
            drop(output);
            cleanup_partial(&part);
            return Err(error.to_string());
        }
        if let Err(error) = super::cancel::check_optional(cancel) {
            drop(output);
            cleanup_partial(&part);
            return Err(error);
        }
        copied = copied.saturating_add(n as u64);
        progress.bytes_done = progress.bytes_done.saturating_add(n as u64);
        super::progress::send_transfer_progress(tx, progress, last, false);
    }
    if let Err(error) = output.flush().and_then(|_| output.sync_all()) {
        drop(output);
        cleanup_partial(&part);
        return Err(error.to_string());
    }
    drop(output);
    if let Err(error) = super::cancel::check_optional(cancel) {
        cleanup_partial(&part);
        return Err(error);
    }
    if read_size.is_some_and(|length| copied != length) {
        cleanup_partial(&part);
        return Err(format!(
            "Download unvollstaendig: {copied} von {expected} Bytes"
        ));
    }
    // The destination was selected as absent, but another actor may create it
    // during the download. Copy publication must never replace that entry.
    if let Err(error) = crate::vfs::promote_local_copy(&part, dest) {
        cleanup_partial(&part);
        return Err(format!("Download-Ziel „{}“ ohne Ersetzen veröffentlichen ({:?}): {error}", dest.display(), error.kind()));
    }
    // Publication was acknowledged. Cancellation may stop the next file, but
    // must not retroactively hide this completed destination from accounting.
    Ok(dest.to_string_lossy().to_string())
}
