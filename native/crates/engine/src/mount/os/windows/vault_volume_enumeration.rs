//! Bounded native-buffer coverage; elapsed time is not a complexity proof.
//! Contracts: Microsoft NtQueryDirectoryFile / FILE_NAMES_INFORMATION /
//! IO_STATUS_BLOCK; Dokany v2.3.1.1000 dokan/directory.c.
use std::{collections::HashSet, ffi::c_void, fs::{File, OpenOptions}, io,
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle}, path::Path};
use windows_sys::Win32::{
    Foundation::{HANDLE, WAIT_OBJECT_0},
    Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE},
    System::{LibraryLoader::{GetModuleHandleW, GetProcAddress},
        Threading::{WaitForSingleObject, INFINITE}},
};

const SUCCESS: i32 = 0;
const PENDING: i32 = 0x103;
const NO_MORE_FILES: i32 = 0x8000_0006u32 as i32;
const BUFFER_OVERFLOW: i32 = 0x8000_0005u32 as i32;
const NO_SUCH_FILE: i32 = 0xc000_000fu32 as i32;
const FILE_NAMES_INFORMATION: i32 = 12;
const SYNCHRONIZE: u32 = 0x0010_0000;

#[repr(C)]
union Status { status: i32, pointer: *mut c_void }
#[repr(C)]
struct IoStatus { status: Status, information: usize }
#[repr(C)]
struct UnicodeString { length: u16, maximum_length: u16, buffer: *mut u16 }
type Apc = unsafe extern "system" fn(*mut c_void, *mut IoStatus, u32);
type Query = unsafe extern "system" fn(HANDLE, HANDLE, Option<Apc>, *mut c_void,
    *mut IoStatus, *mut c_void, u32, i32, u8, *mut UnicodeString, u8) -> i32;

struct Directory { file: File, query: Query }
struct Reply { status: i32, names: Vec<String> }

impl Directory {
    fn open(path: &Path, query: Query) -> io::Result<Self> {
        // No OVERLAPPED flag: native queries are synchronous. SYNCHRONIZE also
        // permits safe completion waiting if the OS ever reports STATUS_PENDING.
        let file = OpenOptions::new().access_mode(FILE_LIST_DIRECTORY | SYNCHRONIZE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(path)?;
        Ok(Self { file, query })
    }

    fn next(&self, size: usize, single: bool, restart: bool, pattern: Option<&str>) -> io::Result<Reply> {
        let mut buffer = [0u64; 32]; // aligned, initialized, never freed before completion
        assert!((16..=std::mem::size_of_val(&buffer)).contains(&size));
        let mut units = pattern.map(|value| value.encode_utf16().collect::<Vec<_>>());
        let mut expression = units.as_mut().map(|units| UnicodeString {
            length: (units.len() * 2) as u16,
            maximum_length: (units.len() * 2) as u16,
            buffer: units.as_mut_ptr(),
        });
        let mut io_status = IoStatus { status: Status { pointer: std::ptr::null_mut() }, information: 0 };
        let handle = self.file.as_raw_handle() as HANDLE;
        let mut status = unsafe { (self.query)(handle, std::ptr::null_mut(), None,
            std::ptr::null_mut(), &mut io_status, buffer.as_mut_ptr().cast(), size as u32,
            FILE_NAMES_INFORMATION, u8::from(single), expression.as_mut()
                .map_or(std::ptr::null_mut(), |value| value), u8::from(restart)) };
        if status == PENDING {
            // The enclosing native-phase timeout unmounts before joining; the
            // outer volume deadline aborts if even kernel teardown cannot finish.
            if unsafe { WaitForSingleObject(handle, INFINITE) } != WAIT_OBJECT_0 {
                eprintln!("[mount vault] cannot establish pending directory-query completion");
                std::process::abort();
            }
            status = unsafe { io_status.status.status };
        }
        if io_status.information > size {
            return Err(io::Error::other("native directory reply exceeded its supplied buffer"));
        }
        let names = if status == SUCCESS {
            let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), io_status.information) };
            decode(bytes)?
        } else { Vec::new() };
        Ok(Reply { status, names })
    }
}

pub(super) fn exercise(root: &Path) -> io::Result<()> {
    let library: Vec<u16> = "ntdll.dll".encode_utf16().chain(Some(0)).collect();
    // ntdll is loaded by every Windows process; this borrowed handle is not freed.
    let module = unsafe { GetModuleHandleW(library.as_ptr()) };
    if module.is_null() { return Err(io::Error::last_os_error()); }
    let address = unsafe { GetProcAddress(module, b"NtQueryDirectoryFile\0".as_ptr()) }
        .ok_or_else(io::Error::last_os_error)?;
    let query: Query = unsafe { std::mem::transmute(address) };
    let wide = root.join("wide");

    // Exactly 10,000 matches from the existing 50,001-entry immutable fixture.
    // The first expression belongs to this handle for all continuations/restarts.
    let expected = (0..10_000).map(|n| format!("f{n:05}.md")).collect::<HashSet<_>>();
    let directory = Directory::open(&wide, query)?;
    collect(&directory, &expected, 64, true, Some("f0*.md"), true)?;
    assert_eq!(directory.next(64, true, false, None)?.status, NO_MORE_FILES);
    // Restart changes the cursor, not the initially captured search expression.
    collect(&directory, &expected, 256, false, None, true)?;

    let overflow = Directory::open(&wide, query)?;
    assert_eq!(overflow.next(16, true, true, Some("f00000.md"))?.status, BUFFER_OVERFLOW,
        "first-call fixed header fits but full name does not");
    let one = HashSet::from(["f00000.md".to_string()]);
    collect(&overflow, &one, 64, true, None, true)?;
    let missing = Directory::open(&wide, query)?;
    assert_eq!(missing.next(64, true, true, Some("absent-*.md"))?.status, NO_SUCH_FILE);
    eprintln!("[mount vault] native directory single=10000 small_buffer=64 restart_buffer=256 pattern/eof/first_overflow=covered");
    Ok(())
}

fn collect(directory: &Directory, expected: &HashSet<String>, size: usize, single: bool,
    pattern: Option<&str>, restart: bool) -> io::Result<()> {
    let mut remaining = expected.clone();
    // At most one successful call per expected entry plus EOF; no unbounded loop.
    for call in 0..=expected.len() {
        let reply = directory.next(size, single, restart && call == 0,
            if call == 0 { pattern } else { None })?;
        if reply.status == NO_MORE_FILES {
            if remaining.is_empty() { return Ok(()); }
            return Err(io::Error::other(format!("directory EOF omitted {} names", remaining.len())));
        }
        if reply.status != SUCCESS || reply.names.is_empty() {
            return Err(io::Error::other(format!("directory query failed/stalled: 0x{:08x}", reply.status as u32)));
        }
        if single && reply.names.len() != 1 {
            return Err(io::Error::other("ReturnSingleEntry returned multiple records"));
        }
        for name in reply.names {
            if !remaining.remove(&name) {
                return Err(io::Error::other(format!("unexpected or duplicated native directory name {name}")));
            }
        }
    }
    Err(io::Error::other("native enumeration exhausted its bounded call allowance"))
}

fn decode(bytes: &[u8]) -> io::Result<Vec<String>> {
    let invalid = || io::Error::other("malformed FILE_NAMES_INFORMATION chain");
    let mut names = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        let header = bytes.get(offset..offset + 12).ok_or_else(invalid)?;
        let next = u32::from_le_bytes(header[0..4].try_into().map_err(|_| invalid())?) as usize;
        let length = u32::from_le_bytes(header[8..12].try_into().map_err(|_| invalid())?) as usize;
        if length == 0 || length % 2 != 0 { return Err(invalid()); }
        let end = offset.checked_add(12).and_then(|value| value.checked_add(length)).ok_or_else(invalid)?;
        let data = bytes.get(offset + 12..end).ok_or_else(invalid)?;
        let units = data.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect::<Vec<_>>();
        names.push(String::from_utf16(&units).map_err(|_| invalid())?);
        if next == 0 { break; }
        if next % 4 != 0 || next < 12 + length { return Err(invalid()); }
        offset = offset.checked_add(next).filter(|value| *value < bytes.len()).ok_or_else(invalid)?;
    }
    Ok(names)
}
