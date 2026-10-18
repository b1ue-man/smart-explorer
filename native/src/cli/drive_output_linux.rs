//! Linux shapes of `se drive` (DRV-M4, strand S2 C7); every Windows output
//! stays as it was.
//!
//! - `runtime [--json]`: text `fuse\tready\tfusermount3=<path>\tversion=<v>\t
//!   setuid=yes\tdevice=/dev/fuse\tkernel_request_timeout=<s>`; a system that
//!   is not ready prints `fuse\tnot-ready\t…` and one `fuse\tproblem\t<text>`
//!   line per problem; hints follow as `fuse\thint\t<text>` (kernel request
//!   timeout above 0, WSL). JSON: `FuseRuntimeReport`. Exit 0 when ready,
//!   else 1.
//! - `install-runtime [--json]`: se installs no system package:
//!   `{"status":"manual_required","command","reason"}` (exit 2) or
//!   `{"status":"already_ready"}` (exit 0); text `fuse\tmanual-required\t
//!   <command>\t<reason>` or `fuse\talready-ready`.
//! - `mount`: `--letter` other than `auto` and `--system-runtime` are
//!   refused (exit 2); `--mount-point` names the folder.
//! - `list`: column 3 is the folder, the status `mounted:<path>`.
//! - `unmount <SELECTOR> [--force]`: the folder is unmounted first, like a
//!   file manager's eject; a busy drive stays mounted (exit 1) unless
//!   `--force`, which unmounts lazily and aborts the FUSE connection.
//!   Selectors also accept the folder.
use std::path::{Path, PathBuf};

use crate::mount::{
    FuseInstallOutcome, FuseRuntimeReport, MountId, MountRecovery, MountSnapshot, MountStatus,
};

pub(super) const LETTERS_ONLY_ON_WINDOWS: &str =
    "Laufwerksbuchstaben gibt es nur unter Windows; nutze --mount-point";

pub(super) fn runtime(json: bool) -> Result<i32, String> {
    let report = crate::mount::fuse_runtime_report(true);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
        );
    } else {
        println!("{}", runtime_text(&report));
    }
    Ok(if report.ready { 0 } else { 1 })
}

pub(super) fn runtime_text(report: &FuseRuntimeReport) -> String {
    let helper = report.fusermount3.path.as_deref().unwrap_or("-");
    let timeout = report.kernel_request_timeout_s.unwrap_or(0);
    let mut lines = vec![if report.ready {
        format!(
            "fuse\tready\tfusermount3={}\tversion={}\tsetuid=yes\tdevice={}\tkernel_request_timeout={timeout}",
            clean(helper),
            clean(report.fusermount3.version.as_deref().unwrap_or("-")),
            report.device.path
        )
    } else {
        format!(
            "fuse\tnot-ready\tfusermount3={}\tsetuid={}\tdevice={}\taccessible={}\tkernel_request_timeout={timeout}",
            clean(helper),
            yes_no(report.fusermount3.setuid),
            report.device.path,
            yes_no(report.device.accessible)
        )
    }];
    lines.extend(
        report
            .problems
            .iter()
            .map(|problem| format!("fuse\tproblem\t{}", clean(problem))),
    );
    if timeout > 0 {
        lines.push(format!(
            "fuse\thint\tDer Kernel bricht ein Laufwerk ab, wenn eine Anfrage länger als {timeout} s dauert; langes Hochladen beim Schließen kann das auslösen"
        ));
    }
    if report.wsl {
        lines.push("fuse\thint\tUnter WSL zeigt der Windows-Explorer FUSE-Laufwerke nur unter /mnt/wsl; dafür --mount-point /mnt/wsl/<Name> nutzen".into());
    }
    lines.join("\n")
}

pub(super) fn install_runtime(json: bool) -> Result<i32, String> {
    let outcome = crate::mount::fuse_install_outcome();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&outcome).map_err(|error| error.to_string())?
        );
    } else {
        println!("{}", install_text(&outcome));
    }
    Ok(outcome.exit_code())
}

pub(super) fn install_text(outcome: &FuseInstallOutcome) -> String {
    match outcome {
        FuseInstallOutcome::AlreadyReady => "fuse\talready-ready".into(),
        FuseInstallOutcome::ManualRequired { command, reason } => {
            format!(
                "fuse\tmanual-required\t{}\t{}",
                clean(command),
                clean(reason)
            )
        }
    }
}

/// `--letter` and `--system-runtime` have no meaning for a folder drive.
pub(super) fn refuse_windows_options(letter: &str, system_runtime: bool) -> Option<&'static str> {
    (!letter.trim().eq_ignore_ascii_case("auto") || system_runtime)
        .then_some(LETTERS_ONLY_ON_WINDOWS)
}

/// Column 3 of `list`: the folder of the drive, `-` before it has one.
pub(super) fn folder_column(snapshot: &MountSnapshot) -> String {
    let folder = match &snapshot.status {
        MountStatus::MountedAt { path } => Some(path.as_path()),
        _ => snapshot.config.location_directory(),
    };
    folder.map_or_else(|| "-".into(), |folder| clean(&folder.to_string_lossy()))
}

/// A drive selected by its folder, when the selector is an absolute path.
pub(super) fn select_by_folder(selector: &str) -> Result<Option<MountId>, String> {
    let selector = selector.trim();
    if !selector.starts_with('/') {
        return Ok(None);
    }
    let wanted = Path::new(selector.trim_end_matches('/'));
    Ok(crate::daemon::list_mounts()?
        .into_iter()
        .find(|mount| folder_of(mount, true).as_deref() == Some(wanted))
        .map(|mount| mount.config.id))
}

/// Before the daemon stops a Linux drive: its folder is unmounted like a
/// file manager's eject. Returns the drive as it was, for the report.
pub(super) fn release_before_stop(
    id: &MountId,
    force: bool,
) -> Result<Option<MountSnapshot>, String> {
    let Some(before) = crate::daemon::list_mounts()?
        .into_iter()
        .find(|mount| mount.config.id == *id)
    else {
        return Ok(None);
    };
    if let Some(folder) = folder_of(&before, force) {
        crate::mount::unmount_drive_folder(&folder, force)
            .map_err(|error| release_error(&before, &folder, &error))?;
    }
    Ok(Some(before))
}

/// The host may end on the unmount and leave the table before the stop
/// request arrives: then the drive is unmounted, not lost.
pub(super) fn stopped_meanwhile(
    before: Option<MountSnapshot>,
    error: String,
) -> Result<MountSnapshot, String> {
    let Some(before) = before else {
        return Err(error);
    };
    let listed = crate::daemon::list_mounts()?
        .into_iter()
        .any(|mount| mount.config.id == before.config.id);
    if listed {
        return Err(error);
    }
    Ok(MountSnapshot {
        status: MountStatus::Unmounted,
        recovery: MountRecovery::Clean,
        recovery_required_compat: false,
        ..before
    })
}

/// The folder a drive is mounted at; with `any`, also its stored folder.
fn folder_of(snapshot: &MountSnapshot, any: bool) -> Option<PathBuf> {
    match &snapshot.status {
        MountStatus::MountedAt { path } => Some(path.clone()),
        _ if any => snapshot.config.location_directory().map(Path::to_path_buf),
        _ => None,
    }
}

fn release_error(snapshot: &MountSnapshot, folder: &Path, error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::ResourceBusy {
        format!(
            "Laufwerk wird noch benutzt: {} (offene Dateien oder Arbeitsverzeichnisse). Schließe sie oder nutze `se drive unmount --force {}`",
            folder.display(),
            snapshot.config.id
        )
    } else {
        format!("Laufwerk aushängen: {error}")
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

#[cfg(test)]
#[path = "drive_output_linux_tests.rs"]
mod tests;
