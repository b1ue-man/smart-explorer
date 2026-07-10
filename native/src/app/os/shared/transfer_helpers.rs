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

#[cfg(test)]
pub(in crate::app) fn download_part_path(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "download".to_string());
    dest.with_file_name(format!(".{name}.smart-explorer.part"))
}

pub(in crate::app) fn create_download_part(
    dest: &Path,
) -> std::io::Result<(PathBuf, std::fs::File)> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let name = dest
        .file_name()
        .map(|name| name.to_string_lossy())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "download".into());
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for attempt in 0..1000u32 {
        let part = parent.join(format!(
            ".{name}.smart-explorer-{}-{nonce:x}-{attempt:x}.part",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
        {
            Ok(file) => return Ok((part, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a unique download staging file",
    ))
}

pub(in crate::app) fn cleanup_partial(path: &Path) {
    let _ = std::fs::remove_file(path);
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

/// Download a backend item to an explicit local path, using `id` when known
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
    let mut r = be.open_read_id(path, id).map_err(|e| e.to_string())?;
    let (part, mut f) = match create_download_part(dest) {
        Ok(part) => part,
        Err(e) => {
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
