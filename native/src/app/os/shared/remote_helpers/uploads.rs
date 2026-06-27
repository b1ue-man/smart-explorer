use super::progress::send_transfer_progress;
use super::{remote_temp_path, rjoin};
use crate::app::app_models::{TransferKind, TransferMsg, TransferProgress};
use std::path::{Path, PathBuf};

struct UploadEntry {
    src: PathBuf,
    rel: String,
    size: u64,
}

fn collect_upload_entries(
    path: &Path,
    rel: String,
    files: &mut Vec<UploadEntry>,
    dirs: &mut Vec<String>,
    errors: &mut Vec<String>,
) {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            errors.push(format!("{}: {}", path.display(), e));
            return;
        }
    };
    if meta.is_dir() {
        dirs.push(rel.clone());
        let rd = match std::fs::read_dir(path) {
            Ok(rd) => rd,
            Err(e) => {
                errors.push(format!("{}: {}", path.display(), e));
                return;
            }
        };
        for entry in rd {
            match entry {
                Ok(entry) => {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let child_rel = if rel.is_empty() {
                        name
                    } else {
                        format!("{}/{}", rel, name)
                    };
                    collect_upload_entries(&entry.path(), child_rel, files, dirs, errors);
                }
                Err(e) => errors.push(format!("{}: {}", path.display(), e)),
            }
        }
    } else {
        files.push(UploadEntry {
            src: path.to_path_buf(),
            rel,
            size: meta.len(),
        });
    }
}

pub(in crate::app) fn upload_file_direct(
    be: &dyn crate::vfs::Backend,
    src: &Path,
    dest: &str,
) -> Result<(), String> {
    use std::io::Write;
    if let Some((parent, _)) = dest.rsplit_once('/') {
        let _ = be.mkdir_all(parent);
    }
    let mut r = std::fs::File::open(src).map_err(|e| e.to_string())?;
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
    if !be.rename_overwrites() {
        return upload_file_direct(be, src, dest);
    }
    let tmp = remote_temp_path(dest);
    if let Err(e) = upload_file_direct(be, src, &tmp) {
        let _ = be.remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = be.rename(&tmp, dest) {
        let _ = be.remove_file(&tmp);
        return Err(e.to_string());
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
) -> Result<(), String> {
    use std::io::{Read, Write};
    if let Some((parent, _)) = dest.rsplit_once('/') {
        let _ = be.mkdir_all(parent);
    }
    let mut r = std::fs::File::open(src).map_err(|e| e.to_string())?;
    let mut w = be.open_write(dest).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        progress.bytes_done = progress.bytes_done.saturating_add(n as u64);
        send_transfer_progress(tx, progress, last, false);
    }
    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}

pub(super) fn upload_file_progress(
    be: &dyn crate::vfs::Backend,
    src: &Path,
    dest: &str,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    progress: &mut TransferProgress,
    last: &mut std::time::Instant,
) -> Result<(), String> {
    if !be.rename_overwrites() {
        return upload_file_direct_progress(be, src, dest, tx, progress, last);
    }
    let tmp = remote_temp_path(dest);
    if let Err(e) = upload_file_direct_progress(be, src, &tmp, tx, progress, last) {
        let _ = be.remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = be.rename(&tmp, dest) {
        let _ = be.remove_file(&tmp);
        return Err(e.to_string());
    }
    Ok(())
}

pub(in crate::app) fn upload_paths_progress(
    be: &dyn crate::vfs::Backend,
    paths: &[String],
    dest_root: &str,
    tx: &crossbeam_channel::Sender<TransferMsg>,
) {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut errors = Vec::new();
    for p in paths {
        let src = PathBuf::from(p);
        let base = match src.file_name().map(|n| n.to_string_lossy().to_string()) {
            Some(base) if !base.is_empty() => base,
            _ => continue,
        };
        collect_upload_entries(&src, base, &mut files, &mut dirs, &mut errors);
    }

    dirs.sort();
    dirs.dedup();
    let bytes_total = files.iter().map(|f| f.size).sum();
    let mut progress = TransferProgress::new(
        TransferKind::Upload,
        "Lade hoch",
        files.len() as u64,
        bytes_total,
    );
    progress.errors = errors.len() as u64;
    let mut last = std::time::Instant::now();
    send_transfer_progress(tx, &progress, &mut last, true);

    for dir in dirs {
        if dir.is_empty() {
            continue;
        }
        let dest = rjoin(dest_root, &dir);
        if let Err(e) = be.mkdir_all(&dest) {
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
        match upload_file_progress(be, &file.src, &dest, tx, &mut progress, &mut last) {
            Ok(()) => {
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
