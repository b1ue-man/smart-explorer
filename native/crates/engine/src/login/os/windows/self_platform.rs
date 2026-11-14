//! Windows adapter of the login self-setup (E21): the broker's grant pipe
//! (the client token names the account), LocalSystem replacement of the
//! policy, ledger and network records, UAC, and the embedded installer.
use super::{
    pipe,
    security::{self, wide, Descriptor, Handle},
};
use crate::login::{
    core::{gateway_types, self_grants::ConsoleAbilities},
    os::shared::{
        denied, installation, nonce,
        self_channel::{self, Caller, Record},
        self_install::{Elevation, ScriptCall},
    },
};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::FromRawHandle,
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    ptr::null_mut,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, GENERIC_WRITE},
    Storage::FileSystem::{
        CreateFileW, MoveFileExW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, MOVEFILE_REPLACE_EXISTING,
        MOVEFILE_WRITE_THROUGH,
    },
};

/// The app binary that handles the login service switches (assumption A2).
pub const APP_FILE: &str = "smart_explorer.exe";
pub const SCRIPT_NAME: &str = "install-login-service.ps1";
/// The checked-in installer, fixed in this release's binary.
pub const SCRIPT: &str = include_str!("../../../../../../install-login-service.ps1");
pub const STAGE_IMAGE: &str = "smart-explorer-login-image.exe";
/// The grant pipe of brokers before E45; a worker tries it only when the
/// broker's status names no pipe of its own.
const LEGACY_GRANT_PIPE: &str = r"\\.\pipe\SmartExplorerLoginGrants";
/// Local accounts only: network logons are denied, LocalSystem owns it.
const GRANT_SDDL: &str = "O:SYG:SYD:P(D;;GA;;;NU)(A;;GA;;;SY)(A;;0x0012019f;;;AU)";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// `ERROR_CANCELLED`: the person declined the UAC prompt.
const DECLINED: i32 = 1223;

/// Elevated administrator or LocalSystem.
pub fn privileged() -> bool {
    security::require_administrator().is_ok()
}

fn session_id() -> Option<u32> {
    let mut session = 0u32;
    // SAFETY: the out pointer refers to a live local.
    let ok = unsafe {
        windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId(
            std::process::id(),
            &mut session,
        )
    };
    (ok != 0).then_some(session)
}

/// An elevated process runs the step itself; a user process asks UAC for
/// Smart Explorer's own `se.exe` next to this program (threat T-R6).
pub fn elevation() -> Result<Elevation, String> {
    if privileged() {
        return Ok(Elevation::InProcess);
    }
    // A service without a desktop cannot show the UAC prompt.
    if session_id().unwrap_or(0) == 0 {
        return Err("no_interactive_session".into());
    }
    let se = std::env::current_exe()
        .map_err(|error| format!("se_missing: {error}"))?
        .with_file_name("se.exe");
    if !se.is_file() {
        return Err("se_missing".into());
    }
    Ok(Elevation::External(se))
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn powershell() -> Result<PathBuf, String> {
    crate::net::system_powershell().map_err(|error| format!("powershell: {error}"))
}

/// `Start-Process -Verb RunAs` with every step of the one bundled call
/// (E44; fixed step names, `elevated::valid_step_name`); a declined prompt
/// becomes 1223. Only the system Management module loads. No time limit
/// (RDP rule r7).
pub fn run_elevated(se: &Path, steps: &[String]) -> Result<i32, String> {
    let se = se.to_str().ok_or("se path is not Unicode")?;
    let arguments = crate::rdp::host_setup::elevated::system_args(steps).join(" ");
    let launch = format!(
        "$env:PSModulePath=$PSHOME+'\\Modules'; $PSModuleAutoLoadingPreference='None'; \
         Microsoft.PowerShell.Core\\Import-Module -Name ($PSHOME+'\\Modules\\Microsoft.PowerShell.Management\\Microsoft.PowerShell.Management.psd1') -ErrorAction Stop; \
         try {{ $p = Start-Process -FilePath {} -Verb RunAs -Wait -WindowStyle Hidden -PassThru -ArgumentList {}; exit $p.ExitCode }} \
         catch {{ $code = 1224; $e = $_.Exception; while ($e) {{ if ($e -is [System.ComponentModel.Win32Exception] -and $e.NativeErrorCode -eq {DECLINED}) {{ $code = {DECLINED} }}; $e = $e.InnerException }}; exit $code }}",
        quote(se),
        quote(&arguments)
    );
    let status = Command::new(powershell()?)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            launch.as_str(),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .status()
        .map_err(|error| format!("powershell start: {error}"))?;
    status
        .code()
        .ok_or_else(|| "setup process ended without exit code".into())
}

/// `%ProgramData%`, the parent of the protected login configuration.
pub fn stage_root() -> io::Result<PathBuf> {
    super::paths()?
        .config
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| denied("ProgramData is unavailable"))
}

/// Writers are denied while the image is hashed and copied.
pub fn open_image(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

/// A new file inside the stage; it inherits the stage's SYSTEM/Administrators DACL.
pub fn create_stage_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
}

pub fn run_install_script(stage: &Path, call: &ScriptCall<'_>) -> io::Result<i32> {
    let mut command = Command::new(powershell().map_err(io::Error::other)?);
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(stage.join(SCRIPT_NAME));
    match call {
        ScriptCall::SelfInstall {
            image,
            sha256,
            version,
            policy,
            key,
            network,
        } => {
            command
                .args(["-Action", "SelfInstall", "-Image"])
                .arg(image)
                .args([
                    "-ExpectedSha256",
                    *sha256,
                    "-Version",
                    *version,
                    "-PolicyFile",
                ])
                .arg(policy)
                .arg("-MachineIdentityFile")
                .arg(key)
                .arg("-NetworkFile")
                .arg(network);
        }
        ScriptCall::Uninstall => {
            command.args(["-Action", "Uninstall"]);
        }
    }
    let status = command
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .status()?;
    Ok(status.code().unwrap_or(-1))
}

fn record_path(record: Record) -> io::Result<PathBuf> {
    let config = super::paths()?.config;
    let state = super::gateway_state_directory()?;
    Ok(match record {
        Record::Policy => config.join("policy.json"),
        Record::Ledger => config.join(crate::login::core::self_grants::LEDGER_FILE),
        Record::Network => state.join(gateway_types::NETWORK_FILE),
        Record::MachineKey => state.join(gateway_types::MACHINE_FILE),
        Record::Address => state.join(gateway_types::ADDRESS_FILE),
    })
}

/// An elevated process reads a protected record; a missing one is `None`.
/// The gateway's own address file is public data with the gateway as owner.
pub fn read_self_record(record: Record, max: usize) -> io::Result<Option<Vec<u8>>> {
    if !privileged() {
        return Err(denied("login records are read only by administrators"));
    }
    let file = match open_image(&record_path(record)?) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || meta.len() > max as u64
    {
        return Err(denied("login record has an unsafe kind or size"));
    }
    if record != Record::Address {
        super::protected::validate_public_status(&file)?;
    }
    let mut bytes = Vec::new();
    file.take(max as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err(denied("login record exceeds its limit"));
    }
    Ok(Some(bytes))
}

/// Atomic replacement with the DACL the installer gives the record:
/// SYSTEM and Administrators full, the gateway service SID read-only.
pub fn write_self_record(record: Record, bytes: &[u8]) -> io::Result<()> {
    if !privileged() {
        return Err(denied("login records are replaced only by administrators"));
    }
    let gateway = match record {
        Record::Policy | Record::Network => {
            let sid = installation::policy()?.gateway_identity;
            Some(sid)
        }
        Record::Ledger => None,
        Record::MachineKey | Record::Address => {
            return Err(denied("this login record is never replaced by the broker"))
        }
    };
    let sddl = format!(
        "O:SYG:SYD:P(A;;FA;;;SY)(A;;FA;;;BA){}",
        gateway
            .map(|sid| format!("(A;;FR;;;{sid})"))
            .unwrap_or_default()
    );
    let descriptor = Descriptor::new(&sddl)?;
    let path = record_path(record)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| denied("invalid login record name"))?;
    let temporary = path.with_file_name(format!(".{name}-{}", nonce()?));
    // SAFETY: wide path and descriptor outlive the call; the handle is checked.
    let raw = unsafe {
        CreateFileW(
            wide(&temporary.to_string_lossy()).as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            &descriptor.attributes(),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    let owned = Handle::new(raw)?;
    // SAFETY: ownership of the checked handle moves into the File.
    let mut file = unsafe { File::from_raw_handle(owned.0.cast()) };
    std::mem::forget(owned);
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        // A reader (the gateway every 200 ms, the broker every second) may
        // hold the record open without delete sharing for a moment.
        let mut attempts = 0;
        loop {
            // SAFETY: both wide paths are NUL-terminated and live for the call.
            let moved = unsafe {
                MoveFileExW(
                    wide(&temporary.to_string_lossy()).as_ptr(),
                    wide(&path.to_string_lossy()).as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            };
            if moved != 0 {
                break Ok(());
            }
            let error = io::Error::last_os_error();
            attempts += 1;
            // ERROR_ACCESS_DENIED or ERROR_SHARING_VIOLATION while it is open.
            if attempts >= 20 || !matches!(error.raw_os_error(), Some(5 | 32)) {
                break Err(error);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Worker side (E45): the pipe the broker's SYSTEM-owned status names; the
/// server must still be the LocalSystem broker (`pipe::verify_server`).
pub fn connect_grants() -> io::Result<pipe::ClientStream> {
    let published = crate::login::os::shared::host_status::published_grant_pipe();
    pipe::connect_grants(&grant_pipe_name(published))
}

/// The pipe to connect to: a well-formed published name, otherwise the
/// fixed name of an older broker.
fn grant_pipe_name(published: Option<String>) -> String {
    published
        .filter(|name| crate::login::os::shared::host_status::valid_grant_pipe(name))
        .unwrap_or_else(|| LEGACY_GRANT_PIPE.to_string())
}

/// A new grant pipe under a name with 128 bits of randomness, created as
/// the first instance (`pipe::create_with` sets `FILE_FLAG_FIRST_PIPE_INSTANCE`)
/// and published only once this broker holds it.
fn create_grant_pipe() -> io::Result<pipe::ClientStream> {
    let random = nonce()?;
    let name = format!(
        "{}{}",
        crate::login::os::shared::host_status::GRANT_PIPE_PREFIX,
        &random[..32]
    );
    let created = pipe::create_with(&name, GRANT_SDDL)?;
    crate::login::os::shared::host_status::note_grant_pipe(&name);
    Ok(created)
}

/// Account SIDs of local, domain and Microsoft Entra users; service,
/// system and capability SIDs never set login grants.
pub(super) fn valid_windows_account(sid: &str) -> bool {
    (sid.starts_with("S-1-5-21-") || sid.starts_with("S-1-12-1-"))
        && crate::login::core::self_grants::valid_account(sid)
}

/// Broker side: one client at a time on its own thread, beside the
/// gateway pipe, so a slow client never delays a login operation. Pipe
/// names are global; a fixed one could be held by any local process for
/// good (L15). The broker therefore takes a fresh random name and publishes
/// it in its status (E45); should even that fail, it retries every 2 s with
/// a new name. The worker refuses a pipe LocalSystem does not serve.
pub(super) fn start_grant_pipe(stop: &'static AtomicBool) -> io::Result<()> {
    let first = create_grant_pipe();
    if let Err(error) = &first {
        eprintln!("login grant channel waits for its pipe name: {error}");
    }
    std::thread::Builder::new()
        .name("login-grants".into())
        .spawn(move || {
            let mut pipe = first.ok();
            while !stop.load(Ordering::SeqCst) {
                match pipe.take() {
                    Some(pipe) => return serve_grants(pipe, stop),
                    None => {
                        std::thread::sleep(Duration::from_secs(2));
                        pipe = create_grant_pipe().ok();
                    }
                }
            }
        })?;
    Ok(())
}

fn serve_grants(mut pipe: pipe::ClientStream, stop: &'static AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        match pipe::accept(&pipe, Duration::from_secs(1), || {
            stop.load(Ordering::SeqCst)
        }) {
            Ok(()) => {
                // Impersonation is defined only after a client read.
                let mut first = [0u8; 1];
                if pipe.read_exact(&mut first).is_ok() {
                    match pipe::client_account(&pipe) {
                        Ok((sid, administrator, interactive)) if valid_windows_account(&sid) => {
                            let caller = Caller {
                                account: sid,
                                abilities: ConsoleAbilities {
                                    before_login: true,
                                    locked: true,
                                    uac: administrator,
                                    sas: true,
                                    console: interactive,
                                },
                            };
                            let mut framed = pipe::Prefixed::new(&mut pipe, first[0]);
                            if let Err(error) = self_channel::serve(&mut framed, &caller) {
                                eprintln!("login grant channel: {error}");
                            }
                        }
                        _ => {}
                    }
                }
                unsafe {
                    windows_sys::Win32::System::Pipes::DisconnectNamedPipe(pipe.raw());
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => {
                eprintln!("login grant channel: {error}");
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

#[cfg(test)]
#[path = "self_platform_task_tests.rs"]
mod self_platform_task_tests;

#[cfg(test)]
#[path = "grant_pipe_name_task_tests.rs"]
mod grant_pipe_name_task_tests;
