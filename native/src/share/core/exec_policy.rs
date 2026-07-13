use serde::{Deserialize, Serialize};

use super::direct_protocol::DirectRequestId;
use super::profiles::ShareProfiles;

/// Local full-code-execution policy for one exact pinned peer identity.
///
/// File access never implies execution access. New and migrated policies are
/// disabled, and identity or base-authorization changes advance the local
/// revision while returning the policy to deny.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecGrant {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub policy_revision: u64,
    #[serde(default)]
    pub changed_at: i64,
    #[serde(default)]
    pub source_request_id: Option<DirectRequestId>,
    #[serde(default)]
    pub source_decision_revision: Option<u64>,
}

impl ExecGrant {
    pub(crate) fn set_runtime_enabled(
        &mut self,
        enabled: bool,
        changed_at: i64,
    ) -> Result<(), &'static str> {
        self.policy_revision = self
            .policy_revision
            .checked_add(1)
            .ok_or("Exec policy revision exhausted")?;
        self.enabled = enabled;
        self.changed_at = changed_at;
        self.source_request_id = None;
        self.source_decision_revision = None;
        Ok(())
    }

    pub(crate) fn reset_for_identity_change(&mut self, changed_at: i64) {
        self.disable(changed_at, None, None);
    }

    pub(crate) fn disable_for_base_decision(
        &mut self,
        changed_at: i64,
        request_id: DirectRequestId,
        decision_revision: u64,
    ) {
        self.disable(changed_at, Some(request_id), Some(decision_revision));
    }

    pub(crate) fn disable_without_decision(&mut self, changed_at: i64) {
        self.disable(changed_at, None, None);
    }

    fn disable(
        &mut self,
        changed_at: i64,
        source_request_id: Option<DirectRequestId>,
        source_decision_revision: Option<u64>,
    ) {
        self.enabled = false;
        self.policy_revision = self.policy_revision.saturating_add(1);
        self.changed_at = changed_at;
        self.source_request_id = source_request_id;
        self.source_decision_revision = source_decision_revision;
    }
}

/// Legacy profiles used one export-wide Exec bit. It was never a safe grant,
/// so migration must ignore it and deny every exact peer independently.
pub(super) fn reset_all_for_legacy_migration(profiles: &mut ShareProfiles) {
    for grant in &mut profiles.direct_grants {
        grant.exec = ExecGrant::default();
    }
    for room in &mut profiles.rooms {
        for member in &mut room.members {
            member.exec = ExecGrant::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ExecGrant;
    use crate::share::DirectRequestId;

    #[test]
    fn policy_is_default_deny_and_disable_advances_revision() {
        let mut policy = ExecGrant::default();
        assert!(!policy.enabled);
        assert_eq!(policy.policy_revision, 0);

        policy.enabled = true;
        policy.disable_without_decision(17);
        assert!(!policy.enabled);
        assert_eq!(policy.policy_revision, 1);
        assert_eq!(policy.changed_at, 17);
        assert!(policy.source_request_id.is_none());
    }

    #[test]
    fn base_decision_source_is_recorded_when_disabling() {
        let request_id = DirectRequestId::parse("01234567-89ab-4def-8123-456789abcdef").unwrap();
        let mut policy = ExecGrant {
            enabled: true,
            ..ExecGrant::default()
        };
        policy.disable_for_base_decision(29, request_id.clone(), 4);
        assert!(!policy.enabled);
        assert_eq!(policy.source_request_id, Some(request_id));
        assert_eq!(policy.source_decision_revision, Some(4));
    }

    #[test]
    fn runtime_change_is_monotonic_and_clears_base_decision_source() {
        let request_id = DirectRequestId::parse("01234567-89ab-4def-8123-456789abcdef").unwrap();
        let mut policy = ExecGrant {
            source_request_id: Some(request_id),
            source_decision_revision: Some(3),
            ..ExecGrant::default()
        };
        policy.set_runtime_enabled(true, 41).unwrap();
        assert!(policy.enabled);
        assert_eq!(policy.policy_revision, 1);
        assert_eq!(policy.changed_at, 41);
        assert!(policy.source_request_id.is_none());
        assert!(policy.source_decision_revision.is_none());
    }
}
