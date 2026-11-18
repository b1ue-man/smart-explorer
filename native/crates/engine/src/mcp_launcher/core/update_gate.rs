//! Whether new work, and above all a background worker, may start while an
//! update transaction holds the installation. A worker started then comes
//! from the image the update is replacing and discards the update's stop
//! request (field failure 2026-11-18: an MCP runtime of 0.5.199 started the
//! worker again about 20 s after the 0.5.200 installer had stopped it). The
//! messages keep their `update_*` prefix, which the relay, the daemon IPC and
//! the updater's failure texts match.
use super::lifecycle::{Phase, UpdateState};
use super::manifest::Manifest;

/// For an MCP client whose call needs the worker during an update.
pub const UPDATE_RUNNING: &str = "update_in_progress: Smart Explorer wird gerade aktualisiert; \
     der Hintergrunddienst startet erst nach dem Update wieder. Den Aufruf danach wiederholen.";
/// For an MCP client while an interrupted update waits for its recovery.
pub const UPDATE_RECOVERY: &str = "update_recovery_required: ein unterbrochenes Update von \
     Smart Explorer ist offen; der naechste Update-Lauf schliesst es ab oder setzt es zurueck.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartGate {
    Open,
    UpdateRunning,
    RecoveryRequired,
}

/// The record holds the installation: Prepared/Stopping, or a committed
/// retirement whose pointer still names the previous manifest.
pub fn blocks(state: Option<&UpdateState>, current: &Manifest) -> bool {
    state.is_some_and(|state| {
        state.blocks_work()
            || (state.retire_managed
                && state.phase == Phase::Committed
                && *current == state.previous)
    })
}

/// `owner_alive` is asked only for a blocking record.
pub fn gate(blocked: bool, owner_alive: impl FnOnce() -> bool) -> StartGate {
    if !blocked {
        StartGate::Open
    } else if owner_alive() {
        StartGate::UpdateRunning
    } else {
        StartGate::RecoveryRequired
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum WorkerStart {
    Start,
    /// Look again after a short pause.
    WaitAgain,
    Refuse(&'static str),
}

/// A running update is waited for until `limit_reached`; then, and at once
/// for an orphaned record, the start is refused with a readable message.
pub fn worker_start(gate: StartGate, limit_reached: bool) -> WorkerStart {
    match gate {
        StartGate::Open => WorkerStart::Start,
        StartGate::RecoveryRequired => WorkerStart::Refuse(UPDATE_RECOVERY),
        StartGate::UpdateRunning if limit_reached => WorkerStart::Refuse(UPDATE_RUNNING),
        StartGate::UpdateRunning => WorkerStart::WaitAgain,
    }
}

/// An ordinary (non-handoff) worker start keeps the legacy stop request and
/// refuses to run while the update transaction that published it holds the
/// installation; any other request it discards as before.
pub fn ordinary_start_allowed(manual_stop: bool, update_holds: impl FnOnce() -> bool) -> bool {
    !(manual_stop && update_holds())
}

#[cfg(test)]
#[path = "update_gate_tests.rs"]
mod update_gate_tests;
