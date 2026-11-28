//! Runs one se command with administrator or root rights through
//! `crate::elevate` (UAC on Windows, polkit or `sudo -n` on Linux) and waits
//! for its end; its output is collected for the error text.
use std::collections::BTreeMap;
use std::path::Path;

/// Longest output kept for an error text.
const KEEP: usize = 8 * 1024;

/// Starts `program args` elevated; returns its exit status. `reason` names
/// the change in the error when the user declines.
pub(super) fn run_elevated_program(
    program: &Path,
    args: &[String],
    reason: &str,
) -> Result<i32, String> {
    let spec = crate::elevate::ElevatedSpec {
        command: crate::elevate::ElevatedCommand::Argv {
            program: program.to_string_lossy().into_owned(),
            args: args.to_vec(),
        },
        cwd: None,
        env: BTreeMap::new(),
    };
    let mut child = crate::elevate::spawn_elevated(spec).map_err(|error| match error {
        crate::elevate::ElevateError::Declined => format!("{reason}: im Systemdialog abgelehnt"),
        other => format!("{reason}: {other}"),
    })?;
    let _ = child.input().eof();
    let mut output = Vec::new();
    while let Some(event) = child.next_event() {
        match event {
            crate::elevate::ElevatedEvent::Stdout(bytes)
            | crate::elevate::ElevatedEvent::Stderr(bytes) => {
                if output.len() < KEEP {
                    output.extend_from_slice(&bytes[..bytes.len().min(KEEP - output.len())]);
                }
            }
            crate::elevate::ElevatedEvent::Exited(exit) => {
                if let Some(error) = exit.error {
                    return Err(format!("{reason}: {error}"));
                }
                let code = exit.status().unwrap_or(-1);
                if code != 0 {
                    return Err(format!(
                        "{reason}: Exitcode {code}: {}",
                        String::from_utf8_lossy(&output).trim()
                    ));
                }
                return Ok(0);
            }
        }
    }
    Err(format!("{reason}: der erhöhte Prozess endete ohne Status"))
}
