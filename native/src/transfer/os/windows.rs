//! Windows filesystem facts for transfers.
use std::path::{Path, PathBuf};

/// Redirecting links are walk boundaries. Data reparse points (cloud,
/// WOF and dedup) retain their ordinary-file transfer behavior.
pub(crate) fn upload_is_link_like(path: &Path, metadata: &std::fs::Metadata) -> bool {
    crate::local_access::metadata_is_link_like(path, metadata)
}

/// `\` separates path components on Windows; it is never part of a name.
pub(crate) fn backslash_is_name_char() -> bool {
    false
}

pub(crate) fn replace_file_atomic(src: &Path, dest: &Path) -> std::io::Result<()> {
    crate::vfs::replace_local_file(src, dest)
}

pub(crate) fn available_space_for_path(path: &Path) -> Option<u64> {
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

fn path_to_wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Physical memory currently available to programs, for the transfer budget.
pub(crate) fn available_memory() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: MEMORYSTATUSEX is plain data; dwLength must name its size.
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: `status` is a valid, writable MEMORYSTATUSEX with dwLength set.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    (ok != 0).then_some(status.ullAvailPhys)
}

/// Total physical memory. Stable from run to run, unlike the available memory,
/// so limits derived from it do not change with the momentary load (sync tree
/// limits, RV1).
pub(crate) fn physical_memory() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: MEMORYSTATUSEX is plain data; dwLength must name its size.
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: `status` is a valid, writable MEMORYSTATUSEX with dwLength set.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    (ok != 0).then_some(status.ullTotalPhys)
}

/// The folder `path` resolves to, links and junctions on the way followed,
/// in its plain form (`C:\…`, `\\server\share\…`) where that names the same
/// folder; otherwise in the verbatim form `canonicalize` returns.
pub(crate) fn canonical_folder(path: &Path) -> std::io::Result<PathBuf> {
    let resolved = std::fs::canonicalize(path)?;
    let plain = plain_form(&resolved.to_string_lossy());
    Ok(plain.map_or(resolved, PathBuf::from))
}

/// The plain form of a verbatim path when Win32 reads it the same: not too
/// long, no name ending in a dot or space, no device name.
fn plain_form(verbatim: &str) -> Option<String> {
    let plain = match verbatim.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => verbatim.strip_prefix(r"\\?\")?.to_string(),
    };
    let same = plain.len() < 260
        && plain
            .split('\\')
            .filter(|name| !name.is_empty())
            .skip(1)
            .all(|name| !name.ends_with(['.', ' ']) && !is_device_name(name));
    same.then_some(plain)
}

fn is_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end()
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

#[cfg(test)]
mod tests {
    use super::plain_form;

    #[test]
    fn transfer_engine_task_canonical_folders_drop_the_verbatim_prefix_only_when_safe() {
        assert_eq!(
            plain_form(r"\\?\C:\Users\me\Downloads").as_deref(),
            Some(r"C:\Users\me\Downloads")
        );
        assert_eq!(
            plain_form(r"\\?\UNC\server\share\dir").as_deref(),
            Some(r"\\server\share\dir")
        );
        assert_eq!(plain_form(r"\\?\C:\data\nul.txt"), None, "a device name");
        assert_eq!(
            plain_form(r"\\?\C:\data\trailing."),
            None,
            "Win32 strips it"
        );
        assert_eq!(plain_form(&format!(r"\\?\C:\{}", "a".repeat(300))), None);
        assert_eq!(plain_form(r"C:\already\plain"), None);
    }
}
