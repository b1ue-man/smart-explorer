use super::download_file::download_file_progress;
use super::entries::{
    compile_remote_filter, validate_transfer_name, RemoteEntryCollector, RemoteFilterCtx,
    TransferCollectionBudget, TransferErrorLog,
};
use super::progress::send_transfer_progress;
use super::{cleanup_temp_copy, open_temp_path};
use crate::app::app_models::{TransferKind, TransferMsg, TransferProgress};
use crate::types::FilterDef;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

struct RemoteDownloadRoot {
    src: String,
    name: String,
    rel: String,
    is_dir: bool,
    files: Vec<super::entries::RemoteFileEntry>,
    dirs: Vec<String>,
}

fn download_collected_file(
    be: &dyn crate::vfs::Backend,
    file: &super::entries::RemoteFileEntry,
    dest: &Path,
) -> Result<String, String> {
    let (tx, rx) = crossbeam_channel::unbounded();
    drop(rx);
    let mut progress = TransferProgress::new(TransferKind::Download, "Lade herunter", 1, file.size);
    let mut last = std::time::Instant::now();
    download_file_progress(
        be,
        &file.src,
        dest,
        file.size,
        &tx,
        &mut progress,
        &mut last,
        None,
    )
}

fn collect_download_root(
    be: &dyn crate::vfs::Backend,
    filter: Option<&RemoteFilterCtx>,
    src: &str,
    requested_rel: Option<String>,
    budget: &mut TransferCollectionBudget,
    cancel: Option<&AtomicBool>,
) -> Result<RemoteDownloadRoot, String> {
    super::cancel::check_optional(cancel)?;
    let meta = be.stat(src).map_err(|error| format!("{src}: {error}"))?;
    super::cancel::check_optional(cancel)?;
    budget
        .ensure_text_fits(&[src, &meta.name])
        .map_err(|error| format!("{src}: {error}"))?;
    let root_name = if meta.name.is_empty() {
        src.trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or("datei")
            .to_string()
    } else {
        meta.name.clone()
    };
    validate_transfer_name(&root_name, src)?;
    let rel = requested_rel.unwrap_or_else(|| root_name.clone());
    if !rel.is_empty() {
        validate_transfer_name(&rel, src)?;
    }
    let is_dir = meta.is_dir;
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    RemoteEntryCollector {
        be,
        filter,
        files: &mut files,
        dirs: &mut dirs,
        budget,
        cancel,
    }
    .collect_with_meta(src, rel.clone(), true, meta, 0)?;
    Ok(RemoteDownloadRoot {
        src: src.to_string(),
        name: root_name,
        rel,
        is_dir,
        files,
        dirs,
    })
}

fn download_remote_dir_for_clipboard(
    be: &dyn crate::vfs::Backend,
    root: &RemoteDownloadRoot,
    local_dir: &Path,
    unfiltered: bool,
) -> Result<(), String> {
    match std::fs::remove_dir_all(local_dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    std::fs::create_dir_all(local_dir).map_err(|e| e.to_string())?;
    if unfiltered && be.supports_bulk_tree() {
        match be.get_tree(&root.src, local_dir) {
            Ok(files) if files == root.files.len() as u64 => return Ok(()),
            Ok(_) | Err(_) => {
                std::fs::remove_dir_all(local_dir).map_err(|error| error.to_string())?;
                std::fs::create_dir_all(local_dir).map_err(|e| e.to_string())?;
            }
        }
    }

    let mut dirs: Vec<&str> = root.dirs.iter().map(String::as_str).collect();
    dirs.sort_unstable();
    dirs.dedup();
    for dir in dirs {
        if dir.is_empty() {
            continue;
        }
        std::fs::create_dir_all(local_dir.join(dir.replace('/', std::path::MAIN_SEPARATOR_STR)))
            .map_err(|e| e.to_string())?;
    }
    for file in &root.files {
        let dest = local_dir.join(file.rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        download_collected_file(be, file, &dest)?;
    }
    Ok(())
}

pub(in crate::app) fn download_remote_clipboard_items(
    be: &dyn crate::vfs::Backend,
    items: &[(String, String, bool)],
    filter: Option<(FilterDef, String)>,
) -> Result<Vec<String>, String> {
    let filter = compile_remote_filter(filter);
    let mut budget = TransferCollectionBudget::default();
    let mut prepared = Vec::new();
    for (path, supplied_name, _) in items {
        let root = collect_download_root(
            be,
            filter.as_ref(),
            path,
            Some(String::new()),
            &mut budget,
            None,
        )?;
        if root.is_dir && filter.is_some() && root.files.is_empty() {
            return Err(format!("{}: Filter liefert keine Dateien", root.src));
        }
        let local_name = if root.is_dir {
            supplied_name.clone()
        } else {
            be.download_name(path, supplied_name)
        };
        if budget.record_text(&[&local_name]).is_err() {
            return Err("Zwischenablage überschreitet das Textbudget".to_string());
        }
        prepared.push((root, local_name));
    }

    let mut local = Vec::new();
    for (root, local_name) in prepared {
        if root.is_dir {
            let local_dir = open_download_temp_path(&local_name, &local)
                .map_err(|error| format!("{}: {error}", root.src))?;
            if let Err(error) =
                download_remote_dir_for_clipboard(be, &root, &local_dir, filter.is_none())
            {
                cleanup_temp_copy(&local_dir);
                cleanup_local_results(&local);
                return Err(format!("{}: {error}", root.src));
            }
            local.push(local_dir.to_string_lossy().to_string());
        } else {
            let file = root
                .files
                .first()
                .ok_or_else(|| format!("{}: keine herunterladbare Datei", root.src))?;
            let dest = open_download_temp_path(&local_name, &local)
                .map_err(|error| format!("{}: {error}", root.src))?;
            match download_collected_file(be, file, &dest) {
                Ok(path) => local.push(path),
                Err(error) => {
                    cleanup_temp_copy(&dest);
                    cleanup_local_results(&local);
                    return Err(format!("{}: {error}", root.src));
                }
            }
        }
    }
    Ok(local)
}

pub(in crate::app) fn download_remote_paths_for_clipboard(
    be: &dyn crate::vfs::Backend,
    paths: &[String],
    filter: Option<(FilterDef, String)>,
) -> Result<Vec<String>, String> {
    let filter = compile_remote_filter(filter);
    let mut budget = TransferCollectionBudget::default();
    let mut prepared = Vec::new();
    for path in paths {
        let root = collect_download_root(
            be,
            filter.as_ref(),
            path,
            Some(String::new()),
            &mut budget,
            None,
        )?;
        if root.is_dir && filter.is_some() && root.files.is_empty() {
            return Err(format!("{}: Filter liefert keine Dateien", root.src));
        }
        let local_name = if root.is_dir {
            root.name.clone()
        } else {
            be.download_name(path, &root.name)
        };
        if budget.record_text(&[&local_name]).is_err() {
            return Err("Drag-and-drop überschreitet das Textbudget".to_string());
        }
        prepared.push((root, local_name));
    }

    let mut local = Vec::new();
    for (root, local_name) in prepared {
        if root.is_dir {
            let local_dir = open_download_temp_path(&local_name, &local)
                .map_err(|error| format!("{}: {error}", root.src))?;
            if let Err(error) =
                download_remote_dir_for_clipboard(be, &root, &local_dir, filter.is_none())
            {
                cleanup_temp_copy(&local_dir);
                cleanup_local_results(&local);
                return Err(format!("{}: {error}", root.src));
            }
            local.push(local_dir.to_string_lossy().to_string());
        } else {
            let file = root
                .files
                .first()
                .ok_or_else(|| format!("{}: keine herunterladbare Datei", root.src))?;
            let dest = open_download_temp_path(&local_name, &local)
                .map_err(|error| format!("{}: {error}", root.src))?;
            match download_collected_file(be, file, &dest) {
                Ok(path) => local.push(path),
                Err(error) => {
                    cleanup_temp_copy(&dest);
                    cleanup_local_results(&local);
                    return Err(format!("{}: {error}", root.src));
                }
            }
        }
    }
    Ok(local)
}

fn open_download_temp_path(name: &str, completed: &[String]) -> Result<PathBuf, String> {
    open_temp_path(name).map_err(|error| {
        cleanup_local_results(completed);
        format!("Temporären Downloadpfad anlegen: {error}")
    })
}

fn cleanup_local_results(paths: &[String]) {
    for path in paths {
        cleanup_temp_copy(Path::new(path));
    }
}

pub(in crate::app) fn download_paths_progress(
    be: &dyn crate::vfs::Backend,
    paths: &[String],
    dest_local: &str,
    filter: Option<(FilterDef, String)>,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    cancel: &AtomicBool,
) {
    let filter = compile_remote_filter(filter);
    let mut errors = TransferErrorLog::default();
    let mut budget = TransferCollectionBudget::default();
    let mut roots = Vec::new();
    let dest_root = PathBuf::from(dest_local.replace('/', std::path::MAIN_SEPARATOR_STR));
    for src in paths {
        if super::cancel::requested(cancel) {
            break;
        }
        match collect_download_root(be, filter.as_ref(), src, None, &mut budget, Some(cancel)) {
            Ok(root) => roots.push(root),
            Err(error) => {
                if !super::cancel::requested(cancel) {
                    errors.push(error);
                }
                break;
            }
        }
    }
    if super::cancel::requested(cancel) {
        super::cancel::send_done(
            tx,
            TransferProgress::new(TransferKind::Download, "Lade herunter", 0, 0),
            Vec::new(),
            cancel,
        );
        return;
    }
    if !errors.is_empty() {
        let mut progress = TransferProgress::new(TransferKind::Download, "Lade herunter", 0, 0);
        progress.errors = errors.total();
        super::cancel::send_done(tx, progress, errors.into_displayed(), cancel);
        return;
    }
    let bytes_total = roots
        .iter()
        .flat_map(|root| root.files.iter())
        .map(|f| f.size)
        .fold(0u64, u64::saturating_add);
    let mut progress = TransferProgress::new(
        TransferKind::Download,
        "Lade herunter",
        roots
            .iter()
            .map(|root| root.files.len() as u64)
            .sum::<u64>(),
        bytes_total,
    );
    progress.errors = errors.total();
    let mut last = std::time::Instant::now();
    send_transfer_progress(tx, &progress, &mut last, true);

    let start = std::time::Instant::now();
    'roots: for root in roots {
        if super::cancel::requested(cancel) {
            break;
        }
        if root.is_dir && filter.is_none() && be.supports_bulk_tree() {
            let dest = dest_root.join(root.rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            progress.current = root.rel.clone();
            progress.elapsed_ms = start.elapsed().as_millis() as u64;
            send_transfer_progress(tx, &progress, &mut last, true);
            if super::cancel::requested(cancel) {
                break;
            }
            let bulk = be.get_tree(&root.src, &dest);
            if super::cancel::requested(cancel) {
                break;
            }
            if bulk.ok() == Some(root.files.len() as u64) {
                progress.files_done = progress.files_done.saturating_add(root.files.len() as u64);
                progress.bytes_done = progress.bytes_done.saturating_add(
                    root.files
                        .iter()
                        .map(|f| f.size)
                        .fold(0u64, u64::saturating_add),
                );
                progress.elapsed_ms = start.elapsed().as_millis() as u64;
                send_transfer_progress(tx, &progress, &mut last, true);
                continue;
            }
        }

        let mut dirs = root.dirs;
        dirs.sort();
        dirs.dedup();
        for dir in dirs {
            if super::cancel::requested(cancel) {
                break 'roots;
            }
            let local = dest_root.join(dir.replace('/', std::path::MAIN_SEPARATOR_STR));
            if let Err(e) = std::fs::create_dir_all(&local) {
                errors.push(format!("{}: {}", local.display(), e));
                progress.errors = errors.total();
            }
            if super::cancel::requested(cancel) {
                break 'roots;
            }
        }

        for file in root.files {
            if super::cancel::requested(cancel) {
                break 'roots;
            }
            let dest = dest_root.join(file.rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            progress.current = file.rel.clone();
            progress.elapsed_ms = start.elapsed().as_millis() as u64;
            send_transfer_progress(tx, &progress, &mut last, true);
            let result = download_file_progress(
                be,
                &file.src,
                &dest,
                file.size,
                tx,
                &mut progress,
                &mut last,
                Some(cancel),
            );
            if super::cancel::requested(cancel) {
                break 'roots;
            }
            match result {
                Ok(_) => {
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
    }

    progress.elapsed_ms = start.elapsed().as_millis() as u64;
    progress.errors = errors.total();
    super::cancel::send_done(tx, progress, errors.into_displayed(), cancel);
}
