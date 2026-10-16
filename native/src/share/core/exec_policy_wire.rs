//! Stored form of an [`ExecGrant`] (profile file, grant journal, IPC).
//!
//! Every version up to 0.5.180 reads the profile of schema 8 with exactly the
//! five fields of the old grant and ignores unknown ones. A grant with a
//! program list or another run-as account must therefore never look enabled
//! to such a reader: it would become the full shell of the worker's account
//! and lose the narrowing at its next save. The stored form keeps `enabled`
//! false for such a grant and carries the real switch in `restricted_enabled`,
//! which only this version reads. Unrestricted grants are stored exactly as
//! before, so a profile written by 0.5.180 reads unchanged and a downgrade
//! reads a restricted grant as switched off (fail closed).
use serde::{Deserialize, Serialize};

use super::super::direct_protocol::DirectRequestId;
use super::{ExecGrant, ExecRunAs};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(in crate::share) struct ExecGrantWire {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    policy_revision: u64,
    #[serde(default)]
    changed_at: i64,
    #[serde(default)]
    source_request_id: Option<DirectRequestId>,
    #[serde(default)]
    source_decision_revision: Option<u64>,
    #[serde(default)]
    run_as: ExecRunAs,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    programs: Vec<String>,
    /// Enabled, and narrowed by `run_as` or `programs`: stored with
    /// `enabled` false so that older readers keep it off.
    #[serde(default, skip_serializing_if = "is_false")]
    restricted_enabled: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl From<ExecGrantWire> for ExecGrant {
    fn from(wire: ExecGrantWire) -> Self {
        // `restricted_enabled` counts only next to a real restriction: a
        // stray flag on a full-shell grant never switches it on.
        let restricted = ExecGrant::restricts(&wire.run_as, &wire.programs);
        Self {
            enabled: wire.enabled || (wire.restricted_enabled && restricted),
            policy_revision: wire.policy_revision,
            changed_at: wire.changed_at,
            source_request_id: wire.source_request_id,
            source_decision_revision: wire.source_decision_revision,
            run_as: wire.run_as,
            programs: wire.programs,
        }
    }
}

impl From<ExecGrant> for ExecGrantWire {
    fn from(grant: ExecGrant) -> Self {
        let hidden = grant.enabled && grant.is_restricted();
        Self {
            enabled: grant.enabled && !hidden,
            policy_revision: grant.policy_revision,
            changed_at: grant.changed_at,
            source_request_id: grant.source_request_id,
            source_decision_revision: grant.source_decision_revision,
            run_as: grant.run_as,
            programs: grant.programs,
            restricted_enabled: hidden,
        }
    }
}
