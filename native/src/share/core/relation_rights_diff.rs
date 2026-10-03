//! Compare authority, rather than presentation or routing snapshots (FA3).
use super::{PrincipalKey, RelationScope, RestrictionReason, RestrictionSet, RightsRestriction};
use crate::share::exec_policy::ExecGrant;
use crate::share::fs::ShareExportConfig;
use crate::share::types::{DirectAccessState, DirectContact, DirectGrantState, ShareAuthState};

/// Only reductions of existing authority close work. Additions, read/write
/// extensions, names, timestamps, status and routes leave existing work alone.
pub(crate) fn authorization_restrictions(
    current: &ShareAuthState,
    candidate: &ShareAuthState,
) -> RestrictionSet {
    if current.identity.device_id != candidate.identity.device_id
        || current.identity.public_key != candidate.identity.public_key
        || current.identity.fingerprint != candidate.identity.fingerprint
        || current.identity.node_id != candidate.identity.node_id
    {
        return RestrictionSet::everything(RestrictionReason::IdentityChanged);
    }
    let mut restrictions = RestrictionSet::default();
    if current.identity.direct_lookup_id != candidate.identity.direct_lookup_id
        || current.direct_secret != candidate.direct_secret
    {
        add(&mut restrictions, RelationScope::Direct, None, RestrictionReason::Reconfirm);
    }
    if current.direct_online && !candidate.direct_online {
        add(&mut restrictions, RelationScope::Direct, None, RestrictionReason::RelationInactive);
    }
    if let Some(reason) = exports_restriction(
        &current.default_direct_exports,
        &candidate.default_direct_exports,
    ) {
        add(&mut restrictions, RelationScope::Direct, None, reason);
    }
    for grant in &current.direct_grants {
        if grant.state != DirectGrantState::Accepted {
            continue;
        }
        let principal = principal(&grant.public_key, &grant.node_id);
        let replacement = candidate.direct_grants.iter().find(|new| new.device_id == grant.device_id);
        let reason = match replacement {
            None => Some(RestrictionReason::Removed),
            Some(new) if new.public_key != grant.public_key
                || new.fingerprint != grant.fingerprint
                || !node_preserves(&grant.node_id, &new.node_id, &grant.public_key) =>
            {
                Some(RestrictionReason::IdentityChanged)
            }
            Some(new) => match new.state {
                DirectGrantState::Ignored => Some(RestrictionReason::Blocked),
                DirectGrantState::Reconfirm => Some(RestrictionReason::Reconfirm),
                DirectGrantState::Accepted if grant.write && !new.write => {
                    Some(RestrictionReason::WriteRevoked)
                }
                DirectGrantState::Accepted if exec_restricted(&grant.exec, &new.exec) => {
                    Some(RestrictionReason::ExecRevoked)
                }
                DirectGrantState::Accepted => None,
            },
        };
        if let Some(reason) = reason {
            add(&mut restrictions, RelationScope::Direct, principal, reason);
        }
    }
    for contact in &current.direct_contacts {
        let replacement = candidate.direct_contacts.iter().find(|new| new.id == contact.id);
        let reason = match replacement {
            None => Some(RestrictionReason::Removed),
            Some(new) if contact_identity_changed(contact, new) => {
                Some(RestrictionReason::IdentityChanged)
            }
            Some(new) if contact.access_state == DirectAccessState::Accepted
                && new.access_state != DirectAccessState::Accepted =>
            {
                Some(RestrictionReason::RelationInactive)
            }
            _ => None,
        };
        if let Some(reason) = reason {
            add(&mut restrictions, RelationScope::Direct, contact_principal(contact), reason);
        }
    }
    for room in &current.rooms {
        if !room.auto_join {
            continue;
        }
        let relation = RelationScope::Room { room_id: room.room_id.clone() };
        let Some(new) = candidate.rooms.iter().find(|new| new.room_id == room.room_id) else {
            add(&mut restrictions, relation, None, RestrictionReason::Removed);
            continue;
        };
        if !new.auto_join {
            add(&mut restrictions, relation, None, RestrictionReason::RelationInactive);
            continue;
        }
        if room.policy.members_may_write && !new.policy.members_may_write {
            add(&mut restrictions, relation.clone(), None, RestrictionReason::WriteRevoked);
        }
        if let Some(reason) = exports_restriction(&room.exports, &new.exports) {
            add(&mut restrictions, relation.clone(), None, reason);
        }
        for member in &room.members {
            if !member.is_admitted() {
                continue;
            }
            let replacement = new.members.iter().find(|new| new.device_id == member.device_id);
            let reason = match replacement {
                None => Some(RestrictionReason::Removed),
                Some(new) if new.public_key != member.public_key
                    || new.fingerprint != member.fingerprint
                    || !node_preserves(&member.node_id, &new.node_id, &member.public_key) =>
                {
                    Some(RestrictionReason::IdentityChanged)
                }
                Some(new) if !new.is_admitted() => Some(RestrictionReason::Blocked),
                Some(new) if exec_restricted(&member.exec, &new.exec) => {
                    Some(RestrictionReason::ExecRevoked)
                }
                _ => None,
            };
            if let Some(reason) = reason {
                add(&mut restrictions, relation.clone(), principal(&member.public_key, &member.node_id), reason);
            }
        }
    }
    restrictions
}

fn add(
    restrictions: &mut RestrictionSet,
    relation: RelationScope,
    principal: Option<PrincipalKey>,
    reason: RestrictionReason,
) {
    restrictions.push(RightsRestriction { relation, principal, reason });
}

fn principal(public_key: &str, node_id: &str) -> Option<PrincipalKey> {
    if public_key.is_empty() && node_id.is_empty() {
        // Old unpinned records cannot identify a stream; close this relation.
        None
    } else {
        Some(PrincipalKey { public_key: public_key.into(), node_id: node_id.into() })
    }
}

fn contact_principal(contact: &DirectContact) -> Option<PrincipalKey> {
    principal(
        contact.remote_public_key.as_deref()
            .or(contact.accepted_public_key.as_deref())
            .unwrap_or_default(),
        &contact.expected_node_id,
    )
}

fn contact_identity_changed(old: &DirectContact, new: &DirectContact) -> bool {
    old.lookup_id != new.lookup_id
        || old.expected_fingerprint != new.expected_fingerprint
        || (!old.expected_node_id.is_empty() && old.expected_node_id != new.expected_node_id)
        || pinned_value_changed(old.remote_device_id.as_deref(), new.remote_device_id.as_deref())
        || pinned_value_changed(old.remote_public_key.as_deref(), new.remote_public_key.as_deref())
        || pinned_value_changed(old.accepted_public_key.as_deref(), new.accepted_public_key.as_deref())
}

fn pinned_value_changed(old: Option<&str>, new: Option<&str>) -> bool {
    old.filter(|value| !value.is_empty()).is_some_and(|old| Some(old) != new)
}

fn node_preserves(old: &str, new: &str, public_key: &str) -> bool {
    old == new || (old.is_empty() && new == public_key)
}

fn exec_restricted(old: &ExecGrant, new: &ExecGrant) -> bool {
    old.enabled && (!new.enabled || old.policy_revision != new.policy_revision)
}

fn exports_restriction(old: &ShareExportConfig, new: &ShareExportConfig) -> Option<RestrictionReason> {
    if old.include_connections && !new.include_connections {
        return Some(RestrictionReason::ExportsNarrowed);
    }
    let mut write_revoked = false;
    for root in &old.roots {
        // Export labels and stored locators retain their exact meaning. A new
        // path is not assumed to contain the former root across backends.
        let Some(replacement) = new.roots.iter().find(|new| new.label == root.label && new.path == root.path) else {
            return Some(RestrictionReason::ExportsNarrowed);
        };
        write_revoked |= root.access.allows_write() && !replacement.access.allows_write()
            || root.allow_system_writes && !replacement.allow_system_writes;
    }
    for connection in &old.shared_connections {
        match new.connection_access(&connection.account) {
            None => return Some(RestrictionReason::ExportsNarrowed),
            Some(access) => write_revoked |= connection.access.allows_write() && !access.allows_write(),
        }
    }
    if old.include_connections {
        write_revoked |= new.shared_connections.iter().any(|connection| {
            old.connection_access(&connection.account).is_some_and(|access| access.allows_write())
                && !connection.access.allows_write()
        });
    }
    write_revoked.then_some(RestrictionReason::WriteRevoked)
}
