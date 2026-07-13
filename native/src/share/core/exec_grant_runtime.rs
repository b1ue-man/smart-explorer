use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::core::eio;
use super::exec_policy::ExecGrant;
use super::exec_registry::ExecRegistry;
use super::exec_types::ExecPrincipal;
use super::profiles::fingerprint_matches;
use super::types::{DirectGrantState, ExecGrantTarget, RoomProfile, ShareAuthState};

#[path = "exec_grant_persistence.rs"]
mod persistence;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecGrantMutation {
    pub target: ExecGrantTarget,
    pub principal: ExecPrincipal,
    pub policy: ExecGrant,
    pub authorization_epoch: u64,
}

pub(super) fn mutate(
    auth: &Arc<Mutex<ShareAuthState>>,
    registry: &ExecRegistry,
    target: ExecGrantTarget,
    enabled: bool,
    changed_at: i64,
) -> io::Result<ExecGrantMutation> {
    let mut state = auth
        .lock()
        .map_err(|_| eio("Share Exec authorization state is locked"))?;
    let next_epoch = state
        .authorization_epoch
        .checked_add(1)
        .ok_or_else(|| eio("Share authorization epoch exhausted"))?;
    let relation_id = state.identity.direct_lookup_id.clone();
    let (principal, policy) = resolve_exact_policy(&mut state, &target, &relation_id, true)?;
    let mut next_policy = policy.clone();
    next_policy
        .set_runtime_enabled(enabled, changed_at)
        .map_err(eio)?;

    // Registry mutation happens while new handshakes are excluded by the auth
    // lock. On disable this establishes the deny barrier and cancellation
    // before the authoritative auth snapshot is changed.
    registry
        .apply_authorization(&principal, next_policy.policy_revision, next_epoch, enabled)
        .map_err(eio)?;
    *policy = next_policy.clone();
    state.authorization_epoch = next_epoch;
    Ok(ExecGrantMutation {
        target,
        principal,
        policy: next_policy,
        authorization_epoch: next_epoch,
    })
}

pub(super) fn seed_registry(
    auth: &Arc<Mutex<ShareAuthState>>,
    registry: &ExecRegistry,
) -> io::Result<()> {
    let state = auth
        .lock()
        .map_err(|_| eio("Share Exec authorization state is locked"))?;
    for policy in effective_policies(&state) {
        registry
            .apply_authorization(
                &policy.principal,
                policy.revision,
                state.authorization_epoch,
                policy.enabled,
            )
            .map_err(eio)?;
    }
    Ok(())
}

pub(super) fn apply_exact(
    auth: &Arc<Mutex<ShareAuthState>>,
    registry: &ExecRegistry,
    target: ExecGrantTarget,
    expected_principal: ExecPrincipal,
    policy: ExecGrant,
) -> io::Result<ExecGrantMutation> {
    let mut state = auth
        .lock()
        .map_err(|_| eio("Share Exec authorization state is locked"))?;
    let next_epoch = state
        .authorization_epoch
        .checked_add(1)
        .ok_or_else(|| eio("Share authorization epoch exhausted"))?;
    let relation_id = state.identity.direct_lookup_id.clone();
    let (principal, current) =
        resolve_exact_policy(&mut state, &target, &relation_id, policy.enabled)?;
    if principal != expected_principal || policy.policy_revision < current.policy_revision {
        return Err(denied(
            "persisted Exec grant no longer matches the exact identity",
        ));
    }
    registry
        .apply_authorization(
            &principal,
            policy.policy_revision,
            next_epoch,
            policy.enabled,
        )
        .map_err(eio)?;
    *current = policy.clone();
    state.authorization_epoch = next_epoch;
    Ok(ExecGrantMutation {
        target,
        principal,
        policy,
        authorization_epoch: next_epoch,
    })
}

/// Applies a complete configuration transition to the runtime registry before
/// the caller replaces `current` under the same auth lock. Any partial failure
/// can only remove authority; it cannot make the uncommitted candidate usable.
pub(super) fn apply_configuration_transition(
    current: &ShareAuthState,
    candidate: &ShareAuthState,
    next_epoch: u64,
    registry: &ExecRegistry,
) -> io::Result<()> {
    let old = effective_policies(current)
        .into_iter()
        .map(|policy| (principal_key(&policy.principal), policy))
        .collect::<HashMap<_, _>>();
    let new = effective_policies(candidate)
        .into_iter()
        .map(|policy| (principal_key(&policy.principal), policy))
        .collect::<HashMap<_, _>>();

    for (key, old_policy) in &old {
        let replacement = new.get(key);
        if replacement.is_none_or(|policy| !policy.enabled) {
            let revision = replacement
                .map(|policy| policy.revision)
                .unwrap_or_else(|| old_policy.revision.saturating_add(1));
            registry
                .apply_authorization(&old_policy.principal, revision, next_epoch, false)
                .map_err(eio)?;
        }
    }
    for policy in new.values() {
        registry
            .apply_authorization(
                &policy.principal,
                policy.revision,
                next_epoch,
                policy.enabled,
            )
            .map_err(eio)?;
    }
    Ok(())
}

fn resolve_exact_policy<'a>(
    state: &'a mut ShareAuthState,
    target: &ExecGrantTarget,
    direct_relation_id: &str,
    require_active: bool,
) -> io::Result<(ExecPrincipal, &'a mut ExecGrant)> {
    match target {
        ExecGrantTarget::Direct {
            device_id,
            public_key,
            fingerprint,
            node_id,
        } => {
            let grant = state
                .direct_grants
                .iter_mut()
                .find(|grant| {
                    &grant.device_id == device_id
                        && &grant.public_key == public_key
                        && &grant.fingerprint == fingerprint
                        && &grant.node_id == node_id
                })
                .ok_or_else(|| denied("exact direct authorization grant not found"))?;
            if (require_active && grant.state != DirectGrantState::Accepted) || !valid_pin(grant) {
                return Err(denied("direct authorization is not active"));
            }
            let principal = ExecPrincipal {
                relation_kind: "direct".into(),
                relation_id: direct_relation_id.to_string(),
                device_id: grant.device_id.clone(),
                device_name: grant.device_name.clone(),
                public_key: grant.public_key.clone(),
                fingerprint: grant.fingerprint.clone(),
                node_id: grant.node_id.clone(),
            };
            Ok((principal, &mut grant.exec))
        }
        ExecGrantTarget::RoomMember {
            room_id,
            device_id,
            public_key,
            fingerprint,
            node_id,
        } => {
            let room = state
                .rooms
                .iter_mut()
                .find(|room| &room.room_id == room_id)
                .ok_or_else(|| denied("room not found"))?;
            if require_active && !room.auto_join {
                return Err(denied("room authorization is inactive"));
            }
            let relation_id = room.room_id.clone();
            let member = room
                .members
                .iter_mut()
                .find(|member| {
                    &member.device_id == device_id
                        && &member.public_key == public_key
                        && &member.fingerprint == fingerprint
                        && &member.node_id == node_id
                })
                .ok_or_else(|| denied("room member not found"))?;
            if (require_active && member.blocked)
                || member.public_key != member.node_id
                || !fingerprint_matches(&member.public_key, &member.fingerprint)
            {
                return Err(denied("room member identity is not authorized"));
            }
            let principal = ExecPrincipal {
                relation_kind: "room".into(),
                relation_id,
                device_id: member.device_id.clone(),
                device_name: member.device_name.clone(),
                public_key: member.public_key.clone(),
                fingerprint: member.fingerprint.clone(),
                node_id: member.node_id.clone(),
            };
            Ok((principal, &mut member.exec))
        }
    }
}

struct EffectivePolicy {
    principal: ExecPrincipal,
    revision: u64,
    enabled: bool,
}

fn effective_policies(state: &ShareAuthState) -> Vec<EffectivePolicy> {
    let mut policies = Vec::new();
    for grant in &state.direct_grants {
        if !valid_pin(grant) {
            continue;
        }
        policies.push(EffectivePolicy {
            principal: ExecPrincipal {
                relation_kind: "direct".into(),
                relation_id: state.identity.direct_lookup_id.clone(),
                device_id: grant.device_id.clone(),
                device_name: grant.device_name.clone(),
                public_key: grant.public_key.clone(),
                fingerprint: grant.fingerprint.clone(),
                node_id: grant.node_id.clone(),
            },
            revision: grant.exec.policy_revision,
            enabled: state.direct_online
                && grant.state == DirectGrantState::Accepted
                && grant.exec.enabled,
        });
    }
    for room in &state.rooms {
        append_room_policies(room, &mut policies);
    }
    policies
}

fn append_room_policies(room: &RoomProfile, policies: &mut Vec<EffectivePolicy>) {
    for member in &room.members {
        if member.public_key != member.node_id
            || !fingerprint_matches(&member.public_key, &member.fingerprint)
        {
            continue;
        }
        policies.push(EffectivePolicy {
            principal: ExecPrincipal {
                relation_kind: "room".into(),
                relation_id: room.room_id.clone(),
                device_id: member.device_id.clone(),
                device_name: member.device_name.clone(),
                public_key: member.public_key.clone(),
                fingerprint: member.fingerprint.clone(),
                node_id: member.node_id.clone(),
            },
            revision: member.exec.policy_revision,
            enabled: room.auto_join && !member.blocked && member.exec.enabled,
        });
    }
}

fn valid_pin(grant: &super::types::DirectGrant) -> bool {
    grant.public_key == grant.node_id && fingerprint_matches(&grant.public_key, &grant.fingerprint)
}

fn principal_key(principal: &ExecPrincipal) -> String {
    format!(
        "{}\0{}\0{}\0{}\0{}\0{}",
        principal.relation_kind,
        principal.relation_id,
        principal.device_id,
        principal.public_key,
        principal.fingerprint,
        principal.node_id
    )
}

fn denied(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

#[cfg(test)]
#[path = "exec_grant_runtime_tests.rs"]
mod tests;
