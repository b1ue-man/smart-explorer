//! Windows encoding and the existing Share path-containment rules.
use std::path::{Path, PathBuf};

use super::fs_policy::{normalized, within};

pub(in crate::share) fn to_os_path(path: &str) -> PathBuf {
    let b = path.as_bytes();
    let rooted;
    let path = if b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        rooted = format!("{path}/");
        rooted.as_str()
    } else {
        path
    };
    PathBuf::from(path.replace('/', "\\"))
}

/// Creates the file of a ranged upload's stage, exclusively (`create_new`).
/// A path that Win32 would not address literally (an invalid character, a
/// trailing dot or space, a device name such as `NUL`) is `Unsupported`: the
/// file then goes as one stream through the provider, which reaches such
/// names by their verbatim form.
pub(in crate::share) fn create_stage_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::path::Component;
    let hostile = path.components().any(|component| match component {
        Component::Normal(name) => hostile_name(&name.to_string_lossy()),
        _ => false,
    });
    if hostile {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Der Pfad braucht den Einzelstrom (Win32-Name)",
        ));
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

fn hostile_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    device
        || name.ends_with('.')
        || name.ends_with(' ')
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
}

/// Physical identity of the directory `path` names (volume serial and the
/// full 128-bit file ID), following links; `None` when the provider reports
/// none (FAT's root has ID 0) or it cannot be read. The host's policy cache
/// compares it to skip a repeated ancestor walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DirectoryIdentity(u64, [u8; 16]);

pub(super) fn directory_identity(path: &Path) -> Option<DirectoryIdentity> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .ok()?;
    if !file.metadata().ok()?.is_dir() {
        return None;
    }
    // SAFETY: an all-zero FILE_ID_INFO is a valid value of this plain struct.
    let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: the handle stays open for the call and `info` is a correctly
    // sized output buffer of the requested class.
    let read = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if read == 0 || info.FileId.Identifier == [0; 16] {
        return None;
    }
    let identity = DirectoryIdentity(info.VolumeSerialNumber, info.FileId.Identifier);
    Some(identity)
}

pub(super) fn from_os_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(super) fn policy_contains(root: &Path, path: &Path) -> bool {
    within(
        &normalized(&path.to_string_lossy()),
        &normalized(&root.to_string_lossy()),
    )
}

pub(super) fn canonical_contains(root: &Path, path: &Path) -> bool {
    // Canonical target resolution retains its original component comparison.
    path.starts_with(root)
}

#[cfg(test)]
pub(super) fn create_directory_link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}
