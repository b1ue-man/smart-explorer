//! Own-process SCM service. It never enables or changes the user's daemons.
use std::{io, ptr::{null, null_mut}, sync::atomic::{AtomicBool, AtomicUsize, Ordering}, time::Duration};
use windows_sys::{core::PWSTR, Win32::{Foundation::{ERROR_CALL_NOT_IMPLEMENTED, NO_ERROR}, System::{Services::*, SystemInformation::GetTickCount64}}};
use crate::login::os::shared::{broker, installation, nonce};
use super::{pipe, security::{self, wide}, SERVICE, PIPE};
static STOP: AtomicBool = AtomicBool::new(false);
static STATUS: AtomicUsize = AtomicUsize::new(0);
pub(crate) fn run_broker() -> io::Result<()> {
    security::require_system()?;
    let mut name = wide(SERVICE);
    let table = [SERVICE_TABLE_ENTRYW { lpServiceName: name.as_mut_ptr(), lpServiceProc: Some(main) }, SERVICE_TABLE_ENTRYW { lpServiceName: null_mut(), lpServiceProc: None }];
    if unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } == 0 { return Err(io::Error::last_os_error()); } Ok(())
}
unsafe extern "system" fn main(_: u32, _: *mut PWSTR) {
    let status = RegisterServiceCtrlHandlerExW(wide(SERVICE).as_ptr(), Some(control), null());
    if status.is_null() { return; } STATUS.store(status as usize, Ordering::SeqCst);
    report(SERVICE_START_PENDING, 0);
    let result = serve();
    report(SERVICE_STOPPED, if result.is_ok() { 0 } else { 1 });
}
unsafe extern "system" fn control(code: u32, _: u32, _: *mut core::ffi::c_void, _: *mut core::ffi::c_void) -> u32 {
    match code { SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => { STOP.store(true, Ordering::SeqCst); if let Some(stop) = GATEWAY_STOP.get() { stop.store(true, Ordering::SeqCst); } report(SERVICE_STOP_PENDING, 0); NO_ERROR }, SERVICE_CONTROL_INTERROGATE => NO_ERROR, _ => ERROR_CALL_NOT_IMPLEMENTED }
}
fn report(state: u32, exit: u32) {
    let handle = STATUS.load(Ordering::SeqCst) as SERVICE_STATUS_HANDLE; if handle.is_null() { return; }
    let status = SERVICE_STATUS { dwServiceType: SERVICE_WIN32_OWN_PROCESS, dwCurrentState: state,
        dwControlsAccepted: if state == SERVICE_RUNNING { SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN } else { 0 },
        dwWin32ExitCode: exit, dwServiceSpecificExitCode: 0, dwCheckPoint: 0, dwWaitHint: 15000 };
    unsafe { SetServiceStatus(handle, &status); }
}
fn serve() -> io::Result<()> {
    let install = installation::load()?;
    super::session::verify_system_process(std::process::id())?;
    // Broker restart and OS boot both invalidate every old signed request.
    let boot = format!("{}:{}", unsafe { GetTickCount64() }, nonce()?);
    let mut pipe = pipe::create(PIPE, Some(&install.policy.gateway_identity))?;
    report(SERVICE_RUNNING, 0);
    while !STOP.load(Ordering::SeqCst) {
        super::super::shared::host_status::publish("idle", &boot, &installation::policy()?)?;
        match pipe::accept(&pipe, Duration::from_secs(1), || STOP.load(Ordering::SeqCst)) {
            Ok(()) => {
                let policy = installation::policy()?;
                // Named-pipe impersonation is defined only after a client read.
                // Read one bounded byte, verify its writer token, then replay it.
                let mut first = [0u8; 1];
                if std::io::Read::read_exact(&mut pipe, &mut first).is_ok()
                    && pipe::authorize_gateway(&pipe, &policy.gateway_identity).is_ok() {
                    let mut framed = pipe::Prefixed::new(&mut pipe, first[0]);
                    if let Err(error) = broker::serve(&mut framed, &boot) { let _ = crate::login::core::wire::send(&mut framed, &crate::login::core::wire::Reply::refused(error)); }
                }
                unsafe { windows_sys::Win32::System::Pipes::DisconnectNamedPipe(pipe.raw()); }
            }
            Err(error) if matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {}
            Err(error) => return Err(error),
        }
    }
    super::super::shared::host_status::publish("stopped", &boot, &installation::policy()?)?;
    Ok(())
}

static GATEWAY_STOP: std::sync::OnceLock<std::sync::Arc<AtomicBool>> = std::sync::OnceLock::new();
pub(crate) fn run_gateway_service() -> io::Result<()> {
    let policy = installation::policy()?;
    super::pipe::require_gateway(&policy)?;
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    GATEWAY_STOP.set(stop).map_err(|_| io::Error::other("gateway lifecycle already initialized"))?;
    let mut name = wide(super::GATEWAY);
    let table = [SERVICE_TABLE_ENTRYW { lpServiceName: name.as_mut_ptr(), lpServiceProc: Some(gateway_main) }, SERVICE_TABLE_ENTRYW { lpServiceName: null_mut(), lpServiceProc: None }];
    if unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } == 0 { return Err(io::Error::last_os_error()); } Ok(())
}
unsafe extern "system" fn gateway_main(_: u32, _: *mut PWSTR) {
    let status = RegisterServiceCtrlHandlerExW(wide(super::GATEWAY).as_ptr(), Some(control), null());
    if status.is_null() { return; } STATUS.store(status as usize, Ordering::SeqCst);
    report(SERVICE_START_PENDING, 0);
    let result = match GATEWAY_STOP.get() {
        Some(stop) => crate::login::os::shared::gateway::run_until(stop.clone(), || report(SERVICE_RUNNING, 0)),
        None => Err(io::Error::other("gateway lifecycle was not initialized")),
    };
    report(SERVICE_STOPPED, if result.is_ok() { 0 } else { 1 });
}
