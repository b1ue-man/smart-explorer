//! The Windows copy of one small local file without `CopyFile2`, and the
//! no-replace publication of a stage through its own handle (A7,
//! 2026-10-29).
//!
//! Before: twelve opens of a path per new file (two looks at the source,
//! its open, two inside `CopyFile2`, the stage reopened, the source path
//! opened twice afterwards, the stage path compared, the destination looked
//! at, the rename opening the stage once more). Now: the source opened once,
//! the stage created once, everything else asked of those two handles.
//!
//! What `CopyFile2` keeps is kept: a source with anything beyond plain data
//! and the simple attributes (named streams, extended attributes, security
//! resource attributes, compressed, sparse, encrypted, reparse, offline,
//! integrity or pinned state) on a local volume, larger than `FAST_MAX`, on
//! a share or on an optical medium still goes through `CopyFile2`.
//! Timestamps and attributes are set as the optical path sets them
//! (`windows_optical::set_basic`).
//!
//! Every stage made here, and every stage `CopyFile2` made (`open_staged`),
//! is held with DELETE access and shared without FILE_SHARE_DELETE: no other
//! process can rename or delete the stage, nor rename its folder, while it
//! is held. Its no-replace rename to a sibling name therefore happens in the
//! folder it was created in, on exactly the object that was copied.
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::STATUS_PENDING;
use windows_sys::Win32::Storage::FileSystem::{
    FileAllocationInfo, FileDispositionInfo, FileNameInfo, GetFileInformationByHandleEx,
    SetFileInformationByHandle, DELETE, FILE_ATTRIBUTE_DEVICE, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use windows_sys::Win32::System::IO::{IO_STATUS_BLOCK, IO_STATUS_BLOCK_0};

use super::{file_state, get_file_information_by_handle, is_unc};
use super::{FileIdentity, FileInformation};

/// Largest file copied here: the local packets' small-file limit
/// (`local_packet::SMALL_FILE`). Larger files keep `CopyFile2`, whose
/// unbuffered mode and block cloning matter there.
const FAST_MAX: u64 = 1024 * 1024;
/// Bytes per read and write: the engine's reservation per local operation
/// (`ops::COPY_BUFFER`), one buffer per worker thread.
const CHUNK: usize = 256 * 1024;
/// Attributes a stage can be given exactly: read-only, hidden, system,
/// archive, normal, temporary, not content indexed. Everything else
/// (offline, reparse, compressed, sparse, encrypted, integrity, pinned,
/// recall states) keeps `CopyFile2`.
const SIMPLE: u32 = 0x1 | 0x2 | 0x4 | 0x20 | 0x80 | 0x100 | 0x2000;

#[path = "windows_fast_probe.rs"]
mod probe;
#[path = "windows_held_rename.rs"]
mod rename;
use probe::plain_local_source;
pub(crate) use rename::commit_staged_held;

thread_local! {
    static BUFFER: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn information(file: &File) -> io::Result<FileInformation> {
    let mut information = FileInformation::default();
    // SAFETY: a live handle and a correctly sized output record.
    if unsafe { get_file_information_by_handle(file.as_raw_handle(), &mut information) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(information)
}

/// The source of a new copy, opened once: read access, every sharing mode,
/// the entry itself (never a reparse target). `None` (the caller takes the
/// checked `local_access` way, with its refusals and protected-source
/// fallbacks) for anything but a plain file without a reparse point, and
/// for every open that fails.
pub(crate) fn open_copy_source(path: &Path) -> Option<File> {
    let file = OpenOptions::new()
        .access_mode(FILE_GENERIC_READ)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(crate::local_access::normalize_scan_root(path))
        .ok()?;
    let information = information(&file).ok()?;
    let other = FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_DEVICE;
    (information.attributes & other == 0).then_some(file)
}

/// What `path` names now, through one attribute-only open (no read access,
/// so no sharing conflict); `None` for anything but a plain file and for a
/// failed open (the caller then asks `local_access`).
pub(super) fn path_state(
    path: &Path,
) -> Option<(FileIdentity, u64, Option<std::time::SystemTime>)> {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(crate::local_access::normalize_scan_root(path))
        .ok()?;
    let (identity, metadata) = file_state(&file).ok()?;
    let other = FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_DEVICE;
    if metadata.file_attributes() & other != 0 || !metadata.is_file() {
        return None;
    }
    Some((identity, metadata.len(), metadata.modified().ok()))
}

/// Whether the open `file` is the only name `path` (a drive-letter path)
/// stands for right now, asked of the handle (FileNameInfo: its name on its
/// volume) without opening the path. False whenever that cannot be told
/// this way (another form of path, a folder mount, a short or differently
/// cased name, a file with more than one link): the caller then looks at
/// the path.
pub(crate) fn handle_has_path(file: &File, path: &Path) -> bool {
    let normalized = crate::local_access::normalize_scan_root(path);
    let Some(text) = normalized.to_str() else {
        return false;
    };
    let Some(rest) = text.strip_prefix(r"\\?\") else {
        return false;
    };
    let bytes = rest.as_bytes();
    let drive =
        bytes.len() > 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    if !drive {
        return false;
    }
    let expected: Vec<u16> = rest[2..].encode_utf16().collect();
    // FILE_NAME_INFO: the name's byte length, then the name; one unit spare
    // so a longer name fails instead of matching a prefix.
    let units = expected.len() + 1;
    let mut buffer = vec![0u32; 1 + (units * 2).div_ceil(4)];
    let size = (buffer.len() * 4) as u32;
    // SAFETY: a live handle and a u32-aligned buffer of `size` bytes.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileNameInfo,
            buffer.as_mut_ptr().cast(),
            size,
        )
    } == 0
    {
        return false;
    }
    let length = buffer[0] as usize / 2;
    if length != expected.len() {
        return false;
    }
    // SAFETY: the call wrote `length` units after the length field, inside
    // the buffer (checked against `expected`, which fits by construction).
    let name = unsafe { std::slice::from_raw_parts(buffer.as_ptr().add(1).cast::<u16>(), length) };
    name == expected.as_slice()
        && information(file).is_ok_and(|information| information.number_of_links == 1)
}

/// The bytes of a small plain local source into a new pinned stage, read
/// through the handle the caller opened and observed. `None` when this file
/// is one for `CopyFile2` (see the module comment). After a failure the
/// reader's position is wherever reading stopped: the caller rewinds it
/// before any further copy (`staging::stage_by_path`).
pub(crate) fn copy_from_reader(
    source: &Path,
    reader: &File,
    stage: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    known: Option<super::super::SourceFacts>,
) -> Option<io::Result<Option<(u64, File)>>> {
    let known = known?;
    let on_share = |path: &Path| path.to_str().is_some_and(is_unc);
    if known.optical || known.size > FAST_MAX || on_share(source) || on_share(stage) {
        return None;
    }
    let information = information(reader).ok()?;
    if information.attributes & !SIMPLE != 0 || !plain_local_source(reader) {
        return None;
    }
    Some(copy_into_stage(
        reader,
        &information,
        known.size,
        stage,
        cancel,
        progress,
    ))
}

/// A new stage, created exclusively, held pinned (see the module comment).
/// Like every stage, it inherits its folder's ACL.
pub(super) fn create_stage(stage: &Path) -> io::Result<File> {
    crate::vfs::check_local_new_name(stage)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .access_mode(FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(crate::local_access::normalize_scan_root(stage))
}

/// Opens the stage a kernel copy produced without following a reparse
/// point, pinned like `create_stage`, and accepts only a plain regular file
/// (K12 identity check); with its length on disk.
pub(super) fn open_staged(stage: &Path) -> io::Result<(File, u64)> {
    let file = OpenOptions::new()
        .access_mode(FILE_GENERIC_READ | DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(crate::local_access::normalize_scan_root(stage))?;
    let metadata = file.metadata()?;
    let class = crate::local_access::classify_open_file(&file)?;
    if !metadata.is_file() || class.link_like || class.special {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Kopierstufe ist keine direkte reguläre Datei",
        ));
    }
    Ok((file, metadata.len()))
}

fn copy_into_stage(
    reader: &File,
    information: &FileInformation,
    size: u64,
    stage: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<Option<(u64, File)>> {
    let writer = create_stage(stage)?;
    let result = fill(reader, &writer, size, cancel, progress).and_then(|copied| match copied {
        Some(bytes) => {
            super::windows_optical::set_basic(&writer, Some(information)).map(|()| Some(bytes))
        }
        None => Ok(None),
    });
    match result {
        Ok(Some(bytes)) => Ok(Some((bytes, writer))),
        other => {
            delete_own(&writer);
            drop(writer);
            other.map(|_| None)
        }
    }
}

/// Reads the observed size of the source from its start into the stage
/// (less when it ends earlier, which the length check then reports). `None`
/// when canceled.
fn fill(
    reader: &File,
    writer: &File,
    size: u64,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<Option<u64>> {
    if size > CHUNK as u64 {
        allocate(writer, size);
    }
    BUFFER.with(|cell| {
        let mut buffer = cell.borrow_mut();
        if buffer.len() < CHUNK {
            buffer.resize(CHUNK, 0);
        }
        let (mut source, mut target) = (reader, writer);
        let mut total = 0u64;
        loop {
            if cancel.load(Ordering::Acquire) {
                return Ok(None);
            }
            let count = match source.read(&mut buffer[..CHUNK]) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            target.write_all(&buffer[..count])?;
            total = total.saturating_add(count as u64);
            progress(count as u64);
            // The observed size is in: no further read only to meet the end
            // (a source that grew meanwhile fails the length comparison
            // after the copy, `staging::unchanged`).
            if total >= size {
                break;
            }
        }
        Ok(Some(total))
    })
}

/// Reserves the stage's clusters at once (one allocation instead of one per
/// write); a refusal changes nothing but speed.
fn allocate(file: &File, size: u64) {
    let allocation = i64::try_from(size).unwrap_or(i64::MAX);
    // SAFETY: FILE_ALLOCATION_INFO is one LARGE_INTEGER.
    unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileAllocationInfo,
            (&allocation as *const i64).cast(),
            std::mem::size_of::<i64>() as u32,
        )
    };
}

/// Deletes the stage this module created, through its own handle (never a
/// path another process could have changed); best effort, as the optical
/// path's cleanup.
fn delete_own(file: &File) {
    // A read-only bit set by `set_basic` would refuse the deletion.
    let _ = super::windows_optical::set_basic(file, None);
    let delete: u8 = 1;
    // SAFETY: FILE_DISPOSITION_INFO is one BOOLEAN; the handle has DELETE.
    unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&delete as *const u8).cast(),
            1,
        )
    };
}

fn pending_block() -> IO_STATUS_BLOCK {
    IO_STATUS_BLOCK {
        Anonymous: IO_STATUS_BLOCK_0 {
            Status: STATUS_PENDING,
        },
        Information: 0,
    }
}

#[cfg(test)]
#[path = "windows_fast_tests.rs"]
mod tests;
