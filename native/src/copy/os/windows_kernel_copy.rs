//! The kernel copy of a new file onto its stage name (`CopyFile2`: SMB
//! server offload, ReFS block cloning), with the flags `copy_flags.rs`
//! chooses for the file.
use std::ffi::c_void;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::super::imp::copy_flags::{
    is_unc, windows_copy_flags, COPY_FILE_DISABLE_PRE_ALLOCATION,
    WINDOWS_REQUEST_COMPRESSED_TRAFFIC,
};
use super::wide;

/// HRESULT of ERROR_INVALID_PARAMETER (E_INVALIDARG): how a system that does
/// not know a flag answers it.
const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;

/// Set when a system refused `COPY_FILE_DISABLE_PRE_ALLOCATION` (Windows 10
/// before build 19041, Learn `COPYFILE2_EXTENDED_PARAMETERS`): later copies
/// of the process leave the flag out at once.
static PRE_ALLOCATION_REFUSED: AtomicBool = AtomicBool::new(false);

struct CopyContext<'a> {
    cancel: &'a AtomicBool,
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
    // the context pointer `copy_file2` passed, which outlives the call.
    let context = unsafe { &mut *(context as *mut CopyContext<'_>) };
    if context.cancel.load(Ordering::Acquire) {
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
pub(super) fn kernel_copy_file(
    source: &Path,
    stage: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    size: Option<u64>,
) -> io::Result<Option<u64>> {
    let source_wide = wide(&crate::local_access::normalize_scan_root(source));
    let stage_wide = wide(&crate::local_access::normalize_scan_root(stage));
    let flags = copy_flags(source, stage, size);
    let mut context = CopyContext {
        cancel,
        progress,
        reported: 0,
    };
    let mut result = copy_file2(&source_wide, &stage_wide, flags, &mut context);
    if result == E_INVALIDARG && flags & COPY_FILE_DISABLE_PRE_ALLOCATION != 0 {
        // A system before Windows 10 build 19041 does not know the flag and
        // refuses the call before it creates anything: ask again without it.
        PRE_ALLOCATION_REFUSED.store(true, Ordering::Relaxed);
        result = copy_file2(
            &source_wide,
            &stage_wide,
            flags & !COPY_FILE_DISABLE_PRE_ALLOCATION,
            &mut context,
        );
    }
    let reported = context.reported;
    if result >= 0 {
        return Ok(Some(reported));
    }
    let error = hresult_error(result);
    if cancel.load(Ordering::Acquire) {
        // A name that was already taken (FAIL_IF_EXISTS) is never ours.
        if error.kind() != io::ErrorKind::AlreadyExists {
            let _ = std::fs::remove_file(crate::local_access::normalize_scan_root(stage));
        }
        return Ok(None);
    }
    Err(error)
}

/// One `CopyFile2` call; the context lives until it returns.
fn copy_file2(source: &[u16], stage: &[u16], flags: u32, context: &mut CopyContext<'_>) -> i32 {
    use windows_sys::Win32::Storage::FileSystem::{CopyFile2, COPYFILE2_EXTENDED_PARAMETERS};
    let routine: unsafe extern "system" fn(
        *const windows_sys::Win32::Storage::FileSystem::COPYFILE2_MESSAGE,
        *const c_void,
    ) -> windows_sys::Win32::Storage::FileSystem::COPYFILE2_MESSAGE_ACTION = copy_progress;
    let parameters = COPYFILE2_EXTENDED_PARAMETERS {
        dwSize: std::mem::size_of::<COPYFILE2_EXTENDED_PARAMETERS>() as u32,
        dwCopyFlags: flags,
        pfCancel: std::ptr::null_mut(),
        pProgressRoutine: Some(routine),
        pvCallbackContext: context as *mut CopyContext<'_> as *mut c_void,
    };
    // SAFETY: both paths are NUL-terminated wide strings and `parameters`
    // (with the context it points to) lives until CopyFile2 returns.
    unsafe { CopyFile2(source.as_ptr(), stage.as_ptr(), &parameters) }
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
    let flags = windows_copy_flags(
        size,
        crate::transfer::available_memory(),
        on_share(source) || on_share(stage),
        WINDOWS_REQUEST_COMPRESSED_TRAFFIC,
    );
    if PRE_ALLOCATION_REFUSED.load(Ordering::Relaxed) {
        flags & !COPY_FILE_DISABLE_PRE_ALLOCATION
    } else {
        flags
    }
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
