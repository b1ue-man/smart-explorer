//! Folders a volume is mounted in (U16, 2026-11-27).
//!
//! Junctions and volume mount points carry the same reparse tag,
//! IO_REPARSE_TAG_MOUNT_POINT; only the substitute name tells them apart: a
//! volume mount point names a whole volume (`\??\Volume{GUID}\`), a junction
//! any other folder. A mount point onto a volume is a volume boundary, not a
//! link: a location that names the folder means the root of the mounted
//! volume, opened through its `\\?\Volume{GUID}\` path, while the folder
//! itself stays pinned. A walk still stops at such a folder below its root
//! (its entry keeps the link class of the listing), so no walk crosses onto
//! another volume unasked; junctions and symbolic links stay links.
use std::{fs::File, io, mem::size_of, os::windows::io::AsRawHandle, path::Path};
use windows_sys::Win32::{
    Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        MAXIMUM_REPARSE_DATA_BUFFER_SIZE,
    },
    System::{Ioctl::FSCTL_GET_REPARSE_POINT, IO::DeviceIoControl},
};

use crate::local_access::protocol::ReadKind;

/// IO_REPARSE_TAG_MOUNT_POINT (winnt.h; windows-sys keeps it in
/// `Win32_System_SystemServices`, a feature this workspace does not enable).
pub(super) const MOUNT_POINT_TAG: u32 = 0xA000_0003;
/// ReparseTag, ReparseDataLength, Reserved, then the mount-point header:
/// substitute name offset and length, print name offset and length.
const HEADER_BYTES: usize = 16;
const NT_PREFIX: &str = r"\??\";

/// The `\\?\Volume{GUID}\` root a mount-point reparse buffer names; `None`
/// for a junction, a truncated buffer and any other reparse data.
pub(super) fn mounted_volume_root(buffer: &[u8]) -> Option<String> {
    let word = |at: usize| -> Option<usize> {
        let bytes = buffer.get(at..at + 2)?;
        Some(usize::from(u16::from_le_bytes([bytes[0], bytes[1]])))
    };
    let tag = u32::from_le_bytes(buffer.get(..4)?.try_into().ok()?);
    if tag != MOUNT_POINT_TAG {
        return None;
    }
    let offset = HEADER_BYTES.checked_add(word(8)?)?;
    let length = word(10)?;
    let name = buffer.get(offset..offset.checked_add(length)?)?;
    if length % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = name
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let name = String::from_utf16(&units).ok()?;
    let volume = name.strip_prefix(NT_PREFIX)?;
    let volume = volume.strip_suffix('\\').unwrap_or(volume);
    super::paths::is_volume_guid_name(volume).then(|| format!(r"\\?\{volume}\"))
}

/// The root of the volume mounted at the reparse point `file` holds (opened
/// with FILE_FLAG_OPEN_REPARSE_POINT). `None` when the entry is no mount
/// point onto a volume, and when its reparse data cannot be read: then it
/// stays the link boundary it was.
pub(super) fn mounted_volume(file: &File) -> Option<String> {
    let info = super::directory::attributes(file).ok()?;
    if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 || info.ReparseTag != MOUNT_POINT_TAG
    {
        return None;
    }
    // u32 words: the reparse header is read with four-byte alignment.
    let mut buffer = vec![0u32; MAXIMUM_REPARSE_DATA_BUFFER_SIZE as usize / size_of::<u32>()];
    let mut returned = 0u32;
    // SAFETY: a live synchronous handle; the output buffer stays valid and
    // its size is passed exactly; no input buffer and no OVERLAPPED.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_GET_REPARSE_POINT,
            std::ptr::null(),
            0,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * size_of::<u32>()) as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return None;
    }
    // SAFETY: the call wrote `returned` bytes of the initialized buffer.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            buffer.as_ptr().cast::<u8>(),
            (returned as usize).min(buffer.len() * size_of::<u32>()),
        )
    };
    mounted_volume_root(bytes)
}

/// Whether `path` names a folder a volume is mounted in (not a junction,
/// not a symbolic link): such a folder, named itself, is that volume's root
/// and no link. False when the entry cannot be opened or its reparse data
/// cannot be read (it then stays the link boundary it was).
pub fn is_volume_mount_point(path: &Path) -> bool {
    super::read::open_direct(
        &super::normalize_scan_root(path),
        ReadKind::Metadata,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    )
    .ok()
    .and_then(|file| mounted_volume(&file))
    .is_some()
}

/// `file` (opened at `path` without following its last component) as the
/// folder a location names: a folder a volume is mounted in is replaced by
/// the root of that volume, opened with `kind` and `sharing`, and handed
/// back together with the held mount folder, which the caller keeps open.
pub(super) fn enter_mounted(
    file: File,
    kind: ReadKind,
    sharing: u32,
) -> io::Result<(File, Option<File>)> {
    let Some(root) = mounted_volume(&file) else {
        return Ok((file, None));
    };
    let volume = super::read::open_direct(Path::new(&root), kind, sharing)?;
    Ok((volume, Some(file)))
}

#[cfg(test)]
#[path = "volume_mount_tests.rs"]
mod tests;
