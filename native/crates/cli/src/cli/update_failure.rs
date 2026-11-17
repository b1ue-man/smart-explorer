//! `se update`: a failed or unfinished earlier update, with its reason and
//! what happens next (the same texts as the app's update page). An update
//! that cannot resume the unfinished one fails here, before staging, with a
//! non-zero exit instead of handing a doomed run to the detached helper.
use crate::updater::{Installation, PendingUpdate, UpdateFailure};

/// The earlier failure or unfinished update of `installation`, if any.
pub(super) fn current(installation: &Installation) -> Option<UpdateFailure> {
    crate::updater::update_failure_at(installation.cli().parent()?)
}

/// The text lines of `se update` for `failure`; `previous_update_error`
/// keeps its earlier form.
pub(super) fn lines(failure: Option<&UpdateFailure>) -> Vec<String> {
    let Some(failure) = failure else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    if let Some(error) = &failure.error {
        lines.push(format!("previous_update_error\t{}", clean(error)));
    }
    lines.push(format!(
        "previous_update_reason\t{}",
        clean(&failure.reason)
    ));
    lines.push(format!(
        "previous_update_next\t{}",
        clean(&failure.next_action)
    ));
    if let Some(pending) = &failure.pending {
        lines.push(format!("pending_update\t{}", pending_line(pending)));
    }
    lines
}

fn pending_line(pending: &PendingUpdate) -> String {
    match pending {
        PendingUpdate::Running { target_version } => format!("running\tv{target_version}"),
        PendingUpdate::RollBack {
            previous_version,
            target_version,
        } => format!("roll_back\tv{target_version} -> v{previous_version}"),
        PendingUpdate::Complete {
            previous_version,
            target_version,
        } => format!("complete\tv{previous_version} -> v{target_version}"),
        PendingUpdate::Refused {
            previous_version,
            target_version,
            detail,
        } => format!(
            "refused\tv{previous_version} -> v{target_version}\t{}",
            clean(detail)
        ),
        PendingUpdate::Unreadable { detail } => format!("unreadable\t{}", clean(detail)),
    }
}

/// Refuses an update that the helper would refuse anyway: the unfinished
/// earlier update has no automatic way on, its state is unreadable, or its
/// updater still runs. A rollback or completion is left to the helper.
pub(super) fn preflight(installation: &Installation) -> Result<(), String> {
    let Some(root) = installation.cli().parent() else {
        return Ok(());
    };
    match crate::updater::pending_update_at(root) {
        Some(
            pending @ (PendingUpdate::Refused { .. }
            | PendingUpdate::Running { .. }
            | PendingUpdate::Unreadable { .. }),
        ) => Err(refused(&pending)),
        _ => Ok(()),
    }
}

fn refused(pending: &PendingUpdate) -> String {
    format!(
        "update not started: an earlier update is unfinished ({})\n{}",
        pending_line(pending).replace('\t', " "),
        crate::updater::update_next_action(Some(pending), false)
    )
}

/// An error of this run with the German reason when it carries a known code.
pub(super) fn explained(error: String) -> String {
    match crate::updater::known_update_failure_reason(&error) {
        Some(reason) => format!("{error}\n{reason}"),
        None => error,
    }
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}
