//! `se exec` on a saved SFTP remote: the command runs through the remote
//! user's login shell on the connection's SSH login, like `ssh host 'cmd'`
//! (`sftp/core/command.rs`). `--cwd`, `--env` and the argv form assume a
//! POSIX-compatible login shell; Windows OpenSSH targets use `--shell` with
//! their own shell syntax.
use crate::creds::{Protocol, SavedConnection};
use crate::share::ExecCommand;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A saved SFTP connection selected as command target; `cwd` comes from a
/// path in the selector (`@label:/srv`).
pub(super) struct SshExecTarget {
    pub(super) connection: SavedConnection,
    pub(super) cwd: Option<String>,
}

/// Recognizes `@label`, `@label:/dir` and saved `sftp://user@host:port/dir`
/// selectors. Other selectors (Share peers) return `None`.
pub(super) fn ssh_target(selector: &str) -> Result<Option<SshExecTarget>, String> {
    let selector = selector.trim();
    if let Some(rest) = selector.strip_prefix('@') {
        let connections = crate::creds::load_connections_checked()?;
        let exact = connections
            .iter()
            .filter(|connection| connection.display() == rest || connection.account() == rest)
            .cloned()
            .collect::<Vec<_>>();
        let (connection, cwd) = match exact.as_slice() {
            [connection] => (connection.clone(), None),
            [] => match super::target::saved_shorthand(selector)? {
                Some((connection, path)) => (connection, Some(path)),
                None => return Err(format!("unknown saved connection: {selector}")),
            },
            _ => {
                return Err(format!(
                    "saved connection label is ambiguous: {}",
                    exact
                        .iter()
                        .map(SavedConnection::account)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        };
        return require_sftp(connection, cwd).map(Some);
    }
    if selector.starts_with("sftp://") {
        let (connection, path) = crate::connect::saved_and_path(selector).ok_or_else(|| {
            format!("no saved SFTP connection matches {selector}; add it with `se connections add sftp …`")
        })?;
        let cwd = (!path.trim_matches('/').is_empty()).then_some(path);
        return require_sftp(connection, cwd).map(Some);
    }
    Ok(None)
}

fn require_sftp(connection: SavedConnection, cwd: Option<String>) -> Result<SshExecTarget, String> {
    if connection.protocol != Protocol::Sftp {
        return Err(format!(
            "{} is a {} connection; remote commands need an SFTP (SSH) connection or a Share peer",
            connection.display(),
            connection.protocol.as_str()
        ));
    }
    Ok(SshExecTarget { connection, cwd })
}

/// The command line handed to the remote login shell.
pub(super) fn remote_command_line(
    command: &ExecCommand,
    cwd: Option<&str>,
    env: &BTreeMap<String, String>,
) -> Result<String, String> {
    let mut line = String::new();
    if let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) {
        line.push_str(&format!("cd -- {} || exit $?; ", quote(cwd)));
    }
    for (name, value) in env {
        if !valid_env_name(name) {
            return Err(format!(
                "environment name {name:?} is not a portable shell variable name"
            ));
        }
        line.push_str(&format!("export {name}={}; ", quote(value)));
    }
    match command {
        ExecCommand::Shell { command } => line.push_str(command),
        ExecCommand::Argv { program, args } => {
            line.push_str(&quote(program));
            for argument in args {
                line.push(' ');
                line.push_str(&quote(argument));
            }
        }
    }
    Ok(line)
}

/// POSIX single-quote quoting: everything is literal except `'` itself.
pub(super) fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// Exit code of an ended SSH command, following the `se exec` conventions:
/// the remote status, `128 + n` for a signal, 124 timeout, 130 cancelled,
/// 125 when the command could not run or the transport failed.
pub(super) fn exit_code(end: &crate::sftp::SshCommandEnd, timed_out: bool) -> i32 {
    if timed_out {
        return 124;
    }
    if end.cancelled {
        return 130;
    }
    if let Some(status) = end.exit_status {
        return i32::try_from(status).unwrap_or(255);
    }
    if let Some(signal) = end.signal.as_deref() {
        return crate::sftp::signal_number(signal).map_or(255, |number| 128 + number);
    }
    125
}

pub(super) struct SshRun {
    pub(super) target: SshExecTarget,
    pub(super) command: ExecCommand,
    pub(super) cwd: Option<String>,
    pub(super) env: BTreeMap<String, String>,
    pub(super) timeout_seconds: u64,
    pub(super) max_output: u64,
}

pub(super) fn run(run: SshRun) -> Result<i32, String> {
    let cwd = run.cwd.or(run.target.cwd);
    let line = remote_command_line(&run.command, cwd.as_deref(), &run.env)?;
    let backend = crate::connect::open_saved_ssh(&run.target.connection)?;
    let mut command = backend
        .start_command(&line)
        .map_err(|error| format!("remote command start: {error}"))?;
    let control = command.control();
    let cancel = control.clone();
    ctrlc::set_handler(move || cancel.cancel())
        .map_err(|error| format!("Ctrl+C handler: {error}"))?;
    let input = control.clone();
    std::thread::Builder::new()
        .name("se-exec-ssh-stdin".into())
        .spawn(move || stream_stdin(input))
        .map_err(|error| format!("stdin worker: {error}"))?;
    let timed_out = Arc::new(AtomicBool::new(false));
    if run.timeout_seconds > 0 {
        let timer = control.clone();
        let flag = timed_out.clone();
        let limit = Duration::from_secs(run.timeout_seconds);
        std::thread::Builder::new()
            .name("se-exec-ssh-timeout".into())
            .spawn(move || {
                std::thread::sleep(limit);
                flag.store(true, Ordering::SeqCst);
                timer.cancel();
            })
            .map_err(|error| format!("timeout worker: {error}"))?;
    }
    let mut written = 0u64;
    let mut truncated = false;
    while let Some(event) = command.next_event() {
        match event {
            crate::sftp::SshCommandEvent::Started => {}
            crate::sftp::SshCommandEvent::Stdout(bytes) => emit(
                std::io::stdout(),
                &bytes,
                &mut written,
                &run,
                &mut truncated,
                &control,
            )?,
            crate::sftp::SshCommandEvent::Stderr(bytes) => emit(
                std::io::stderr(),
                &bytes,
                &mut written,
                &run,
                &mut truncated,
                &control,
            )?,
            crate::sftp::SshCommandEvent::Finished(end) => {
                if truncated {
                    let _ = writeln!(
                        std::io::stderr(),
                        "se: remote output was truncated by the explicit --max-output limit"
                    );
                }
                if let Some(error) = end.error.as_deref() {
                    let _ = writeln!(std::io::stderr(), "se: {error}");
                }
                let timed_out = timed_out.load(Ordering::SeqCst) && end.exit_status.is_none();
                return Ok(exit_code(&end, timed_out));
            }
        }
    }
    Err("remote command ended without a final status".into())
}

fn emit(
    mut writer: impl Write,
    bytes: &[u8],
    written: &mut u64,
    run: &SshRun,
    truncated: &mut bool,
    control: &crate::sftp::SshCommandControl,
) -> Result<(), String> {
    let allowed = if run.max_output == 0 {
        bytes.len()
    } else {
        let remaining = run.max_output.saturating_sub(*written);
        usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(bytes.len())
    };
    if allowed < bytes.len() && !*truncated {
        *truncated = true;
        control.cancel();
    }
    *written = written.saturating_add(allowed as u64);
    writer
        .write_all(&bytes[..allowed])
        .and_then(|()| writer.flush())
        .map_err(|error| error.to_string())
}

fn stream_stdin(input: crate::sftp::SshCommandControl) {
    let mut stdin = std::io::stdin();
    let mut bytes = [0u8; 64 * 1024];
    loop {
        match stdin.read(&mut bytes) {
            Ok(0) => {
                let _ = input.eof();
                return;
            }
            Ok(read) => {
                if input.stdin(&bytes[..read]).is_err() {
                    return;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                let _ = input.eof();
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{exit_code, quote, remote_command_line};
    use crate::share::ExecCommand;
    use std::collections::BTreeMap;

    #[test]
    fn mcp_task_ssh_command_line_quotes_every_literal() {
        assert_eq!(quote("it's $HOME"), "'it'\\''s $HOME'");
        let argv = ExecCommand::Argv {
            program: "printf".into(),
            args: vec!["%s\n".into(), "a b".into(), "$(rm -rf /)".into()],
        };
        let mut env = BTreeMap::new();
        env.insert("MODE".to_string(), "prod uction".to_string());
        assert_eq!(
            remote_command_line(&argv, Some("/srv/my app"), &env).unwrap(),
            "cd -- '/srv/my app' || exit $?; export MODE='prod uction'; 'printf' '%s\n' 'a b' '$(rm -rf /)'"
        );
        let shell = ExecCommand::Shell {
            command: "echo $HOME | wc -c".into(),
        };
        assert_eq!(
            remote_command_line(&shell, None, &BTreeMap::new()).unwrap(),
            "echo $HOME | wc -c"
        );
        let mut bad = BTreeMap::new();
        bad.insert("A-B".to_string(), String::new());
        assert!(remote_command_line(&shell, None, &bad).is_err());
    }

    #[test]
    fn mcp_task_ssh_exit_codes_follow_exec_conventions() {
        let mut end = crate::sftp::SshCommandEnd {
            exit_status: Some(3),
            ..Default::default()
        };
        assert_eq!(exit_code(&end, false), 3);
        assert_eq!(exit_code(&end, true), 124);
        end.exit_status = None;
        end.signal = Some("TERM".into());
        assert_eq!(exit_code(&end, false), 143);
        end.cancelled = true;
        assert_eq!(exit_code(&end, false), 130);
        let failed = crate::sftp::SshCommandEnd {
            error: Some("rejected".into()),
            ..Default::default()
        };
        assert_eq!(exit_code(&failed, false), 125);
    }
}
