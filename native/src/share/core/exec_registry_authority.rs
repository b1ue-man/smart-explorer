//! Per-principal barriers keep unrelated launch reservations alive (B04/FA3).
use super::{
    cancel_matching, ExecCancelReason, ExecPrincipal, ExecRegistry, ExecRegistryError,
    PolicyAuthority, PrincipalIdentity, RegistryState,
};
use crate::share::relation_rights::RestrictionSet;
use std::collections::HashSet;

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
        validate_policy(&state, principal, revision, enabled)?;
        install_policy(&mut state, principal, revision, epoch, enabled);
        Ok(())
    }

    /// Validate the complete snapshot before advancing the deny barrier. A
    /// stale policy must not leave the registry ahead of the unpublished auth
    /// snapshot, poisoning all subsequent unrelated configuration refreshes.
    pub(crate) fn apply_configuration_authorizations(
        &self,
        epoch: u64,
        restrictions: &RestrictionSet,
        updates: &[(ExecPrincipal, u64, bool)],
    ) -> Result<(), ExecRegistryError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if epoch < state.authorization_epoch {
            return Err(ExecRegistryError::StaleAuthorization);
        }
        let mut identities = HashSet::new();
        for (principal, revision, enabled) in updates {
            if !identities.insert(PrincipalIdentity::from(principal)) {
                return Err(ExecRegistryError::InvalidAuthorization);
            }
            validate_policy(&state, principal, *revision, *enabled)?;
        }
        restrict_state(&mut state, epoch, restrictions);
        for (principal, revision, enabled) in updates {
            install_policy(&mut state, principal, *revision, epoch, *enabled);
        }
        Ok(())
    }

    /// Apply reductions before replacing the auth snapshot under its lock.
    pub(crate) fn restrict_authorization(
        &self,
        epoch: u64,
        restrictions: &RestrictionSet,
    ) -> Result<(), ExecRegistryError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if epoch < state.authorization_epoch {
            return Err(ExecRegistryError::StaleAuthorization);
        }
        restrict_state(&mut state, epoch, restrictions);
        Ok(())
    }
}

fn validate_policy(
    state: &RegistryState,
    principal: &ExecPrincipal,
    revision: u64,
    enabled: bool,
) -> Result<(), ExecRegistryError> {
    if let Some(current) = state.policies.get(&PrincipalIdentity::from(principal)) {
        if revision < current.revision
            || (revision == current.revision && enabled && !current.enabled)
        {
            return Err(ExecRegistryError::StaleAuthorization);
        }
    }
    Ok(())
}

fn install_policy(
    state: &mut RegistryState,
    principal: &ExecPrincipal,
    revision: u64,
    epoch: u64,
    enabled: bool,
) {
    let identity = PrincipalIdentity::from(principal);
    let changed = state
        .policies
        .get(&identity)
        .is_none_or(|current| current.revision != revision || current.enabled != enabled);
    let minimum_epoch = if changed {
        epoch
    } else {
        state
            .policies
            .get(&identity)
            .map_or(epoch, |policy| policy.minimum_epoch)
    };
    state.authorization_epoch = epoch;
    state.policies.insert(
        identity.clone(),
        PolicyAuthority {
            revision,
            enabled,
            minimum_epoch,
        },
    );
    if changed || !enabled {
        cancel_matching(
            state,
            |job| PrincipalIdentity::from(&job.lease.principal) == identity,
            ExecCancelReason::Revoked,
        );
    }
}

fn restrict_state(state: &mut RegistryState, epoch: u64, restrictions: &RestrictionSet) {
    state.authorization_epoch = epoch;
    for (principal, policy) in &mut state.policies {
        if restrictions.affects(&principal.0, &principal.1, &principal.3, &principal.5) {
            policy.minimum_epoch = epoch;
        }
    }
    cancel_matching(
        state,
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
}
