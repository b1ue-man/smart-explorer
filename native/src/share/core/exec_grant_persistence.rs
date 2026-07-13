use super::super::exec_policy::ExecGrant;
use super::super::exec_types::ExecPrincipal;
use super::super::identity::ShareIdentity;
use super::super::profiles::{fingerprint_matches, ShareProfiles};
use super::super::types::{DirectGrantState, ExecGrantTarget};
use super::ExecGrantMutation;

impl ExecGrantMutation {
    pub(crate) fn prepare_persisted(
        profiles: &mut ShareProfiles,
        identity: &ShareIdentity,
        target: ExecGrantTarget,
        enabled: bool,
        changed_at: i64,
    ) -> Result<(Self, u64), String> {
        let (principal, current) = resolve_profile_policy(profiles, identity, &target, enabled)?;
        let expected_revision = current.policy_revision;
        let mut policy = current.clone();
        policy
            .set_runtime_enabled(enabled, changed_at)
            .map_err(str::to_string)?;
        Ok((
            Self {
                target,
                principal,
                policy,
                authorization_epoch: 0,
            },
            expected_revision,
        ))
    }

    pub(crate) fn apply_persisted_cas(
        &self,
        profiles: &mut ShareProfiles,
        identity: &ShareIdentity,
        expected_revision: u64,
    ) -> Result<(), String> {
        self.validate_persisted_shape(expected_revision)?;
        let (principal, current) =
            resolve_profile_policy(profiles, identity, &self.target, self.policy.enabled)?;
        if principal != self.principal {
            return Err("Exec-Grant identity pins changed concurrently; reload and retry".into());
        }
        if *current == self.policy {
            return Ok(());
        }
        if current.policy_revision != expected_revision {
            return Err(format!(
                "Exec-Grant revision changed concurrently: expected {expected_revision}, found {}",
                current.policy_revision
            ));
        }
        *current = self.policy.clone();
        Ok(())
    }

    pub(crate) fn mask_pending_policy(
        &self,
        profiles: &mut ShareProfiles,
        identity: &ShareIdentity,
        expected_revision: u64,
    ) -> Result<(), String> {
        self.validate_persisted_shape(expected_revision)?;
        let (principal, current) = resolve_profile_policy(profiles, identity, &self.target, false)?;
        if principal != self.principal {
            return Err("pending Exec-Grant identity pins no longer match".into());
        }
        if current.policy_revision != expected_revision
            && current.policy_revision != self.policy.policy_revision
        {
            return Err(format!(
                "pending Exec-Grant revision mismatch: expected {expected_revision} or {}, found {}",
                self.policy.policy_revision, current.policy_revision
            ));
        }
        current.enabled = false;
        Ok(())
    }

    pub(crate) fn mask_all_policies(profiles: &mut ShareProfiles) {
        for grant in &mut profiles.direct_grants {
            grant.exec.enabled = false;
        }
        for room in &mut profiles.rooms {
            for member in &mut room.members {
                member.exec.enabled = false;
            }
        }
    }

    pub(crate) fn validate_persisted_shape(&self, expected_revision: u64) -> Result<(), String> {
        if self.authorization_epoch != 0 {
            return Err("persisted Exec-Grant must not contain a runtime epoch".into());
        }
        let desired_revision = expected_revision
            .checked_add(1)
            .ok_or_else(|| "Exec policy revision exhausted".to_string())?;
        if self.policy.policy_revision != desired_revision {
            return Err(format!(
                "invalid persisted Exec-Grant revision: expected {desired_revision}, found {}",
                self.policy.policy_revision
            ));
        }
        if self.principal.device_id.trim().is_empty()
            || self.principal.public_key.trim().is_empty()
            || self.principal.fingerprint.trim().is_empty()
            || self.principal.node_id.trim().is_empty()
        {
            return Err("persisted Exec-Grant contains an incomplete principal".into());
        }
        let exact_target = match &self.target {
            ExecGrantTarget::Direct {
                device_id,
                public_key,
                fingerprint,
                node_id,
            } => {
                self.principal.relation_kind == "direct"
                    && self.principal.device_id == *device_id
                    && self.principal.public_key == *public_key
                    && self.principal.fingerprint == *fingerprint
                    && self.principal.node_id == *node_id
            }
            ExecGrantTarget::RoomMember {
                room_id,
                device_id,
                public_key,
                fingerprint,
                node_id,
            } => {
                self.principal.relation_kind == "room"
                    && self.principal.relation_id == *room_id
                    && self.principal.device_id == *device_id
                    && self.principal.public_key == *public_key
                    && self.principal.fingerprint == *fingerprint
                    && self.principal.node_id == *node_id
            }
        };
        if !exact_target {
            return Err("persisted Exec-Grant target does not match its exact principal".into());
        }
        Ok(())
    }
}

fn resolve_profile_policy<'a>(
    profiles: &'a mut ShareProfiles,
    identity: &ShareIdentity,
    target: &ExecGrantTarget,
    require_active: bool,
) -> Result<(ExecPrincipal, &'a mut ExecGrant), String> {
    match target {
        ExecGrantTarget::Direct {
            device_id,
            public_key,
            fingerprint,
            node_id,
        } => {
            let grant = profiles
                .direct_grants
                .iter_mut()
                .find(|grant| {
                    &grant.device_id == device_id
                        && &grant.public_key == public_key
                        && &grant.fingerprint == fingerprint
                        && &grant.node_id == node_id
                })
                .ok_or_else(|| "exact direct authorization grant not found".to_string())?;
            if (require_active && grant.state != DirectGrantState::Accepted)
                || !valid_pin(&grant.public_key, &grant.node_id, &grant.fingerprint)
            {
                return Err("direct authorization is not active".into());
            }
            let principal = ExecPrincipal {
                relation_kind: "direct".into(),
                relation_id: identity.direct_lookup_id.clone(),
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
            let room = profiles
                .rooms
                .iter_mut()
                .find(|room| &room.room_id == room_id)
                .ok_or_else(|| "room not found".to_string())?;
            if require_active && !room.auto_join {
                return Err("room authorization is inactive".into());
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
                .ok_or_else(|| "room member not found".to_string())?;
            if (require_active && member.blocked)
                || !valid_pin(&member.public_key, &member.node_id, &member.fingerprint)
            {
                return Err("room member identity is not authorized".into());
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

fn valid_pin(public_key: &str, node_id: &str, fingerprint: &str) -> bool {
    public_key == node_id && fingerprint_matches(public_key, fingerprint)
}

#[cfg(test)]
#[path = "exec_grant_persistence_tests.rs"]
mod tests;
