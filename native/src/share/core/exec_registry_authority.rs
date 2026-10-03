//! Per-principal barriers keep unrelated launch reservations alive (B04/FA3).
use super::{
    cancel_matching, ExecCancelReason, ExecPrincipal, ExecRegistry, ExecRegistryError,
    PolicyAuthority, PrincipalIdentity,
};
use crate::share::relation_rights::RestrictionSet;

impl ExecRegistry {
    pub(crate) fn apply_authorization(
        &self,
        principal: &ExecPrincipal,
        revision: u64,
        epoch: u64,
        enabled: bool,
    ) -> Result<(), ExecRegistryError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if epoch < state.authorization_epoch {
            return Err(ExecRegistryError::StaleAuthorization);
        }
        let identity = PrincipalIdentity::from(principal);
        let changed = match state.policies.get(&identity) {
            Some(current) => {
                if revision < current.revision
                    || (revision == current.revision && enabled && !current.enabled)
                {
                    return Err(ExecRegistryError::StaleAuthorization);
                }
                current.revision != revision || current.enabled != enabled
            }
            None => true,
        };
        let minimum_epoch = if changed {
            epoch
        } else {
            state.policies.get(&identity).map_or(epoch, |policy| policy.minimum_epoch)
        };
        state.authorization_epoch = epoch;
        state.policies.insert(identity.clone(), PolicyAuthority {
            revision,
            enabled,
            minimum_epoch,
        });
        if changed || !enabled {
            cancel_matching(
                &mut state,
                |job| PrincipalIdentity::from(&job.lease.principal) == identity,
                ExecCancelReason::Revoked,
            );
        }
        Ok(())
    }

    /// Apply the reduction before replacing the auth snapshot under its lock.
    /// Filesystem restrictions also terminate the affected principal's Exec
    /// jobs even when its separate Exec grant remains enabled. Stale launch
    /// tokens cannot pass the new barrier; unaffected tokens keep their epoch.
    pub(crate) fn restrict_authorization(
        &self,
        epoch: u64,
        restrictions: &RestrictionSet,
    ) -> Result<(), ExecRegistryError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if epoch < state.authorization_epoch {
            return Err(ExecRegistryError::StaleAuthorization);
        }
        state.authorization_epoch = epoch;
        for (principal, policy) in &mut state.policies {
            if restrictions.affects(&principal.0, &principal.1, &principal.3, &principal.5) {
                policy.minimum_epoch = epoch;
            }
        }
        cancel_matching(
            &mut state,
            |job| {
                let principal = &job.lease.principal;
                restrictions.affects(
                    &principal.relation_kind,
                    &principal.relation_id,
                    &principal.public_key,
                    &principal.node_id,
                )
            },
            ExecCancelReason::Revoked,
        );
        Ok(())
    }
}
