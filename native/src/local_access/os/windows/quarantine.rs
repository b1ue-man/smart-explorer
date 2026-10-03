//! Windows capture pins the expected file with DELETE access, compares its
//! full identity, then renames that handle without replacing any destination.
use std::collections::hash_map::RandomState;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Storage::FileSystem::{
    FileIdInfo, FileRenameInfo, GetFileInformationByHandleEx, SetFileInformationByHandle,
    DELETE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_READ_ATTRIBUTES,
    FILE_RENAME_INFO, FILE_SHARE_READ,
};

use super::{validate_name, DirectoryHandle};

/// Dropping an unfinished guard retains its content; callers must explicitly
/// restore or move it and report failures with `retained_location`.
#[must_use = "restore the captured child or move it to a checked destination"]
pub(crate) struct QuarantinedChild {
    parent: DirectoryHandle,
    original: OsString,
    name: OsString,
    guard: File,
    file: File,
    active: bool,
}

/// A persisted recovery intent may choose only our exact private slot form.
/// Construct through DirectoryHandle::checked_quarantine_slot; raw paths are
/// never accepted by the capture method.
pub(crate) struct QuarantineSlot(OsString);

impl QuarantineSlot {
    pub(crate) fn name(&self) -> &OsStr { &self.0 }
}

impl DirectoryHandle {
    pub(crate) fn checked_quarantine_slot(name: &OsStr) -> io::Result<QuarantineSlot> {
        validate_name(name)?;
        let valid = name.to_str().and_then(|name| name.strip_prefix(".held.se-recycle-"))
            .is_some_and(|nonce| nonce.len() == 16
                && nonce.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
        if !valid {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid chosen quarantine slot"));
        }
        Ok(QuarantineSlot(name.to_os_string()))
    }

    /// The caller durably records this slot before the first rename.
    pub(crate) fn quarantine_regular_child_in(
        &self, name: &OsStr, expected: &File, slot: &QuarantineSlot,
    ) -> io::Result<QuarantinedChild> {
        validate_name(name)?;
        super::super::regular::validate_file(expected, true)?;
        let file = expected.try_clone()?;
        let guard = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES | DELETE)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.0.path.join(name))?;
        super::super::regular::validate_file(&guard, true)?;
        if identity(&guard)? != identity(expected)? {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "expected recycle child changed"));
        }
        rename_no_replace(&guard, &self.0.path.join(slot.name()))?;
        Ok(QuarantinedChild { parent: self.clone(), original: name.to_os_string(),
            name: slot.0.clone(), guard, file, active: true })
    }

    pub(crate) fn quarantine_regular_child(
        &self,
        name: &OsStr,
        expected: &File,
    ) -> io::Result<QuarantinedChild> {
        validate_name(name)?;
        super::super::regular::validate_file(expected, true)?;
        let file = expected.try_clone()?;
        let guard = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES | DELETE)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.0.path.join(name))?;
        super::super::regular::validate_file(&guard, true)?;
        if identity(&guard)? != identity(expected)? {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "expected recycle child changed"));
        }
        // The source ACL stays on the file; no additional read permission is
        // granted. This handle excludes write/delete sharing throughout.
        for _attempt in 0..1000u32 {
            let suffix = RandomState::new().build_hasher().finish();
            let held = OsString::from(format!(".held.se-recycle-{suffix:016x}"));
            match rename_no_replace(&guard, &self.0.path.join(&held)) {
                Ok(()) => {
                    return Ok(QuarantinedChild {
                        parent: self.clone(),
                        original: name.to_os_string(),
                        name: held,
                        guard,
                        file,
                        active: true,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "quarantine names kept colliding"))
    }
}

impl QuarantinedChild {
    pub(crate) fn file(&self) -> &File {
        &self.file
    }

    /// Diagnostics/recovery only; use `move_to` to retain the handle contract.
    pub(crate) fn retained_location(&self) -> PathBuf {
        self.parent.0.path.join(&self.name)
    }

    pub(crate) fn restore(&mut self) -> io::Result<()> {
        if self.active {
            rename_no_replace(&self.guard, &self.parent.0.path.join(&self.original))?;
            self.active = false;
        }
        Ok(())
    }

    pub(crate) fn move_to(&mut self, target: &DirectoryHandle, name: &OsStr) -> io::Result<()> {
        validate_name(name)?;
        if !self.active {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "quarantine is no longer active"));
        }
        rename_no_replace(&self.guard, &target.0.path.join(name))?;
        self.active = false;
        Ok(())
    }
}

fn identity(file: &File) -> io::Result<(u64, [u8; 16])> {
    // SAFETY: FILE_ID_INFO is a plain integer record, and the live handle and
    // correctly sized buffer remain valid for the synchronous query.
    let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else if info.VolumeSerialNumber == 0 || info.FileId.Identifier == [0; 16] {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "filesystem provides no safe recycle file identity",
        ))
    } else {
        Ok((info.VolumeSerialNumber, info.FileId.Identifier))
    }
}

fn rename_no_replace(file: &File, target: &Path) -> io::Result<()> {
    let name: Vec<u16> = target.as_os_str().encode_wide().collect();
    let name_bytes = name
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|length| u32::try_from(length).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "rename target too long"))?;
    let bytes = offset_of!(FILE_RENAME_INFO, FileName)
        .checked_add(name_bytes as usize)
        .and_then(|length| u32::try_from(length).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "rename record too long"))?;
    let mut buffer = vec![0u64; (bytes as usize).div_ceil(size_of::<u64>())];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: u64 storage provides the structure's alignment on supported
    // targets and includes the full trailing filename. Zero ReplaceIfExists
    // and a null RootDirectory choose an absolute no-replace destination.
    unsafe {
        (*info).Anonymous.ReplaceIfExists = 0;
        (*info).RootDirectory = std::ptr::null_mut();
        (*info).FileNameLength = name_bytes;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            name.len(),
        );
        if SetFileInformationByHandle(file.as_raw_handle(), FileRenameInfo, info.cast(), bytes) == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
