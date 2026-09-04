//! Diagnostics for the one serialized, synthetic mounted-volume test only.
//! This module is reachable only through host's cfg(test) fixture declaration.

use super::super::super::{
    callbacks_metadata, callbacks_open,
    dokany_abi::{DokanFileInfo, DokanIoSecurityContext, DokanOperations, FillFindData, NtStatus},
};
use std::{
    io::{self, Write},
    os::windows::ffi::OsStrExt,
    panic::AssertUnwindSafe,
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use windows_sys::Win32::{
    Foundation::{GetLastError, SetLastError},
    Storage::FileSystem::{
        GetFileAttributesExW, GetFileExInfoStandard, BY_HANDLE_FILE_INFORMATION,
        WIN32_FILE_ATTRIBUTE_DATA,
    },
};

const MAX_LINES: usize = 256;
const MAX_PATH_UNITS: usize = 256;
static VERBOSE: AtomicBool = AtomicBool::new(false);
static LINES: AtomicUsize = AtomicUsize::new(0);

pub(super) fn install(operations: &mut DokanOperations) {
    // The task entrypoint selects exactly one real-volume fixture and runs it
    // with --test-threads=1. Reset before Dokany can dispatch any callback.
    VERBOSE.store(false, Ordering::Relaxed);
    LINES.store(0, Ordering::Relaxed);
    operations.create_file = Some(create_file);
    operations.get_file_information = Some(get_file_information);
    operations.find_files = Some(find_files);
    operations.find_files_with_pattern = Some(find_files_with_pattern);
}

pub(super) fn arm_verbose() {
    // Do not reset the counter: failures during setup share this total budget.
    VERBOSE.store(true, Ordering::Release);
    eprintln!("[mount fixture] callback tracing armed; total callback line limit={MAX_LINES}");
}

pub(super) fn probe_root_attributes(root: &Path) {
    let path: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    // All fields are integers/FILETIMEs. Read returned attributes only after
    // the BOOL succeeds; on failure capture GetLastError before other calls.
    let mut data: WIN32_FILE_ATTRIBUTE_DATA = unsafe { std::mem::zeroed() };
    let result = unsafe {
        GetFileAttributesExW(path.as_ptr(), GetFileExInfoStandard,
            (&mut data as *mut WIN32_FILE_ATTRIBUTE_DATA).cast())
    };
    let error = if result == 0 { Some(unsafe { GetLastError() }) } else { None };
    let attributes = if result != 0 { Some(data.dwFileAttributes) } else { None };
    eprintln!("[mount fixture] parent GetFileAttributesExW pid={} path={:?} BOOL={result} error={error:?} attributes={attributes:?}",
        std::process::id(), root);
}

unsafe extern "system" fn create_file(
    file_name: *const u16,
    security_context: *mut DokanIoSecurityContext,
    desired_access: u32,
    file_attributes: u32,
    share_access: u32,
    create_disposition: u32,
    create_options: u32,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    let before = unsafe { snapshot(file_info) };
    let status = unsafe {
        callbacks_open::create_file(
            file_name, security_context, desired_access, file_attributes,
            share_access, create_disposition, create_options, file_info,
        )
    };
    record(status, || format!(
        "CreateFile path={:?} access=0x{desired_access:08x} attributes=0x{file_attributes:08x} share=0x{share_access:08x} disposition={create_disposition} options=0x{create_options:08x} before={before:?} after={:?}",
        unsafe { bounded_wide(file_name) }, unsafe { snapshot(file_info) },
    ));
    status
}

unsafe extern "system" fn get_file_information(
    file_name: *const u16,
    output: *mut BY_HANDLE_FILE_INFORMATION,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    let before = unsafe { snapshot(file_info) };
    let status = unsafe {
        callbacks_metadata::get_file_information(file_name, output, file_info)
    };
    record(status, || {
        let attributes = if status >= 0 && !output.is_null() {
            Some(unsafe { (*output).dwFileAttributes })
        } else { None };
        format!("GetFileInformation path={:?} attributes={attributes:?} before={before:?} after={:?}",
            unsafe { bounded_wide(file_name) }, unsafe { snapshot(file_info) })
    });
    status
}

unsafe extern "system" fn find_files(
    file_name: *const u16,
    fill: FillFindData,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    let before = unsafe { snapshot(file_info) };
    let status = unsafe { callbacks_metadata::find_files(file_name, fill, file_info) };
    record(status, || format!("FindFiles path={:?} before={before:?} after={:?}",
        unsafe { bounded_wide(file_name) }, unsafe { snapshot(file_info) }));
    status
}

unsafe extern "system" fn find_files_with_pattern(
    file_name: *const u16,
    search_pattern: *const u16,
    fill: FillFindData,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    let before = unsafe { snapshot(file_info) };
    let status = unsafe {
        callbacks_metadata::find_files_with_pattern(file_name, search_pattern, fill, file_info)
    };
    record(status, || format!("FindFilesWithPattern path={:?} pattern={:?} before={before:?} after={:?}",
        unsafe { bounded_wide(file_name) }, unsafe { bounded_wide(search_pattern) },
        unsafe { snapshot(file_info) }));
    status
}

#[derive(Debug)]
#[allow(dead_code)] // Fields are intentionally consumed through diagnostic Debug.
struct FileInfoSnapshot {
    pid: u32,
    context: u64,
    directory: u8,
    delete_pending: u8,
    paging_io: u8,
    synchronous_io: u8,
    no_cache: u8,
    write_to_end: u8,
}

unsafe fn snapshot(info: *mut DokanFileInfo) -> Option<FileInfoSnapshot> {
    // Copy only scalars; no reference or lock survives the production call.
    let info = unsafe { info.as_ref() }?;
    Some(FileInfoSnapshot {
        pid: info.process_id,
        context: info.context,
        directory: info.is_directory,
        delete_pending: info.delete_pending,
        paging_io: info.paging_io,
        synchronous_io: info.synchronous_io,
        no_cache: info.no_cache,
        write_to_end: info.write_to_end_of_file,
    })
}

unsafe fn bounded_wide(pointer: *const u16) -> String {
    if pointer.is_null() { return "<null>".into(); }
    let mut units = Vec::with_capacity(MAX_PATH_UNITS);
    // Dokany owns this NUL-terminated input for the duration of its callback.
    // Read one unit at a time: a short input need not have 256 readable units.
    for index in 0..MAX_PATH_UNITS {
        let unit = unsafe { *pointer.add(index) };
        if unit == 0 { return String::from_utf16_lossy(&units); }
        units.push(unit);
    }
    format!("{}<truncated>", String::from_utf16_lossy(&units))
}

fn record(status: NtStatus, details: impl FnOnce() -> String) {
    if status >= 0 && !VERBOSE.load(Ordering::Acquire) { return; }
    let Ok(index) = LINES.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
        (count < MAX_LINES).then(|| count + 1)
    }) else { return; };
    // Logging must neither unwind across the FFI boundary nor alter the
    // caller's last-error slot. It happens only after production returns.
    let last_error = unsafe { GetLastError() };
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let _ = writeln!(io::stderr(),
            "[mount fixture callback {}/{MAX_LINES}] status=0x{:08x} {}",
            index + 1, status as u32, details());
    }));
    unsafe { SetLastError(last_error) };
}
