use super::entries::{compile_remote_filter, RemoteEntryCollector, RemoteFilterCtx};
use super::open_temp_path;
use super::progress::send_transfer_progress;
use crate::app::app_models::{TransferKind, TransferMsg, TransferProgress};
use crate::app::platform_helpers::replace_file_atomic;
use crate::app::support_paths::download_to_temp;
use crate::app::transfer_helpers::{
    cleanup_partial, download_part_path, download_to, ensure_local_space,
};
use crate::types::FilterDef;
use std::path::{Path, PathBuf};

pub(super) fn download_file_progress(
    be: &dyn crate::vfs::Backend,
    src: &str,
    dest: &Path,
    expected: u64,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    progress: &mut TransferProgress,
    last: &mut std::time::Instant,
) -> Result<String, String> {
    use std::io::{Read, Write};

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    ensure_local_space(dest, expected)?;
    let part = download_part_path(dest);
    cleanup_partial(&part);
    let mut r = be.open_read(src).map_err(|e| e.to_string())?;
    let mut f = match std::fs::File::create(&part) {
        Ok(f) => f,
        Err(e) => {
            cleanup_partial(&part);
            return Err(e.to_string());
        }
    };
    let mut copied = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = match r.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                cleanup_partial(&part);
                return Err(e.to_string());
            }
        };
        if n == 0 {
            break;
        }
        if let Err(e) = f.write_all(&buf[..n]) {
            cleanup_partial(&part);
            return Err(e.to_string());
        }
        copied = copied.saturating_add(n as u64);
        progress.bytes_done = progress.bytes_done.saturating_add(n as u64);
        send_transfer_progress(tx, progress, last, false);
    }
    if let Err(e) = f.flush().and_then(|_| f.sync_all()) {
        cleanup_partial(&part);
        return Err(e.to_string());
    }
    drop(f);
    if expected != 0 && copied != expected {
        cleanup_partial(&part);
        return Err(format!(
            "Download unvollstaendig: {} von {} Bytes",
            copied, expected
        ));
    }
    if let Err(e) = replace_file_atomic(&part, dest) {
        cleanup_partial(&part);
        return Err(e.to_string());
    }
    Ok(dest.to_string_lossy().to_string())
}

fn download_remote_dir_for_clipboard(
    be: &dyn crate::vfs::Backend,
    src: &str,
    local_dir: &Path,
    filter: Option<&RemoteFilterCtx>,
) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(local_dir);
    std::fs::create_dir_all(local_dir).map_err(|e| e.to_string())?;
    if filter.is_none() && be.supports_bulk_tree() {
        match be.get_tree(src, local_dir) {
            Ok(_) => return Ok(()),
            Err(_) => {
                let _ = std::fs::remove_dir_all(local_dir);
                std::fs::create_dir_all(local_dir).map_err(|e| e.to_string())?;
            }
        }
    }

    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut errors = Vec::new();
    RemoteEntryCollector {
        be,
        filter,
        files: &mut files,
        dirs: &mut dirs,
        errors: &mut errors,
    }
    .collect(src, String::new(), true);
    if filter.is_some() && files.is_empty() {
        return Err("Keine passenden Dateien".to_string());
    }
    dirs.sort();
    dirs.dedup();
    for dir in dirs {
        if dir.is_empty() {
            continue;
        }
        std::fs::create_dir_all(local_dir.join(dir.replace('/', std::path::MAIN_SEPARATOR_STR)))
            .map_err(|e| e.to_string())?;
    }
    for file in files {
        let dest = local_dir.join(file.rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        download_to(be, &file.src, &dest)?;
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub(in crate::app) fn download_remote_clipboard_items(
    be: &dyn crate::vfs::Backend,
    items: &[(String, String, bool)],
    filter: Option<(FilterDef, String)>,
) -> Vec<String> {
    let filter = compile_remote_filter(filter);
    let mut local = Vec::new();
    for (path, name, is_dir) in items {
        if *is_dir {
            let local_dir = open_temp_path(name);
            if download_remote_dir_for_clipboard(be, path, &local_dir, filter.as_ref()).is_ok() {
                local.push(local_dir.to_string_lossy().to_string());
            } else {
                let _ = std::fs::remove_dir_all(&local_dir);
            }
        } else {
            let local_name = be.download_name(path, name);
            if let Ok(p) = download_to_temp(be, path, &local_name) {
                local.push(p);
            }
        }
    }
    local
}

pub(in crate::app) fn download_remote_paths_for_clipboard(
    be: &dyn crate::vfs::Backend,
    paths: &[String],
    filter: Option<(FilterDef, String)>,
) -> Vec<String> {
    let filter = compile_remote_filter(filter);
    let mut local = Vec::new();
    for path in paths {
        let meta = match be.stat(path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        let name = if meta.name.is_empty() {
            path.trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or("datei")
                .to_string()
        } else {
            meta.name.clone()
        };
        if meta.is_dir {
            let local_dir = open_temp_path(&name);
            if download_remote_dir_for_clipboard(be, path, &local_dir, filter.as_ref()).is_ok() {
                local.push(local_dir.to_string_lossy().to_string());
            } else {
                let _ = std::fs::remove_dir_all(&local_dir);
            }
        } else {
            let local_name = be.download_name(path, &name);
            if let Ok(p) = download_to_temp(be, path, &local_name) {
                local.push(p);
            }
        }
    }
    local
}

pub(in crate::app) fn download_paths_progress(
    be: &dyn crate::vfs::Backend,
    paths: &[String],
    dest_local: &str,
    filter: Option<(FilterDef, String)>,
    tx: &crossbeam_channel::Sender<TransferMsg>,
) {
    let filter = compile_remote_filter(filter);
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut errors = Vec::new();
    let dest_root = PathBuf::from(dest_local.replace('/', std::path::MAIN_SEPARATOR_STR));
    for src in paths {
        let name = src
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("datei");
        RemoteEntryCollector {
            be,
            filter: filter.as_ref(),
            files: &mut files,
            dirs: &mut dirs,
            errors: &mut errors,
        }
        .collect(src, name.to_string(), true);
    }
    dirs.sort();
    dirs.dedup();
    let bytes_total = files.iter().map(|f| f.size).sum();
    let mut progress = TransferProgress::new(
        TransferKind::Download,
        "Lade herunter",
        files.len() as u64,
        bytes_total,
    );
    progress.errors = errors.len() as u64;
    let mut last = std::time::Instant::now();
    send_transfer_progress(tx, &progress, &mut last, true);

    for dir in dirs {
        let local = dest_root.join(dir.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Err(e) = std::fs::create_dir_all(&local) {
            errors.push(format!("{}: {}", local.display(), e));
            progress.errors = errors.len() as u64;
        }
    }

    let start = std::time::Instant::now();
    for file in files {
        let dest = dest_root.join(file.rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        progress.current = file.rel.clone();
        progress.elapsed_ms = start.elapsed().as_millis() as u64;
        send_transfer_progress(tx, &progress, &mut last, true);
        match download_file_progress(
            be,
            &file.src,
            &dest,
            file.size,
            tx,
            &mut progress,
            &mut last,
        ) {
            Ok(_) => {
                progress.files_done = progress.files_done.saturating_add(1);
            }
            Err(e) => {
                errors.push(format!("{}: {}", file.rel, e));
                progress.errors = errors.len() as u64;
                progress.files_done = progress.files_done.saturating_add(1);
                progress.bytes_done = progress.bytes_done.saturating_add(file.size);
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
