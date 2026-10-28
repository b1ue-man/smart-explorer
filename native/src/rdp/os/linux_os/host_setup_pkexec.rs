//! The polkit path of the Linux RDP self-setup (design 3.3): a non-root
//! service runs one fixed root step through `pkexec <se> rdp host-setup
//! --system <step>`, only on an administrator-owned se, and reads the
//! child's `state=` line or pkexec's documented exit codes.
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
pub(crate) fn pkexec_outcome(code: Option<i32>, stdout: &str, stderr: &str) -> SetupState {
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

pub(super) fn via_pkexec(step: SystemStep) -> SetupState {
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
    let Some(se) = se.to_str() else {
        return SetupState::Impossible("se_binary_not_root_owned".into());
    };
    let args = [se, "rdp", "host-setup", "--system", step.name()];
    crate::rdp::host_setup::prompt_follows();
    match run_unbounded(&pkexec, &args, &[]) {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            pkexec_outcome(output.status.code(), &stdout, &stderr)
        }
        Err(error) => SetupState::Failed(error),
    }
}
