use super::prelude::*;
use super::*;

/// Case-insensitive subsequence match (fuzzy), used to filter command palette
/// entries by the text typed after `>`.
pub(in crate::app) fn fuzzy_contains(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut chars = haystack.chars().flat_map(|c| c.to_lowercase());
    for n in needle.chars().flat_map(|c| c.to_lowercase()) {
        loop {
            match chars.next() {
                Some(h) if h == n => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

pub(in crate::app) fn download_to(
    be: &dyn crate::vfs::Backend,
    path: &str,
    dest: &std::path::Path,
) -> Result<String, String> {
    download_to_id(be, path, None, dest)
}

pub(in crate::app) fn download_part_path(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "download".to_string());
    dest.with_file_name(format!(".{name}.smart-explorer.part"))
}

pub(in crate::app) fn cleanup_partial(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(windows)]
pub(in crate::app) fn path_to_wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
pub(in crate::app) fn available_space_for_path(path: &Path) -> Option<u64> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let dir = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or_else(|| Path::new("."))
    };
    let wide = path_to_wide(dir);
    let mut free = 0u64;
    let mut total = 0u64;
    let mut total_free = 0u64;
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut total_free) };
    (ok != 0).then_some(free)
}

#[cfg(not(windows))]
pub(in crate::app) fn available_space_for_path(_path: &Path) -> Option<u64> {
    None
}

pub(in crate::app) fn ensure_local_space(dest: &Path, expected_bytes: u64) -> Result<(), String> {
    if expected_bytes == 0 {
        return Ok(());
    }
    let needed = expected_bytes.saturating_add(DOWNLOAD_SPACE_MARGIN_BYTES);
    if let Some(free) = available_space_for_path(dest) {
        if free < needed {
            return Err(format!(
                "Nicht genug lokaler Speicher fuer den Temp-Download: benoetigt ca. {}, frei {}",
                format_bytes(needed),
                format_bytes(free)
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(in crate::app) fn replace_file_atomic(src: &Path, dest: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let src_w = path_to_wide(src);
    let dest_w = path_to_wide(dest);
    let ok = unsafe {
        MoveFileExW(
            src_w.as_ptr(),
            dest_w.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
pub(in crate::app) fn replace_file_atomic(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::rename(src, dest)
}

/// Like `download_to`, but targets a specific backend item by `id` when known
/// (so duplicate-named files open the exact one the user clicked).
pub(in crate::app) fn download_to_id(
    be: &dyn crate::vfs::Backend,
    path: &str,
    id: Option<&str>,
    dest: &std::path::Path,
) -> Result<String, String> {
    use std::io::Write;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let expected = be
        .stat(path)
        .ok()
        .filter(|m| !m.is_dir)
        .map(|m| m.size)
        .unwrap_or(0);
    ensure_local_space(dest, expected)?;
    let part = download_part_path(dest);
    cleanup_partial(&part);
    let mut r = be.open_read_id(path, id).map_err(|e| e.to_string())?;
    let mut f = match std::fs::File::create(&part) {
        Ok(f) => f,
        Err(e) => {
            cleanup_partial(&part);
            return Err(e.to_string());
        }
    };
    let copied = match std::io::copy(&mut r, &mut f) {
        Ok(n) => n,
        Err(e) => {
            cleanup_partial(&part);
            return Err(e.to_string());
        }
    };
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
