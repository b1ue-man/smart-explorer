//! Whether a Windows volume keeps the file IDs the digest cache keys on
//! (`hash_cache.rs`). NTFS and ReFS name a file by a record that lives as long
//! as the file; FAT, FAT32 and exFAT derive the ID from the position of the
//! directory entry (a new file can take the slot of a deleted one, with the
//! same size and the coarse times of those file systems), and optical media
//! and unknown file systems promise nothing. Only NTFS and ReFS are cached.
//!
//! The name alone is not enough: over SMB `GetVolumeInformationByHandleW`
//! reports the file system name of the server, and a Samba server names its
//! exports "NTFS" by default (its `fstype` parameter) whatever they are
//! stored on, so the IDs it hands out can come from inode numbers of a file
//! system that does not keep them (assumption, not checked on a system). So a
//! file on a network volume has no stamp either: the volume of the open
//! handle is asked for its device characteristics
//! (`NtQueryVolumeInformationFile`, `FileFsDeviceInformation`,
//! `FILE_FS_DEVICE_INFORMATION.Characteristics`, whose `FILE_REMOTE_DEVICE`
//! bit (0x10) is documented under `DEVICE_OBJECT` in the Windows Driver Kit).
//! It names the actual device of the handle, a mapped drive, a UNC path and a
//! link into a share alike, where `GetDriveTypeW(DRIVE_REMOTE)` would need a
//! drive root and could be answered for another volume after a drive letter
//! was reassigned. An answer that is missing counts as "remote": the digest is
//! then only computed again.
use std::os::windows::io::AsRawHandle;

use windows_sys::Wdk::Storage::FileSystem::{
    FileFsDeviceInformation, NtQueryVolumeInformationFile,
};
use windows_sys::Win32::Foundation::STATUS_PENDING;
use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationByHandleW;
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

/// `FILE_REMOTE_DEVICE`: the device is a remote one (a network redirector).
const REMOTE_DEVICE: u32 = 0x10;
/// `FILE_WEBDAV_FS`: a network file system on WebDAV (the same masks as
/// `copy/os/windows_file_proof.rs` uses to refuse a remote volume).
const WEBDAV_DEVICE: u32 = 0x2000;

/// Whether the device characteristics name a network volume.
pub(super) fn characteristics_are_remote(characteristics: u32) -> bool {
    characteristics & (REMOTE_DEVICE | WEBDAV_DEVICE) != 0
}

/// Whether the volume of an open file is a network volume; `None` when the
/// device cannot be asked.
pub(super) fn file_on_remote_volume(file: &std::fs::File) -> Option<bool> {
    // FILE_FS_DEVICE_INFORMATION: the device type, then the characteristics.
    let mut device = Box::new([0u32; 2]);
    // SAFETY: an all-zero IO_STATUS_BLOCK is the state the call expects.
    let mut completion: Box<IO_STATUS_BLOCK> = Box::new(unsafe { std::mem::zeroed() });
    // SAFETY: a live synchronous handle (`std::fs::File` does not open
    // overlapped); both outputs live in boxes that are leaked below if the
    // call were ever still pending.
    let status = unsafe {
        NtQueryVolumeInformationFile(
            file.as_raw_handle(),
            completion.as_mut(),
            device.as_mut_ptr().cast(),
            std::mem::size_of_val(&*device) as u32,
            FileFsDeviceInformation,
        )
    };
    if status == STATUS_PENDING {
        std::mem::forget((device, completion));
        return None;
    }
    (status >= 0 && completion.Information == std::mem::size_of_val(&*device))
        .then(|| characteristics_are_remote(device[1]))
}

/// Whether the file system called `name` (`GetVolumeInformationByHandleW`)
/// keeps durable file IDs.
pub(super) fn name_has_durable_ids(name: &str) -> bool {
    matches!(name.to_ascii_uppercase().as_str(), "NTFS" | "REFS")
}

/// The volume of an open file: a local one whose file system name says
/// NTFS or ReFS. A network volume, a device that cannot be asked and a name
/// that cannot be read count as "no durable IDs": the digest is then only
/// computed again.
pub(super) fn file_has_durable_ids(file: &std::fs::File) -> bool {
    if file_on_remote_volume(file) != Some(false) {
        return false;
    }
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

    #[test]
    fn abgleich_task_network_devices_keep_no_durable_ids() {
        assert!(characteristics_are_remote(REMOTE_DEVICE));
        assert!(characteristics_are_remote(WEBDAV_DEVICE | 0x1));
        // A removable local volume (FILE_REMOVABLE_MEDIA 0x1) is not remote.
        assert!(!characteristics_are_remote(0x1));
        assert!(!characteristics_are_remote(0));
    }

    #[test]
    fn abgleich_task_a_file_on_the_local_temp_volume_is_not_remote() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local.bin");
        std::fs::write(&path, b"abc").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        assert_eq!(file_on_remote_volume(&file), Some(false));
    }
}
