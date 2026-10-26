use crate::rdp::core::{
    CertificatePolicy, ClientCapability, ClientKind, HostService, StartRequest,
};
use crate::rdp::os::shared::clients::ClientProcess;
use std::{
    net::SocketAddr,
    path::PathBuf,
    process::{Command, Stdio},
};
fn executable() -> Option<PathBuf> {
    system_windows_directory()
        .map(|path| path.join("mstsc.exe"))
        .filter(|path| path.is_file())
}
pub(crate) fn inventory() -> Vec<ClientCapability> {
    vec![ClientCapability { id: "mstsc".into(), kind: ClientKind::Mstsc, version: None,
        certificate_policy: CertificatePolicy::NativeDialog, remote_app: true,
        available: executable().is_some(), detail: "Windows Remote Desktop Connection: private read-only profile requires a certificate warning on authentication failure and keeps CredSSP enabled. Compare the authenticated host SHA-256 fingerprint using View certificate in the native dialog. Credentials are entered only in Windows. Desktop and published Server/RDS RemoteApp aliases are supported; exclusive peer pinning is unavailable.".into() }]
}
pub(crate) fn launch(
    request: &StartRequest,
    _: &HostService,
    listener: SocketAddr,
) -> Result<ClientProcess, String> {
    let profile = super::profile::Profile::create(listener, request.remote_app.as_deref())?;
    let child =
        Command::new(executable().ok_or_else(|| "System mstsc.exe is unavailable".to_string())?)
            .arg(&profile.path)
            .args(["/prompt", "/public"])
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

pub(crate) fn hide_probe_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000);
}
