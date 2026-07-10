use super::entries::{validate_transfer_name, TransferCollectionBudget, TransferErrorLog};
use super::progress::send_transfer_progress;
use super::{find_remote_unique_name_avoiding, numbered_remote_name, rjoin};
use crate::app::app_models::{TransferKind, TransferMsg, TransferProgress};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

struct UploadEntry {
    src: PathBuf,
    rel: String,
    size: u64,
}

struct UploadRoot {
    src: PathBuf,
    rel: String,
    is_dir: bool,
    files: Vec<UploadEntry>,
    dirs: Vec<String>,
}

fn collect_upload_entries(
    path: &Path,
    rel: String,
    files: &mut Vec<UploadEntry>,
    dirs: &mut Vec<String>,
    budget: &mut TransferCollectionBudget,
    depth: usize,
    cancel: &AtomicBool,
) -> Result<(), String> {
    super::cancel::check(cancel)?;
    let display = path.to_string_lossy();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{}: ungültiger Dateiname", path.display()))?;
    validate_transfer_name(name, &display)?;
    budget
        .record_node(depth, &[&display, &rel, name])
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let meta =
        std::fs::symlink_metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
    super::cancel::check(cancel)?;
    if super::super::upload_is_link_like(&meta) {
        return Err(format!(
            "{}: Links und Reparse-Punkte werden nicht hochgeladen",
            path.display()
        ));
    }
    if meta.is_dir() {
        dirs.push(rel.clone());
        let rd = std::fs::read_dir(path).map_err(|error| format!("{}: {error}", path.display()))?;
        for entry in rd {
            super::cancel::check(cancel)?;
            let entry = entry.map_err(|error| format!("{}: {error}", path.display()))?;
            let name = entry.file_name().into_string().map_err(|_| {
                format!(
                    "{}: Dateiname ist kein gültiges Unicode",
                    entry.path().display()
                )
            })?;
            validate_transfer_name(&name, &display)?;
            budget
                .ensure_text_fits(&[&name])
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let child_rel = if rel.is_empty() {
                name
            } else {
                format!("{}/{}", rel, name)
            };
            collect_upload_entries(
                &entry.path(),
                child_rel,
                files,
                dirs,
                budget,
                depth + 1,
                cancel,
            )?;
        }
    } else if meta.is_file() {
        files.push(UploadEntry {
            src: path.to_path_buf(),
            rel,
            size: meta.len(),
        });
    } else {
        return Err(format!(
            "{}: Nur reguläre Dateien und Verzeichnisse werden hochgeladen",
            path.display()
        ));
    }
    Ok(())
}

fn collect_upload_root(
    src: PathBuf,
    rel: String,
    budget: &mut TransferCollectionBudget,
    cancel: &AtomicBool,
) -> Result<UploadRoot, String> {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    collect_upload_entries(&src, rel.clone(), &mut files, &mut dirs, budget, 0, cancel)?;
    let is_dir = dirs.first() == Some(&rel);
    Ok(UploadRoot {
        src,
        rel,
        is_dir,
        files,
        dirs,
    })
}

fn open_upload_source(src: &Path) -> Result<std::fs::File, String> {
    let meta = std::fs::symlink_metadata(src).map_err(|error| error.to_string())?;
    if super::super::upload_is_link_like(&meta) {
        return Err("Links und Reparse-Punkte werden nicht hochgeladen".to_string());
    }
    if !meta.is_file() {
        return Err("Upload-Quelle ist keine reguläre Datei".to_string());
    }
    std::fs::File::open(src).map_err(|error| error.to_string())
}

pub(in crate::app) fn upload_file_direct(
    be: &dyn crate::vfs::Backend,
    src: &Path,
    dest: &str,
) -> Result<(), String> {
    use std::io::Write;
    let mut r = open_upload_source(src)?;
    if let Some((parent, _)) = dest.rsplit_once('/') {
        be.mkdir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut w = be.open_write(dest).map_err(|e| e.to_string())?;
    std::io::copy(&mut r, &mut w).map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}

pub(in crate::app) fn upload_file(
    be: &dyn crate::vfs::Backend,
    src: &Path,
    dest: &str,
) -> Result<(), String> {
    let staged =
        crate::vfs::unique_staging_path(be, dest, "upload").map_err(|error| error.to_string())?;
    if let Err(error) = upload_file_direct(be, src, &staged) {
        let _ = be.remove_file(&staged);
        return Err(error);
    }
    if let Err(error) = crate::vfs::promote_staged_replace(be, &staged, dest) {
        let _ = be.remove_file(&staged);
        return Err(error.to_string());
    }
    Ok(())
}

pub(super) fn upload_file_direct_progress(
    be: &dyn crate::vfs::Backend,
    src: &Path,
    dest: &str,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    progress: &mut TransferProgress,
    last: &mut std::time::Instant,
    cancel: &AtomicBool,
) -> Result<(), String> {
    use std::io::{Read, Write};
    super::cancel::check(cancel)?;
    let mut r = open_upload_source(src)?;
    super::cancel::check(cancel)?;
    if let Some((parent, _)) = dest.rsplit_once('/') {
        be.mkdir_all(parent).map_err(|error| error.to_string())?;
        super::cancel::check(cancel)?;
    }
    let mut w = be.open_write(dest).map_err(|e| e.to_string())?;
    super::cancel::check(cancel)?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        super::cancel::check(cancel)?;
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        super::cancel::check(cancel)?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        super::cancel::check(cancel)?;
        progress.bytes_done = progress.bytes_done.saturating_add(n as u64);
        send_transfer_progress(tx, progress, last, false);
    }
    super::cancel::check(cancel)?;
    w.flush().map_err(|e| e.to_string())?;
    super::cancel::check(cancel)?;
    Ok(())
}

pub(super) fn upload_file_progress(
    be: &dyn crate::vfs::Backend,
    src: &Path,
    dest: &str,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    progress: &mut TransferProgress,
    last: &mut std::time::Instant,
    cancel: &AtomicBool,
) -> Result<(), String> {
    super::cancel::check(cancel)?;
    let staged =
        crate::vfs::unique_staging_path(be, dest, "upload").map_err(|error| error.to_string())?;
    if let Err(error) = upload_file_direct_progress(be, src, &staged, tx, progress, last, cancel) {
        let _ = be.remove_file(&staged);
        return Err(error);
    }
    if let Err(error) = super::cancel::check(cancel) {
        let _ = be.remove_file(&staged);
        return Err(error);
    }
    if let Err(error) = crate::vfs::promote_staged_replace(be, &staged, dest) {
        let _ = be.remove_file(&staged);
        return Err(error.to_string());
    }
    super::cancel::check(cancel)?;
    Ok(())
}

pub(in crate::app) fn upload_paths_progress(
    be: &dyn crate::vfs::Backend,
    paths: &[String],
    dest_root: &str,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    cancel: &AtomicBool,
) {
    let mut errors = TransferErrorLog::default();
    let mut budget = TransferCollectionBudget::default();
    let mut roots = Vec::new();
    let mut reserved = std::collections::HashSet::new();
    for p in paths {
        if super::cancel::requested(cancel) {
            break;
        }
        let src = PathBuf::from(p);
        let base = match src.file_name().and_then(|name| name.to_str()) {
            Some(base) if !base.is_empty() => base,
            _ => {
                errors.push(format!("{}: ungültiger Dateiname", src.display()));
                break;
            }
        };
        if let Err(error) = validate_transfer_name(base, &src.to_string_lossy()) {
            errors.push(error);
            break;
        }
        if let Err(error) = budget.ensure_text_fits(&[&src.to_string_lossy(), base]) {
            errors.push(format!("{}: {error}", src.display()));
            break;
        }
        let target_name = match find_remote_unique_name_avoiding(
            be,
            dest_root,
            |index| numbered_remote_name(base, index),
            &reserved,
            cancel,
        ) {
            Ok(name) => name,
            Err(error) => {
                errors.push(format!("{}: {error}", src.display()));
                break;
            }
        };
        reserved.insert(rjoin(dest_root, &target_name));
        match collect_upload_root(src, target_name, &mut budget, cancel) {
            Ok(root) => roots.push(root),
            Err(error) => {
                errors.push(error);
                break;
            }
        }
    }
    if super::cancel::requested(cancel) {
        super::cancel::send_done(
            tx,
            TransferProgress::new(TransferKind::Upload, "Lade hoch", 0, 0),
            Vec::new(),
            cancel,
        );
        return;
    }
    if !errors.is_empty() {
        let mut progress = TransferProgress::new(TransferKind::Upload, "Lade hoch", 0, 0);
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
        TransferKind::Upload,
        "Lade hoch",
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
        if root.is_dir && be.supports_bulk_tree() {
            let dest = rjoin(dest_root, &root.rel);
            progress.current = root.rel.clone();
            progress.elapsed_ms = start.elapsed().as_millis() as u64;
            send_transfer_progress(tx, &progress, &mut last, true);
            if super::cancel::requested(cancel) {
                break;
            }
            let bulk = be.put_tree(&root.src, &dest);
            if super::cancel::requested(cancel) {
                break;
            }
            if bulk.is_ok() {
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
            if dir.is_empty() {
                continue;
            }
            let dest = rjoin(dest_root, &dir);
            if let Err(e) = be.mkdir_all(&dest) {
                errors.push(format!("{}: {}", dest, e));
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
            let dest = rjoin(dest_root, &file.rel);
            progress.current = file.rel.clone();
            progress.elapsed_ms = start.elapsed().as_millis() as u64;
            send_transfer_progress(tx, &progress, &mut last, true);
            let result =
                upload_file_progress(be, &file.src, &dest, tx, &mut progress, &mut last, cancel);
            if super::cancel::requested(cancel) {
                break 'roots;
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
    }

    progress.elapsed_ms = start.elapsed().as_millis() as u64;
    progress.errors = errors.total();
    super::cancel::send_done(tx, progress, errors.into_displayed(), cancel);
}
