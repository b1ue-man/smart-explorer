//! Renewal of this device's Direct relation data (lookup id and Direct
//! secret) without losing a contact (E17).
//!
//! Removing a contact leaves the removed device with this device's Direct
//! code. A renewal gives the remaining contacts new data over their pinned
//! sessions, while the old data keep working for exactly the identities that
//! still hold a grant until each of them has confirmed the new data. Older
//! peers that never confirm keep the old data for good: nobody is lost.
//!
//! This module is the pure state machine. The durable file and the secure
//! store live in `os/shared/code_renewal_store.rs`, the per-process view in
//! `code_renewal_registry.rs`.
use serde::{Deserialize, Serialize};

/// Lookup ids one registered client of the share server may publish at the
/// same time (`share-server/src/limits.rs`, `MAX_PUBLISHED_DIRECTS_PER_CLIENT`;
/// refused in `share-server/src/direct_presence.rs` from 64 on). The engine
/// does not depend on the server crate, so the value is mirrored here.
pub const SERVER_PUBLISHED_DIRECTS_PER_CLIENT: usize = 64;
/// At most this many generations run out at the same time. The current lookup
/// id holds one of the server's published slots, so the retiring ids can use
/// the other 63: `64 - 1`. One more removal waits until one of them has ended
/// (`RenewalNeed::Deferred`).
pub const MAX_RETIRING_CODES: usize = SERVER_PUBLISHED_DIRECTS_PER_CLIENT - 1;
/// Pending devices of one retiring generation (grants of this device).
pub const MAX_RENEWAL_PEERS: usize = 1024;
/// Names a status shows at most.
pub const MAX_STATUS_NAMES: usize = 32;
const STATE_VERSION: u32 = 1;

/// One device that still has to confirm the newer data.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenewalPeer {
    pub device_id: String,
    #[serde(default)]
    pub device_name: String,
    pub public_key: String,
    #[serde(default)]
    pub node_id: String,
    /// Seen on the old data without renewal support (an older version, or
    /// a peer that has not yet seen the renewal presence).
    #[serde(default)]
    pub legacy: bool,
}

impl RenewalPeer {
    /// Same device, key and node pin (an empty pin names the key's node).
    pub fn is(&self, device_id: &str, public_key: &str, node_id: &str) -> bool {
        !device_id.is_empty()
            && self.device_id == device_id
            && self.public_key == public_key
            && (self.node_id == node_id || (self.node_id.is_empty() && public_key == node_id))
    }
}

/// A lookup id that runs out: valid only for its pending devices' grants.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetiringCode {
    pub lookup_id: String,
    pub generation: u64,
    pub started_at: i64,
    #[serde(default)]
    pub pending: Vec<RenewalPeer>,
}

/// First half of a renewal: the new data exist, the identity file may not
/// yet name them (two-phase write, see `settle`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreparedRenewal {
    pub old_lookup_id: String,
    pub new_lookup_id: String,
    #[serde(default)]
    pub covered_removals_at: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeRenewalState {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub generation: u64,
    /// The latest removal a renewal has answered; `None` before the first
    /// run of this version (then history is not renewed retroactively).
    #[serde(default)]
    pub covered_removals_at: Option<i64>,
    #[serde(default)]
    pub retiring: Vec<RetiringCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepared: Option<PreparedRenewal>,
}

/// What the latest removal requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenewalNeed {
    Nothing,
    /// First run: remember the latest removal without renewing.
    Initialize(i64),
    /// A removal after the last renewal: renew now.
    Renew(i64),
    /// A renewal is due, but `MAX_RETIRING_CODES` generations still run out.
    Deferred,
}

/// Lookup ids whose old secret is to be deleted after a state change.
pub type RetiredLookups = Vec<String>;

/// Display state for every surface (CLI, desktop, Android, MCP, IPC).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeRenewalStatus {
    /// A transition runs: old data are still valid for pending devices.
    pub active: bool,
    pub generation: u64,
    /// Devices that have not yet confirmed the new data.
    pub pending_devices: usize,
    /// Of these, seen without renewal support (stay on the old data).
    pub legacy_devices: usize,
    pub pending_names: Vec<String>,
    /// Earliest start of a running transition (Unix seconds).
    pub since: Option<i64>,
    /// A further renewal waits for a running one to end.
    pub deferred: bool,
    /// Last failure of the renewal bookkeeping, if any.
    pub error: Option<String>,
}

impl CodeRenewalState {
    pub fn need(&self, latest_removal: Option<i64>) -> RenewalNeed {
        let latest = latest_removal.unwrap_or(0);
        match self.covered_removals_at {
            None => RenewalNeed::Initialize(latest),
            Some(covered) if latest <= covered => RenewalNeed::Nothing,
            Some(_) if self.retiring.len() >= MAX_RETIRING_CODES => RenewalNeed::Deferred,
            Some(_) => RenewalNeed::Renew(latest),
        }
    }

    pub fn initialize(&mut self, latest: i64) {
        self.version = STATE_VERSION;
        self.covered_removals_at = Some(latest);
    }

    /// Phase one of T1/T1m: the current data `old_lookup_id` start to run out
    /// for `holders` (every grant of this device that may come back).
    /// Holders still pending in an older generation stay there: they hold
    /// that older data, not `old_lookup_id`.
    pub fn prepare(
        &mut self,
        old_lookup_id: &str,
        new_lookup_id: &str,
        holders: Vec<RenewalPeer>,
        covered_removals_at: Option<i64>,
        now: i64,
    ) {
        self.version = STATE_VERSION;
        self.generation = self.generation.saturating_add(1);
        let mut pending: Vec<RenewalPeer> = Vec::new();
        for holder in holders {
            let elsewhere = self.retiring.iter().any(|code| {
                code.pending
                    .iter()
                    .any(|peer| peer.is(&holder.device_id, &holder.public_key, &holder.node_id))
            });
            let duplicate = pending
                .iter()
                .any(|peer| peer.is(&holder.device_id, &holder.public_key, &holder.node_id));
            if !elsewhere && !duplicate && pending.len() < MAX_RENEWAL_PEERS {
                pending.push(RenewalPeer {
                    legacy: false,
                    ..holder
                });
            }
        }
        self.retiring.retain(|code| code.lookup_id != old_lookup_id);
        self.retiring.push(RetiringCode {
            lookup_id: old_lookup_id.to_string(),
            generation: self.generation,
            started_at: now,
            pending,
        });
        self.prepared = Some(PreparedRenewal {
            old_lookup_id: old_lookup_id.to_string(),
            new_lookup_id: new_lookup_id.to_string(),
            covered_removals_at,
        });
    }

    /// Phase two, run on every load: completes or rolls back a prepared
    /// renewal against the lookup id the identity file names, and drops any
    /// entry that names the current data. Returns the lookup ids whose
    /// secret is to be deleted (a rolled-back new secret, an old secret
    /// nobody waits for).
    pub fn settle(&mut self, current_lookup_id: &str) -> RetiredLookups {
        let mut retired = Vec::new();
        if let Some(prepared) = self.prepared.take() {
            if current_lookup_id == prepared.new_lookup_id {
                if prepared.covered_removals_at.is_some() {
                    self.covered_removals_at = prepared.covered_removals_at;
                }
            } else if current_lookup_id == prepared.old_lookup_id {
                // The identity file was not replaced: the new secret is an
                // orphan and the old data are still the current ones.
                retired.push(prepared.new_lookup_id);
            }
        }
        self.retiring
            .retain(|code| code.lookup_id != current_lookup_id);
        retired.extend(self.take_finished());
        retired
    }

    /// T4: pending devices whose grant is gone end; generations nobody waits
    /// for are finished.
    pub fn prune(&mut self, holds_grant: impl Fn(&RenewalPeer) -> bool) -> RetiredLookups {
        for code in &mut self.retiring {
            code.pending.retain(|peer| holds_grant(peer));
        }
        self.take_finished()
    }

    /// T2: the device now holds `accepted_lookup_id`. Removes it from its
    /// generation when that is the current data, or moves it to the younger
    /// generation it took over; any other lookup id changes nothing.
    pub fn confirm(
        &mut self,
        device_id: &str,
        public_key: &str,
        node_id: &str,
        accepted_lookup_id: &str,
        current_lookup_id: &str,
    ) -> RetiredLookups {
        let target = self
            .retiring
            .iter()
            .position(|code| code.lookup_id == accepted_lookup_id);
        if target.is_none() && accepted_lookup_id != current_lookup_id {
            return Vec::new();
        }
        let Some(from) = self.retiring.iter().position(|code| {
            code.pending
                .iter()
                .any(|peer| peer.is(device_id, public_key, node_id))
        }) else {
            return Vec::new();
        };
        let from_generation = self.retiring[from].generation;
        if let Some(target) = target {
            if self.retiring[target].generation <= from_generation {
                return Vec::new();
            }
        }
        let index = self.retiring[from]
            .pending
            .iter()
            .position(|peer| peer.is(device_id, public_key, node_id));
        let Some(index) = index else {
            return Vec::new();
        };
        let mut peer = self.retiring[from].pending.remove(index);
        peer.legacy = false;
        if let Some(target) = target {
            if self.retiring[target].pending.len() < MAX_RENEWAL_PEERS {
                self.retiring[target].pending.push(peer);
            }
        }
        self.take_finished()
    }

    /// T3: the device used `lookup_id` without renewal support.
    pub fn mark_legacy(
        &mut self,
        device_id: &str,
        public_key: &str,
        node_id: &str,
        lookup_id: &str,
    ) -> bool {
        let Some(code) = self
            .retiring
            .iter_mut()
            .find(|code| code.lookup_id == lookup_id)
        else {
            return false;
        };
        let Some(peer) = code
            .pending
            .iter_mut()
            .find(|peer| peer.is(device_id, public_key, node_id))
        else {
            return false;
        };
        let changed = !peer.legacy;
        peer.legacy = true;
        changed
    }

    pub fn status(&self, deferred: bool, error: Option<String>) -> CodeRenewalStatus {
        let pending = self.retiring.iter().flat_map(|code| code.pending.iter());
        let mut names = Vec::new();
        let mut count = 0;
        let mut legacy = 0;
        for peer in pending {
            count += 1;
            legacy += usize::from(peer.legacy);
            if names.len() < MAX_STATUS_NAMES {
                let name = if peer.device_name.trim().is_empty() {
                    peer.device_id.clone()
                } else {
                    peer.device_name.clone()
                };
                names.push(name);
            }
        }
        CodeRenewalStatus {
            active: !self.retiring.is_empty(),
            generation: self.generation,
            pending_devices: count,
            legacy_devices: legacy,
            pending_names: names,
            since: self.retiring.iter().map(|code| code.started_at).min(),
            deferred,
            error,
        }
    }

    fn take_finished(&mut self) -> RetiredLookups {
        let mut retired = Vec::new();
        self.retiring.retain(|code| {
            if code.pending.is_empty() {
                retired.push(code.lookup_id.clone());
                false
            } else {
                true
            }
        });
        retired
    }
}

#[cfg(test)]
#[path = "code_renewal_task_fall2_tests.rs"]
mod task_fall2_tests;
#[cfg(test)]
#[path = "code_renewal_task_fall3_tests.rs"]
mod task_fall3_tests;
#[cfg(test)]
#[path = "code_renewal_task_fall5_tests.rs"]
mod task_fall5_tests;
#[cfg(test)]
#[path = "code_renewal_grenze_task_tests.rs"]
mod task_grenze_tests;
#[cfg(test)]
#[path = "code_renewal_task_tests.rs"]
mod task_tests;
