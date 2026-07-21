#[path = "remote_helpers/cancel.rs"]
mod cancel;
#[path = "remote_helpers/download_file.rs"]
mod download_file;
#[path = "remote_helpers/downloads.rs"]
mod downloads;
#[path = "remote_helpers/entries.rs"]
mod entries;
#[path = "remote_helpers/progress.rs"]
mod progress;
#[path = "remote_helpers/recovery.rs"]
mod recovery;
#[path = "remote_helpers/recovery_manifest.rs"]
mod recovery_manifest;
#[path = "remote_helpers/remote_copy.rs"]
mod remote_copy;
#[path = "remote_helpers/temp.rs"]
mod temp;
#[path = "remote_helpers/temp_delete.rs"]
mod temp_delete;
#[path = "remote_helpers/uploads.rs"]
mod uploads;

#[cfg(test)]
#[path = "remote_helpers/cancel_tests.rs"]
mod cancel_tests;
#[cfg(test)]
#[path = "remote_helpers/tests.rs"]
mod tests;

pub(in crate::app) use downloads::{
    download_paths_progress, download_remote_clipboard_items, download_remote_paths_for_clipboard,
};
pub(in crate::app) use recovery::{
    recovery_delete_plan, recovery_session_count, remove_recovery_session_controlled,
};
pub(in crate::app) use recovery_manifest::sync_recovery_manifest;
pub(in crate::app) use remote_copy::copy_remote_paths_progress;
#[cfg(test)]
pub(in crate::app) use temp::safe_temp_name;
pub(in crate::app) use temp::{
    cleanup_session_temp, cleanup_temp_copy, file_mtime_ms, init_temp_session, open_temp_path,
    temp_root, RemoteEdit, SaveResult,
};
pub(in crate::app) use uploads::{upload_file, upload_paths_progress};

const MAX_MERGE_TEXT_BYTES: u64 = 16 * 1024 * 1024;

/// Line-merge editor state: a side-by-side aligned diff of the two versions.
pub(in crate::app) struct MergeUi {
    pub(in crate::app) rel: String,
    pub(in crate::app) rows: Vec<crate::linemerge::Row>,
}

pub(in crate::app) fn ep_join(root: &str, rel: &str) -> String {
    format!("{}/{}", root.trim_end_matches('/'), rel)
}

/// Insert " (Konflikt <timestamp>)" before the extension of a relative path.
pub(in crate::app) fn conflict_rel_name(rel: &str) -> String {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let seg_start = rel.rfind('/').map(|i| i + 1).unwrap_or(0);
    match rel[seg_start..].rfind('.') {
        Some(d) => {
            let dot = seg_start + d;
            format!("{} (Konflikt {}){}", &rel[..dot], ts, &rel[dot..])
        }
        None => format!("{} (Konflikt {})", rel, ts),
    }
}

pub(in crate::app) fn numbered_remote_name(name: &str, index: usize) -> String {
    if index <= 1 {
        return name.to_string();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({index}){}", &name[..dot], &name[dot..]),
        _ => format!("{name} ({index})"),
    }
}

/// Read a remote file as UTF-8 text (errors on binary), for the line-merge view.
pub(in crate::app) fn read_text(
    be: &dyn crate::vfs::Backend,
    path: &str,
) -> Result<String, String> {
    use std::io::Read;
    let metadata = be.stat(path).map_err(|error| error.to_string())?;
    if metadata.is_dir || metadata.is_symlink {
        return Err("Nur reguläre Dateien können als Text zusammengeführt werden.".to_string());
    }
    if metadata.size > MAX_MERGE_TEXT_BYTES {
        return Err("Text-Zusammenführung ist auf 16 MiB pro Datei begrenzt.".to_string());
    }
    let mut r = be.open_read(path).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    r.by_ref()
        .take(MAX_MERGE_TEXT_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    if buf.len() as u64 > MAX_MERGE_TEXT_BYTES {
        return Err("Text-Zusammenführung ist auf 16 MiB pro Datei begrenzt.".to_string());
    }
    if buf.contains(&0) {
        return Err("Keine Textdatei (binär) — bitte „A/B behalten“ nutzen.".to_string());
    }
    String::from_utf8(buf)
        .map_err(|_| "Keine Textdatei (binär) — bitte „A/B behalten“ nutzen.".to_string())
}

pub(in crate::app) fn write_bytes(
    be: &dyn crate::vfs::Backend,
    path: &str,
    data: &[u8],
) -> Result<(), String> {
    use std::io::Write;
    if let Some((parent, _)) = path.rsplit_once('/') {
        be.mkdir_all(parent).map_err(|error| error.to_string())?;
    }
    let staged =
        crate::vfs::unique_staging_path(be, path, "merge").map_err(|error| error.to_string())?;
    let result = (|| {
        let mut writer = be.open_write(&staged).map_err(|error| error.to_string())?;
        writer.write_all(data).map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())?;
        drop(writer);
        crate::vfs::promote_staged_replace(be, &staged, path).map_err(|error| error.to_string())
    })();
    if result.is_err() {
        let _ = be.remove_file(&staged);
    }
    result
}

pub(in crate::app) fn sig_from(
    be: &dyn crate::vfs::Backend,
    path: &str,
) -> Result<crate::bisync::Sig, String> {
    let metadata = be.stat(path).map_err(|error| error.to_string())?;
    if metadata.is_dir || metadata.is_symlink {
        return Err(format!("Konfliktziel ist keine reguläre Datei: {path}"));
    }
    Ok(crate::bisync::Sig {
        size: metadata.size,
        mtime_ms: metadata.mtime_ms,
        hash: 0,
    })
}

pub(in crate::app) fn rjoin(root: &str, name: &str) -> String {
    format!("{}/{}", root.trim_end_matches('/'), name)
}

const REMOTE_UNIQUE_ATTEMPTS: usize = 1000;

pub(in crate::app) fn find_remote_unique_name(
    backend: &dyn crate::vfs::Backend,
    parent: &str,
    candidate: impl FnMut(usize) -> String,
) -> Result<String, String> {
    find_remote_unique_name_with(|path| backend.try_exists(path), parent, candidate)
}

pub(in crate::app) fn find_remote_unique_name_avoiding(
    backend: &dyn crate::vfs::Backend,
    parent: &str,
    mut candidate: impl FnMut(usize) -> String,
    reserved: &std::collections::HashSet<String>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<String, String> {
    for index in 1..=REMOTE_UNIQUE_ATTEMPTS {
        cancel::check(cancel)?;
        let name = candidate(index);
        let path = rjoin(parent, &name);
        if reserved.contains(&path) {
            continue;
        }
        let exists = backend.try_exists(&path);
        cancel::check(cancel)?;
        match exists {
            Ok(false) => return Ok(name),
            Ok(true) => {}
            Err(error) => return Err(format!("Ziel prüfen „{path}“: {error}")),
        }
    }
    Err(format!(
        "Kein freier Name nach {REMOTE_UNIQUE_ATTEMPTS} Versuchen"
    ))
}

fn find_remote_unique_name_with(
    mut try_exists: impl FnMut(&str) -> crate::vfs::VfsResult<bool>,
    parent: &str,
    mut candidate: impl FnMut(usize) -> String,
) -> Result<String, String> {
    for index in 1..=REMOTE_UNIQUE_ATTEMPTS {
        let name = candidate(index);
        let path = rjoin(parent, &name);
        match try_exists(&path) {
            Ok(false) => return Ok(name),
            Ok(true) => {}
            Err(error) => return Err(format!("Ziel prüfen „{path}“: {error}")),
        }
    }
    Err(format!(
        "Kein freier Name nach {REMOTE_UNIQUE_ATTEMPTS} Versuchen"
    ))
}

#[cfg(test)]
pub(in crate::app) fn ensure_remote_destination_free(
    backend: &dyn crate::vfs::Backend,
    path: &str,
) -> Result<(), String> {
    match backend.try_exists(path) {
        Ok(false) => Ok(()),
        Ok(true) => Err(format!("Ziel existiert bereits: {path}")),
        Err(error) => Err(format!("Ziel prüfen „{path}“: {error}")),
    }
}

#[cfg(test)]
pub(in crate::app) fn remote_temp_path(dest: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{dest}.se-upload-{}-{nanos:x}.part", std::process::id())
}

/// A bare drive letter like `C:` is drive-relative on Windows; normalize it to
/// the drive root `C:/`.
pub(in crate::app) fn ensure_dir_root(p: &str) -> String {
    let t = p.trim();
    let b = t.as_bytes();
    if b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        format!("{}/", t)
    } else {
        t.to_string()
    }
}

pub(crate) fn is_local_style(path: &str) -> bool {
    let p = path.trim_start();
    let b = p.as_bytes();
    let has_drive = b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic();
    has_drive || p.starts_with("//") || p.starts_with("\\\\")
}

/// A ZIP archive we can browse in-app / extract.
pub(in crate::app) fn is_zip_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".zip")
}
