//! Stored form of an [`ExecGrant`] (profile file, grant journal, IPC).
//!
//! Every version up to 0.5.180 reads the profile of schema 8 with exactly the
//! five fields of the old grant and ignores unknown ones. A grant with a
//! program list or another run-as account must therefore never look enabled
//! to such a reader: it would become the full shell of the worker's account
//! and lose the narrowing at its next save. The stored form keeps `enabled`
//! false for such a grant and carries the real switch in `restricted_enabled`,
//! which only 0.5.181 and later read. Unrestricted grants are stored as
//! before, so a profile written by 0.5.180 reads unchanged and a downgrade
//! reads a restricted grant as switched off (fail closed).
//!
//! A grant those fields cannot express (`ask`, an end, the remote account,
//! screen capabilities) also carries the object `access` and is stored with
//! `enabled` false and without `restricted_enabled`, so every reader up to
//! 0.5.182 sees it switched off (`access_grant_wire.rs`).
use serde::{Deserialize, Serialize};

use super::super::access_grant_wire::{self, AccessRecord};
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
    /// The access right when the fields above cannot express it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    access: Option<serde_json::Value>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl From<ExecGrantWire> for ExecGrant {
    fn from(wire: ExecGrantWire) -> Self {
        if let Some(access) = wire.access {
            // `access` decides; `programs` and the revision fields stay.
            let mut grant = Self {
                enabled: false,
                policy_revision: wire.policy_revision,
                changed_at: wire.changed_at,
                source_request_id: wire.source_request_id,
                source_decision_revision: wire.source_decision_revision,
                run_as: wire.run_as,
                programs: wire.programs,
                access: Some(access_grant_wire::parse(access)),
            };
            grant.settle_stored_access();
            return grant;
        }
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
            access: None,
        }
    }
}

impl From<ExecGrant> for ExecGrantWire {
    fn from(grant: ExecGrant) -> Self {
        let access = match &grant.access {
            None => None,
            Some(AccessRecord::Unknown(value)) => Some(value.clone()),
            Some(AccessRecord::Known { kept, .. }) => {
                Some(access_grant_wire::known_value(&grant.access_grant(), kept))
            }
        };
        if access.is_some() {
            return Self {
                enabled: false,
                policy_revision: grant.policy_revision,
                changed_at: grant.changed_at,
                source_request_id: grant.source_request_id,
                source_decision_revision: grant.source_decision_revision,
                run_as: grant.run_as,
                programs: grant.programs,
                restricted_enabled: false,
                access,
            };
        }
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
            access: None,
        }
    }
}
