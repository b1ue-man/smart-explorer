use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use super::imp::copy_flags::{is_unc, windows_copy_flags, WINDOWS_REQUEST_COMPRESSED_TRAFFIC};
use super::imp::pipe_copy::{pipe_copy, PipeError, PipeSide};

#[path = "windows_fast.rs"]
mod fast;
#[path = "windows_optical.rs"]
mod windows_optical;
pub(super) use fast::{commit_staged_held, copy_from_reader, handle_has_path, open_copy_source};

#[path = "windows_identity.rs"]
mod windows_identity;
pub(super) use windows_identity::{same_file_object, stable_handle_identity, stable_path_identity};
#[path = "windows_recovery.rs"]
mod windows_recovery;
pub(super) use windows_recovery::{recover_transfer_child, recover_transfer_child_live};
#[path = "windows_publication_failure.rs"]
mod publication_failure;
pub(super) fn require_local_publication_observation(parent: &File) -> io::Result<()> {
    publication_failure::require_local_publication_observation(parent)
}

pub(super) fn open_publication_file(
    parent: &crate::local_access::ConfinedDirectory,
    name: &std::ffi::OsStr,
) -> io::Result<File> {
    parent.open_publication_file(name)
}

pub(super) fn open_move_publication_file(
    parent: &crate::local_access::ConfinedDirectory,
    name: &std::ffi::OsStr,
) -> io::Result<File> {
    parent.open_move_publication_file(name)
}

pub(super) fn confirm_publication_namespace(
    parent: &crate::local_access::ConfinedDirectory,
) -> io::Result<()> {
    parent.confirm_publication_namespace()
}

pub(super) fn confirm_live_publication_namespace(
    _parent: &crate::local_access::ConfinedDirectory,
    target: &Path,
) -> io::Result<()> {
    sync_parent(target)
}

pub(super) fn hard_link(source: &Path, target: &Path) -> io::Result<()> {
    std::fs::hard_link(
        crate::local_access::normalize_scan_root(source),
        crate::local_access::normalize_scan_root(target),
    )
}

pub(super) type FileIdentity = (u32, u64);

#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}

#[repr(C)]
#[derive(Default)]
struct FileInformation {
    attributes: u32,
    creation_time: FileTime,
    last_access_time: FileTime,
    last_write_time: FileTime,
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[link(name = "kernel32")]
extern "system" {
    #[link_name = "GetFileInformationByHandle"]
    fn get_file_information_by_handle(
        handle: *mut c_void,
        information: *mut FileInformation,
    ) -> i32;
}

/// Which file `path` names, through a handle that may only read attributes
/// (an unreadable destination can still be compared and replaced); a
/// reparse point is identified itself, never its target.
pub(super) fn path_identity(path: &Path) -> io::Result<FileIdentity> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    };
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(crate::local_access::normalize_scan_root(path))?;
    file_identity(&file)
}

pub(super) fn file_identity(file: &File) -> io::Result<FileIdentity> {
    let mut information = FileInformation::default();
    let ok =
        unsafe { get_file_information_by_handle(file.as_raw_handle(), &mut information as *mut _) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let index =
        (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low);
    Ok((information.volume_serial_number, index))
}

pub(super) fn different_rename_boundary(source: &File, target_parent: &File) -> io::Result<bool> {
    let source_volume = file_identity(source)?.0;
    let target_volume = file_identity(target_parent)?.0;
    // Zero is unknown, never proof of another volume. A move without
    // MOVEFILE_COPY_ALLOWED has no cross-volume copy/delete fallback.
    Ok(source_volume != 0 && target_volume != 0 && source_volume != target_volume)
}

/// An open file's identity together with its metadata.
pub(super) fn file_state(file: &File) -> io::Result<(FileIdentity, std::fs::Metadata)> {
    Ok((file_identity(file)?, file.metadata()?))
}

/// Identity, length and modification time of the regular file `path` names
/// now, read through a handle that refuses links and special files.
pub(super) fn path_state(
    path: &Path,
) -> io::Result<(FileIdentity, u64, Option<std::time::SystemTime>)> {
    // One attribute-only open for a plain file (A7); links, folders, special
    // files and refused opens take the checked way below.
    if let Some(state) = fast::path_state(path) {
        return Ok(state);
    }
    let file = crate::local_access::open_regular(path, crate::local_access::FinalLink::Refuse)?;
    let (identity, metadata) = file_state(&file)?;
    Ok((identity, metadata.len(), metadata.modified().ok()))
}

pub(super) fn path_matches_identity(path: &Path, expected: FileIdentity) -> io::Result<bool> {
    let actual = match path_identity(path) {
        Ok(identity) => identity,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    Ok(actual == expected)
}

pub(super) fn copy_permissions(
    writer: &File,
    source: &std::fs::Metadata,
    _destination: Option<&Path>,
) -> io::Result<()> {
    writer.set_permissions(source.permissions())
}

pub(super) fn path_text(path: &Path) -> io::Result<String> {
    path.to_str()
        .map(|_| {
            crate::local_access::display_path(&crate::local_access::normalize_scan_root(path))
                .replace('\\', "/")
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "path is not valid Unicode"))
}

pub(super) fn path_key(path: &Path) -> io::Result<String> {
    path_text(path).map(|path| path.to_lowercase())
}

/// Only `durable` publications (moves, replacements) rename write-through;
/// a new copy's stage is not flushed either (M9, 2026-10-28).
pub(super) fn commit_staged(
    staged: &Path,
    dest: &Path,
    overwrite: bool,
    durable: bool,
) -> io::Result<()> {
    if overwrite {
        replace_file_atomic(staged, dest)
    } else {
        crate::copy::publication_proof::preflight_no_replace(dest)?;
        if durable {
            move_file_no_replace(staged, dest)
        } else {
            move_file_ex(staged, dest, 0)
        }
    }
}

pub(super) fn move_file(src: &Path, dest: &Path, overwrite: bool) -> io::Result<()> {
    if overwrite {
        replace_file_atomic(src, dest)
    } else {
        crate::copy::publication_proof::preflight_no_replace(dest)?;
        move_file_no_replace(src, dest)
    }
}

fn move_file_no_replace(src: &Path, dest: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH;
    move_file_ex(src, dest, MOVEFILE_WRITE_THROUGH)
}

/// Write-through replace that also replaces a read-only destination (its
/// attribute moves to the new file).
fn replace_file_atomic(src: &Path, dest: &Path) -> io::Result<()> {
    crate::vfs::replace_local_file(src, dest)
}

fn move_file_ex(src: &Path, dest: &Path, flags: u32) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
    let src = crate::local_access::normalize_scan_root(src);
    let dest = crate::local_access::normalize_scan_root(dest);
    let src: Vec<u16> = src.as_os_str().encode_wide().chain(Some(0)).collect();
    let dest: Vec<u16> = dest.as_os_str().encode_wide().chain(Some(0)).collect();
    let ok = unsafe { MoveFileExW(src.as_ptr(), dest.as_ptr(), flags) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) fn is_cross_device(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::CrossesDevices || error.raw_os_error() == Some(17)
}

/// Block of the handle copy (protected sources, moves, replacements): the
/// size the older copy loop used, large enough for sequential disk speed.
const HANDLE_COPY_BUFFER: usize = 1024 * 1024;
/// Blocks circulating between the reading thread and the writing caller: a
/// slow read and a slow write overlap (the older loop alternated).
const HANDLE_COPY_DEPTH: usize = 4;

/// New copies go through `CopyFile2` onto the fresh stage name.
/// `known` is what the caller already learned of the source (its size
/// and whether it is on an optical medium, M10): no second probe and no
/// second metadata look per file; `None` looks both up here.
pub(super) fn copy_by_path(
    source: &Path,
    stage: &Path,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(u64),
    known: Option<super::SourceFacts>,
) -> Option<io::Result<Option<(u64, File)>>> {
    let optical = match known {
        Some(facts) => facts.optical,
        None => match crate::source_media::probe(source) {
            Ok(found) => found.is_some(),
            Err(error) => return Some(Err(error)),
        },
    };
    if optical {
        if let Some(result) = windows_optical::copy(source, stage, cancel, progress) {
            return Some(result);
        }
        crate::source_media::mark_mode(source, crate::source_media::ReadMode::MetadataFallback);
    }
    let size = known.map(|facts| facts.size);
    Some(copy_to_fresh(source, stage, cancel, progress, size))
}

/// `CopyFile2` onto `stage`, then the stage opened without following a
/// reparse point. Its length on disk is the copied size: an offloaded copy
/// (SMB, ReFS) may report fewer progress chunks than bytes.
fn copy_to_fresh(
    source: &Path,
    stage: &Path,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(u64),
    size: Option<u64>,
) -> io::Result<Option<(u64, File)>> {
    let Some(reported) = kernel_copy_file(source, stage, cancel, progress, size)? else {
        return Ok(None);
    };
    let (file, length) = match fast::open_staged(stage) {
        Ok(staged) => staged,
        Err(error) => {
            // Not a plain file (a link copied from a swapped source): remove
            // that entry itself, never what it points to.
            let _ = std::fs::remove_file(crate::local_access::normalize_scan_root(stage));
            return Err(error);
        }
    };
    if length > reported {
        progress(length - reported);
    }
    Ok(Some((length, file)))
}

/// Windows copies files through `CopyFile2` in one call and has no ranges to
/// measure: the measured copy of other platforms is a plain `copy_handles`
/// here, and the caller never learns a verdict.
pub(super) fn copy_probed(
    reader: &File,
    writer: &mut File,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(u64),
    _threads: usize,
    _report: &dyn Fn(bool),
) -> io::Result<Option<u64>> {
    copy_handles(reader, writer, cancel, progress, 1)
}

/// Copies through the opened handles (sources readable only through
/// `local_access`, durable moves and replacements): a reading thread runs
/// ahead of the writing caller (`pipe_copy`). `None` when canceled. The
/// error keeps the side the staging code sorts it by.
pub(super) fn copy_handles(
    reader: &File,
    writer: &mut File,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(u64),
    _parallel: usize,
) -> io::Result<Option<u64>> {
    let mut reader = reader;
    pipe_copy(
        &mut reader,
        writer,
        cancel,
        progress,
        HANDLE_COPY_BUFFER,
        HANDLE_COPY_DEPTH,
    )
    .map_err(side_error)
}

/// Staging sorts errors by kind: the ones only a destination can cause are
/// the target's, the rest the source's. A read error must not look like a
/// target error, so such a kind becomes a plain one.
fn side_error(failure: PipeError) -> io::Error {
    let PipeError { side, error } = failure;
    let target_only = matches!(
        error.kind(),
        io::ErrorKind::StorageFull
            | io::ErrorKind::QuotaExceeded
            | io::ErrorKind::ReadOnlyFilesystem
            | io::ErrorKind::FileTooLarge
            | io::ErrorKind::WriteZero
    );
    if side == PipeSide::Read && target_only {
        io::Error::other(error)
    } else {
        error
    }
}

struct CopyContext<'a> {
    cancel: &'a std::sync::atomic::AtomicBool,
    progress: &'a mut dyn FnMut(u64),
    reported: u64,
}

extern "system" fn copy_progress(
    message: *const windows_sys::Win32::Storage::FileSystem::COPYFILE2_MESSAGE,
    context: *const c_void,
) -> windows_sys::Win32::Storage::FileSystem::COPYFILE2_MESSAGE_ACTION {
    use windows_sys::Win32::Storage::FileSystem::{
        COPYFILE2_CALLBACK_CHUNK_FINISHED, COPYFILE2_PROGRESS_CANCEL, COPYFILE2_PROGRESS_CONTINUE,
    };
    if message.is_null() || context.is_null() {
        return COPYFILE2_PROGRESS_CONTINUE;
    }
    // SAFETY: CopyFile2 calls back synchronously on the copying thread with
    // the context pointer `kernel_copy_file` passed, which outlives the call.
    let context = unsafe { &mut *(context as *mut CopyContext<'_>) };
    if context.cancel.load(std::sync::atomic::Ordering::Acquire) {
        return COPYFILE2_PROGRESS_CANCEL;
    }
    // SAFETY: `message` is valid for this callback; `Type` selects the union
    // member, and only the chunk-finished member is read.
    let total = unsafe {
        let message = &*message;
        if message.Type == COPYFILE2_CALLBACK_CHUNK_FINISHED {
            Some(message.Info.ChunkFinished.uliTotalBytesTransferred)
        } else {
            None
        }
    };
    if let Some(total) = total {
        if total > context.reported {
            (context.progress)(total - context.reported);
            context.reported = total;
        }
    }
    COPYFILE2_PROGRESS_CONTINUE
}

/// Copies `source` to the new name `stage` in the kernel (`CopyFile2`: SMB
/// server offload, ReFS block cloning), failing instead of replacing an
/// existing `stage`. `None` when canceled (the partial copy is removed).
fn kernel_copy_file(
    source: &Path,
    stage: &Path,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(u64),
    size: Option<u64>,
) -> io::Result<Option<u64>> {
    use windows_sys::Win32::Storage::FileSystem::{CopyFile2, COPYFILE2_EXTENDED_PARAMETERS};
    let source_wide = wide(&crate::local_access::normalize_scan_root(source));
    let stage_wide = wide(&crate::local_access::normalize_scan_root(stage));
    let flags = copy_flags(source, stage, size);
    let mut context = CopyContext {
        cancel,
        progress,
        reported: 0,
    };
    let routine: unsafe extern "system" fn(
        *const windows_sys::Win32::Storage::FileSystem::COPYFILE2_MESSAGE,
        *const c_void,
    ) -> windows_sys::Win32::Storage::FileSystem::COPYFILE2_MESSAGE_ACTION = copy_progress;
    let parameters = COPYFILE2_EXTENDED_PARAMETERS {
        dwSize: std::mem::size_of::<COPYFILE2_EXTENDED_PARAMETERS>() as u32,
        dwCopyFlags: flags,
        pfCancel: std::ptr::null_mut(),
        pProgressRoutine: Some(routine),
        pvCallbackContext: &mut context as *mut CopyContext<'_> as *mut c_void,
    };
    // SAFETY: both paths are NUL-terminated wide strings and `parameters`
    // (with the context it points to) lives until CopyFile2 returns.
    let result = unsafe { CopyFile2(source_wide.as_ptr(), stage_wide.as_ptr(), &parameters) };
    let reported = context.reported;
    if result >= 0 {
        return Ok(Some(reported));
    }
    let error = hresult_error(result);
    if cancel.load(std::sync::atomic::Ordering::Acquire) {
        // A name that was already taken (FAIL_IF_EXISTS) is never ours.
        if error.kind() != io::ErrorKind::AlreadyExists {
            let _ = std::fs::remove_file(crate::local_access::normalize_scan_root(stage));
        }
        return Ok(None);
    }
    Err(error)
}

/// The flags of one copy (`copy_flags.rs`): the size of the source (the
/// caller's, else looked up), the memory to spare and whether a share is
/// on either end.
fn copy_flags(source: &Path, stage: &Path, size: Option<u64>) -> u32 {
    let size = size.unwrap_or_else(|| {
        std::fs::metadata(crate::local_access::normalize_scan_root(source))
            .map(|metadata| metadata.len())
            .unwrap_or(0)
    });
    let on_share = |path: &Path| path.to_str().is_some_and(is_unc);
    windows_copy_flags(
        size,
        crate::transfer::available_memory(),
        on_share(source) || on_share(stage),
        WINDOWS_REQUEST_COMPRESSED_TRAFFIC,
    )
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// A failed HRESULT as the Win32 error it wraps (`HRESULT_FROM_WIN32`).
fn hresult_error(result: i32) -> io::Error {
    let code = result as u32;
    if code & 0xFFFF_0000 == 0x8007_0000 {
        io::Error::from_raw_os_error((code & 0xFFFF) as i32)
    } else {
        io::Error::other(format!("CopyFile2 schlug fehl (HRESULT 0x{code:08x})"))
    }
}

pub(super) fn sync_parent(_path: &Path) -> io::Result<()> {
    // Finish an already successful MOVEFILE_WRITE_THROUGH operation. Both
    // no-replace moves and replacement commits use it, including on FAT/exFAT.
    // This is not confirmation of an arbitrary parent or a lost move reply;
    // those callers use confirm_publication_namespace on the held directory.
    Ok(())
}
