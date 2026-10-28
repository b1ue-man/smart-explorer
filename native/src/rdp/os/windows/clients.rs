//! Windows Remote Desktop Connection (`mstsc`) as the one client (design
//! 2.4). se stores the host's authenticated SHA-1 thumbprint as `CertHash`
//! for the device address (`trust.rs`), so the profile runs without
//! `/prompt` and `/public` and saved Windows credentials apply.
use crate::rdp::core::{
    CertificatePolicy, ClientCapability, ClientKind, HostService, StartRequest,
};
use crate::rdp::os::shared::clients::{ClientProcess, Credentials};
use std::{
    net::SocketAddr,
    path::PathBuf,
    process::{Command, Stdio},
};

#[path = "trust.rs"]
pub(crate) mod trust;
pub(crate) use trust::prepare_trust;

fn executable() -> Option<PathBuf> {
    system_windows_directory()
        .map(|path| path.join("mstsc.exe"))
        .filter(|path| path.is_file())
}
pub(crate) fn inventory() -> Vec<ClientCapability> {
    vec![ClientCapability { id: "mstsc".into(), kind: ClientKind::Mstsc, version: None,
        certificate_policy: CertificatePolicy::NativeDialog, remote_app: true,
        available: executable().is_some(), detail: "Windows Remote Desktop Connection: se stores the authenticated host certificate thumbprint for the device address, so no certificate question appears; a changed host certificate shows the native warning. Account credentials are entered once in the Windows dialog and can be saved there. Desktop and published Server/RDS RemoteApp aliases are supported.".into() }]
}
/// mstsc asks for account credentials in its own dialog.
pub(crate) fn needs_credential_input(_: &str) -> bool {
    false
}
pub(crate) fn launch(
    request: &StartRequest,
    _: &HostService,
    listener: SocketAddr,
    _: Option<&Credentials>,
) -> Result<ClientProcess, String> {
    let profile = super::profile::Profile::create(listener, request.remote_app.as_deref())?;
    // Without /prompt and /public: the stored `CertHash` and saved
    // credentials of this device address apply (design 2.7.2).
    let child =
        Command::new(executable().ok_or_else(|| "System mstsc.exe is unavailable".to_string())?)
            .arg(&profile.path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Cannot start Windows Remote Desktop Connection: {e}"))?;
    Ok(ClientProcess {
        child,
        resources: Box::new(profile),
    })
}
pub(crate) fn system_windows_directory() -> Option<PathBuf> {
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    let mut value = vec![0u16; 32768];
    // SAFETY: the buffer is writable for the complete supplied capacity.
    let length = unsafe { GetSystemDirectoryW(value.as_mut_ptr(), value.len() as u32) } as usize;
    if length == 0 || length >= value.len() {
        return None;
    }
    use std::os::windows::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_wide(
        &value[..length],
    )))
}

/// One process handle, closed once.
struct Process(windows_sys::Win32::Foundation::HANDLE);
// SAFETY: a process handle may be waited on and closed from any thread.
unsafe impl Send for Process {}
impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: the handle came from OpenProcess and is closed exactly once.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// Holds a handle of the bound client (design 2.6) and calls `on_exit` once
/// the process ended. The creation time is checked on that handle, so a
/// reused PID is never watched.
pub(crate) fn watch_process_exit(
    pid: u32,
    started: u64,
    on_exit: Box<dyn FnOnce() + Send>,
) -> Result<(), String> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, WaitForSingleObject, INFINITE,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    };
    // SAFETY: plain access rights, no inheritance, a PID.
    let handle = unsafe {
        OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    };
    if handle.is_null() {
        return Err(format!(
            "Prozess {pid} nicht beobachtbar: {}",
            std::io::Error::last_os_error()
        ));
    }
    let process = Process(handle);
    // SAFETY: zeroed FILETIMEs are valid outputs for GetProcessTimes.
    let (mut created, mut exited, mut kernel, mut user): (FILETIME, FILETIME, FILETIME, FILETIME) =
        unsafe { std::mem::zeroed() };
    // SAFETY: owned handle, writable outputs.
    if unsafe { GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user) } == 0
    {
        return Err(format!(
            "Prozess {pid} nicht prüfbar: {}",
            std::io::Error::last_os_error()
        ));
    }
    let creation = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    if creation != started {
        return Err(format!(
            "Prozess {pid} wurde neu gestartet; Bindung abgelehnt"
        ));
    }
    std::thread::Builder::new()
        .name("rdp-client-exit".into())
        .spawn(move || {
            // SAFETY: the owned handle carries SYNCHRONIZE; no time limit.
            unsafe { WaitForSingleObject(process.0, INFINITE) };
            drop(process);
            on_exit();
        })
        .map(|_| ())
        .map_err(|error| format!("Prozessbeobachtung nicht gestartet: {error}"))
}

pub(crate) fn hide_probe_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000);
}
