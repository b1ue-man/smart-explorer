//! Same-handle directory type, reparse-tag and filesystem identity checks.
use std::{
    io,
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::Path,
};
use windows_sys::Win32::Storage::FileSystem::*;

pub(crate) fn identity(path: &Path) -> io::Result<Option<String>> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| {
            if path.has_root() {
                path
            } else {
                Path::new(".")
            }
        });
    super::super::promotion::validate_destination_root(parent)?;
    let directory = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = directory.metadata()?;
    if !metadata.is_dir() {
        return Ok(None);
    }
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        let mut info: FILE_ATTRIBUTE_TAG_INFO = unsafe { std::mem::zeroed() };
        // SAFETY: this owned handle and the exact output structure stay live.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                directory.as_raw_handle(),
                FileAttributeTagInfo,
                (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        };
        let (link, special) =
            super::classify_tag(ok != 0, info.ReparseTag, metadata.file_attributes());
        if link || special {
            return Ok(None);
        }
    }
    let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    // ReFS needs the full 128-bit ID; its 64-bit compatibility ID can be
    // all-ones and must never make different folders look identical.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            directory.as_raw_handle(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if ok == 0 {
        let error = io::Error::last_os_error();
        return if matches!(error.raw_os_error(), Some(1 | 50 | 87 | 120)) {
            Ok(None)
        } else {
            Err(error)
        };
    }
    let index = u128::from_le_bytes(info.FileId.Identifier);
    let volume = info.VolumeSerialNumber;
    Ok((index != 0 && index != u128::MAX)
        .then(|| format!("agent-directory-windows:{volume:016x}:{index:032x}")))
}
