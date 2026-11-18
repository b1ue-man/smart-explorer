//! Runs the login service installer of the self-setup (E21) and keeps the
//! last line it wrote to its error output: the status names the installer's
//! own reason (`self_plan::state_for_failure`) instead of only „der
//! Einrichtungsschritt des Betriebssystems ist fehlgeschlagen“. Every line
//! also goes on to this process's error output (journal, log), unchanged.
use crate::login::core::self_plan::{installer_detail, script_detail};
use std::{
    io::{self, BufRead, BufReader, Write},
    process::{Command, Stdio},
};

/// The installer's exit code and its last non-empty error line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptOutcome {
    pub code: i32,
    pub detail: Option<String>,
}

/// Starts `command` with its error output piped, forwards each line to this
/// process's error output and waits for the end. Standard output stays as
/// the caller set it (inherited: the elevated `se` step prints there).
pub fn run(command: &mut Command) -> io::Result<ScriptOutcome> {
    let mut child = command.stderr(Stdio::piped()).spawn()?;
    // The last two non-empty lines, oldest first (`installer_detail`).
    let mut recent: Vec<String> = Vec::with_capacity(2);
    if let Some(stderr) = child.stderr.take() {
        let mut forward = io::stderr();
        for line in BufReader::new(stderr).split(b'\n') {
            let Ok(line) = line else {
                break;
            };
            let text = String::from_utf8_lossy(&line);
            let _ = writeln!(forward, "{}", text.trim_end_matches('\r'));
            if let Some(found) = script_detail(&text) {
                if recent.len() == 2 {
                    recent.remove(0);
                }
                recent.push(found);
            }
        }
    }
    let status = child.wait()?;
    Ok(ScriptOutcome {
        code: status.code().unwrap_or(-1),
        detail: installer_detail(&recent),
    })
}
