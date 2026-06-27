use super::downloads::download_file_progress;
use super::entries::{compile_remote_filter, RemoteEntryCollector};
use super::progress::send_transfer_progress;
use super::uploads::upload_file_progress;
use super::{cleanup_temp_copy, open_temp_path, rjoin};
use crate::app::app_models::{TransferKind, TransferMsg, TransferProgress};
use crate::types::FilterDef;

pub(in crate::app) fn copy_remote_paths_progress(
    src: &dyn crate::vfs::Backend,
    paths: &[String],
    tgt: &dyn crate::vfs::Backend,
    dest_root: &str,
    same_server: bool,
    filter: Option<(FilterDef, String)>,
    tx: &crossbeam_channel::Sender<TransferMsg>,
) {
    let filter = compile_remote_filter(filter);
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut errors = Vec::new();
    for src_path in paths {
        let name = src_path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("datei");
        RemoteEntryCollector {
            be: src,
            filter: filter.as_ref(),
            files: &mut files,
            dirs: &mut dirs,
            errors: &mut errors,
        }
        .collect(src_path, name.to_string(), true);
    }
    dirs.sort();
    dirs.dedup();
    let file_bytes = files.iter().map(|f| f.size).sum::<u64>();
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
    progress.errors = errors.len() as u64;
    let mut last = std::time::Instant::now();
    send_transfer_progress(tx, &progress, &mut last, true);

    for dir in dirs {
        let dest = rjoin(dest_root, &dir);
        if let Err(e) = tgt.mkdir_all(&dest) {
            errors.push(format!("{}: {}", dest, e));
            progress.errors = errors.len() as u64;
        }
    }

    let start = std::time::Instant::now();
    for file in files {
        let dest = rjoin(dest_root, &file.rel);
        progress.current = file.rel.clone();
        progress.elapsed_ms = start.elapsed().as_millis() as u64;
        send_transfer_progress(tx, &progress, &mut last, true);
        let result = if same_server {
            if let Some((parent, _)) = dest.rsplit_once('/') {
                let _ = tgt.mkdir_all(parent);
            }
            tgt.copy_file(&file.src, &dest)
                .map(|_| {
                    progress.bytes_done = progress.bytes_done.saturating_add(file.size);
                })
                .map_err(|e| e.to_string())
        } else {
            let name = file.rel.rsplit('/').next().unwrap_or("datei");
            let tmp = open_temp_path(name);
            let downloaded = download_file_progress(
                src,
                &file.src,
                &tmp,
                file.size,
                tx,
                &mut progress,
                &mut last,
            );
            let uploaded = downloaded
                .and_then(|_| upload_file_progress(tgt, &tmp, &dest, tx, &mut progress, &mut last));
            cleanup_temp_copy(&tmp);
            uploaded
        };
        match result {
            Ok(()) => {
                progress.files_done = progress.files_done.saturating_add(1);
            }
            Err(e) => {
                errors.push(format!("{}: {}", file.rel, e));
                progress.errors = errors.len() as u64;
                progress.files_done = progress.files_done.saturating_add(1);
                let missing = if same_server {
                    file.size
                } else {
                    file.size.saturating_mul(2)
                };
                progress.bytes_done = progress.bytes_done.saturating_add(missing);
            }
        }
        progress.elapsed_ms = start.elapsed().as_millis() as u64;
        send_transfer_progress(tx, &progress, &mut last, true);
    }

    progress.done = true;
    progress.elapsed_ms = start.elapsed().as_millis() as u64;
    progress.errors = errors.len() as u64;
    let _ = tx.send(TransferMsg::Done { progress, errors });
}
