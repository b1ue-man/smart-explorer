//! Whether a Windows volume keeps the file IDs the digest cache keys on
//! (`hash_cache.rs`). NTFS and ReFS name a file by a record that lives as long
//! as the file; FAT, FAT32 and exFAT derive the ID from the position of the
//! directory entry (a new file can take the slot of a deleted one, with the
//! same size and the coarse times of those file systems), and optical media
//! and unknown file systems promise nothing. Only NTFS and ReFS are cached.
use std::os::windows::io::AsRawHandle;

use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationByHandleW;

/// Whether the file system called `name` (`GetVolumeInformationByHandleW`)
/// keeps durable file IDs.
pub(super) fn name_has_durable_ids(name: &str) -> bool {
    matches!(name.to_ascii_uppercase().as_str(), "NTFS" | "REFS")
}

/// The volume of an open file. A name that cannot be read counts as "no
/// durable IDs": the digest is then only computed again.
pub(super) fn file_has_durable_ids(file: &std::fs::File) -> bool {
    let mut name = [0u16; 261];
    // SAFETY: the handle is open for the call; the name buffer has the length
    // given; the other outputs are not requested.
    let ok = unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        )
    };
    if ok == 0 {
        return false;
    }
    let length = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    name_has_durable_ids(&String::from_utf16_lossy(&name[..length]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abgleich_task_only_ntfs_and_refs_keep_durable_ids() {
        for name in ["NTFS", "ntfs", "ReFS"] {
            assert!(name_has_durable_ids(name), "{name}");
        }
        for name in ["FAT", "FAT32", "exFAT", "UDF", "CDFS", ""] {
            assert!(!name_has_durable_ids(name), "{name}");
        }
    }
}
