//! IPC view of a running Direct-code renewal (E17, plan M22): every surface
//! (CLI `se`, desktop, Android, MCP) reads the same values from the Share
//! snapshot (`ShareWorkerSnapshot::code_renewal`).
use serde::{Deserialize, Serialize};

/// Transition state of this device's Direct code. Absent when no renewal
/// runs and nothing failed, and from workers predating it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeRenewalView {
    /// Old data are still valid for the devices that have not switched.
    pub active: bool,
    /// Renewals so far (generation counter).
    pub generation: u64,
    /// Devices that have not yet taken the new data.
    pub pending_devices: usize,
    /// Of these, seen without renewal support (stay on the old data).
    pub legacy_devices: usize,
    /// Display names of pending devices (at most 32).
    pub pending_names: Vec<String>,
    /// Start of the oldest running transition (Unix seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
    /// Another removal waits until a running transition ends.
    #[serde(default)]
    pub deferred: bool,
    /// Last failure of the renewal bookkeeping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl CodeRenewalView {
    /// The view of `identity`, `None` while nothing is to show.
    pub fn of(identity: &crate::share::ShareIdentity) -> Option<Self> {
        let status = identity.code_renewal_status();
        if !status.active && !status.deferred && status.error.is_none() {
            return None;
        }
        Some(Self {
            active: status.active,
            generation: status.generation,
            pending_devices: status.pending_devices,
            legacy_devices: status.legacy_devices,
            pending_names: status.pending_names,
            since: status.since,
            deferred: status.deferred,
            error: status.error,
        })
    }
}
