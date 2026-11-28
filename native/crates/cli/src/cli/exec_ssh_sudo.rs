//! Elevated commands on SSH remotes through `sudo` (plan
//! docs/plaene/2026-11-28-ersetzen-masken-shell, part C „Erhöhung
//! entfernt“): `sudo -n true` tells whether the account may run without a
//! password; otherwise se asks the user locally (`exec_password.rs`),
//! checks the password once with `sudo -S -p '' true` and hands it to the
//! command's `sudo -S` as the first line of its input. The password lives
//! only in memory for that start; it is never stored or shown.
use std::sync::Arc;

use zeroize::Zeroizing;

use crate::sftp::{SftpBackend, SshCommandEvent};

/// How `sudo` runs on the remote.
pub(in crate::cli) enum Sudo {
    /// `sudo -n`: the account needs no password.
    NoPassword,
    /// `sudo -S -p ''` with this password as the first input line.
    Password(Zeroizing<String>),
}

impl Sudo {
    /// The result name of the way (`elevation` in the MCP results).
    pub(in crate::cli) fn label(&self) -> &'static str {
        match self {
            Self::NoPassword => "sudo -n",
            Self::Password(_) => "sudo -S (password asked locally)",
        }
    }

    /// `line` as root: `sudo … -- sh -c '<line>'`.
    pub(in crate::cli) fn wrap(&self, line: &str) -> String {
        let quoted = super::quote(line);
        match self {
            Self::NoPassword => format!("sudo -n -- sh -c {quoted}"),
            Self::Password(_) => format!("sudo -S -p '' -- sh -c {quoted}"),
        }
    }

    /// The bytes that go to the command's input before anything else.
    pub(in crate::cli) fn input_prefix(&self) -> Option<Zeroizing<Vec<u8>>> {
        match self {
            Self::NoPassword => None,
            Self::Password(password) => {
                let mut line = Zeroizing::new(password.as_bytes().to_vec());
                line.push(b'\n');
                Some(line)
            }
        }
    }
}

/// What a short probe command reported.
struct Probe {
    exit: Option<u32>,
    stderr: String,
}

fn probe(backend: &Arc<SftpBackend>, line: &str, input: Option<&[u8]>) -> Result<Probe, String> {
    let mut command = backend
        .clone()
        .start_command(line)
        .map_err(|error| format!("remote command start: {error}"))?;
    let control = command.control();
    if let Some(input) = input {
        control.stdin(input).map_err(|error| error.to_string())?;
    }
    control.eof().map_err(|error| error.to_string())?;
    let mut stderr = Vec::new();
    while let Some(event) = command.next_event() {
        match event {
            SshCommandEvent::Stderr(bytes) if stderr.len() < 8192 => stderr.extend(bytes),
            SshCommandEvent::Finished(end) => {
                return Ok(Probe {
                    exit: end.exit_status,
                    stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
                })
            }
            _ => {}
        }
    }
    Err("the remote command ended without a final status".into())
}

/// Decides how `sudo` runs for `label` (the remote as the user named it);
/// asks for the password when needed. `ask` is the dialog
/// (`exec_password::ask` outside tests).
pub(in crate::cli) fn prepare(
    backend: &Arc<SftpBackend>,
    label: &str,
    ask: &dyn Fn(&str, &str) -> Result<Option<Zeroizing<String>>, String>,
) -> Result<Sudo, String> {
    let first = probe(backend, "sudo -n true", None)?;
    if first.exit == Some(0) {
        return Ok(Sudo::NoPassword);
    }
    if !needs_password(&first.stderr) {
        return Err(refusal(label, &first));
    }
    let password = ask(
        "sudo-Passwort",
        &format!("Passwort des Kontos auf {label} für sudo (ein erhöhter Befehl des KI-Agenten)"),
    )?
    .ok_or_else(|| format!("the sudo password for {label} was not given (dialog cancelled)"))?;
    let mut line = Zeroizing::new(password.as_bytes().to_vec());
    line.push(b'\n');
    let check = probe(backend, "sudo -S -p '' true", Some(&line))?;
    if check.exit != Some(0) {
        return Err(format!(
            "sudo on {label} refused the password: {}",
            if check.stderr.is_empty() {
                "no reason given"
            } else {
                &check.stderr
            }
        ));
    }
    Ok(Sudo::Password(password))
}

/// `sudo -n` fails for want of a password (sudo 1.9 words it „a password is
/// required“; with `requiretty` „a terminal is required“).
fn needs_password(stderr: &str) -> bool {
    stderr.contains("password is required") || stderr.contains("a terminal is required")
}

fn refusal(label: &str, probe: &Probe) -> String {
    let reason = if probe.stderr.is_empty() {
        format!(
            "exit {}",
            probe
                .exit
                .map_or("unknown".to_string(), |code| code.to_string())
        )
    } else {
        probe.stderr.clone()
    };
    format!("elevated commands on {label} run through sudo, and `sudo -n true` failed: {reason}")
}

#[cfg(test)]
mod tests {
    use super::{needs_password, refusal, Probe, Sudo};
    use zeroize::Zeroizing;

    #[test]
    fn shell_elevate_task_sudo_lines_and_password_input() {
        let line = "cd -- '/srv' || exit $?; echo it's";
        assert_eq!(
            Sudo::NoPassword.wrap(line),
            "sudo -n -- sh -c 'cd -- '\\''/srv'\\'' || exit $?; echo it'\\''s'"
        );
        let with = Sudo::Password(Zeroizing::new("geheim".into()));
        assert!(with.wrap("id -u").starts_with("sudo -S -p '' -- sh -c "));
        assert_eq!(with.input_prefix().unwrap().as_slice(), b"geheim\n");
        assert!(Sudo::NoPassword.input_prefix().is_none());
        assert!(needs_password("sudo: a password is required\n"));
        assert!(needs_password(
            "sudo: a terminal is required to read the password"
        ));
        assert!(!needs_password("sh: 1: sudo: not found"));
        let missing = refusal(
            "@prod",
            &Probe {
                exit: Some(127),
                stderr: "sh: 1: sudo: not found".into(),
            },
        );
        assert!(
            missing.contains("@prod") && missing.contains("not found"),
            "{missing}"
        );
    }
}
