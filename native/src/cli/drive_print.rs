//! Text and JSON output of the drive commands of `se drive`, moved out of
//! `drive.rs` without a change (U17, before DRV-M5 and DRV-M6 extend it): one
//! drive as a tab-separated line or as JSON, the status column and the exit
//! code of a status.
use crate::mount::{MountMode, MountSnapshot, MountStatus};

pub(super) fn print_snapshot(snapshot: &MountSnapshot, json: bool) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(snapshot).map_err(|error| error.to_string())?
        );
    } else {
        print_snapshot_text(snapshot);
    }
    Ok(())
}

pub(super) fn print_snapshot_text(snapshot: &MountSnapshot) {
    println!("{}", snapshot_line(snapshot));
}

/// `drive\t<id>\t<letter|auto>\t<mode>\t<status>\t<label>`; under Linux
/// column 3 is the folder (DRV-M4).
pub(super) fn snapshot_line(snapshot: &MountSnapshot) -> String {
    #[cfg(target_os = "linux")]
    let requested = super::drive_output_linux::folder_column(snapshot);
    #[cfg(not(target_os = "linux"))]
    let requested = match snapshot.config.drive {
        crate::mount::DriveSelection::Automatic => "auto".to_string(),
        crate::mount::DriveSelection::Letter(letter) => letter.to_string(),
    };
    let mode = match snapshot.config.mode {
        MountMode::ReadOnly => "read-only",
        MountMode::ReadWrite => "read-write",
    };
    format!(
        "drive\t{}\t{}\t{}\t{}\t{}",
        snapshot.config.id,
        requested,
        mode,
        status_column(snapshot),
        clean(&snapshot.config.label),
    )
}

/// DRV-M7: a vault drive whose vault is locked shows `requires-unlock:<n>`
/// with sealed changes and `failed:vault_locked` without; JSON keeps the
/// full status and `recovery`.
pub(super) fn status_column(snapshot: &MountSnapshot) -> String {
    if let Some(pending) = snapshot.recovery.pending_unlock() {
        return format!("requires-unlock:{pending}");
    }
    match &snapshot.status {
        MountStatus::Failed { detail } if is_vault_locked(detail) => "failed:vault_locked".into(),
        status => status_text(status),
    }
}

pub(super) fn is_vault_locked(detail: &str) -> bool {
    detail
        .strip_prefix(crate::vault::VAULT_LOCKED_CODE)
        .is_some_and(|rest| rest.starts_with(": "))
}

pub(super) fn status_text(status: &MountStatus) -> String {
    match status {
        MountStatus::Unmounted => "unmounted".into(),
        MountStatus::Mounting => "mounting".into(),
        MountStatus::Mounted { drive } => format!("mounted:{drive}"),
        MountStatus::MountedAt { path } => format!("mounted:{}", clean(&path.to_string_lossy())),
        MountStatus::Unmounting => "unmounting".into(),
        MountStatus::RuntimeUnavailable { detail } => {
            format!("runtime-unavailable:{}", clean(detail))
        }
        MountStatus::Conflict { path, detail, .. } => {
            format!("conflict:{}:{}", clean(path), clean(detail))
        }
        MountStatus::Failed { detail } => format!("failed:{}", clean(detail)),
    }
}

pub(super) fn exit_for_status(status: &MountStatus) -> i32 {
    if matches!(
        status,
        MountStatus::Conflict { .. }
            | MountStatus::Failed { .. }
            | MountStatus::RuntimeUnavailable { .. }
    ) {
        1
    } else {
        0
    }
}

pub(super) fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}
