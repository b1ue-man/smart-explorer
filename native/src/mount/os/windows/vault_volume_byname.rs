//! Dynamic API coverage, separate from Node's potentially successful fallback.
use std::{
    ffi::c_void,
    fs::OpenOptions,
    io,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
};
use windows_sys::Win32::{
    Foundation::{FreeLibrary, GetLastError, HMODULE, ERROR_BAD_NET_NAME, ERROR_CALL_NOT_IMPLEMENTED,
        ERROR_FILE_NOT_FOUND, ERROR_INVALID_FUNCTION, ERROR_INVALID_LEVEL,
        ERROR_INVALID_PARAMETER, ERROR_NOT_READY, ERROR_NOT_SUPPORTED, ERROR_PATH_NOT_FOUND},
    Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE},
    System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32},
};

// windows-sys 0.59 predates this declaration. Exact SDK layout and enum:
// https://learn.microsoft.com/windows-hardware/drivers/ddi/ntifs/ns-ntifs-file_stat_basic_information
// https://learn.microsoft.com/windows/win32/api/minwinbase/ne-minwinbase-file_info_by_name_class
#[repr(C)]
#[derive(Default)]
struct StatBasic {
    file_id: i64,
    creation_time: i64,
    last_access_time: i64,
    last_write_time: i64,
    change_time: i64,
    allocation_size: i64,
    end_of_file: i64,
    attributes: u32,
    reparse_tag: u32,
    links: u32,
    device_type: u32,
    device_characteristics: u32,
    reserved: u32,
    volume_serial: i64,
    file_id_128: [u8; 16],
}
const _: () = assert!(std::mem::size_of::<StatBasic>() == 104);
const FILE_STAT_BASIC_BY_NAME: i32 = 3;
type QueryByName = unsafe extern "system" fn(*const u16, i32, *mut c_void, u32) -> i32;

struct ApiModule { handle: HMODULE, owned: bool }

impl ApiModule {
    fn acquire() -> io::Result<Self> {
        // Record the pinned libuv lookup, but our Rust harness need not import
        // the same modules as Node. Force a system-only load if not loaded yet;
        // this proves OS API availability, not Node's separate-process route.
        let module_name = wide(Path::new("api-ms-win-core-file-l2-1-4.dll"));
        let handle = unsafe { GetModuleHandleW(module_name.as_ptr()) };
        eprintln!("[mount vault] by_name_api already_loaded_in_rust={}", !handle.is_null());
        if !handle.is_null() { return Ok(Self { handle, owned: false }); }
        let handle = unsafe { LoadLibraryExW(module_name.as_ptr(), std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32) };
        if handle.is_null() { return Err(io::Error::last_os_error()); }
        eprintln!("[mount vault] by_name_api system_load_for_native_probe=true");
        Ok(Self { handle, owned: true })
    }
}

impl Drop for ApiModule {
    fn drop(&mut self) {
        // Borrowed GetModuleHandle references must never be released.
        if self.owned { unsafe { FreeLibrary(self.handle); } }
    }
}

pub(super) fn exercise(root: &Path) -> io::Result<()> {
    let module = ApiModule::acquire()?;
    let address = unsafe { GetProcAddress(module.handle, b"GetFileInformationByName\0".as_ptr()) }
        .ok_or_else(io::Error::last_os_error)?;
    let query: QueryByName = unsafe { std::mem::transmute(address) };
    for (path, directory) in [(root.join("large"), true),
        (root.join("large").join("b511").join("d0").join("note0.md"), false)] {
        let encoded = wide(&path);
        let mut output = StatBasic::default();
        let ok = unsafe { query(encoded.as_ptr(), FILE_STAT_BASIC_BY_NAME,
            (&mut output as *mut StatBasic).cast(), std::mem::size_of::<StatBasic>() as u32) };
        if ok == 0 {
            let error = unsafe { GetLastError() };
            eprintln!("[mount vault] by_name path={} success=false win32={error}", path.display());
            match error {
                ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED | ERROR_CALL_NOT_IMPLEMENTED
                    | ERROR_INVALID_PARAMETER | ERROR_INVALID_LEVEL => {}
                ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_NOT_READY | ERROR_BAD_NET_NAME => {
                    return Err(io::Error::other(format!(
                        "by-name query falsely rejected known fixture path {}: {error}", path.display())));
                }
                _ => return Err(io::Error::from_raw_os_error(error as i32)),
            }
        } else {
            check(&path, output.attributes, output.end_of_file as u64, directory)?;
            eprintln!("[mount vault] by_name path={} success=true", path.display());
        }
        // Always exercise the classic handle query as well, including when the
        // new information class is legitimately unsupported by the filesystem.
        let handle = OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(&path)?;
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(handle.as_raw_handle() as _, &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        check(&path, info.dwFileAttributes,
            (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow), directory)?;
    }
    Ok(())
}

fn check(path: &Path, attributes: u32, size: u64, directory: bool) -> io::Result<()> {
    if (attributes & FILE_ATTRIBUTE_DIRECTORY != 0) != directory
        || attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 || (!directory && size != 4)
    {
        return Err(io::Error::other(format!("incorrect native metadata for {}", path.display())));
    }
    Ok(())
}

fn wide(path: &Path) -> Vec<u16> { path.as_os_str().encode_wide().chain(Some(0)).collect() }
