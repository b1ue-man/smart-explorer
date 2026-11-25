//! The shell of a shell command (`ExecCommand::Shell`) on a Share peer.
//!
//! A start names it in `ExecStart::shell`: `standard` is PowerShell on a
//! Windows host (PowerShell 7 when installed, else Windows PowerShell 5.1)
//! and the login shell on Linux and Android; `powershell`, `cmd` and `sh`
//! choose one explicitly. A start without the field runs as before shell
//! selection existed (Windows: `cmd.exe /D /S /C`), so starts of older
//! clients and the polyglot start line of `se` shell sessions keep working.
//!
//! A host that knows the field lists `EXEC_SHELL_FEATURE` in its hello and
//! names the shell that ran in `ExecTerminal::shell`. An older host drops the
//! unknown field and then refuses the start digest, so the client sends it
//! without the field (`for_host`); such a host runs its own shell, and the
//! client names that shell from the host's provider (`HostFamily`).
//!
//! PowerShell commands run through one statement (`powershell_statement`)
//! here and in the shell sessions of `se`: a parse error is reported as
//! PowerShell words it (with line and column), and the exit status maps
//! `$LASTEXITCODE` and `$?` the same way in both.
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;

use super::exec_types::{ExecCommand, ExecStart};

/// Entry of `ExecHelloOk::features` of a host that takes `ExecStart::shell`
/// and names the shell in `ExecTerminal::shell`.
pub const EXEC_SHELL_FEATURE: &str = "exec_shell_v1";
/// Result name of PowerShell 7 (`pwsh.exe`).
pub const PWSH_LABEL: &str = "pwsh 7";
/// Result name of Windows PowerShell (`powershell.exe`), 5.1 on every
/// supported Windows.
pub const WINDOWS_POWERSHELL_LABEL: &str = "powershell 5.1";
/// Options before the script of a one-shot PowerShell command.
/// `-InputFormat None` keeps PowerShell from reading standard input for
/// `$input`: the input stays for the programs the command starts, and an
/// input that is never closed does not hold the process.
pub const POWERSHELL_ARGS: [&str; 6] = [
    "-NoLogo",
    "-NoProfile",
    "-NonInteractive",
    "-InputFormat",
    "None",
    "-Command",
];
/// Longest command line `CreateProcessW` accepts, in UTF-16 units.
const MAX_WINDOWS_COMMAND_LINE: usize = 32_767;

/// The shell a start asks for.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExecShell {
    /// The host's shell for agents: PowerShell on Windows, the login shell
    /// on Linux and Android.
    Standard,
    /// PowerShell 7 when installed, else Windows PowerShell 5.1 (Windows).
    PowerShell,
    /// `cmd.exe` (Windows).
    Cmd,
    /// The login shell (Linux, Android).
    Sh,
}

impl ExecShell {
    /// A choice as the `exec` tool and `se exec --shell-kind` name it.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "powershell" | "pwsh" => Ok(Self::PowerShell),
            "cmd" => Ok(Self::Cmd),
            "sh" => Ok(Self::Sh),
            other => Err(format!(
                "unknown shell `{other}`; use powershell, cmd or sh"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::PowerShell => "powershell",
            Self::Cmd => "cmd",
            Self::Sh => "sh",
        }
    }
}

/// `start` as a host with `features` takes it: a host without
/// `EXEC_SHELL_FEATURE` would refuse the digest of a field it does not
/// know, so the choice is left out and that host runs its own shell.
pub fn for_host(mut start: ExecStart, features: &[String]) -> ExecStart {
    if start.shell.is_some() && !features.iter().any(|name| name == EXEC_SHELL_FEATURE) {
        start.shell = None;
    }
    start
}

/// Operating system of an Exec host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostFamily {
    Windows,
    Linux,
    Android,
}

impl HostFamily {
    pub fn label(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::Android => "android",
        }
    }

    /// The family of a host by its `ExecProviderStatus::provider`
    /// (`share/os/windows/exec.rs`, `linux_os/exec.rs`, `android/exec.rs`).
    pub fn of_provider(provider: &str) -> Option<Self> {
        match provider {
            "Windows Job Object" => Some(Self::Windows),
            "systemd transient cgroup" => Some(Self::Linux),
            "android-subreaper" => Some(Self::Android),
            _ => None,
        }
    }

    /// The shell a host without `EXEC_SHELL_FEATURE` runs a shell command in.
    pub fn legacy_shell(self) -> &'static str {
        match self {
            Self::Windows => "cmd",
            Self::Linux => "login shell",
            Self::Android => "sh",
        }
    }
}

/// Windows host: decides the shell of `request` and returns its result
/// name (`None` for a start without a shell command). PowerShell runs as the
/// argv start of `powershell` (path and name of the host's PowerShell, asked
/// only when needed) with the one-shot script; `cmd` and a start without a
/// choice keep the shell command, which the supervisor runs in `cmd.exe`.
pub fn windows(
    request: &mut ExecStart,
    powershell: impl FnOnce() -> (String, &'static str),
) -> io::Result<Option<String>> {
    let ExecCommand::Shell { command } = &request.command else {
        return Ok(None);
    };
    match request.shell {
        None | Some(ExecShell::Cmd) => Ok(Some("cmd".to_string())),
        Some(ExecShell::Sh) => Err(refused(
            "this peer runs Windows; `sh` is for Linux and Android peers and SSH remotes: \
             use powershell (the default) or cmd"
                .to_string(),
        )),
        Some(ExecShell::Standard | ExecShell::PowerShell) => {
            let script = powershell_one_shot(command);
            let (program, label) = powershell();
            let options: usize = POWERSHELL_ARGS.iter().map(|option| option.len() + 1).sum();
            if program.len() + options + script.len() + 4 > MAX_WINDOWS_COMMAND_LINE {
                return Err(refused(format!(
                    "this command is too long for a PowerShell command line on Windows ({} \
                     bytes); write it to a script file first and run that file",
                    command.len()
                )));
            }
            let mut args: Vec<String> = POWERSHELL_ARGS.iter().map(|o| o.to_string()).collect();
            args.push(script);
            request.command = ExecCommand::Argv { program, args };
            request.shell = None;
            Ok(Some(label.to_string()))
        }
    }
}

/// Linux or Android host: the login shell runs every shell command; returns
/// its result name (the file name, e.g. `bash`), `None` for a start without
/// a shell command. `powershell` and `cmd` are refused.
pub fn posix(
    request: &ExecStart,
    family: HostFamily,
    environment: &BTreeMap<String, String>,
) -> io::Result<Option<String>> {
    if !matches!(request.command, ExecCommand::Shell { .. }) {
        return Ok(None);
    }
    match request.shell {
        None | Some(ExecShell::Standard | ExecShell::Sh) => {
            Ok(Some(posix_shell_name(family, environment)))
        }
        Some(other) => Err(refused(format!(
            "this peer runs {}; its commands run in the login shell, and `{}` is only for \
             Windows peers: omit `shell` or use sh",
            family.label(),
            other.as_str()
        ))),
    }
}

/// The shell the supervisor starts (`linux_os/exec_supervisor.rs`
/// `shell_program`): Linux an absolute `SHELL` of the job, else `/bin/sh`;
/// Android `/system/bin/sh`.
fn posix_shell_name(family: HostFamily, environment: &BTreeMap<String, String>) -> String {
    let path = match family {
        HostFamily::Linux => environment
            .get("SHELL")
            .map(String::as_str)
            .filter(|shell| shell.starts_with('/'))
            .unwrap_or("/bin/sh"),
        HostFamily::Android | HostFamily::Windows => "/system/bin/sh",
    };
    path.rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("sh")
        .to_string()
}

/// The `-Command` script of a one-shot command: UTF-8 output, no progress
/// records, the command through `powershell_statement`, its status as the
/// exit code.
pub fn powershell_one_shot(command: &str) -> String {
    format!(
        "try{{[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)}}catch{{}};\
$global:OutputEncoding=[Console]::OutputEncoding;$global:ProgressPreference='SilentlyContinue';\
{};[Console]::Out.Flush();exit $global:__se_s",
        powershell_statement(command, "", None)
    )
}

/// One line of PowerShell that runs `command` after `prelude` (statements
/// of `se` itself, such as a working directory) and sets `$global:__se_s`:
/// 1 after a parse error or a terminating error, else a non-zero
/// `$LASTEXITCODE`, else 1 when a non-native error record was written, else
/// `$LASTEXITCODE` or 0. `command` is parsed first: its parse errors go to
/// stderr as PowerShell words them, with line and column, instead of the
/// exception of `[ScriptBlock]::Create`. `input` becomes the command's
/// pipeline input. Every text is a Base64 literal, so no quoting rule of
/// the command matters.
pub fn powershell_statement(command: &str, prelude: &str, input: Option<&str>) -> String {
    let script = if prelude.is_empty() {
        "$global:__se_c".to_string()
    } else {
        format!("{}+$global:__se_c", powershell_text(prelude))
    };
    let block = format!("([ScriptBlock]::Create({script}))");
    let invoke = match input {
        Some(input) => format!("{} | . {block}", powershell_text(input)),
        None => format!(". {block}"),
    };
    STATEMENT
        .replace("@COMMAND@", &powershell_text(command))
        .replace("@INVOKE@", &invoke)
}

/// `powershell_statement` with `@COMMAND@` (the command text) and
/// `@INVOKE@` (the dot-sourced script block) still open. A parse error at
/// `&&` or `||` adds how Windows PowerShell 5.1 writes the same.
const STATEMENT: &str = "$global:LASTEXITCODE=$null;$global:__se_n=$false;$global:__se_f=$false;\
$global:__se_e=$null;$global:__se_c=@COMMAND@;\
$null=[System.Management.Automation.Language.Parser]::ParseInput($global:__se_c,[ref]$null,[ref]$global:__se_e);\
if($global:__se_e){$global:__se_f=$true;foreach($__se_x in $global:__se_e){\
[Console]::Error.WriteLine('se: PowerShell cannot parse line '+$__se_x.Extent.StartLineNumber+\
', column '+$__se_x.Extent.StartColumnNumber+': '+$__se_x.Message+\
$(if(@('&&','||') -contains $__se_x.Extent.Text){' (Windows PowerShell 5.1 has no && and ||: \
separate commands with ; and run the next one only on success with if ($?) { ... })'}else{''}))}}\
else{try{@INVOKE@ 2>&1 | ForEach-Object { if ($_ -is [System.Management.Automation.ErrorRecord]) \
{ if ($_.FullyQualifiedErrorId -notlike 'NativeCommandError*') { $global:__se_n=$true }; \
[Console]::Error.WriteLine($_.ToString()) } else { $_ } } | Out-Default}\
catch{$global:__se_f=$true;[Console]::Error.WriteLine($_.ToString())}};\
$global:__se_s=if($global:__se_f){1}elseif($null -ne $global:LASTEXITCODE -and $global:LASTEXITCODE -ne 0)\
{$global:LASTEXITCODE}elseif($global:__se_n){1}elseif($null -ne $global:LASTEXITCODE){$global:LASTEXITCODE}else{0}";

/// A PowerShell expression for `value` without any quoting of its own.
pub fn powershell_text(value: &str) -> String {
    format!(
        "[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{}'))",
        base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
    )
}

fn refused(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
#[path = "exec_shell_tests.rs"]
mod tests;
