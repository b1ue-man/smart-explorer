use super::download_file::download_file_progress;
use super::entries::{
    compile_remote_filter, validate_transfer_name, RemoteEntryCollector, TransferCollectionBudget,
    TransferErrorLog,
};
use super::progress::send_transfer_progress;
use super::uploads::upload_file_progress;
use super::{
    cleanup_temp_copy, find_remote_unique_name_avoiding, numbered_remote_name, open_temp_path,
    rjoin,
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
    same_server: bool,
    filter: Option<(FilterDef, String)>,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    cancel: &AtomicBool,
) {
    let filter = compile_remote_filter(filter);
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut errors = TransferErrorLog::default();
    let mut budget = TransferCollectionBudget::default();
    let mut reserved = std::collections::HashSet::new();
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
        let target_name = match find_remote_unique_name_avoiding(
            tgt,
            dest_root,
            |index| numbered_remote_name(name, index),
            &reserved,
            cancel,
        ) {
            Ok(name) => name,
            Err(error) => {
                errors.push(format!("{src_path}: {error}"));
                break;
            }
        };
        reserved.insert(rjoin(dest_root, &target_name));
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
                if same_server {
                    "Kopiere remote"
                } else {
                    "Uebertrage remote"
                },
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
            if same_server {
                "Kopiere remote"
            } else {
                "Uebertrage remote"
            },
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
    let bytes_total = if same_server {
        file_bytes
    } else {
        file_bytes.saturating_mul(2)
    };
    let mut progress = TransferProgress::new(
        TransferKind::RemoteCopy,
        if same_server {
            "Kopiere remote"
        } else {
            "Uebertrage remote"
        },
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
        let result = if same_server {
            if super::cancel::requested(cancel) {
                break;
            }
            let parent_ready = dest
                .rsplit_once('/')
                .map(|(parent, _)| tgt.mkdir_all(parent))
                .transpose()
                .map_err(|error| error.to_string());
            parent_ready.and_then(|_| {
                super::cancel::check(cancel)?;
                tgt.copy_file(&file.src, &dest)
                    .map(|_| {
                        progress.bytes_done = progress.bytes_done.saturating_add(file.size);
                    })
                    .map_err(|error| error.to_string())
            })
        } else {
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
        if super::cancel::requested(cancel) {
            break;
        }
        match result {
            Ok(()) => {
                progress.files_done = progress.files_done.saturating_add(1);
            }
            Err(e) => {
                errors.push(format!("{}: {}", file.rel, e));
                progress.errors = errors.total();
            }
        }
        progress.elapsed_ms = start.elapsed().as_millis() as u64;
        send_transfer_progress(tx, &progress, &mut last, true);
    }

    progress.elapsed_ms = start.elapsed().as_millis() as u64;
    progress.errors = errors.total();
    super::cancel::send_done(tx, progress, errors.into_displayed(), cancel);
}
