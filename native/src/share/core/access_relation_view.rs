//! Readable authority per exact relationship. Reachable alternatives retain
//! their source and conditions; this view is never an authorization union.
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use crate::share::access_constraints::AccessConstraints;
use crate::share::access_grant::{AccessGrant, AccountProfile, Capability, CapabilityDecision};
use crate::share::access_authorize::principal_grant;
use crate::share::exec_policy::ExecGrant;
use crate::share::fs::ShareExportConfig;
use crate::share::relation_proof::ProvenPrincipal;
use crate::share::types::{DirectGrantState, ExecGrantTarget, ShareAuthState};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessGrantView {
    pub account: AccountProfile,
    pub account_label: String,
    pub capabilities: BTreeMap<Capability, CapabilityDecision>,
    pub valid_until: Option<i64>,
    pub expired: bool,
    #[serde(default = "legacy_version")]
    pub version: u64,
    #[serde(default)]
    pub constraints: AccessConstraints,
    /// V2 deactivates the complete grant in readers that understand only V1.
    #[serde(default)]
    pub legacy_reader_disables_grant: bool,
}
fn legacy_version() -> u64 { 1 }
impl AccessGrantView {
    pub fn new(grant: &AccessGrant, own_label: &str, now: i64) -> Self {
        Self {
            account: grant.account.clone(),
            account_label: grant.account.label(own_label),
            capabilities: grant.capabilities()
                .filter(|(capability, decision)| capability.is_exec() || decision.is_on()).collect(),
            valid_until: grant.valid_until,
            expired: grant.expired_at(now),
            version: grant.version(),
            constraints: grant.constraints.clone(),
            legacy_reader_disables_grant: grant.version() == 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessRelationView {
    pub target: ExecGrantTarget,
    pub relation_kind: String,
    pub relation_id: String,
    pub relation_name: String,
    pub active: bool,
    pub policy_revision: u64,
    pub changed_at: i64,
    pub grant: AccessGrantView,
    pub programs: Vec<String>,
    /// Per-export read/write bounds remain visible alongside this relation's
    /// additional write permission. Provider permissions still apply.
    pub exports: ShareExportConfig,
    pub may_write: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReachableAccess {
    pub capability: Capability,
    pub source: ExecGrantTarget,
    pub decision: CapabilityDecision,
    pub account: AccountProfile,
    pub programs: Vec<String>,
    pub valid_until: Option<i64>,
    pub constraints: AccessConstraints,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAccessView {
    pub relations: Vec<AccessRelationView>,
    /// Alternative routes, with conditions, rather than one maximum rank.
    pub reachable: Vec<ReachableAccess>,
}

/// Every exact grant for one pinned device, including inactive relationships.
/// Matching solely on a display name, or joining different keys, is forbidden.
pub(crate) fn device_access_view(
    state: &ShareAuthState,
    device_id: &str,
    public_key: &str,
    own_label: &str,
    now: i64,
) -> DeviceAccessView {
    let mut view = DeviceAccessView::default();
    for direct in &state.direct_grants {
        if direct.device_id != device_id || direct.public_key != public_key { continue; }
        let principal = ProvenPrincipal {
            relation_kind: "direct".into(), relation_id: state.identity.direct_lookup_id.clone(),
            device_id: direct.device_id.clone(), device_name: direct.device_name.clone(),
            public_key: direct.public_key.clone(), fingerprint: direct.fingerprint.clone(),
            node_id: direct.node_id.clone(),
        };
        let active = direct.state == DirectGrantState::Accepted && principal_grant(state, &principal).is_some();
        append(&mut view, AccessRelationView {
            target: ExecGrantTarget::Direct {
                device_id: direct.device_id.clone(), public_key: direct.public_key.clone(),
                fingerprint: direct.fingerprint.clone(), node_id: direct.node_id.clone(),
            },
            relation_kind: "direct".into(), relation_id: state.identity.direct_lookup_id.clone(),
            relation_name: "Direkt".into(), active,
            policy_revision: direct.exec.policy_revision, changed_at: direct.exec.changed_at,
            grant: AccessGrantView::new(&direct.exec.access_grant(), own_label, now),
            programs: direct.exec.programs.clone(), exports: state.default_direct_exports.clone(),
            may_write: direct.write,
        }, &direct.exec, now);
    }
    for room in &state.rooms {
        for member in &room.members {
            if member.device_id != device_id || member.public_key != public_key { continue; }
            let principal = ProvenPrincipal {
                relation_kind: "room".into(), relation_id: room.room_id.clone(),
                device_id: member.device_id.clone(), device_name: member.device_name.clone(),
                public_key: member.public_key.clone(), fingerprint: member.fingerprint.clone(),
                node_id: member.node_id.clone(),
            };
            append(&mut view, AccessRelationView {
                target: ExecGrantTarget::RoomMember {
                    room_id: room.room_id.clone(), device_id: member.device_id.clone(),
                    public_key: member.public_key.clone(), fingerprint: member.fingerprint.clone(),
                    node_id: member.node_id.clone(),
                },
                relation_kind: "room".into(), relation_id: room.room_id.clone(),
                relation_name: room.name.clone(), active: principal_grant(state, &principal).is_some(),
                policy_revision: member.exec.policy_revision, changed_at: member.exec.changed_at,
                grant: AccessGrantView::new(&member.exec.access_grant(), own_label, now),
                programs: member.exec.programs.clone(), exports: room.exports.clone(),
                may_write: room.policy.members_may_write,
            }, &member.exec, now);
        }
    }
    view
}
fn append(view: &mut DeviceAccessView, relation: AccessRelationView, policy: &ExecGrant, now: i64) {
    let access = policy.access_grant();
    if relation.active && !access.expired_at(now) {
        for (capability, decision) in access.capabilities().filter(|(capability, decision)| {
            decision.is_on() && access.constraints.available_at(*capability, now)
        }) {
            view.reachable.push(ReachableAccess {
                capability, source: relation.target.clone(), decision, account: access.account.clone(),
                programs: policy.programs.clone(), valid_until: access.valid_until,
                constraints: access.constraints.clone(),
            });
        }
    }
    view.relations.push(relation);
}

impl crate::share::ShareService {
    /// Authoritative worker snapshot, rather than a UI-side union of profiles.
    pub(crate) fn device_access_view(
        &self, device_id: &str, public_key: &str, own_label: &str, now: i64,
    ) -> Result<DeviceAccessView, String> {
        let state = self.auth.lock().map_err(|_| "Share authorization state is locked".to_string())?;
        Ok(device_access_view(&state, device_id, public_key, own_label, now))
    }
}
