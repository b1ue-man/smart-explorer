//! Protected, hash-bound helper installation; no user-writable elevated script.
use std::io::{self, Read, Seek};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PROGRAM: &str = include_str!("uplink_install.ps1");
// No user PSModulePath/autoload code may run in a protected elevated helper.
const SYSTEM_MODULES: &str = "$env:PSModulePath=$PSHOME+'\\Modules'; $PSModuleAutoLoadingPreference='None'; \
    foreach($module in @('Microsoft.PowerShell.Management','Microsoft.PowerShell.Security','Microsoft.PowerShell.Utility','ScheduledTasks')) \
    { Microsoft.PowerShell.Core\\Import-Module -Name ($PSHOME+'\\Modules\\'+$module+'\\'+$module+'.psd1') -ErrorAction Stop }; ";

pub(crate) fn system_powershell() -> io::Result<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    let length = unsafe { windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(
        buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if length == 0 || length >= buffer.len() { return Err(io::Error::last_os_error()); }
    use std::os::windows::ffi::OsStringExt;
    Ok(PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length]))
        .join("WindowsPowerShell/v1.0/powershell.exe"))
}

pub(crate) fn powershell(program: &str) -> io::Result<std::process::Output> {
    let program = format!("{SYSTEM_MODULES}{program}");
    std::process::Command::new(system_powershell()?).args(["-NoProfile", "-NonInteractive",
        "-ExecutionPolicy", "Bypass", "-Command", &program]).creation_flags(CREATE_NO_WINDOW).output()
}

pub(crate) fn quote(value: &str) -> String { format!("'{}'", value.replace('\'', "''")) }

fn checked_text(path: &Path) -> io::Result<&str> {
    path.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Windows helper path is not Unicode"))
}

struct Context { sid: String, root: String, request_directory: String }

fn context() -> io::Result<Context> {
    let output = powershell("[Security.Principal.WindowsIdentity]::GetCurrent().User.Value; [Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles)")?;
    if !output.status.success() { return Err(io::Error::other("Windows installation context is unavailable")); }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let sid = lines.next().unwrap_or_default().to_owned();
    let program_files = lines.next().unwrap_or_default();
    if !sid.starts_with("S-1-5-") || !sid.bytes().all(|c| c.is_ascii_digit() || c == b'S' || c == b'-')
        || !Path::new(program_files).is_absolute() { return Err(io::Error::other("Invalid Windows installation context")); }
    let root = Path::new(program_files).join("Smart Explorer LAN-Uplink").join(&sid);
    let request = crate::support_dirs::app_data_dir().join("lan_uplink");
    crate::support_dirs::ensure_private_dir(&request)?;
    Ok(Context { sid, root: checked_text(&root)?.to_owned(), request_directory: checked_text(&request)?.to_owned() })
}

fn program(context: &Context, mode: &str, source: &str, hash: &str) -> String {
    let program = PROGRAM.lines().filter(|line| !line.trim_start().starts_with('#'))
        .map(str::trim).collect::<Vec<_>>().join("\n");
    format!("$PrivateGuid='';$PublicGuid='';$Mode={};$Sid={};$Root={};$RequestDirectory={};$Source={};$ExpectedHash={};\n{program}",
        quote(mode), quote(&context.sid), quote(&context.root), quote(&context.request_directory), quote(source), quote(hash))
}

pub(crate) fn task_name() -> io::Result<String> { Ok(format!("Smart Explorer LAN-Uplink {}", context()?.sid)) }

fn executable_snapshot() -> io::Result<(std::fs::File, PathBuf, String)> {
    let exe = std::env::current_exe()?;
    let mut source = std::fs::OpenOptions::new().read(true)
        .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ).open(&exe)?;
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = [0u8; 65536];
    loop { let count = source.read(&mut buffer)?; if count == 0 { break; } digest.update(&buffer[..count]); }
    let hash: String = digest.finish().as_ref().iter().map(|byte| format!("{byte:02x}")).collect();
    source.rewind()?;
    Ok((source, exe, hash))
}

pub(crate) fn ready() -> io::Result<bool> {
    let context = context()?;
    let (_source, exe, hash) = executable_snapshot()?;
    let output = powershell(&program(&context, "probe", checked_text(&exe)?, &hash))?;
    if !output.status.success() { return Err(io::Error::other(format!("Uplink installation requires repair: {}",
        String::from_utf8_lossy(&output.stderr).trim()))); }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "ready")
}

pub(crate) fn elevated(program: &str) -> io::Result<()> {
    use base64::Engine;
    let program = format!("{SYSTEM_MODULES}{program}");
    let bytes: Vec<_> = program.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let engine = system_powershell()?;
    let launch = format!("$p=Start-Process -FilePath {} -Verb RunAs -Wait -WindowStyle Hidden -PassThru -ArgumentList {}; exit $p.ExitCode",
        quote(checked_text(&engine)?), quote(&format!("-NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand {encoded}")));
    if launch.encode_utf16().count() > 30000 { return Err(io::Error::other("Uplink setup command exceeds Windows limit")); }
    let output = powershell(&launch)?;
    if !output.status.success() { return Err(io::Error::new(io::ErrorKind::PermissionDenied,
        "UAC-Freigabe abgelehnt oder Einrichtung fehlgeschlagen; Reparatur erneut versuchen")); }
    Ok(())
}

pub(crate) fn install() -> io::Result<String> {
    let context = context()?;
    // Prevent replacement of the exact source for the whole confirmation/copy.
    let (_source, exe, hash) = executable_snapshot()?;
    elevated(&program(&context, "install", checked_text(&exe)?, &hash))?;
    if !ready()? { return Err(io::Error::other("Installation remained incomplete; repair is available")); }
    Ok("Geschuetzter Uplink-Helfer mit geprueftem SHA-256 eingerichtet".into())
}

pub(crate) fn cleanup() -> io::Result<String> {
    let context = context()?;
    let state = crate::net::UplinkState::load().map_err(io::Error::other)?;
    let mut cleanup = program(&context, "cleanup", "", "");
    if let Some(record) = &state.sharing {
        if !crate::net::valid_adapter_id(&record.private_id) || !crate::net::valid_adapter_id(&record.public_id)
            || !record.private_id.starts_with('{') || !record.public_id.starts_with('{') {
            return Err(io::Error::other("Unsafe recorded uplink adapters"));
        }
        cleanup = cleanup.replacen("$PrivateGuid='';$PublicGuid='';", &format!("$PrivateGuid={};$PublicGuid={};",
            quote(&record.private_id), quote(&record.public_id)), 1);
    }
    let status = powershell(&program(&context, "probe", "", ""))?;
    if state.sharing.is_none() && status.status.success() && String::from_utf8_lossy(&status.stdout).trim() == "missing" {
        return Ok("Keine Uplink-Installation vorhanden".into());
    }
    elevated(&cleanup)?;
    let status = powershell(&program(&context, "probe", "", ""))?;
    if !status.status.success() || String::from_utf8_lossy(&status.stdout).trim() != "missing" {
        return Err(io::Error::other("Uplink-Installation blieb vorhanden; Entfernung erneut versuchen"));
    }
    let mut stopped = state;
    stopped.sharing = None;
    stopped.last_error = None;
    stopped.save().map_err(io::Error::other)?;
    Ok("Uplink-Aufgabe und geschuetzter Helfer entfernt".into())
}

/// The scheduled entrypoint must itself be the protected, manifest-bound copy.
pub(crate) fn validate_running_helper() -> io::Result<()> {
    let context = context()?;
    let expected = Path::new(&context.root).join("se.exe");
    if std::env::current_exe()? != expected || !ready()? {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Untrusted uplink helper entrypoint"));
    }
    Ok(())
}
