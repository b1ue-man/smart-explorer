#[path = "remote_helpers/downloads.rs"]
mod downloads;
#[path = "remote_helpers/entries.rs"]
mod entries;
#[path = "remote_helpers/progress.rs"]
mod progress;
#[path = "remote_helpers/remote_copy.rs"]
mod remote_copy;
#[path = "remote_helpers/temp.rs"]
mod temp;
#[path = "remote_helpers/uploads.rs"]
mod uploads;

#[cfg(test)]
#[path = "remote_helpers/tests.rs"]
mod tests;

pub(in crate::app) use downloads::{
    download_paths_progress, download_remote_clipboard_items, download_remote_paths_for_clipboard,
};
pub(in crate::app) use remote_copy::copy_remote_paths_progress;
#[cfg(test)]
pub(in crate::app) use temp::safe_temp_name;
pub(in crate::app) use temp::{
    cleanup_session_temp, cleanup_temp_copy, file_mtime_ms, init_temp_session, open_temp_path,
    RemoteEdit, SaveResult,
};
pub(in crate::app) use uploads::{upload_file, upload_paths_progress};

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

/// Read a remote file as UTF-8 text (errors on binary), for the line-merge view.
pub(in crate::app) fn read_text(
    be: &dyn crate::vfs::Backend,
    path: &str,
) -> Result<String, String> {
    use std::io::Read;
    let mut r = be.open_read(path).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    r.read_to_end(&mut buf).map_err(|e| e.to_string())?;
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
        let _ = be.mkdir_all(parent);
    }
    let mut w = be.open_write(path).map_err(|e| e.to_string())?;
    w.write_all(data).map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}

pub(in crate::app) fn sig_from(be: &dyn crate::vfs::Backend, path: &str) -> crate::bisync::Sig {
    let m = be.stat(path).ok();
    crate::bisync::Sig {
        size: m.as_ref().map(|m| m.size).unwrap_or(0),
        mtime_ms: m.as_ref().map(|m| m.mtime_ms).unwrap_or(0),
        hash: 0,
    }
}

pub(in crate::app) fn rjoin(root: &str, name: &str) -> String {
    format!("{}/{}", root.trim_end_matches('/'), name)
}

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
