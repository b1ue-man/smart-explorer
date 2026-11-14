//! The polkit path of the Linux RDP self-setup (design 3.3): a non-root
//! service runs its fixed root step through `pkexec <se> rdp host-setup
//! --system <step>`, bundled with the login step of the same trigger (E44),
//! only on an administrator-owned se, and reads the child's `state=` line or
//! pkexec's documented exit codes.
use super::{find_tool, has_display, last_line, run_unbounded, SystemStep};
use crate::rdp::core::SetupState;
use std::path::Path;

/// Administrator-owned file in administrator-owned directories without
/// group/world write access (as `client_trust::trusted_system_path`).
fn root_owned(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let path = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    for ancestor in path.ancestors() {
        let metadata = std::fs::metadata(ancestor).map_err(|e| e.to_string())?;
        if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            return Err(format!("{} is not administrator-owned", ancestor.display()));
        }
    }
    Ok(())
}

/// The result of `pkexec <se> rdp host-setup --system <step>`: the child's
/// `state=` line when it ran; otherwise pkexec's documented exit codes
/// (126 dialog dismissed, 127 not authorized or no agent).
pub fn pkexec_outcome(code: Option<i32>, stdout: &str, stderr: &str) -> SetupState {
    let reported = stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("state="))
        .and_then(|state| SetupState::parse(state).ok());
    if let Some(state) = reported {
        return state;
    }
    match code {
        Some(126) => SetupState::Declined,
        Some(127) if stderr.contains("No authentication agent") => {
            SetupState::Failed("no_polkit_agent".into())
        }
        Some(127) if stderr.contains("Not authorized") => SetupState::Declined,
        Some(code) => {
            let detail = last_line(stderr.as_bytes());
            SetupState::Failed(format!("pkexec_exit_{code}: {detail}"))
        }
        None => SetupState::Failed("pkexec_ended_by_signal".into()),
    }
}

/// The root step through the elevation bundle (E44): one `pkexec <se> rdp
/// host-setup --system <login> --system <step>` also carries the login step
/// of the same trigger. This setup's checks (display, pkexec, an
/// administrator-owned `se`) run before the step is handed in; its launcher
/// starts the call. The `state=` line is this step's own result.
pub(super) fn via_pkexec(step: SystemStep) -> SetupState {
    use crate::rdp::host_setup::elevated::{self, Launched, Party};
    if !has_display() {
        return SetupState::Failed("no_interactive_session".into());
    }
    let Some(pkexec) = find_tool(&["pkexec"]) else {
        return SetupState::Impossible("polkit_missing".into());
    };
    let se = match crate::updater::installed_cli_path() {
        Ok(se) => se,
        Err(error) => return SetupState::Failed(format!("se_path: {error}")),
    };
    if root_owned(&se).is_err() {
        return SetupState::Impossible("se_binary_not_root_owned".into());
    }
    let Some(se) = se.to_str().map(str::to_string) else {
        return SetupState::Impossible("se_binary_not_root_owned".into());
    };
    let launcher: elevated::Launcher = Box::new(move |steps: &[String]| {
        let mut args = vec![se];
        args.extend(elevated::system_args(steps));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        crate::rdp::host_setup::prompt_follows();
        let output = run_unbounded(&pkexec, &args, &[])?;
        Ok(Launched {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    });
    match elevated::submit(Party::Rdp, step.name().into(), launcher) {
        (call, Ok(launched)) => {
            // A failed login step before it leaves the code unknown; the
            // `state=` line still tells (`pkexec_outcome` reads it first).
            let code = elevated::step_code(&call, Party::Rdp, launched.code);
            let reported = launched
                .stdout
                .lines()
                .any(|line| line.trim().starts_with("state="));
            if code.is_none() && launched.code.is_some() && !reported {
                return SetupState::Failed("bundled_step_unreported".into());
            }
            pkexec_outcome(code, &launched.stdout, &launched.stderr)
        }
        (_, Err(error)) => SetupState::Failed(error),
    }
}
