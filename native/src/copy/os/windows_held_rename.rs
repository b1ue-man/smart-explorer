//! The no-replace publication of a pinned stage through its own handle
//! (A7, 2026-10-29): one rename of the held object within its folder, no
//! path opened (before: a look at the destination, `MoveFileExW` opening the
//! stage path again).
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows_sys::Wdk::Storage::FileSystem::{
    FileRenameInformation, NtSetInformationFile, FILE_RENAME_INFORMATION,
};
use windows_sys::Win32::Foundation::{RtlNtStatusToDosError, STATUS_PENDING};

use super::super::is_unc;
use super::pending_block;

/// `commit_staged` for a stage whose pinned handle is `held` (A7): a new
/// copy is renamed through that handle within its folder, without opening a
/// path; everything else (replacements, durable publications, another
/// folder, a share) renames by path as before.
pub(crate) fn commit_staged_held(
    staged: &Path,
    dest: &Path,
    held: Option<&File>,
    overwrite: bool,
    durable: bool,
) -> io::Result<()> {
    if let (Some(file), false, false) = (held, overwrite, durable) {
        if let Some(result) = rename_held(file, staged, dest) {
            return result;
        }
    }
    super::super::commit_staged(staged, dest, overwrite, durable)
}

/// Publishes the pinned stage `file` (at `staged`) as `dest` without
/// replacing anything, by renaming the held object within its folder
/// (FILE_RENAME_INFORMATION with a simple name, ReplaceIfExists false: an
/// existing name fails as ERROR_ALREADY_EXISTS and nothing changed). `None`
/// when this does not apply (another folder, a share or network volume, a
/// name Windows would read as a stream or a path): the caller renames by
/// path.
fn rename_held(file: &File, staged: &Path, dest: &Path) -> Option<io::Result<()>> {
    let on_share = |path: &Path| path.to_str().is_some_and(is_unc);
    if staged.parent()? != dest.parent()? || on_share(dest) {
        return None;
    }
    // A network volume keeps the rename by path: its collision answer may
    // come from a replayed request, so it proves nothing (see below).
    if super::probe::remote_volume(file) != Some(false) {
        return None;
    }
    let name = dest.file_name()?;
    crate::vfs::check_local_new_name(Path::new(name)).ok()?;
    let wide: Vec<u16> = name.encode_wide().collect();
    let separator = |unit: &u16| matches!(*unit, 0 | 0x2f | 0x3a | 0x5c);
    if wide.is_empty() || wide.iter().any(separator) {
        return None;
    }
    Some(rename_in_folder(file, wide).map_err(|error| {
        // On a local volume a refused no-replace rename changed nothing: the
        // name was taken, as the look before the old path rename reported.
        if error.kind() == io::ErrorKind::AlreadyExists {
            crate::vfs::unpublished_error(
                error.kind(),
                format!(
                    "no-replace destination already exists, nothing renamed: {}",
                    dest.display()
                ),
            )
        } else {
            error
        }
    }))
}

fn rename_in_folder(file: &File, mut wide: Vec<u16>) -> io::Result<()> {
    let name_bytes = u32::try_from(wide.len() * 2)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Zielname zu lang"))?;
    wide.push(0);
    let bytes = std::mem::size_of::<FILE_RENAME_INFORMATION>() + wide.len() * 2;
    let bytes_u32 = u32::try_from(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Zielname zu lang"))?;
    let mut buffer = vec![0u64; bytes.div_ceil(8)];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    let mut completion = Box::new(pending_block());
    // SAFETY: u64 storage aligns the record; it holds the record and the
    // whole name with its terminator. RootDirectory null with a simple name
    // renames within the object's own folder. The handle is synchronous.
    let status = unsafe {
        (*info).Anonymous.ReplaceIfExists = 0;
        (*info).RootDirectory = std::ptr::null_mut();
        (*info).FileNameLength = name_bytes;
        std::ptr::copy_nonoverlapping(
            wide.as_ptr(),
            std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            wide.len(),
        );
        NtSetInformationFile(
            file.as_raw_handle(),
            completion.as_mut(),
            info.cast(),
            bytes_u32,
            FileRenameInformation,
        )
    };
    if status == STATUS_PENDING {
        // Never on a synchronous handle; the outcome is then unknown and the
        // buffers stay alive.
        std::mem::forget((buffer, completion));
        return Err(io::Error::other(
            "Umbenennen der Kopierstufe ohne Abschlussmeldung",
        ));
    }
    if status < 0 {
        // SAFETY: RtlNtStatusToDosError accepts any NTSTATUS.
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ));
    }
    Ok(())
}
