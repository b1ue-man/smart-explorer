//! Reads what `core::failure` explains: the update helper's last report and
//! the unfinished update transaction of the installation beside the
//! installed `se`. Changes nothing (the app's start and update page,
//! `se update`, `se update --check`).
use std::path::Path;

use super::config::{peek_updater_error, startup_check_enabled};
use super::failure::{explain, PendingUpdate, UpdateFailure};
use crate::mcp_launcher::core::recovery::{file_states, Resolution};
use crate::mcp_launcher::InterruptedUpdate;

/// The unfinished update transaction of the installation at `root` (the
/// folder of the installed `se`), by what its installed files allow.
pub fn pending_update_at(root: &Path) -> Option<PendingUpdate> {
    let record = root.join("mcp/update.v1.json");
    if !root.join("mcp/current.v1.json").exists() || !record.exists() {
        return None;
    }
    match crate::mcp_launcher::inspect_interrupted_update(root) {
        Ok(found) => found.map(pending),
        // Unreadable counts only for a record that still blocks work: an
        // installation the checks refuse for another reason is no update.
        Err(detail) => unfinished_phase(&record).then_some(PendingUpdate::Unreadable { detail }),
    }
}

/// The record's phase read leniently, for the message only.
fn unfinished_phase(record: &Path) -> bool {
    let phase = std::fs::read(record)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value["phase"].as_str().map(str::to_string));
    matches!(phase.as_deref(), Some("prepared" | "stopping"))
}

fn pending(found: InterruptedUpdate) -> PendingUpdate {
    let previous_version = found.state.previous.product_version.clone();
    let target_version = found.state.target.product_version.clone();
    if found.running {
        return PendingUpdate::Running { target_version };
    }
    match found.resolution {
        Resolution::RollBack => PendingUpdate::RollBack {
            previous_version,
            target_version,
        },
        Resolution::Complete => PendingUpdate::Complete {
            previous_version,
            target_version,
        },
        Resolution::Refuse(entries) => PendingUpdate::Refused {
            detail: file_states(&found.state, &entries),
            previous_version,
            target_version,
        },
    }
}

/// The helper's last failure or an unfinished update of the installation at
/// `root`, explained; `None` when there is neither.
pub fn update_failure_at(root: &Path) -> Option<UpdateFailure> {
    let error = peek_updater_error();
    explain(
        error.as_deref(),
        pending_update_at(root),
        startup_check_enabled(),
    )
}

/// `update_failure_at` for the installation of the running app.
pub fn last_update_failure() -> Option<UpdateFailure> {
    let cli = super::os::installed_cli_path().ok()?;
    update_failure_at(cli.parent()?)
}
