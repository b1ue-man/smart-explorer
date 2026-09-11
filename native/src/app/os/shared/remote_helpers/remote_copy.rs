use super::download_file::download_file_progress;
use super::entries::{
    compile_remote_filter, validate_transfer_name, RemoteEntryCollector, TransferCollectionBudget,
    TransferErrorLog,
};
use super::progress::send_transfer_progress;
use super::upload_plan::DestinationNames;
use super::uploads::upload_file_progress;
use super::{
    cleanup_temp_copy, open_temp_path, rjoin,
};
use crate::app::app_models::{TransferKind, TransferMsg, TransferProgress};
use crate::types::FilterDef;
use std::sync::atomic::AtomicBool;

// This worker entry point keeps source, destination, progress reporting, and
// cancellation inputs explicit because they cross the background-task boundary.
#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn copy_remote_paths_progress(
    src: &dyn crate::vfs::Backend,
    paths: &[String],
    tgt: &dyn crate::vfs::Backend,
    dest_root: &str,
    _same_server: bool,
    filter: Option<(FilterDef, String)>,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    cancel: &AtomicBool,
) {
    let filter = compile_remote_filter(filter);
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut errors = TransferErrorLog::default();
    let mut budget = TransferCollectionBudget::default();
    // Even same-backend copies use the owned local bridge: no replacing
    // server-copy primitive or overlapping read/write on a single session.
    let mut names = if super::cancel::requested(cancel) { None }
        else { Some(DestinationNames::new(tgt, dest_root)) };
    for src_path in paths {
        if super::cancel::requested(cancel) {
            break;
        }
        let name = src_path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("datei");
        if let Err(error) = validate_transfer_name(name, src_path) {
            errors.push(error);
            break;
        }
        if let Err(error) = budget.ensure_text_fits(&[src_path, name]) {
            errors.push(format!("{src_path}: {error}"));
            break;
        }
        let Some(names) = names.as_mut() else { break; };
        let target_name = match names.reserve(tgt, dest_root, name, cancel) {
            Ok(name) => name,
            Err(error) => {
                errors.push(format!("{src_path}: {error}"));
                break;
            }
        };
        let collected = RemoteEntryCollector {
            be: src,
            filter: filter.as_ref(),
            files: &mut files,
            dirs: &mut dirs,
            budget: &mut budget,
            cancel: Some(cancel),
        }
        .collect(src_path, target_name, true);
        if let Err(error) = collected {
            if !super::cancel::requested(cancel) {
                errors.push(error);
            }
            break;
        }
    }
    if super::cancel::requested(cancel) {
        super::cancel::send_done(
            tx,
            TransferProgress::new(
                TransferKind::RemoteCopy,
                "Uebertrage remote",
                0,
                0,
            ),
            Vec::new(),
            cancel,
        );
        return;
    }
    if !errors.is_empty() {
        let mut progress = TransferProgress::new(
            TransferKind::RemoteCopy,
            "Uebertrage remote",
            0,
            0,
        );
        progress.errors = errors.total();
        super::cancel::send_done(tx, progress, errors.into_displayed(), cancel);
        return;
    }
    dirs.sort();
    dirs.dedup();
    let file_bytes = files.iter().map(|f| f.size).fold(0u64, u64::saturating_add);
    let bytes_total = file_bytes.saturating_mul(2);
    let mut progress = TransferProgress::new(
        TransferKind::RemoteCopy,
        "Uebertrage remote",
        files.len() as u64,
        bytes_total,
    );
    progress.errors = errors.total();
    let mut last = std::time::Instant::now();
    send_transfer_progress(tx, &progress, &mut last, true);

    for dir in dirs {
        if super::cancel::requested(cancel) {
            break;
        }
        let dest = rjoin(dest_root, &dir);
        if let Err(e) = tgt.mkdir_all(&dest) {
            errors.push(format!("{}: {}", dest, e));
            progress.errors = errors.total();
        }
        if super::cancel::requested(cancel) {
            break;
        }
    }

    let start = std::time::Instant::now();
    for file in files {
        if super::cancel::requested(cancel) {
            break;
        }
        let dest = rjoin(dest_root, &file.rel);
        progress.current = file.rel.clone();
        progress.elapsed_ms = start.elapsed().as_millis() as u64;
        send_transfer_progress(tx, &progress, &mut last, true);
        let result = {
            let name = file.rel.rsplit('/').next().unwrap_or("datei");
            (|| -> Result<(), String> {
                let tmp = open_temp_path(name)
                    .map_err(|error| format!("Temporären Transferpfad anlegen: {error}"))?;
                let downloaded = download_file_progress(
                    src,
                    &file.src,
                    &tmp,
                    file.size,
                    tx,
                    &mut progress,
                    &mut last,
                    Some(cancel),
                );
                let uploaded = downloaded.and_then(|_| {
                    super::cancel::check(cancel)?;
                    upload_file_progress(tgt, &tmp, &dest, tx, &mut progress, &mut last, cancel)
                });
                cleanup_temp_copy(&tmp);
                uploaded
            })()
        };
        match result {
            Ok(()) => {
                progress.files_done = progress.files_done.saturating_add(1);
            }
            Err(e) if e == super::cancel::CANCELED_ERROR => {}
            Err(e) => {
                errors.push(format!("{}: {}", file.rel, e));
                progress.errors = errors.total();
            }
        }
        if super::cancel::requested(cancel) {
            break;
        }
        progress.elapsed_ms = start.elapsed().as_millis() as u64;
        send_transfer_progress(tx, &progress, &mut last, true);
    }

    progress.elapsed_ms = start.elapsed().as_millis() as u64;
    progress.errors = errors.total();
    super::cancel::send_done(tx, progress, errors.into_displayed(), cancel);
}
