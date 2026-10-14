//! The two shell dialects of a session, how the login shell is recognized,
//! and the exact text sent for each command.
//!
//! POSIX: `sh -c` replaces the login shell with `bash -l` (or `sh -l`); every
//! command runs as `eval '<command>'` in that shell, so `cd`, `export`,
//! variables and functions persist. PowerShell (Windows): `powershell.exe
//! -Command -` executes one statement per input line, as if typed at its
//! prompt; every command is a dot-sourced script block decoded from Base64
//! UTF-8, so no quoting rule of the command matters, and its markers follow
//! as a second statement after the command's output was written.
//!
//! A terminal session needs no markers; only its start line with a working
//! directory and environment depends on the dialect (`terminal_command`).
use super::types::{CommandRequest, Dialect, StdinMode};
use crate::cli::exec_ssh::quote;
use base64::Engine as _;
use std::collections::BTreeMap;

/// Command and input text of one request; larger input belongs in a file
/// (`copy`) rather than in a shell's standard input.
pub(super) const MAX_REQUEST_BYTES: usize = 1024 * 1024;
/// Runs in the login shell before a session starts: cmd.exe prints
/// `Windows_NT`, PowerShell prints its version table (`PSVersion`), POSIX
/// shells print `%OS%`.
pub(super) const PROBE: &str = "echo %OS%;$PSVersionTable";
/// Ends an idle session; both dialects read `exit`.
pub(super) const EXIT: &[u8] = b"\nexit\n";
const POSIX_START: &str =
    "exec sh -c 'if command -v bash >/dev/null 2>&1; then exec bash -l; fi; exec sh -l'";
const POWERSHELL_START: &str = "powershell.exe -NoLogo -NoProfile -NonInteractive -Command -";
/// First command of a PowerShell session: UTF-8 output, no progress
/// records, an empty prompt.
const POWERSHELL_SETUP: &str =
    "try { [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false) } catch {}
$global:OutputEncoding = [Console]::OutputEncoding
$global:ProgressPreference = 'SilentlyContinue'
function global:prompt { '' }";

pub(super) fn classify(probe_stdout: &str) -> Dialect {
    if probe_stdout.contains("Windows_NT") || probe_stdout.contains("PSVersion") {
        Dialect::PowerShell
    } else {
        Dialect::Posix
    }
}

/// The exec command that turns the login shell into the session's shell.
pub(super) fn start_line(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Posix => POSIX_START,
        Dialect::PowerShell => POWERSHELL_START,
    }
}

/// The startup command; its marker proves the shell reads commands.
pub(super) fn startup_command(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Posix => ":",
        Dialect::PowerShell => POWERSHELL_SETUP,
    }
}

/// A command line for a program and its literal arguments.
pub(in crate::cli) fn argv_command(dialect: Dialect, program: &str, args: &[String]) -> String {
    match dialect {
        Dialect::Posix => std::iter::once(program)
            .chain(args.iter().map(String::as_str))
            .map(quote)
            .collect::<Vec<_>>()
            .join(" "),
        Dialect::PowerShell => {
            let mut line = format!("& ({})", text(program));
            for argument in args {
                line.push_str(&format!(" ({})", text(argument)));
            }
            line
        }
    }
}

/// The `exec` request that starts the login shell of an SSH terminal in
/// `cwd` with `env` (the terminal's `shell` request needs no line when it has
/// neither). POSIX: `cd`, `export` and then the login shell replaces the
/// command shell. PowerShell: a `-Command` script of Base64-decoded literals
/// only, free of `$`, quotes and backticks, so that neither `cmd.exe` nor an
/// outer PowerShell changes it; `-NoExit` keeps the interactive shell.
pub(super) fn terminal_command(
    dialect: Dialect,
    cwd: Option<&str>,
    env: &BTreeMap<String, String>,
) -> Result<String, String> {
    // The same value rules as for a command: no NUL, no empty directory, and
    // names the dialect can export.
    let values = CommandRequest {
        command: ":".into(),
        cwd: cwd.map(str::to_string),
        env: env.clone(),
        stdin: StdinMode::Null,
    };
    check(dialect, "", &values)?;
    Ok(match dialect {
        Dialect::Posix => posix_terminal(cwd, env),
        Dialect::PowerShell => powershell_terminal(cwd, env),
    })
}

fn posix_terminal(cwd: Option<&str>, env: &BTreeMap<String, String>) -> String {
    let mut line = String::new();
    if let Some(cwd) = cwd {
        line.push_str(&format!("cd -- {} && ", quote(cwd)));
    }
    for (name, value) in env {
        line.push_str(&format!("export {name}={} && ", quote(value)));
    }
    line.push_str("exec \"${SHELL:-/bin/sh}\" -l");
    line
}

fn powershell_terminal(cwd: Option<&str>, env: &BTreeMap<String, String>) -> String {
    let mut statements = Vec::new();
    if let Some(cwd) = cwd {
        let directory = text(cwd);
        statements.push(format!(
            "if (-not (Test-Path -LiteralPath ({directory}) -PathType Container)) {{ [Console]::Error.WriteLine('se: the working directory does not exist'); exit 1 }}"
        ));
        statements.push(format!("Set-Location -LiteralPath ({directory})"));
    }
    for (name, value) in env {
        statements.push(format!(
            "[Environment]::SetEnvironmentVariable(({}), ({}))",
            text(name),
            text(value)
        ));
    }
    if statements.is_empty() {
        return "powershell.exe -NoLogo".to_string();
    }
    format!(
        "powershell.exe -NoLogo -NoExit -Command \"{}\"",
        statements.join("; ")
    )
}

/// Rejects requests the session could not run exactly as given.
pub(super) fn check(dialect: Dialect, nonce: &str, request: &CommandRequest) -> Result<(), String> {
    if request.command.trim().is_empty() {
        return Err("the command is empty".into());
    }
    let input = match &request.stdin {
        StdinMode::Text(text) => text.as_str(),
        StdinMode::Null | StdinMode::Interactive => "",
    };
    if request.command.len().saturating_add(input.len()) > MAX_REQUEST_BYTES {
        return Err(format!(
            "command and input exceed {MAX_REQUEST_BYTES} bytes; copy larger input to a file first"
        ));
    }
    let has_nul = std::iter::once(request.command.as_str())
        .chain(request.cwd.as_deref())
        .chain(
            request
                .env
                .iter()
                .flat_map(|(name, value)| [name.as_str(), value.as_str()]),
        )
        .chain(std::iter::once(input))
        .any(|text| text.contains('\0'));
    if has_nul {
        return Err("the request contains a NUL character".into());
    }
    if request
        .cwd
        .as_deref()
        .is_some_and(|cwd| cwd.trim().is_empty())
    {
        return Err("the working directory is empty".into());
    }
    for name in request.env.keys() {
        let valid = match dialect {
            Dialect::Posix => posix_name(name),
            Dialect::PowerShell => !name.is_empty() && !name.contains('='),
        };
        if !valid {
            return Err(format!(
                "environment name {name:?} is not valid in a {} session",
                dialect.label()
            ));
        }
    }
    if let (Dialect::Posix, StdinMode::Text(text)) = (dialect, &request.stdin) {
        let delimiter = heredoc_delimiter(nonce);
        if text.lines().any(|line| line == delimiter) {
            return Err("the input contains this session's end-of-input line".into());
        }
    }
    Ok(())
}

/// The exact text written to the shell's standard input for command `seq`.
pub(super) fn wrap(
    dialect: Dialect,
    nonce: &str,
    seq: u64,
    request: &CommandRequest,
) -> Result<String, String> {
    check(dialect, nonce, request)?;
    Ok(match dialect {
        Dialect::Posix => posix(nonce, seq, request),
        Dialect::PowerShell => powershell(nonce, seq, request),
    })
}

/// A leading newline ends a partial line left by input the previous command
/// did not read; `eval` keeps the command one word for the outer shell. The
/// markers follow on the same input line, so the shell has read them before
/// the command can read its standard input: input typed for an interactive
/// command reaches only that command. A here-document starts after that
/// line.
fn posix(nonce: &str, seq: u64, request: &CommandRequest) -> String {
    let mut text = String::from("\n");
    if let Some(cwd) = &request.cwd {
        text.push_str(&format!("cd -- {} && ", quote(cwd)));
    }
    for (name, value) in &request.env {
        text.push_str(&format!("export {name}={} && ", quote(value)));
    }
    text.push_str("eval ");
    text.push_str(&quote(&request.command));
    let heredoc = match &request.stdin {
        StdinMode::Null => {
            text.push_str(" </dev/null");
            None
        }
        StdinMode::Interactive => None,
        StdinMode::Text(input) => {
            let delimiter = heredoc_delimiter(nonce);
            text.push_str(&format!(" <<'{delimiter}'"));
            Some((input, delimiter))
        }
    };
    text.push_str(&format!(
        "; __se_s=$?; printf '\\036SE{nonce}:{seq}:%s:%s\\036' \"$__se_s\" \"$PWD\"; printf '\\036SE{nonce}:{seq}\\036' >&2\n"
    ));
    if let Some((input, delimiter)) = heredoc {
        text.push_str(input);
        if !input.is_empty() && !input.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&delimiter);
        text.push('\n');
    }
    text
}

/// Exit status: 1 after a terminating error, else a non-zero
/// `$LASTEXITCODE`, else 1 when a non-native error record was written, else
/// `$LASTEXITCODE` or 0. The marker statement runs after the host wrote the
/// command's output.
fn powershell(nonce: &str, seq: u64, request: &CommandRequest) -> String {
    let mut script = String::new();
    if let Some(cwd) = &request.cwd {
        // In argument mode an unparenthesized `[..]::..(..)` is a string up to
        // its first `(`, not an expression; the parentheses make it one.
        script.push_str(&format!(
            "Set-Location -LiteralPath ({}) -ErrorAction Stop\n",
            text(cwd)
        ));
    }
    for (name, value) in &request.env {
        script.push_str(&format!(
            "[Environment]::SetEnvironmentVariable({}, {})\n",
            text(name),
            text(value)
        ));
    }
    script.push_str(&request.command);
    let block = format!("([ScriptBlock]::Create({}))", text(&script));
    let invoke = match &request.stdin {
        StdinMode::Text(input) => format!("{} | . {block}", text(input)),
        StdinMode::Null | StdinMode::Interactive => format!(". {block}"),
    };
    let run = format!(
        "$global:LASTEXITCODE=$null;$global:__se_n=$false;$global:__se_f=$false;\
try{{{invoke} 2>&1 | ForEach-Object {{ if ($_ -is [System.Management.Automation.ErrorRecord]) {{ if ($_.FullyQualifiedErrorId -notlike 'NativeCommandError*') {{ $global:__se_n=$true }}; [Console]::Error.WriteLine($_.ToString()) }} else {{ $_ }} }} | Out-Default}}\
catch{{$global:__se_f=$true;[Console]::Error.WriteLine($_.ToString())}};\
$global:__se_s=if($global:__se_f){{1}}elseif($null -ne $global:LASTEXITCODE -and $global:LASTEXITCODE -ne 0){{$global:LASTEXITCODE}}elseif($global:__se_n){{1}}elseif($null -ne $global:LASTEXITCODE){{$global:LASTEXITCODE}}else{{0}}"
    );
    let markers = format!(
        "[Console]::Out.Flush();[Console]::Out.Write([string][char]30+'SE{nonce}:{seq}:'+$global:__se_s+':'+(Get-Location).Path+[char]30);[Console]::Out.Flush();[Console]::Error.Write([string][char]30+'SE{nonce}:{seq}'+[char]30);[Console]::Error.Flush()"
    );
    format!("\n{run}\n{markers}\n")
}

/// A PowerShell expression for `value` without any quoting of its own.
fn text(value: &str) -> String {
    format!(
        "[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{}'))",
        base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
    )
}

fn heredoc_delimiter(nonce: &str) -> String {
    format!("SE_IN_{nonce}")
}

fn posix_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::{
        argv_command, check, classify, start_line, terminal_command, text, wrap, MAX_REQUEST_BYTES,
    };
    use crate::cli::shell_session::types::{CommandRequest, Dialect, StdinMode};
    use std::collections::BTreeMap;

    fn request(command: &str) -> CommandRequest {
        CommandRequest {
            command: command.into(),
            cwd: None,
            env: BTreeMap::new(),
            stdin: StdinMode::Null,
        }
    }

    #[test]
    fn shell_session_task_probe_output_selects_the_dialect() {
        assert_eq!(
            classify("Windows_NT;$PSVersionTable\r\n"),
            Dialect::PowerShell
        );
        assert_eq!(
            classify("%OS%\r\n\r\nName   Value\r\n----   -----\r\nPSVersion   5.1.26100.1\r\n"),
            Dialect::PowerShell
        );
        assert_eq!(classify("%OS%\n"), Dialect::Posix);
        assert_eq!(classify(""), Dialect::Posix);
        assert!(start_line(Dialect::Posix).starts_with("exec sh -c "));
        assert!(start_line(Dialect::PowerShell).ends_with("-Command -"));
    }

    #[test]
    fn shell_session_task_posix_wrapper_quotes_and_redirects() {
        let mut placed = request("echo 'a b' $X");
        placed.cwd = Some("/srv/my app".into());
        placed.env.insert("MODE".into(), "it's".into());
        assert_eq!(
            wrap(Dialect::Posix, "00ff", 4, &placed).unwrap(),
            "\ncd -- '/srv/my app' && export MODE='it'\\''s' && eval 'echo '\\''a b'\\'' $X' </dev/null; __se_s=$?; printf '\\036SE00ff:4:%s:%s\\036' \"$__se_s\" \"$PWD\"; printf '\\036SE00ff:4\\036' >&2\n"
        );
        let mut input = request("cat");
        input.stdin = StdinMode::Text("line1\nline2".into());
        let heredoc = wrap(Dialect::Posix, "00ff", 5, &input).unwrap();
        assert!(
            heredoc.starts_with("\neval 'cat' <<'SE_IN_00ff'; __se_s=$?"),
            "{heredoc}"
        );
        assert!(
            heredoc.ends_with(">&2\nline1\nline2\nSE_IN_00ff\n"),
            "{heredoc}"
        );
        // The markers share the command's input line: a command reading its
        // standard input gets only what the participant types.
        let mut typed = request("read x");
        typed.stdin = StdinMode::Interactive;
        let typed = wrap(Dialect::Posix, "00ff", 6, &typed).unwrap();
        assert!(typed.starts_with("\neval 'read x'; __se_s=$?"), "{typed}");
        assert_eq!(typed.matches('\n').count(), 2, "{typed}");
        assert_eq!(
            argv_command(Dialect::Posix, "printf", &["%s|".into(), "it's".into()]),
            "'printf' '%s|' 'it'\\''s'"
        );
    }

    #[test]
    fn shell_session_task_powershell_wrapper_encodes_every_literal() {
        let mut placed = request("Write-Output 'ü'; exit 3");
        placed.cwd = Some("C:\\Temp\\it's".into());
        placed.env.insert("A B".into(), "x\u{2019}y".into());
        placed.stdin = StdinMode::Text("data".into());
        let wrapped = wrap(Dialect::PowerShell, "00ff", 9, &placed).unwrap();
        for literal in ["Write-Output", "Temp", "x\u{2019}y", "A B", "data"] {
            assert!(!wrapped.contains(literal), "{literal} in {wrapped}");
        }
        assert!(wrapped.contains("'SE00ff:9:'+$global:__se_s"), "{wrapped}");
        assert!(wrapped.contains("'SE00ff:9'+[char]30"), "{wrapped}");
        // A blank line, the command statement, the marker statement.
        assert_eq!(wrapped.split('\n').count(), 4, "{wrapped}");
        assert_eq!(
            argv_command(Dialect::PowerShell, "git", &["status".into()]),
            format!("& ({}) ({})", text("git"), text("status"))
        );
    }

    #[test]
    fn shell_session_task_requests_are_checked_before_queueing() {
        assert!(check(Dialect::Posix, "00ff", &request("  ")).is_err());
        assert!(check(Dialect::Posix, "00ff", &request("a\0b")).is_err());
        let mut env = request("env");
        env.env.insert("A-B".into(), String::new());
        assert!(check(Dialect::Posix, "00ff", &env).is_err());
        assert!(check(Dialect::PowerShell, "00ff", &env).is_ok());
        let mut delimiter = request("cat");
        delimiter.stdin = StdinMode::Text("x\nSE_IN_00ff\ny".into());
        assert!(check(Dialect::Posix, "00ff", &delimiter).is_err());
        let mut big = request("cat");
        big.stdin = StdinMode::Text("x".repeat(MAX_REQUEST_BYTES));
        assert!(check(Dialect::Posix, "00ff", &big).is_err());
        let mut empty_cwd = request("ls");
        empty_cwd.cwd = Some(" ".into());
        assert!(check(Dialect::Posix, "00ff", &empty_cwd).is_err());
        assert!(check(Dialect::Posix, "00ff", &request("ls")).is_ok());
    }

    #[test]
    fn terminal_task_start_lines_quote_and_encode_every_literal() {
        let mut env = BTreeMap::new();
        env.insert("MODE".to_string(), "it's $HOME".to_string());
        assert_eq!(
            terminal_command(Dialect::Posix, Some("/srv/my app"), &env).unwrap(),
            "cd -- '/srv/my app' && export MODE='it'\\''s $HOME' && exec \"${SHELL:-/bin/sh}\" -l"
        );
        assert_eq!(
            terminal_command(Dialect::Posix, None, &BTreeMap::new()).unwrap(),
            "exec \"${SHELL:-/bin/sh}\" -l"
        );
        let mut named = BTreeMap::new();
        named.insert("A B".to_string(), "x\u{2019}y".to_string());
        let line = terminal_command(Dialect::PowerShell, Some("C:\\Temp\\it's"), &named).unwrap();
        assert!(
            line.starts_with("powershell.exe -NoLogo -NoExit -Command \""),
            "{line}"
        );
        for literal in ["Temp", "x\u{2019}y", "A B"] {
            assert!(!line.contains(literal), "{literal} in {line}");
        }
        // Nothing for cmd.exe or an outer PowerShell to expand or end early.
        let script = &line["powershell.exe -NoLogo -NoExit -Command \"".len()..line.len() - 1];
        for forbidden in ['$', '"', '`', '%', '!', '&', '|', '<', '>', '^'] {
            assert!(!script.contains(forbidden), "{forbidden} in {script}");
        }
        assert!(script.contains("Set-Location -LiteralPath ("), "{script}");
        assert_eq!(
            terminal_command(Dialect::PowerShell, None, &BTreeMap::new()).unwrap(),
            "powershell.exe -NoLogo"
        );
        // The value rules of commands apply.
        assert!(terminal_command(Dialect::Posix, Some(" "), &BTreeMap::new()).is_err());
        assert!(terminal_command(Dialect::Posix, Some("a\0b"), &BTreeMap::new()).is_err());
        let mut bad = BTreeMap::new();
        bad.insert("A-B".to_string(), String::new());
        assert!(terminal_command(Dialect::Posix, None, &bad).is_err());
        assert!(terminal_command(Dialect::PowerShell, None, &bad).is_ok());
    }
}
