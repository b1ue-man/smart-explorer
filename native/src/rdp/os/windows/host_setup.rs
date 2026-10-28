//! Windows RDP self-setup (design 3.2): switch Remote Desktop on, start
//! TermService, admit the signed-in user — once per version and trigger,
//! with one UAC approval (or directly when the service already runs
//! elevated). No firewall rule: se reaches the service over loopback.
use crate::rdp::core::{HostProbe, HostService, SetupState};
use serde::Deserialize;
use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Only system modules load in the elevated script (as the uplink helper).
const SYSTEM_MODULES: &str = "$env:PSModulePath=$PSHOME+'\\Modules'; \
    $PSModuleAutoLoadingPreference='None'; \
    foreach($module in @('Microsoft.PowerShell.Management','Microsoft.PowerShell.Security',\
    'Microsoft.PowerShell.Utility','CimCmdlets','Microsoft.PowerShell.LocalAccounts')) \
    { Microsoft.PowerShell.Core\\Import-Module -Name ($PSHOME+'\\Modules\\'+$module+'\\'+$module+'.psd1') \
    -ErrorAction Stop }; ";
const SETUP_SCRIPT: &str = include_str!("host_setup.ps1");
/// The user whose session gets RDP access, the elevation state and the
/// session of this process (session 0 = no interactive desktop for UAC).
const CONTEXT_SCRIPT: &str = "$ErrorActionPreference='Stop'; \
    $id=[Security.Principal.WindowsIdentity]::GetCurrent(); \
    $admin=(New-Object Security.Principal.WindowsPrincipal($id)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator); \
    $sid=$id.User.Value; \
    if ($sid -eq 'S-1-5-18') { $name=(Get-CimInstance -ClassName Win32_ComputerSystem).UserName; \
    if ($name) { $sid=(New-Object Security.Principal.NTAccount($name)).Translate([Security.Principal.SecurityIdentifier]).Value } else { $sid='' } }; \
    [pscustomobject]@{ sid=[string]$sid; elevated=[bool]$admin; session=[uint32][Diagnostics.Process]::GetCurrentProcess().SessionId } | ConvertTo-Json -Compress";
/// `ERROR_CANCELLED`: the person declined the UAC prompt.
const DECLINED: i32 = 1223;

#[derive(Deserialize)]
struct UserContext {
    sid: String,
    elevated: bool,
    session: u32,
}

/// A string SID such as `S-1-5-21-…`: digits and dashes only, so it can be
/// placed into the fixed script without any quoting question.
pub(crate) fn valid_sid(sid: &str) -> bool {
    let Some(rest) = sid.strip_prefix("S-1-") else {
        return false;
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    sid.len() <= 184 && rest.split('-').all(digits)
}

/// The exit code of `host_setup.ps1` (or of the UAC launcher) as a state.
pub(crate) fn state_for_exit(code: i32) -> SetupState {
    match code {
        0 => SetupState::Done,
        10 => SetupState::Impossible("policy_denies_rdp".into()),
        11 => SetupState::Impossible("windows_home".into()),
        12 => SetupState::Impossible("windows_edition_unsupported".into()),
        20 => SetupState::Failed("termservice_not_running".into()),
        21 => SetupState::Failed("remote_desktop_users_membership".into()),
        DECLINED => SetupState::Declined,
        other => SetupState::Failed(format!("setup_script_exit_{other}")),
    }
}

fn encoded(program: &str) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = program.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn powershell() -> Result<std::path::PathBuf, String> {
    crate::net::system_powershell().map_err(|error| format!("powershell: {error}"))
}

fn user_context() -> Result<UserContext, String> {
    let output = super::super::shared::process::capture(
        &powershell()?,
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            CONTEXT_SCRIPT,
        ],
    )?;
    serde_json::from_str(output.trim_start_matches('\u{feff}').trim())
        .map_err(|error| format!("user context: {error}"))
}

/// Runs `program` (`-EncodedCommand` for the setup script itself,
/// `-Command` for the short UAC launcher that already carries the encoded
/// script) and returns its exit code. No time limit: a UAC prompt waits for
/// the person (user rule r7).
fn exit_code(program: &str, encode: bool) -> Result<i32, String> {
    let (switch, text) = if encode {
        ("-EncodedCommand", encoded(program))
    } else {
        ("-Command", program.to_string())
    };
    let status = Command::new(powershell()?)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            switch,
            text.as_str(),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|error| format!("powershell start: {error}"))?;
    status
        .code()
        .ok_or_else(|| "setup process ended without exit code".into())
}

/// The elevated run through `Start-Process -Verb RunAs`; a declined prompt
/// becomes exit code 1223.
fn uac_program(script: &str) -> Result<String, String> {
    let engine = powershell()?;
    let engine = engine
        .to_str()
        .ok_or_else(|| "powershell path is not Unicode".to_string())?;
    let arguments = format!(
        "-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand {}",
        encoded(script)
    );
    let launch = format!(
        "{SYSTEM_MODULES}try {{ $p = Start-Process -FilePath {} -Verb RunAs -Wait -WindowStyle Hidden -PassThru \
         -ArgumentList {}; exit $p.ExitCode }} catch {{ $code = 1224; $e = $_.Exception; \
         while ($e) {{ if ($e -is [System.ComponentModel.Win32Exception] -and \
         $e.NativeErrorCode -eq {DECLINED}) {{ $code = {DECLINED} }}; $e = $e.InnerException }}; \
         exit $code }}",
        quote(engine),
        quote(&arguments)
    );
    if launch.encode_utf16().count() > 30000 {
        return Err("setup command exceeds the Windows command-line limit".into());
    }
    Ok(launch)
}

/// Windows hosts RDP; the edition check runs inside `ensure`.
pub(crate) fn impossible() -> Option<SetupState> {
    None
}

pub(crate) fn ensure(_probe: &HostProbe, _device_id: Option<&str>) -> SetupState {
    let facts = match super::probe::inventory() {
        Ok(facts) => facts,
        Err(error) => return SetupState::Failed(format!("inventory: {error}")),
    };
    if facts.home {
        return SetupState::Impossible("windows_home".into());
    }
    if !facts.supported {
        return SetupState::Impossible("windows_edition_unsupported".into());
    }
    if facts.policy_denies {
        return SetupState::Impossible("policy_denies_rdp".into());
    }
    let context = match user_context() {
        Ok(context) => context,
        Err(error) => return SetupState::Failed(format!("user_context: {error}")),
    };
    if context.sid.is_empty() {
        return SetupState::Failed("no_interactive_session".into());
    }
    if !valid_sid(&context.sid) {
        return SetupState::Failed("invalid_user_sid".into());
    }
    let script = format!(
        "{SYSTEM_MODULES}$UserSid={};\n{SETUP_SCRIPT}",
        quote(&context.sid)
    );
    let code = if context.elevated {
        exit_code(&script, true)
    } else if context.session == 0 {
        // A service without a desktop cannot show the UAC prompt.
        return SetupState::Failed("no_interactive_session".into());
    } else {
        uac_program(&script).and_then(|launch| exit_code(&launch, false))
    };
    match code {
        Ok(code) => state_for_exit(code),
        Err(error) => SetupState::Failed(error),
    }
}

/// Windows RDP uses the account's own sign-in; se manages no credentials.
pub(crate) fn issue_credential(_: &HostService, _: &str) -> Result<(String, String), String> {
    Err("Windows-Remote-Desktop meldet mit dem Windows-Konto an; se verwaltet hier keine Zugangsdaten".into())
}

pub(crate) fn rotate_credentials(_: &str) -> Result<(), String> {
    Ok(())
}

/// The Linux root steps have no Windows counterpart.
pub(crate) fn run_system_step(step: &str) -> Result<String, String> {
    Err(format!("Systemschritt {step} gibt es nur unter Linux"))
}
