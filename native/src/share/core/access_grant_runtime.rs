//! Access rights at runtime (C5): the restriction each narrowed capability
//! of a changed grant causes, whether the Exec registry keeps the running
//! jobs of a changed grant (a pure widening binds them to the new revision),
//! and the end (`valid_until`) of grants, which ends their sessions with
//! `RestrictionReason::GrantExpired` and the access-log event `grant_expired`.
use std::sync::{Arc, Mutex};

use crate::share::access_audit::{AccessEntry, AccessEvent, AccessKind};
use crate::share::access_authorize::{capability_for, principal_grant};
use crate::share::access_grant::{AccountProfile, Capability};
use crate::share::core::now_secs;
use crate::share::exec_policy::ExecGrant;
use crate::share::exec_registry::{ExecCancelReason, ExecRegistry};
use crate::share::exec_types::ExecId;
use crate::share::relation_proof::ProvenPrincipal;
use crate::share::relation_rights::{
    PrincipalKey, RelationScope, RestrictionReason, RestrictionSet, RightsRestriction,
};
use crate::share::types::{DirectGrant, DirectGrantState, RoomMember, ShareAuthState};

/// One restriction `CapabilityNarrowed` per capability that `candidate`
/// narrows against `current`, for grants whose relation and identity stay;
/// a changed relation or identity has restrictions of its own.
pub(crate) fn capability_restrictions(
    current: &ShareAuthState,
    candidate: &ShareAuthState,
) -> RestrictionSet {
    let mut restrictions = RestrictionSet::default();
    for grant in &current.direct_grants {
        if grant.state != DirectGrantState::Accepted {
            continue;
        }
        let replacement = candidate.direct_grants.iter().find(|new| {
            new.state == DirectGrantState::Accepted && same_direct_identity(grant, new)
        });
        if let Some(new) = replacement {
            push_narrowed(
                &mut restrictions,
                RelationScope::Direct,
                (grant.public_key.as_str(), grant.node_id.as_str()),
                &grant.exec,
                &new.exec,
            );
        }
    }
    for room in current.rooms.iter().filter(|room| room.auto_join) {
        let Some(new_room) = candidate
            .rooms
            .iter()
            .find(|new| new.room_id == room.room_id && new.auto_join)
        else {
            continue;
        };
        for member in room.members.iter().filter(|member| member.is_admitted()) {
            let replacement = new_room
                .members
                .iter()
                .find(|new| new.is_admitted() && same_member_identity(member, new));
            if let Some(new) = replacement {
                push_narrowed(
                    &mut restrictions,
                    RelationScope::Room {
                        room_id: room.room_id.clone(),
                    },
                    (member.public_key.as_str(), member.node_id.as_str()),
                    &member.exec,
                    &new.exec,
                );
            }
        }
    }
    restrictions
}

/// Whether the Exec registry keeps the running jobs of a grant that goes
/// from `old` to `new`: no Exec capability narrows (C5).
pub(crate) fn keeps_exec_jobs(old: &ExecGrant, new: &ExecGrant) -> bool {
    !new.narrowed_against(old)
        .iter()
        .any(|capability| capability.is_exec())
}

/// One restriction `GrantExpired` for every grant of an active relation that
/// allowed something and whose end lies in `(since, now]`.
pub(crate) fn expired_restrictions(state: &ShareAuthState, since: i64, now: i64) -> RestrictionSet {
    let mut restrictions = RestrictionSet::default();
    let ended = |grant: &ExecGrant| {
        grant.grants_any()
            && grant
                .valid_until()
                .is_some_and(|end| end > since && end <= now)
    };
    for grant in &state.direct_grants {
        if grant.state == DirectGrantState::Accepted && ended(&grant.exec) {
            push(
                &mut restrictions,
                RelationScope::Direct,
                (grant.public_key.as_str(), grant.node_id.as_str()),
                RestrictionReason::GrantExpired,
            );
        }
    }
    for room in state.rooms.iter().filter(|room| room.auto_join) {
        for member in &room.members {
            if member.is_admitted() && ended(&member.exec) {
                push(
                    &mut restrictions,
                    RelationScope::Room {
                        room_id: room.room_id.clone(),
                    },
                    (member.public_key.as_str(), member.node_id.as_str()),
                    RestrictionReason::GrantExpired,
                );
            }
        }
    }
    restrictions
}

/// The earliest end after `now` of a grant that allows something.
pub(crate) fn next_expiry(state: &ShareAuthState, now: i64) -> Option<i64> {
    let direct = state
        .direct_grants
        .iter()
        .filter(|grant| grant.state == DirectGrantState::Accepted)
        .map(|grant| &grant.exec);
    let members = state
        .rooms
        .iter()
        .filter(|room| room.auto_join)
        .flat_map(|room| room.members.iter())
        .filter(|member| member.is_admitted())
        .map(|member| &member.exec);
    direct
        .chain(members)
        .filter(|grant| grant.grants_any())
        .filter_map(ExecGrant::valid_until)
        .filter(|end| *end > now)
        .min()
}

/// The end of the grant behind one running Exec job. When it comes, the job
/// ends as revoked and the access log gets `grant_expired` (C5), unless the
/// host moved or removed the end meanwhile.
pub(crate) struct GrantEndWatch {
    auth: Arc<Mutex<ShareAuthState>>,
    principal: ProvenPrincipal,
    capability: Capability,
    end: i64,
    /// Where the `grant_expired` entry goes: the access log; unit tests write
    /// nothing unless they capture it (`with_audit`).
    audit: fn(AccessEntry<'_>),
}

fn ignore_entry(_: AccessEntry<'_>) {}

impl GrantEndWatch {
    pub(crate) fn new(
        auth: Arc<Mutex<ShareAuthState>>,
        principal: ProvenPrincipal,
        capability: Capability,
        end: Option<i64>,
    ) -> Option<Self> {
        let audit: fn(AccessEntry<'_>) = if cfg!(test) {
            ignore_entry
        } else {
            crate::share::access_audit::record_access
        };
        end.map(|end| Self {
            auth,
            principal,
            capability,
            end,
            audit,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_audit(self, audit: fn(AccessEntry<'_>)) -> Self {
        Self { audit, ..self }
    }

    /// Unix seconds (host clock) at which the job is checked.
    pub(crate) fn end(&self) -> i64 {
        self.end
    }

    /// At the end: the watch of a moved end, or `None` once the grant has
    /// no end any more or the job was ended (it then ends as revoked, like
    /// every other Exec job of the principal).
    pub(crate) fn expire(self, registry: &ExecRegistry, exec_id: &ExecId) -> Option<Self> {
        let now = now_secs();
        let Ok(state) = self.auth.lock() else {
            registry.cancel(exec_id, ExecCancelReason::Revoked);
            return None;
        };
        if capability_for(&state, &self.principal, self.capability, now)
            .decision
            .is_on()
        {
            let end = principal_grant(&state, &self.principal).and_then(ExecGrant::valid_until);
            drop(state);
            return end.map(|end| Self { end, ..self });
        }
        let restrictions = expired_restrictions(&state, self.end.saturating_sub(1), now);
        let _ = registry.restrict_authorization(state.authorization_epoch, &restrictions);
        let account =
            principal_grant(&state, &self.principal).map(|grant| grant.access_grant().account);
        drop(state);
        registry.cancel(exec_id, ExecCancelReason::Revoked);
        let reason = format!("exec {exec_id} ended: the grant reached its end");
        (self.audit)(AccessEntry {
            kind: AccessKind::Exec { start: None },
            principal: &self.principal,
            outcome: Some(AccessEvent::GrantExpired),
            capability: self.capability,
            approval: None,
            approval_id: None,
            account_profile: account.as_ref().map(AccountProfile::key),
            account: account.as_ref().and_then(AccountProfile::account_name),
            reason: Some(&reason),
        });
        None
    }
}

fn push_narrowed(
    restrictions: &mut RestrictionSet,
    relation: RelationScope,
    key: (&str, &str),
    old: &ExecGrant,
    new: &ExecGrant,
) {
    for capability in new.narrowed_against(old) {
        push(
            restrictions,
            relation.clone(),
            key,
            RestrictionReason::CapabilityNarrowed(capability),
        );
    }
}

fn push(
    restrictions: &mut RestrictionSet,
    relation: RelationScope,
    (public_key, node_id): (&str, &str),
    reason: RestrictionReason,
) {
    // Old unpinned records cannot identify a session: the whole relation.
    let principal = (!public_key.is_empty() || !node_id.is_empty()).then(|| PrincipalKey {
        public_key: public_key.into(),
        node_id: node_id.into(),
    });
    restrictions.push(RightsRestriction {
        relation,
        principal,
        reason,
    });
}

fn same_direct_identity(old: &DirectGrant, new: &DirectGrant) -> bool {
    old.device_id == new.device_id
        && old.public_key == new.public_key
        && old.fingerprint == new.fingerprint
        && node_preserved(&old.node_id, &new.node_id, &old.public_key)
}

fn same_member_identity(old: &RoomMember, new: &RoomMember) -> bool {
    old.device_id == new.device_id
        && old.public_key == new.public_key
        && old.fingerprint == new.fingerprint
        && node_preserved(&old.node_id, &new.node_id, &old.public_key)
}

/// As in the relation diff: a node pin stays, or an unpinned record gains
/// the node of its key.
fn node_preserved(old: &str, new: &str, public_key: &str) -> bool {
    old == new || (old.is_empty() && new == public_key)
}

#[cfg(test)]
mod expiry_audit_tests {
    use std::sync::{Arc, Mutex};

    use super::GrantEndWatch;
    use crate::share::access_audit::{access_line, AccessEntry};
    use crate::share::access_grant::Capability;
    use crate::share::exec_registry::{ExecRegistry, ExecRegistryLimits};
    use crate::share::exec_types::ExecId;
    use crate::share::identity::ShareIdentity;
    use crate::share::relation_proof::ProvenPrincipal;
    use crate::share::types::ShareAuthState;

    static SEEN: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());

    fn capture(entry: AccessEntry<'_>) {
        SEEN.lock().unwrap().push(access_line(&entry));
    }

    #[test]
    fn grant_end_watch_logs_grant_expired() {
        let secret = iroh::SecretKey::from_bytes(&[7; 32]);
        let node_id = secret.public().to_string();
        let identity = ShareIdentity {
            device_id: "host".into(),
            device_name: "Host".into(),
            direct_lookup_id: "lookup-host".into(),
            public_key: node_id.clone(),
            fingerprint: "fp-host".into(),
            node_id,
            iroh_secret: secret,
            direct_secret: [7; 32],
        };
        // No grant allows anything any more: the end has come.
        let state = ShareAuthState {
            identity,
            direct_secret: b"secret".to_vec(),
            default_direct_exports: Default::default(),
            direct_contacts: Vec::new(),
            direct_grants: Vec::new(),
            rooms: Vec::new(),
            direct_requests: Vec::new(),
            direct_request_tombstones: Vec::new(),
            seen_nonces: Default::default(),
            direct_online: true,
            authorization_epoch: 1,
        };
        let principal = ProvenPrincipal {
            relation_kind: "direct".into(),
            relation_id: String::new(),
            device_id: "client".into(),
            device_name: "Client".into(),
            public_key: "key-client".into(),
            fingerprint: "fp-client".into(),
            node_id: "key-client".into(),
        };
        let watch = GrantEndWatch::new(
            Arc::new(Mutex::new(state)),
            principal,
            Capability::ExecTerminal,
            Some(1),
        )
        .unwrap()
        .with_audit(capture);
        let registry = ExecRegistry::new(ExecRegistryLimits::default());
        let exec_id = ExecId::parse("ab".repeat(16)).unwrap();
        assert!(watch.expire(&registry, &exec_id).is_none());
        let seen = SEEN.lock().unwrap();
        let line = seen.last().expect("no access-log entry");
        assert_eq!(line["event"], "grant_expired");
        assert_eq!(line["kind"], "exec");
        assert_eq!(line["capability"], "exec_terminal");
        assert!(line["reason"].as_str().unwrap().contains(&"ab".repeat(16)));
    }
}
