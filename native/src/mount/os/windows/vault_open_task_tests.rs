//! Attribute/option admission using real Dokany requests and production callbacks.
//! Win32 may discard ignored FileAttributes before the adapter sees them, so this
//! test-only wrapper supplies them at that exact boundary without fake pointers.
use std::{ffi::c_void, fs::{self, File, OpenOptions}, io,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::{FromRawHandle, OwnedHandle}},
    path::Path, sync::{OnceLock, atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering}}};
use super::super::super::dokany_abi::{CreateFileCallback, DokanFileInfo,
    DokanIoSecurityContext, DokanOperations, NtStatus};
use windows_sys::Win32::{Foundation::{HANDLE, INVALID_HANDLE_VALUE,
        ERROR_DIRECTORY, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED},
    Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE},
    System::LibraryLoader::{GetModuleHandleW, GetProcAddress}};

static ORIGINAL: OnceLock<CreateFileCallback> = OnceLock::new();
static INJECT: AtomicU64 = AtomicU64::new(0);
static DELIVERED: AtomicUsize = AtomicUsize::new(0);
static OBSERVED_DISPOSITIONS: AtomicU32 = AtomicU32::new(0);
const OPEN_BY_ID: u32 = 0x0000_2000;
const FILE_OVERWRITE: u32 = 4;
const STATUS_NOT_SUPPORTED: NtStatus = 0xc000_00bbu32 as i32;
const DIRECTORY_ATTRIBUTE: u32 = 0x10;
const HIDDEN_ATTRIBUTE: u32 = 0x2;
const TARGETS: [&str; 4] = ["\\vault\\open-attrs.txt", "\\vault\\open-attrs-dir",
    "\\vault\\attrs-new.txt", "\\vault\\attrs-new-dir"];

pub(super) fn install(operations: &mut DokanOperations) {
    let original = operations.create_file.expect("production create callback");
    if let Err(other) = ORIGINAL.set(original) {
        assert_eq!(*ORIGINAL.get().expect("installed callback") as usize, other as usize,
            "serialized runtime cases changed their underlying create callback");
    }
    INJECT.store(0, Ordering::Release);
    operations.create_file = Some(create);
}

unsafe extern "system" fn create(name: *const u16, security: *mut DokanIoSecurityContext,
    access: u32, attributes: u32, share: u32, disposition: u32, options: u32,
    info: *mut DokanFileInfo) -> NtStatus {
    let Some(original) = ORIGINAL.get() else { return 0xc000_000du32 as i32; };
    let injection = INJECT.load(Ordering::Acquire);
    // Real open-by-ID inputs can be binary. Never inspect those as paths here.
    if injection != 0 && options & OPEN_BY_ID == 0
        && TARGETS.iter().any(|target| unsafe { super::errors::path_matches(name, target) })
    {
        DELIVERED.fetch_add(1, Ordering::Release);
        OBSERVED_DISPOSITIONS.fetch_or(1u32.checked_shl(disposition).unwrap_or(1 << 31), Ordering::Release);
        let extra_options = (injection >> 32) as u32;
        // A null name is safe even if a regression tries path decoding: the
        // expected NOT_SUPPORTED rather than INVALID_PARAMETER proves ordering.
        // No out-of-bounds synthetic binary-name reads can occur in this fixture.
        let injected_name = if extra_options & OPEN_BY_ID != 0 { std::ptr::null() } else { name };
        return unsafe { original(injected_name, security, access, injection as u32,
            share, disposition, options | extra_options, info) };
    }
    unsafe { original(name, security, access, attributes, share, disposition, options, info) }
}

struct Injection;
impl Drop for Injection { fn drop(&mut self) { INJECT.store(0, Ordering::Release); } }

fn injected<T>(attributes: u32, options: u32, work: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    let before = DELIVERED.load(Ordering::Acquire);
    let reset = Injection;
    OBSERVED_DISPOSITIONS.store(0, Ordering::Release);
    INJECT.store(u64::from(attributes) | (u64::from(options) << 32), Ordering::Release);
    let result = work();
    drop(reset);
    assert!(DELIVERED.load(Ordering::Acquire) > before, "native open missed the production callback boundary");
    result
}

fn metadata_open(path: &Path) -> io::Result<File> {
    OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(path)
}

fn error_code<T: std::fmt::Debug>(result: io::Result<T>, expected: u32) {
    let error = result.expect_err("unsupported creation/options unexpectedly succeeded");
    assert_eq!(error.raw_os_error(), Some(expected as i32), "wrong admission error: {error}");
}

// Exact user-mode ABI/layouts from Microsoft's NtCreateFile, OBJECT_ATTRIBUTES,
// UNICODE_STRING and IO_STATUS_BLOCK documentation. NtCreateFile permits an
// absolute ObjectName such as \??\Z:\file with a null RootDirectory.
#[repr(C)]
union Status { status: NtStatus, pointer: *mut c_void }
#[repr(C)]
struct IoStatus { status: Status, information: usize }
#[repr(C)]
struct UnicodeString { length: u16, maximum_length: u16, buffer: *mut u16 }
#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root_directory: HANDLE,
    object_name: *mut UnicodeString,
    attributes: u32,
    security_descriptor: *mut c_void,
    security_quality_of_service: *mut c_void,
}
type NtCreate = unsafe extern "system" fn(*mut HANDLE, u32, *mut ObjectAttributes,
    *mut IoStatus, *mut i64, u32, u32, u32, u32, *mut c_void, u32) -> NtStatus;

const _: () = {
    if std::mem::size_of::<usize>() == 8 {
        assert!(std::mem::size_of::<ObjectAttributes>() == 48);
        assert!(std::mem::size_of::<UnicodeString>() == 16);
        assert!(std::mem::size_of::<IoStatus>() == 16);
    }
};

#[derive(Debug)]
struct NativeOutcome { status: NtStatus, io_status: NtStatus, information: usize }

fn native_overwrite(path: &Path) -> io::Result<NativeOutcome> {
    // Compiler 48a229ceaefd4985c50990b14116b6d856af0985 std/sys/fs/windows.rs
    // sends truncate-without-create through CreateFileW(TRUNCATE_EXISTING).
    // That Win32 operation does not guarantee a FILE_OVERWRITE callback. Send
    // the native disposition explicitly; do not synthesize DokanFileInfo.
    let library: Vec<u16> = "ntdll.dll".encode_utf16().chain(Some(0)).collect();
    let module = unsafe { GetModuleHandleW(library.as_ptr()) };
    if module.is_null() { return Err(io::Error::last_os_error()); }
    // Borrowed module: ntdll stays loaded; no FreeLibrary ownership is acquired.
    let address = unsafe { GetProcAddress(module, b"NtCreateFile\0".as_ptr()) }
        .ok_or_else(io::Error::last_os_error)?;
    let create: NtCreate = unsafe { std::mem::transmute(address) };
    if !path.is_absolute() { return Err(io::Error::other("native overwrite requires the discovered absolute drive path")); }
    let mut units: Vec<u16> = "\\??\\".encode_utf16().chain(path.as_os_str().encode_wide())
        .chain(Some(0)).collect();
    let bytes = units.len().checked_mul(2).and_then(|length| u16::try_from(length).ok())
        .ok_or_else(|| io::Error::other("native overwrite path exceeds UNICODE_STRING"))?;
    let mut name = UnicodeString { length: bytes - 2, maximum_length: bytes, buffer: units.as_mut_ptr() };
    let mut attributes = ObjectAttributes {
        length: std::mem::size_of::<ObjectAttributes>() as u32,
        root_directory: std::ptr::null_mut(), object_name: &mut name,
        attributes: 0x40, // OBJ_CASE_INSENSITIVE; not inheritable or kernel-only
        security_descriptor: std::ptr::null_mut(), security_quality_of_service: std::ptr::null_mut(),
    };
    let mut output = IoStatus { status: Status { pointer: std::ptr::null_mut() }, information: 0 };
    let mut handle: HANDLE = std::ptr::null_mut();
    // FILE_SYNCHRONOUS_IO_NONALERT requires SYNCHRONIZE. All stack/UTF-16
    // storage remains owned until the synchronous create has completed. The
    // enclosing existing native-phase/volume deadlines cover a wedged driver.
    let status = unsafe { create(&mut handle, 0x2 | 0x0010_0000, &mut attributes,
        &mut output, std::ptr::null_mut(), HIDDEN_ATTRIBUTE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, FILE_OVERWRITE,
        0x20 | 0x40, std::ptr::null_mut(), 0) };
    if status == 0x103 {
        // Never unwind/free request storage while completion could be pending.
        eprintln!("[mount vault] synchronous NtCreateFile unexpectedly returned STATUS_PENDING");
        std::process::abort();
    }
    if status >= 0 {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::other(format!("NtCreateFile succeeded without a valid handle: 0x{:08x}", status as u32)));
        }
        // Even unexpected success must close its real handle before assertions.
        drop(unsafe { OwnedHandle::from_raw_handle(handle as _) });
    }
    Ok(NativeOutcome { status, io_status: unsafe { output.status.status }, information: output.information })
}

pub(super) fn exercise(root: &Path) -> io::Result<()> {
    let file = root.join("vault").join("open-attrs.txt");
    let directory = root.join("vault").join("open-attrs-dir");
    let new_file = root.join("vault").join("attrs-new.txt");
    let new_directory = root.join("vault").join("attrs-new-dir");
    // READONLY, HIDDEN, SYSTEM, TEMPORARY and DIRECTORY cannot change an
    // existing object's type or persisted attributes on OPEN / existing OPEN_IF.
    for attributes in [1, HIDDEN_ATTRIBUTE, 4, 0x100, DIRECTORY_ATTRIBUTE] {
        let opened = injected(attributes, 0, || metadata_open(&file))?;
        assert!(opened.metadata()?.is_file());
        assert_eq!(opened.metadata()?.len(), 4);
        drop(opened);
        let opened = injected(attributes, 0, || metadata_open(&directory))?;
        assert!(opened.metadata()?.is_dir());
        drop(opened);
        let opened = injected(attributes, 0, || OpenOptions::new().read(true).write(true)
            .create(true).open(&file))?;
        assert_eq!(opened.metadata()?.len(), 4, "existing OPEN_IF overwrote content");
        drop(opened);
    }
    assert!(!fs::metadata(&file)?.permissions().readonly(), "ignored attributes became persisted state");
    error_code(injected(HIDDEN_ATTRIBUTE, 0, || OpenOptions::new().write(true)
        .create_new(true).open(&new_file)), ERROR_NOT_SUPPORTED);
    error_code(injected(HIDDEN_ATTRIBUTE, 0, || fs::create_dir(&new_directory)), ERROR_NOT_SUPPORTED);
    let overwrite = injected(HIDDEN_ATTRIBUTE, 0, || native_overwrite(&file))?;
    let observed = OBSERVED_DISPOSITIONS.load(Ordering::Acquire);
    eprintln!("[mount vault] native overwrite requested={FILE_OVERWRITE} callback_disposition_bits=0x{observed:08x} status=0x{:08x} io_status=0x{:08x} information={}",
        overwrite.status as u32, overwrite.io_status as u32, overwrite.information);
    assert_eq!(observed, 1 << FILE_OVERWRITE,
        "native overwrite did not deliver the intended callback disposition: {overwrite:?}");
    assert_eq!(overwrite.status, STATUS_NOT_SUPPORTED,
        "native FILE_OVERWRITE accepted unsupported creation attributes: {overwrite:?}");
    error_code(injected(DIRECTORY_ATTRIBUTE, 0, || OpenOptions::new().write(true)
        .create_new(true).open(&new_file)), ERROR_DIRECTORY);
    assert_eq!(injected(HIDDEN_ATTRIBUTE, 0, || metadata_open(&new_file))
        .expect_err("missing OPEN unexpectedly succeeded").kind(), io::ErrorKind::NotFound);
    assert_eq!(injected(HIDDEN_ATTRIBUTE, 0, || OpenOptions::new().write(true)
        .create_new(true).open(&file)).expect_err("existing CREATE unexpectedly succeeded").kind(),
        io::ErrorKind::AlreadyExists);
    error_code(injected(0, OPEN_BY_ID, || metadata_open(&file)), ERROR_NOT_SUPPORTED);
    error_code(injected(0, 0x1 | 0x40, || metadata_open(&file)), ERROR_INVALID_PARAMETER);
    assert_eq!(fs::metadata(&file)?.len(), 4, "rejected overwrite changed file size");
    for path in [new_file, new_directory] {
        assert_eq!(fs::metadata(path).expect_err("rejected create changed namespace").kind(), io::ErrorKind::NotFound);
    }
    eprintln!("[mount vault] ignored existing attributes/create rejection/open-by-ID ordering=covered");
    Ok(())
}
