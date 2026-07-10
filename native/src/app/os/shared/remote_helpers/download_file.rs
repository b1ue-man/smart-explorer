use crate::app::app_models::{TransferMsg, TransferProgress};
use crate::app::platform_helpers::replace_file_atomic;
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
        let n = match reader.read(&mut buf) {
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
    if expected != 0 && copied != expected {
        cleanup_partial(&part);
        return Err(format!(
            "Download unvollstaendig: {copied} von {expected} Bytes"
        ));
    }
    if let Err(error) = replace_file_atomic(&part, dest) {
        cleanup_partial(&part);
        return Err(error.to_string());
    }
    super::cancel::check_optional(cancel)?;
    Ok(dest.to_string_lossy().to_string())
}
