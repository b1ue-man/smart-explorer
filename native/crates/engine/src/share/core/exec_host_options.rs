//! Two options of an Exec start that only hosts announcing them get
//! (plan docs/plaene/2026-11-28-ersetzen-masken-shell, part C, rows
//! „Erhöhung entfernt“ and „`exec`/`shell` unter Share-Arbeitsziel“):
//!
//! - `share_cwd`: the working directory as a Share path (`/Docs/proj`). The
//!   host resolves it in the exports the calling relation sees, exactly as a
//!   file request (`fs::resolve`: inside the share, no link out of it) and
//!   runs the command in that folder on its disk. This closes TODO
//!   SUCHE-EXEC-SHARE-CWD: before, the caller had to know the peer's disk
//!   path.
//! - `elevated`: the host's se asks its own user for administrator or root
//!   rights (UAC, polkit) and runs the command with them. The contained job
//!   runs `se elevated run … -- <command>` (`engine::elevate`): ending the
//!   job ends that wrapper, whose helper then ends the elevated tree.
//!
//! A client never sends an option to a host without its feature; it fails
//! the start with a clear error instead (an older host would drop the
//! unknown field and then refuse the start digest).
use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use crate::share::exec_client::{ExecClientFailure, ExecClientFailureKind};
use crate::share::exec_client_messages::failure;
use crate::share::exec_protocol::ExecWireError;
use crate::share::exec_supervisor_protocol::RunAsAccount;
use crate::share::exec_types::{ExecCommand, ExecPrincipal, ExecStart};
use crate::share::export_config::ShareExportConfig;
use crate::share::types::ShareAuthState;

/// Hello feature of a host that resolves `ExecStart::share_cwd`.
pub const EXEC_SHARE_CWD_FEATURE: &str = "exec_share_cwd_v1";
/// Hello feature of a host that runs `ExecStart::elevated` starts.
pub const EXEC_ELEVATED_FEATURE: &str = "exec_elevated_v1";
/// Error code of a start whose Share folder the host cannot use.
pub const SHARE_CWD_INVALID: &str = "share_cwd_invalid";
/// Error code of an option the host does not know or cannot serve.
pub const OPTION_UNSUPPORTED: &str = "exec_option_unsupported";
/// Longest Share path of a working directory.
const MAX_SHARE_CWD_BYTES: usize = 4096;

/// The checks of `ExecStart::validate` for both options.
pub(in crate::share) fn validate(start: &ExecStart) -> io::Result<()> {
    if let Some(share_cwd) = &start.share_cwd {
        if start.cwd.is_some() {
            return Err(invalid(
                "give either a working directory or a Share folder, not both",
            ));
        }
        if share_cwd.as_bytes().contains(&0) || share_cwd.len() > MAX_SHARE_CWD_BYTES {
            return Err(invalid(
                "the Share folder of the working directory is invalid",
            ));
        }
        if !share_cwd.starts_with('/') || share_cwd.trim_matches('/').is_empty() {
            return Err(invalid(
                "the Share folder of the working directory must name a shared folder",
            ));
        }
    }
    if start.elevated && start.terminal.is_some() {
        return Err(invalid("an elevated start runs without a terminal"));
    }
    Ok(())
}

/// The features this host lists for both options: every host resolves Share
/// folders; elevation exists where `engine::elevate` does (Linux, Windows).
pub(in crate::share) fn host_features() -> Vec<String> {
    let mut features = vec![EXEC_SHARE_CWD_FEATURE.to_string()];
    if elevation_host() {
        features.push(EXEC_ELEVATED_FEATURE.to_string());
    }
    features
}

fn elevation_host() -> bool {
    matches!(std::env::consts::OS, "linux" | "windows")
}

/// Client side: the start may go to a host with `features`.
pub(in crate::share) fn require_host_options(
    start: &ExecStart,
    features: &[String],
) -> Result<(), ExecClientFailure> {
    let has = |feature: &str| features.iter().any(|name| name == feature);
    if start.share_cwd.is_some() && !has(EXEC_SHARE_CWD_FEATURE) {
        return Err(failure(
            ExecClientFailureKind::Remote,
            OPTION_UNSUPPORTED,
            "the peer's Smart Explorer cannot start a command in a Share folder; update it, or pass the peer's own path as `cwd`",
            false,
        ));
    }
    if start.elevated && !has(EXEC_ELEVATED_FEATURE) {
        return Err(failure(
            ExecClientFailureKind::Remote,
            OPTION_UNSUPPORTED,
            "the peer's Smart Explorer cannot run commands elevated (it is older, or an Android device); update it",
            false,
        ));
    }
    Ok(())
}

/// Host side, before the job: `share_cwd` becomes `cwd`, the disk folder of
/// that Share path in the exports the caller's relation sees.
pub(in crate::share) fn resolve_on_host(
    mut start: ExecStart,
    auth: &Arc<Mutex<ShareAuthState>>,
    principal: &ExecPrincipal,
) -> Result<ExecStart, ExecWireError> {
    if start.elevated && !elevation_host() {
        return Err(wire_error(
            OPTION_UNSUPPORTED,
            "this device cannot run commands elevated".into(),
        ));
    }
    let Some(share_cwd) = start.share_cwd.take() else {
        return Ok(start);
    };
    let exports = {
        let state = auth
            .lock()
            .map_err(|_| wire_error(SHARE_CWD_INVALID, "Share-Auth gesperrt".into()))?;
        relation_exports(&state, principal)?
    };
    start.cwd = Some(share_folder_on_disk(&share_cwd, exports)?);
    Ok(start)
}

/// The disk folder of the Share path `share_cwd` in `exports`.
fn share_folder_on_disk(
    share_cwd: &str,
    exports: ShareExportConfig,
) -> Result<String, ExecWireError> {
    let exports = Arc::new(Mutex::new(exports));
    let target = crate::share::fs::resolve(share_cwd, &exports)
        .map_err(|error| wire_error(SHARE_CWD_INVALID, format!("{share_cwd}: {error}")))?;
    if !target.backend.is_local() {
        return Err(wire_error(
            SHARE_CWD_INVALID,
            format!("{share_cwd} is no folder on this device's disks; pass `cwd` instead"),
        ));
    }
    let disk = crate::share::fs_local_paths::to_os_path(&target.path);
    if !disk.is_dir() {
        return Err(wire_error(
            SHARE_CWD_INVALID,
            format!("{share_cwd} is no folder on this device"),
        ));
    }
    Ok(disk.to_string_lossy().into_owned())
}

/// The exports a relation sees (`SessionAuthorization`): Direct the device
/// folders of the caller in the Direct exports, a room its exports.
fn relation_exports(
    state: &ShareAuthState,
    principal: &ExecPrincipal,
) -> Result<ShareExportConfig, ExecWireError> {
    match principal.relation_kind.as_str() {
        "direct" => Ok(state
            .default_direct_exports
            .visible_to(&principal.device_id, &principal.public_key)),
        "room" => state
            .rooms
            .iter()
            .find(|room| room.room_id == principal.relation_id)
            .map(|room| room.exports.clone())
            .ok_or_else(|| wire_error(SHARE_CWD_INVALID, "Unbekannter Raum".into())),
        other => Err(wire_error(
            SHARE_CWD_INVALID,
            format!("unknown relation kind {other}"),
        )),
    }
}

/// Host side, in the commit after the shell was decided: an elevated start
/// becomes `se elevated run [--cwd D] [--env N=V]… (--cmd-line L | -- P A…)`
/// of the se beside this program. `os` is `std::env::consts::OS`.
pub(in crate::share) fn wrap_elevated(
    request: &mut ExecStart,
    environment: &BTreeMap<String, String>,
    account: Option<&RunAsAccount>,
    os: &str,
) -> io::Result<()> {
    if !request.elevated {
        return Ok(());
    }
    if account.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "an elevated start runs as this device's own user; the grant names another account",
        ));
    }
    let se = std::env::current_exe()?.with_file_name(format!("se{}", exe_suffix(os)));
    let cwd = request.cwd.clone().or_else(|| {
        ["HOME", "USERPROFILE"]
            .iter()
            .find_map(|name| environment.get(*name).filter(|value| !value.is_empty()))
            .cloned()
    });
    let mut args = vec!["elevated".to_string(), "run".to_string()];
    if let Some(cwd) = cwd {
        args.extend(["--cwd".to_string(), cwd]);
    }
    for (name, value) in &request.env {
        args.extend(["--env".to_string(), format!("{name}={value}")]);
    }
    match &request.command {
        ExecCommand::Argv {
            program,
            args: rest,
        } => {
            args.push("--".into());
            args.push(program.clone());
            args.extend(rest.iter().cloned());
        }
        ExecCommand::Shell { command } if os == "windows" => {
            args.extend(["--cmd-line".to_string(), command.clone()]);
        }
        ExecCommand::Shell { command } => {
            let shell = environment
                .get("SHELL")
                .filter(|shell| shell.starts_with('/'))
                .cloned()
                .unwrap_or_else(|| "/bin/sh".to_string());
            args.extend(["--".to_string(), shell, "-c".to_string(), command.clone()]);
        }
        ExecCommand::Login => return Err(invalid("an elevated start runs without a terminal")),
    }
    request.command = ExecCommand::Argv {
        program: se.to_string_lossy().into_owned(),
        args,
    };
    request.shell = None;
    request.elevated = false;
    Ok(())
}

fn exe_suffix(os: &str) -> &'static str {
    if os == "windows" {
        ".exe"
    } else {
        ""
    }
}

fn wire_error(code: &str, message: String) -> ExecWireError {
    ExecWireError {
        code: code.to_string(),
        message,
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
#[path = "exec_host_options_tests.rs"]
mod tests;
