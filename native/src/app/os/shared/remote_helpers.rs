//! GUI side of remote files: open/edit temp copies with their recovery
//! manifest, the line-merge view state, and path helpers. The transfer workers
//! live in `crate::transfer`, the backend path helpers in `vfs::remote_util`;
//! both are re-exported here under their previous names.
#[path = "remote_helpers/recovery.rs"]
mod recovery;
#[path = "remote_helpers/recovery_manifest.rs"]
mod recovery_manifest;
#[path = "remote_helpers/temp.rs"]
mod temp;
#[path = "remote_helpers/temp_delete.rs"]
mod temp_delete;

pub(in crate::app) use crate::transfer::{
    download_clipboard_snapshot, download_remote_clipboard_items,
    download_remote_paths_for_clipboard, upload_file,
};
pub(in crate::app) use crate::vfs::remote_util::{
    conflict_rel_name, ep_join, find_remote_unique_name, read_text, rjoin, sig_from, write_bytes,
};
pub(in crate::app) use recovery::{
    recovery_delete_plan, recovery_session_count, remove_recovery_session_controlled,
};
pub(in crate::app) use recovery_manifest::sync_recovery_manifest;
#[cfg(test)]
pub(in crate::app) use temp::safe_temp_name;
pub(in crate::app) use temp::{
    cleanup_session_temp, cleanup_temp_copy, file_mtime_ms, init_temp_session, open_temp_path,
    temp_root, RemoteEdit, SaveResult,
};

/// Line-merge editor state: a side-by-side aligned diff of the two versions.
pub(in crate::app) struct MergeUi {
    pub(in crate::app) rel: String,
    pub(in crate::app) rows: Vec<crate::linemerge::Row>,
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
