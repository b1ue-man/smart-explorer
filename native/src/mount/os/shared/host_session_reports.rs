//! Status reports of the shared drive-host flow (`host_session.rs`,
//! DRV-M2), moved out of it without a change of behaviour (DRV-M7) before
//! the vault drives extended the flow: the running status of a volume, the
//! recovery text, the failure reports and the journal audit.
use std::io;

use crate::mount::{EntryCondition, MountEngine, MountRecovery, MountStatus};

use super::super::host_platform::VolumeLocation;
use super::HostControl;

pub(super) fn mounted_status(location: VolumeLocation) -> MountStatus {
    match location {
        VolumeLocation::Drive(drive) => MountStatus::Mounted { drive },
        VolumeLocation::Directory(path) => MountStatus::MountedAt { path },
    }
}

pub(super) fn conflict_status(
    location: VolumeLocation,
    first: &(String, EntryCondition),
) -> MountStatus {
    let (path, condition) = first;
    let detail = match condition {
        EntryCondition::Conflict(conflict) => conflict.detail.clone(),
        EntryCondition::Dirty => "a cached change still requires safe remote recovery".into(),
        EntryCondition::Clean => "mounted-drive recovery is incomplete".into(),
    };
    match location {
        VolumeLocation::Drive(drive) => MountStatus::Conflict {
            drive,
            path: path.clone(),
            detail,
        },
        // `Conflict` names a drive letter; a folder drive reports where it
        // is, and the journal keeps the conflict for the unmount report.
        VolumeLocation::Directory(path) => MountStatus::MountedAt { path },
    }
}

pub(super) fn recovery_detail(entries: &[(String, EntryCondition)]) -> String {
    let dirty = entries
        .iter()
        .filter(|(_, condition)| matches!(condition, EntryCondition::Dirty))
        .count();
    let conflicts = entries
        .iter()
        .filter(|(_, condition)| matches!(condition, EntryCondition::Conflict(_)))
        .count();
    format!(
        "{} cached change(s) remain after unmount ({} dirty, {} conflict); keep the mount and use Retry",
        entries.len(),
        dirty,
        conflicts
    )
}

pub(super) fn report_failure(control: &dyn HostControl, message: &str) -> String {
    let _ = control.report(
        MountStatus::Failed {
            detail: message.to_string(),
        },
        None,
    );
    message.to_string()
}

pub(super) fn report_failure_with_recovery(
    control: &dyn HostControl,
    message: &str,
    recovery: MountRecovery,
) -> String {
    let _ = control.report(
        MountStatus::Failed {
            detail: message.to_string(),
        },
        Some(recovery),
    );
    message.to_string()
}

/// A failure before the volume exists: the recovery value says whether the
/// journal still owns changes.
pub(super) fn report_engine_failure(
    control: &dyn HostControl,
    engine: &MountEngine,
    message: &str,
) -> String {
    let recovery = match engine.dirty_entries() {
        Ok(entries) if entries.is_empty() => MountRecovery::Clean,
        Ok(_) => MountRecovery::Required,
        Err(_) => MountRecovery::Unknown,
    };
    report_failure_with_recovery(control, message, recovery)
}

pub(super) fn inspect_recovery(engine: &MountEngine) -> io::Result<MountRecovery> {
    engine.dirty_entries().map(|entries| {
        if entries.is_empty() {
            MountRecovery::Clean
        } else {
            MountRecovery::Required
        }
    })
}
