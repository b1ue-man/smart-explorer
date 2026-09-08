//! Attribute/option admission using real Dokany requests and production callbacks.
//! Win32 may discard ignored FileAttributes before the adapter sees them, so this
//! test-only wrapper supplies them at that exact boundary without fake pointers.
use std::{fs::{self, File, OpenOptions}, io, os::windows::fs::OpenOptionsExt,
    path::Path, sync::{OnceLock, atomic::{AtomicU64, AtomicUsize, Ordering}}};
use super::super::super::dokany_abi::{CreateFileCallback, DokanFileInfo,
    DokanIoSecurityContext, DokanOperations, NtStatus};
use windows_sys::Win32::{Foundation::{ERROR_DIRECTORY, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED},
    Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE}};

static ORIGINAL: OnceLock<CreateFileCallback> = OnceLock::new();
static INJECT: AtomicU64 = AtomicU64::new(0);
static DELIVERED: AtomicUsize = AtomicUsize::new(0);
const OPEN_BY_ID: u32 = 0x0000_2000;
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
    error_code(injected(HIDDEN_ATTRIBUTE, 0, || OpenOptions::new().write(true)
        .truncate(true).open(&file)), ERROR_NOT_SUPPORTED);
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
