//! Windows Remote Desktop Connection (`mstsc`) as the one client (design
//! 2.4). se stores the host's authenticated SHA-1 thumbprint as `CertHash`
//! for the device address (`trust.rs`), so mstsc runs without `/prompt` and
//! `/public` and saved Windows credentials apply. Since P4 (plan 2026-11-17)
//! the whole desktop starts as `mstsc /v:<address>` without a `.rdp` file:
//! mstsc shows its security warning with redirection checkboxes for every
//! opened unsigned file since April 2026 (CVE-2026-26151), not for a start
//! by name. A RemoteApp keeps its profile, which names the host's account.
//! `UsernameHint` makes mstsc propose the host's account, not this one's.
use crate::rdp::core::{
    mstsc_start, CertificatePolicy, ClientCapability, ClientKind, HostAccount, HostService,
    MstscStart, StartRequest,
};
use crate::rdp::os::shared::clients::{ClientProcess, Credentials};
use std::{
    net::SocketAddr,
    path::PathBuf,
    process::{Command, Stdio},
};

#[path = "trust.rs"]
pub mod trust;
pub use trust::prepare_trust;

fn executable() -> Option<PathBuf> {
    system_windows_directory()
        .map(|path| path.join("mstsc.exe"))
        .filter(|path| path.is_file())
}
pub fn inventory() -> Vec<ClientCapability> {
    vec![ClientCapability { id: "mstsc".into(), kind: ClientKind::Mstsc, version: None,
        certificate_policy: CertificatePolicy::NativeDialog, remote_app: true,
        available: executable().is_some(), detail: "Windows Remote Desktop Connection: se stores the authenticated host certificate thumbprint for the device address, so no certificate question appears; a changed host certificate shows the native warning. The desktop starts by address without a .rdp file, so no redirection warning appears. mstsc proposes the device's account; se hands over the device's sign-in when the device issues one, otherwise the password is entered once in the Windows dialog. Desktop and published Server/RDS RemoteApp aliases are supported.".into() }]
}
/// mstsc asks for account credentials in its own dialog.
pub fn needs_credential_input(_: &str) -> bool {
    false
}
pub fn launch(
    request: &StartRequest,
    _: &HostService,
    listener: SocketAddr,
    credentials: Option<&Credentials>,
    account: Option<&HostAccount>,
) -> Result<ClientProcess, String> {
    let executable = executable().ok_or_else(|| "System mstsc.exe is unavailable".to_string())?;
    // P4: mstsc proposes the device's account instead of this computer's. A
    // failed write only loses the proposal; the session credential and
    // `CertHash` still apply.
    let hint = account
        .map(HostAccount::qualified)
        .or_else(|| credentials.map(qualified));
    if let Some(hint) = &hint {
        let _ = trust::write_username_hint(listener, hint);
    }
    let (argument, resources): (std::ffi::OsString, Box<dyn Send>) =
        match mstsc_start(listener, request.remote_app.as_deref()) {
            MstscStart::Address(argument) => (argument.into(), Box::new(()) as Box<dyn Send>),
            MstscStart::Profile => {
                let alias = request.remote_app.as_deref();
                let profile = super::profile::Profile::create(listener, alias, account)?;
                (
                    profile.path.clone().into_os_string(),
                    Box::new(profile) as Box<dyn Send>,
                )
            }
        };
    // Without /prompt and /public: the stored `CertHash` and saved
    // credentials of this device address apply (design 2.7.2).
    let child = Command::new(executable)
        .arg(argument)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Cannot start Windows Remote Desktop Connection: {e}"))?;
    Ok(ClientProcess { child, resources })
}

/// `DOMAIN\user` of handed-over credentials.
fn qualified(credentials: &Credentials) -> String {
    match &credentials.domain {
        Some(domain) => format!("{domain}\\{}", credentials.username),
        None => credentials.username.clone(),
    }
}
pub fn system_windows_directory() -> Option<PathBuf> {
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
pub fn watch_process_exit(
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

pub fn hide_probe_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000);
}
