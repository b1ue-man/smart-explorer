//! Test-only metadata failure injection on a real, already-open Dokany request.
use std::{fs::OpenOptions, io, os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::Path, sync::{OnceLock, atomic::{AtomicI32, AtomicU64, AtomicUsize, Ordering}}};
use super::super::super::dokany_abi::{DokanFileInfo, DokanOperations, GetFileInformationCallback, NtStatus};
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_INVALID_PARAMETER},
    Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE},
};

static ORIGINAL: OnceLock<GetFileInformationCallback> = OnceLock::new();
static INJECT: AtomicI32 = AtomicI32::new(0);
static DELIVERED: AtomicUsize = AtomicUsize::new(0);
static CONTEXT: AtomicU64 = AtomicU64::new(0);
const SENTINEL: &str = "\\vault\\dll-status.txt";

pub(super) fn install(operations: &mut DokanOperations) {
    let original = operations.get_file_information.expect("production metadata callback");
    if let Err(other) = ORIGINAL.set(original) {
        assert_eq!(*ORIGINAL.get().expect("installed callback") as usize, other as usize,
            "serialized runtime cases changed their underlying metadata callback");
    }
    INJECT.store(0, Ordering::SeqCst);
    operations.get_file_information = Some(information);
}

unsafe extern "system" fn information(name: *const u16, output: *mut BY_HANDLE_FILE_INFORMATION,
    info: *mut DokanFileInfo) -> NtStatus {
    let status = INJECT.load(Ordering::Acquire);
    if status != 0 && unsafe { path_matches(name, SENTINEL) } {
        // Capture evidence of the real open-file context, not a failed CreateFile
        // path or a fabricated request pointer. Never panic across this callback.
        if let Some(info) = unsafe { info.as_ref() } {
            CONTEXT.store(info.context, Ordering::Release);
        }
        DELIVERED.fetch_add(1, Ordering::Release);
        return status;
    }
    match ORIGINAL.get() {
        Some(original) => unsafe { original(name, output, info) },
        None => 0xc000_000du32 as i32,
    }
}

pub(super) unsafe fn path_matches(name: *const u16, expected: &str) -> bool {
    if name.is_null() { return false; }
    for (index, unit) in expected.encode_utf16().chain(Some(0)).enumerate() {
        // Real Dokany callback names are NUL-terminated. Stop at the first
        // mismatch, including a shorter input; never read beyond its terminator.
        if unsafe { *name.add(index) } != unit { return false; }
    }
    true
}

struct Injection;
impl Drop for Injection { fn drop(&mut self) { INJECT.store(0, Ordering::Release); } }

pub(super) fn exercise(root: &Path, private: bool) -> io::Result<()> {
    let handle = OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(root.join("vault").join("dll-status.txt"))?;
    let mut output: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // Success first establishes an ordinary working handle before fault arming.
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle() as _, &mut output) } == 0 {
        return Err(io::Error::last_os_error());
    }
    for (status, expected) in [(0xc000_0034u32 as i32, ERROR_FILE_NOT_FOUND),
        (0xc000_0022u32 as i32, ERROR_ACCESS_DENIED)] {
        let delivered = DELIVERED.load(Ordering::Acquire);
        CONTEXT.store(0, Ordering::Release);
        let reset = Injection;
        INJECT.store(status, Ordering::Release);
        let result = unsafe { GetFileInformationByHandle(handle.as_raw_handle() as _, &mut output) };
        let error = if result == 0 { unsafe { GetLastError() } } else { 0 };
        drop(reset);
        assert!(DELIVERED.load(Ordering::Acquire) > delivered, "query bypassed injected metadata callback");
        assert_ne!(CONTEXT.load(Ordering::Acquire), 0, "metadata failure lacked an open context");
        assert_eq!(result, 0, "injected failure returned success");
        if private {
            assert_eq!(error, expected, "private DLL replaced the real callback failure");
        } else {
            // The unchanged official DLL can retain its known INVALID_PARAMETER
            // conversion; do not falsely require the private patch from it.
            assert!(error == expected || error == ERROR_INVALID_PARAMETER,
                "unexpected official metadata mapping: {error}");
        }
        eprintln!("[mount vault] metadata status private={private} injected=0x{:08x} win32={error}", status as u32);
    }
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle() as _, &mut output) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
