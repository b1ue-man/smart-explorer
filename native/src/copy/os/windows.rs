use std::ffi::c_void;
use std::fs::{File, Metadata};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

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

pub(super) fn same_file(left: &Path, right: &Path) -> io::Result<bool> {
    let left = crate::local_access::open_read(left)?;
    let right = crate::local_access::open_read(right)?;
    Ok(file_identity(&left)? == file_identity(&right)?)
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

pub(super) fn path_matches_identity(path: &Path, expected: FileIdentity) -> io::Result<bool> {
    let file = match crate::local_access::open_read(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    Ok(file_identity(&file)? == expected)
}

pub(super) fn metadata_is_link_like(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
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

pub(super) fn commit_staged(staged: &Path, dest: &Path, overwrite: bool) -> io::Result<()> {
    if overwrite {
        replace_file_atomic(staged, dest)
    } else {
        move_file_no_replace(staged, dest)
    }
}

pub(super) fn move_file(src: &Path, dest: &Path, overwrite: bool) -> io::Result<()> {
    if overwrite {
        replace_file_atomic(src, dest)
    } else {
        move_file_no_replace(src, dest)
    }
}

fn move_file_no_replace(src: &Path, dest: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH;
    move_file_ex(src, dest, MOVEFILE_WRITE_THROUGH)
}

fn replace_file_atomic(src: &Path, dest: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    move_file_ex(
        src,
        dest,
        MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
    )
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

/// Buffer of the handle copy (protected sources, moves, replacements): the
/// size the older copy loop used, large enough for sequential disk speed.
const HANDLE_COPY_BUFFER: usize = 1024 * 1024;

/// New copies go through `CopyFile2` onto the fresh stage name.
pub(super) fn copy_by_path(
    source: &Path,
    stage: &Path,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Option<io::Result<Option<(u64, File)>>> {
    Some(
        kernel_copy_file(source, stage, cancel, progress).and_then(|copied| match copied {
            Some(bytes) => match open_staged(stage) {
                Ok(file) => Ok(Some((bytes, file))),
                Err(error) => {
                    // Not a plain file (a link copied from a swapped source):
                    // remove that entry itself, never what it points to.
                    let _ = std::fs::remove_file(crate::local_access::normalize_scan_root(stage));
                    Err(error)
                }
            },
            None => Ok(None),
        }),
    )
}

/// Copies through the opened handles (sources readable only through
/// `local_access`, durable moves and replacements). `None` when canceled.
pub(super) fn copy_handles(
    reader: &File,
    writer: &mut File,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<Option<u64>> {
    use std::io::{Read, Write};
    let mut reader = reader;
    let mut buffer = vec![0u8; HANDLE_COPY_BUFFER];
    let mut copied = 0u64;
    loop {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(None);
        }
        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if read == 0 {
            return Ok(Some(copied));
        }
        writer.write_all(&buffer[..read])?;
        copied = copied.saturating_add(read as u64);
        progress(read as u64);
    }
}

/// `COPY_FILE_FAIL_IF_EXISTS` (WinBase.h; windows-sys 0.59 lists it under
/// `Win32::System::WindowsProgramming`, value 1): never replace the stage name.
const COPY_FILE_FAIL_IF_EXISTS: u32 = 1;
/// `COPY_FILE_COPY_SYMLINK` (value 0x800): a source swapped for a link is
/// copied as a link, which the reparse check of the stage then refuses.
const COPY_FILE_COPY_SYMLINK: u32 = 0x800;

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
) -> io::Result<Option<u64>> {
    use windows_sys::Win32::Storage::FileSystem::{CopyFile2, COPYFILE2_EXTENDED_PARAMETERS};
    let source_wide = wide(&crate::local_access::normalize_scan_root(source));
    let stage_wide = wide(&crate::local_access::normalize_scan_root(stage));
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
        dwCopyFlags: COPY_FILE_FAIL_IF_EXISTS | COPY_FILE_COPY_SYMLINK,
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
        let _ = std::fs::remove_file(crate::local_access::normalize_scan_root(stage));
        return Ok(None);
    }
    Err(error)
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

/// Opens the stage a kernel copy produced without following a reparse
/// point, and accepts only a plain regular file (K12 identity check).
fn open_staged(stage: &Path) -> io::Result<File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(crate::local_access::normalize_scan_root(stage))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Kopierstufe ist keine direkte reguläre Datei",
        ));
    }
    Ok(file)
}

pub(super) fn sync_parent(_path: &Path) -> io::Result<()> {
    // MOVEFILE_WRITE_THROUGH is used by replacement commits. Opening a
    // directory for FlushFileBuffers requires extra Win32 privileges and is
    // not needed for the no-replace MoveFile path.
    Ok(())
}
