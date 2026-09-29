use super::download_file::download_file_progress;
use super::entries::{
    compile_remote_filter, validate_transfer_name, RemoteEntryCollector, RemoteFilterCtx,
    TransferCollectionBudget,
};
use super::temp::{cleanup_temp_copy, open_temp_path};
use super::types::{TransferKind, TransferMsg, TransferProgress};
use crate::types::FilterDef;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

struct RemoteDownloadRoot {
    src: String,
    name: String,
    is_dir: bool,
    files: Vec<super::entries::RemoteFileEntry>,
    dirs: Vec<String>,
    omitted: u64,
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
    let mut omitted = 0;
    RemoteEntryCollector {
        be,
        filter,
        files: &mut files,
        dirs: &mut dirs,
        budget,
        cancel,
        omitted: &mut omitted,
    }
    .collect_with_meta(src, rel, true, meta, 0)?;
    Ok(RemoteDownloadRoot {
        src: src.to_string(),
        name: root_name,
        is_dir,
        files,
        dirs,
        omitted,
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
    // A bulk tree copy cannot leave out protected omissions.
    if unfiltered && root.omitted == 0 && be.supports_bulk_tree() {
        let files = be.get_tree(&root.src, local_dir)
            .map_err(|error| format!("Bulk-Download „{}“: {error}; kein Wiederholungsversuch in einem möglicherweise teilweise beschriebenen Ziel", root.src))?;
        if files != root.files.len() as u64 {
            return Err(format!(
                "Bulk-Download „{}“ lieferte {files} statt {} Dateien",
                root.src,
                root.files.len()
            ));
        }
        return Ok(());
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

pub fn download_remote_clipboard_items(
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

pub fn download_remote_paths_for_clipboard(
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

/// Downloads remote entries (files and whole folders) into the local folder
/// `dest_local` through the streaming engine; name conflicts keep both.
pub fn download_paths_progress(
    be: &dyn crate::vfs::Backend,
    paths: &[String],
    dest_local: &str,
    filter: Option<(FilterDef, String)>,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    cancel: &AtomicBool,
) {
    super::engine::run_legacy(
        super::engine::Side::Remote(be),
        super::engine::Side::Local,
        dest_local,
        super::job::JobItems::Roots {
            paths: paths.to_vec(),
            base: None,
        },
        filter,
        tx,
        cancel,
    );
}
