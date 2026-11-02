//! The Windows account service (PRM-M7, C2): `--exec-account-service`,
//! started on demand by the service control manager as LocalSystem from
//! the protected program copy. It serves the pipe
//! `\\.\pipe\SmartExplorerExecAccounts`: remote clients are refused
//! (`PIPE_REJECT_REMOTE_CLIENTS`), SYSTEM owns it, and only SYSTEM and the
//! users of the configuration may open it, without the right to add an
//! instance. Each connection has a thread of its own
//! (`exec_account_connection.rs`). The service ends after 10 minutes
//! without connection and request, or when the service control manager
//! stops it; the processes of every session end with it (kill-on-close
//! Jobs). It notices a changed configuration within 5 seconds and gives the
//! pipe the new access list. Its log is
//! `%ProgramData%\Smart Explorer\exec-accounts.log`.
use std::ffi::OsString;
use std::io;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use windows_sys::core::PWSTR;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_CALL_NOT_IMPLEMENTED, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
    NO_ERROR,
};
use windows_sys::Win32::Security::Authorization::{SetSecurityInfo, SE_KERNEL_OBJECT};
use windows_sys::Win32::Security::{
    WinLocalSystemSid, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    SE_BACKUP_NAME, SE_RESTORE_NAME,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WRITE_DAC,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Services::{
    RegisterServiceCtrlHandlerExW, SetServiceStatus, StartServiceCtrlDispatcherW,
    SERVICE_ACCEPT_SHUTDOWN, SERVICE_ACCEPT_STOP, SERVICE_CONTROL_INTERROGATE,
    SERVICE_CONTROL_SHUTDOWN, SERVICE_CONTROL_STOP, SERVICE_RUNNING, SERVICE_START_PENDING,
    SERVICE_STATUS, SERVICE_STATUS_HANDLE, SERVICE_STOPPED, SERVICE_STOP_PENDING,
    SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentProcessId, SetEvent, WaitForSingleObject, INFINITE,
};

use super::exec_account_config::{
    config_path, load_config, program_copy, SERVICE_NAME, SERVICE_PIPE,
};
use super::exec_account_connection::serve_connection;
use super::exec_account_files::append_log;
use super::exec_account_pipes::CLIENT_ACCESS;
use super::exec_account_s4u::Lsa;
use super::exec_account_scm::SERVICE_MODE;
use super::exec_account_security::{
    current_user_sid, enable_privileges, wide, SecurityDescriptor, Sid,
};
use super::exec_account_spawn::remember_image;
use super::exec_handles::OwnedHandle;

/// The service ends after this long without connection and request (C2).
const IDLE_END: Duration = Duration::from_secs(10 * 60);
const MONITOR_TICK: Duration = Duration::from_secs(5);
/// Connections at the same time; a further client waits for a free one.
const MAX_INSTANCES: u32 = 16;
const PIPE_BUFFER: u32 = 64 * 1024;

/// The service status handle and the stop event, for the callbacks.
static STATUS: AtomicUsize = AtomicUsize::new(0);
static STOP_EVENT: AtomicUsize = AtomicUsize::new(0);

/// What every thread of the service shares.
pub(super) struct Shared {
    pub(super) lsa: Mutex<Lsa>,
    connections: AtomicUsize,
    activity: Mutex<Instant>,
    stopping: AtomicBool,
    refresh: AtomicBool,
}

impl Shared {
    /// A request or a connection end: the idle time starts again.
    pub(super) fn touch(&self) {
        *lock(&self.activity) = Instant::now();
    }

    fn idle(&self) -> bool {
        self.connections.load(Ordering::SeqCst) == 0 && lock(&self.activity).elapsed() >= IDLE_END
    }

    fn connection_ended(&self) {
        self.connections.fetch_sub(1, Ordering::SeqCst);
        self.touch();
    }
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn run_if_requested(arguments: &[OsString]) -> Option<io::Result<()>> {
    (arguments.len() == 1 && arguments[0] == SERVICE_MODE).then(run)
}

fn run() -> io::Result<()> {
    let event = unsafe { CreateEventW(null(), 1, 0, null()) };
    if event.is_null() {
        return Err(io::Error::last_os_error());
    }
    STOP_EVENT.store(event as usize, Ordering::SeqCst);
    let mut name = wide(SERVICE_NAME);
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: name.as_mut_ptr(),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW {
            lpServiceName: null_mut(),
            lpServiceProc: None,
        },
    ];
    if unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } == 0 {
        let error = io::Error::last_os_error();
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("the account service runs only under the service control manager ({error})"),
        ));
    }
    Ok(())
}

unsafe extern "system" fn service_main(_count: u32, _arguments: *mut PWSTR) {
    let name = wide(SERVICE_NAME);
    let status = RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(control), null());
    if status.is_null() {
        return;
    }
    STATUS.store(status as usize, Ordering::SeqCst);
    report(SERVICE_START_PENDING, 0, 15_000);
    let exit = match serve() {
        Ok(()) => 0,
        Err(error) => {
            append_log(&format!("the account service stopped: {error}"));
            error.raw_os_error().map_or(1, |code| code as u32)
        }
    };
    report(SERVICE_STOPPED, exit, 0);
}

unsafe extern "system" fn control(
    code: u32,
    _event: u32,
    _data: *mut core::ffi::c_void,
    _context: *mut core::ffi::c_void,
) -> u32 {
    match code {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            report(SERVICE_STOP_PENDING, 0, 5_000);
            request_stop();
            NO_ERROR
        }
        SERVICE_CONTROL_INTERROGATE => NO_ERROR,
        _ => ERROR_CALL_NOT_IMPLEMENTED,
    }
}

fn report(state: u32, exit: u32, wait_hint: u32) {
    let handle = STATUS.load(Ordering::SeqCst) as SERVICE_STATUS_HANDLE;
    if handle.is_null() {
        return;
    }
    let accepted = if state == SERVICE_RUNNING {
        SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
    } else {
        0
    };
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: accepted,
        dwWin32ExitCode: exit,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: wait_hint,
    };
    unsafe { SetServiceStatus(handle, &status) };
}

fn request_stop() {
    let event = STOP_EVENT.load(Ordering::SeqCst) as HANDLE;
    if !event.is_null() {
        unsafe { SetEvent(event) };
    }
}

/// Runs until the service control manager or the idle time stops it.
fn serve() -> io::Result<()> {
    check_identity()?;
    enable_privileges(&[SE_BACKUP_NAME, SE_RESTORE_NAME])?;
    remember_image()?;
    let shared = Arc::new(Shared {
        lsa: Mutex::new(Lsa::connect()?),
        connections: AtomicUsize::new(0),
        activity: Mutex::new(Instant::now()),
        stopping: AtomicBool::new(false),
        refresh: AtomicBool::new(false),
    });
    // Someone else holding the name makes this fail: the service stops.
    let first = create_instance(true)?;
    let accept = shared.clone();
    std::thread::Builder::new()
        .name("exec-accounts-accept".into())
        .spawn(move || accept_loop(first, &accept))?;
    let watched = shared.clone();
    std::thread::Builder::new()
        .name("exec-accounts-monitor".into())
        .spawn(move || monitor(&watched))?;
    report(SERVICE_RUNNING, 0, 0);
    append_log("the account service started");
    unsafe { WaitForSingleObject(STOP_EVENT.load(Ordering::SeqCst) as HANDLE, INFINITE) };
    shared.stopping.store(true, Ordering::SeqCst);
    wake_accept();
    append_log("the account service stops; its sessions end with it");
    Ok(())
}

/// LocalSystem from the protected program copy only (C2).
fn check_identity() -> io::Result<()> {
    if current_user_sid()? != Sid::well_known(WinLocalSystemSid)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the account service runs as LocalSystem only",
        ));
    }
    let (running, copy) = (std::env::current_exe()?, program_copy()?);
    let running = running.to_string_lossy().to_lowercase();
    if running != copy.to_string_lossy().to_lowercase() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("the account service runs only from {}", copy.display()),
        ));
    }
    Ok(())
}

/// The access list of the service's pipe: SYSTEM, and `users` (SIDs) with
/// read and write but not the right to add an instance.
pub(super) fn pipe_sddl(users: &[String]) -> String {
    let mut sddl = String::from("O:SYG:SYD:P(A;;GA;;;SY)");
    for sid in users {
        sddl.push_str(&format!("(A;;{CLIENT_ACCESS:#x};;;{sid})"));
    }
    sddl
}

fn pipe_descriptor() -> io::Result<SecurityDescriptor> {
    let users: Vec<String> = load_config()
        .ok()
        .flatten()
        .map(|config| config.user_sids().into_iter().map(str::to_string).collect())
        .unwrap_or_default();
    SecurityDescriptor::from_sddl(&pipe_sddl(&users))
}

fn create_instance(first: bool) -> io::Result<OwnedHandle> {
    let descriptor = pipe_descriptor()?;
    let attributes = descriptor.attributes(false);
    let name = wide(SERVICE_PIPE);
    let mut mode = PIPE_ACCESS_DUPLEX | WRITE_DAC;
    if first {
        mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            MAX_INSTANCES,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            &attributes,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    OwnedHandle::new(handle)
}

/// Gives the pipe the access list of the current configuration.
fn apply_access(instance: &OwnedHandle) -> io::Result<()> {
    let descriptor = pipe_descriptor()?;
    let status = unsafe {
        SetSecurityInfo(
            instance.raw(),
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            descriptor.dacl()?,
            null(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    Ok(())
}

fn accept_loop(mut instance: OwnedHandle, shared: &Arc<Shared>) {
    let own = unsafe { GetCurrentProcessId() };
    loop {
        if shared.stopping.load(Ordering::SeqCst) {
            return;
        }
        shared.refresh.store(false, Ordering::SeqCst);
        if let Err(error) = apply_access(&instance) {
            append_log(&format!("the pipe keeps its access list: {error}"));
        }
        let connected = unsafe { ConnectNamedPipe(instance.raw(), null_mut()) } != 0
            || io::Error::last_os_error().raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32);
        let woken = client_process(instance.raw()) == Some(own);
        if connected && !woken && !shared.stopping.load(Ordering::SeqCst) {
            shared.connections.fetch_add(1, Ordering::SeqCst);
            shared.touch();
            let worker = shared.clone();
            let spawned = std::thread::Builder::new()
                .name("exec-accounts-connection".into())
                .spawn(move || {
                    serve_connection(instance, &worker);
                    worker.connection_ended();
                });
            if spawned.is_err() {
                shared.connection_ended();
            }
        } else {
            unsafe { DisconnectNamedPipe(instance.raw()) };
            drop(instance);
        }
        instance = loop {
            match create_instance(false) {
                Ok(next) => break next,
                Err(_) if shared.stopping.load(Ordering::SeqCst) => return,
                // Every instance is busy: wait until a connection ends.
                Err(_) => std::thread::sleep(Duration::from_secs(1)),
            }
        };
    }
}

fn client_process(pipe: HANDLE) -> Option<u32> {
    let mut process = 0u32;
    (unsafe { GetNamedPipeClientProcessId(pipe, &mut process) } != 0 && process != 0)
        .then_some(process)
}

/// Connects to the service's own pipe, so that the accept thread wakes up
/// and applies a new access list or sees the end of the service.
fn wake_accept() {
    let name = wide(SERVICE_PIPE);
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            CLIENT_ACCESS,
            0,
            null(),
            OPEN_EXISTING,
            SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            null_mut(),
        )
    };
    if handle != INVALID_HANDLE_VALUE {
        unsafe { CloseHandle(handle) };
    }
}

fn config_stamp() -> Option<(u64, SystemTime)> {
    let metadata = std::fs::metadata(config_path().ok()?).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

/// Watches the configuration and the idle time.
fn monitor(shared: &Arc<Shared>) {
    let mut stamp = config_stamp();
    loop {
        std::thread::sleep(MONITOR_TICK);
        if shared.stopping.load(Ordering::SeqCst) {
            return;
        }
        let now = config_stamp();
        if now != stamp {
            stamp = now;
            shared.refresh.store(true, Ordering::SeqCst);
        }
        if shared.refresh.load(Ordering::SeqCst) {
            wake_accept();
        }
        if shared.idle() {
            append_log("no connection for 10 minutes; the account service ends");
            request_stop();
            return;
        }
    }
}
