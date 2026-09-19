//! Explicit source/destination snapshots used by filtered clipboard paste.
use super::outcome::CopyErrorLog;
use super::relative::safe_rel_path;
use super::safe_file::{transfer_file, TransferResult};
use super::{planning, read_access, send_copy_failure, CopyHandle, CopyMsg};
use crate::types::{Conflict, CopyMode, CopyProgress};
use crossbeam_channel::Sender;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

/// Copy explicit (absolute source, relative destination) pairs into `dest`.
/// Used for the in-app paste fast path of the filter-aware clipboard, where
/// the relative structure was computed at copy time.
pub fn start_copy_pairs(
    pairs: Vec<(String, String)>,
    dest: PathBuf,
    conflict: Conflict,
    tx: Sender<CopyMsg>,
) -> CopyHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();
    let failure_tx = tx.clone();

    let spawn = std::thread::Builder::new()
        .name("copy-driver".into())
        .spawn(move || {
            let start = Instant::now();
            if let Err(error) = planning::validate_pair_budget(&pairs) {
                send_copy_failure(&tx, dest.display().to_string(), error);
                return;
            }
            if let Err(error) = read_access::admit(
                pairs.iter().map(|(path, _)| path.as_str()),
                None,
                &cancel_clone,
            ) {
                send_copy_failure(&tx, dest.display().to_string(), error.to_string());
                return;
            }
            let files_total = pairs.len() as u64;
            let bytes_total: u64 = pairs
                .iter()
                .filter_map(|(abs, _)| {
                    crate::local_access::symlink_metadata(Path::new(abs))
                        .ok()
                        .map(|m| m.len())
                })
                .sum();
            let mut files_done = 0u64;
            let mut bytes_done = 0u64;
            let mut errors = CopyErrorLog::default();
            let mut last_progress = Instant::now();

            for (abs, rel) in &pairs {
                if cancel_clone.load(Ordering::Relaxed) {
                    break;
                }
                let Some(rel_path) = safe_rel_path(rel) else {
                    errors.record(abs.clone(), "ungueltiger relativer Zielpfad".to_string());
                    files_done += 1;
                    continue;
                };
                let target = dest.join(rel_path);
                match transfer_file(
                    Path::new(abs),
                    &target,
                    &dest,
                    conflict,
                    CopyMode::Copy,
                    &cancel_clone,
                ) {
                    Ok(TransferResult::Completed(n)) => {
                        files_done += 1;
                        bytes_done = bytes_done.saturating_add(n);
                    }
                    Ok(TransferResult::Skipped) => files_done += 1,
                    Ok(TransferResult::Canceled) => break,
                    Err(e) => {
                        errors.record(abs.clone(), e.to_string());
                        files_done += 1;
                    }
                }
                if last_progress.elapsed().as_millis() > 80 {
                    let _ = tx.send(CopyMsg::Progress(CopyProgress {
                        files_done,
                        files_total,
                        bytes_done,
                        bytes_total,
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        errors: errors.total(),
                        canceled: false,
                        done: false,
                    }));
                    last_progress = Instant::now();
                }
            }

            let _ = tx.send(CopyMsg::Done {
                progress: CopyProgress {
                    files_done,
                    files_total,
                    bytes_done,
                    bytes_total,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    errors: errors.total(),
                    canceled: cancel_clone.load(Ordering::Relaxed),
                    done: true,
                },
                errors: errors.into_items(),
            });
        });
    if let Err(error) = spawn {
        send_copy_failure(&failure_tx, "Kopieren".to_string(), error.to_string());
    }

    CopyHandle { cancel }
}
