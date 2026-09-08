//! Bounded diagnostics for the owned PowerShell PID and fixture script paths.
//! Registered only by the cfg(test) fixture; preserves its existing wrappers.
use super::super::super::dokany_abi::{CreateFileCallback, DokanFileInfo,
    DokanIoSecurityContext, DokanOperations, GetFileInformationCallback,
    GetFileSecurityCallback, NtStatus, ReadFileCallback};
use std::{ffi::c_void, io::{self, Write}, panic::AssertUnwindSafe,
    sync::{OnceLock, atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering}}, time::Instant};
use windows_sys::Win32::{Foundation::{GetLastError, SetLastError},
    Storage::FileSystem::BY_HANDLE_FILE_INFORMATION};

struct Originals {
    create: CreateFileCallback,
    information: GetFileInformationCallback,
    read: ReadFileCallback,
    security: GetFileSecurityCallback,
}
static ORIGINAL: OnceLock<Originals> = OnceLock::new();
static ARMED: AtomicBool = AtomicBool::new(false);
static PROCESS: AtomicU32 = AtomicU32::new(0);
static NEXT: AtomicUsize = AtomicUsize::new(0);
const MAX_EVENTS: usize = 128;
const INVALID_PARAMETER: NtStatus = 0xc000_000du32 as i32;

pub(super) fn install(operations: &mut DokanOperations) {
    let originals = Originals {
        create: operations.create_file.expect("fixture create callback"),
        information: operations.get_file_information.expect("fixture metadata callback"),
        read: operations.read_file.expect("fixture read callback"),
        security: operations.get_file_security.expect("fixture security callback"),
    };
    if let Err(other) = ORIGINAL.set(originals) {
        let original = ORIGINAL.get().expect("installed script diagnostic callbacks");
        assert_eq!([original.create as usize, original.information as usize,
            original.read as usize, original.security as usize],
            [other.create as usize, other.information as usize, other.read as usize, other.security as usize]);
    }
    ARMED.store(false, Ordering::Release);
    operations.create_file = Some(create);
    operations.get_file_information = Some(information);
    operations.read_file = Some(read);
    operations.get_file_security = Some(security);
}

pub(super) struct Trace;
pub(super) fn arm() -> Trace {
    NEXT.store(0, Ordering::Relaxed);
    PROCESS.store(0, Ordering::Relaxed);
    ARMED.store(true, Ordering::Release);
    Trace
}
pub(super) fn set_process(pid: u32) { PROCESS.store(pid, Ordering::Release); }
impl Drop for Trace { fn drop(&mut self) { ARMED.store(false, Ordering::Release); } }

struct Event { id: usize, started: Instant }

unsafe fn begin(name: *const u16, info: *mut DokanFileInfo, operation: &'static str,
    detail: impl FnOnce() -> String) -> Option<Event> {
    if !ARMED.load(Ordering::Acquire) || name.is_null() { return None; }
    let previous_error = unsafe { GetLastError() };
    let event = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut units = Vec::new();
        // Dokany owns a NUL-terminated path for these callbacks. Read one unit
        // at a time; never assume the input has a whole fixed buffer behind it.
        for index in 0..512 {
            let unit = unsafe { *name.add(index) };
            if unit == 0 { break; }
            units.push(unit);
        }
        let path = String::from_utf16_lossy(&units);
        let folded = path.to_ascii_lowercase();
        let identity = unsafe { info.as_ref() }.map(|value| (value.process_id, value.context));
        let process = PROCESS.load(Ordering::Acquire);
        let owned_process = process != 0 && identity.is_some_and(|(pid, _)| pid == process);
        if !owned_process && folded != "\\scripts" && !folded.starts_with("\\scripts\\") { return None; }
        let id = NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed,
            |value| (value < MAX_EVENTS).then(|| value + 1)).ok()?;
        let event = Event { id, started: Instant::now() };
        let _ = writeln!(io::stderr(), "[mount script callback] begin={id} op={operation} pid_context={identity:?} path={path:?} {}", detail());
        Some(event)
    })).ok().flatten();
    unsafe { SetLastError(previous_error) };
    event
}

fn finish(event: Option<Event>, status: NtStatus) {
    let Some(event) = event else { return; };
    let previous_error = unsafe { GetLastError() };
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let _ = writeln!(io::stderr(), "[mount script callback] end={} status=0x{:08x} elapsed_us={}",
            event.id, status as u32, event.started.elapsed().as_micros());
    }));
    unsafe { SetLastError(previous_error) };
}

unsafe extern "system" fn create(name: *const u16, security: *mut DokanIoSecurityContext,
    access: u32, attributes: u32, share: u32, disposition: u32, options: u32,
    info: *mut DokanFileInfo) -> NtStatus {
    let Some(original) = ORIGINAL.get() else { return INVALID_PARAMETER; };
    // A FILE_OPEN_BY_FILE_ID name may be binary, not NUL-terminated.
    let event = if options & 0x2000 == 0 {
        unsafe { begin(name, info, "create", || format!("access=0x{access:x} attributes=0x{attributes:x} share=0x{share:x} disposition={disposition} options=0x{options:x}")) }
    } else { None };
    let status = unsafe { (original.create)(name, security, access, attributes, share, disposition, options, info) };
    finish(event, status);
    status
}

unsafe extern "system" fn information(name: *const u16, output: *mut BY_HANDLE_FILE_INFORMATION,
    info: *mut DokanFileInfo) -> NtStatus {
    let Some(original) = ORIGINAL.get() else { return INVALID_PARAMETER; };
    let event = unsafe { begin(name, info, "information", String::new) };
    let status = unsafe { (original.information)(name, output, info) };
    finish(event, status);
    status
}

unsafe extern "system" fn read(name: *const u16, buffer: *mut c_void, length: u32,
    transferred: *mut u32, offset: i64, info: *mut DokanFileInfo) -> NtStatus {
    let Some(original) = ORIGINAL.get() else { return INVALID_PARAMETER; };
    let event = unsafe { begin(name, info, "read", || format!("offset={offset} requested={length}")) };
    let status = unsafe { (original.read)(name, buffer, length, transferred, offset, info) };
    finish(event, status);
    status
}

unsafe extern "system" fn security(name: *const u16, requested: *mut u32, descriptor: *mut c_void,
    length: u32, needed: *mut u32, info: *mut DokanFileInfo) -> NtStatus {
    let Some(original) = ORIGINAL.get() else { return INVALID_PARAMETER; };
    let event = unsafe { begin(name, info, "security", || format!("buffer={length}")) };
    let status = unsafe { (original.security)(name, requested, descriptor, length, needed, info) };
    finish(event, status);
    status
}
