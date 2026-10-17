//! Per-principal barriers keep unrelated launch reservations alive (B04/FA3).
//! A narrowed grant ends the principal's jobs; a widened one (C5) keeps them
//! and the principal's current authorizations, so „Immer erlauben“ never
//! ends the session it just allowed.
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
        self.install_authorization(principal, revision, epoch, enabled, false)
    }

    /// The next revision of a grant that only widened (no Exec capability
    /// narrowed): running jobs continue and the principal's earlier
    /// authorizations stay usable at the new revision.
    pub(crate) fn apply_widened_authorization(
        &self,
        principal: &ExecPrincipal,
        revision: u64,
        epoch: u64,
    ) -> Result<(), ExecRegistryError> {
        self.install_authorization(principal, revision, epoch, true, true)
    }

    fn install_authorization(
        &self,
        principal: &ExecPrincipal,
        revision: u64,
        epoch: u64,
        enabled: bool,
        keep_jobs: bool,
    ) -> Result<(), ExecRegistryError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if epoch < state.authorization_epoch {
            return Err(ExecRegistryError::StaleAuthorization);
        }
        validate_policy(&state, principal, revision, enabled)?;
        install_policy(&mut state, principal, revision, epoch, enabled, keep_jobs);
        Ok(())
    }

    /// Validate the complete snapshot before advancing the deny barrier. A
    /// stale policy must not leave the registry ahead of the unpublished auth
    /// snapshot, poisoning all subsequent unrelated configuration refreshes.
    /// Each update is `(principal, revision, enabled, keep_jobs)`; `keep_jobs`
    /// marks a grant that only widened.
    pub(crate) fn apply_configuration_authorizations(
        &self,
        epoch: u64,
        restrictions: &RestrictionSet,
        updates: &[(ExecPrincipal, u64, bool, bool)],
    ) -> Result<(), ExecRegistryError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if epoch < state.authorization_epoch {
            return Err(ExecRegistryError::StaleAuthorization);
        }
        let mut identities = HashSet::new();
        for (principal, revision, enabled, _) in updates {
            if !identities.insert(PrincipalIdentity::from(principal)) {
                return Err(ExecRegistryError::InvalidAuthorization);
            }
            validate_policy(&state, principal, *revision, *enabled)?;
        }
        restrict_state(&mut state, epoch, restrictions);
        for (principal, revision, enabled, keep_jobs) in updates {
            install_policy(
                &mut state, principal, *revision, epoch, *enabled, *keep_jobs,
            );
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
    keep_jobs: bool,
) {
    let identity = PrincipalIdentity::from(principal);
    let changed = state
        .policies
        .get(&identity)
        .is_none_or(|current| current.revision != revision || current.enabled != enabled);
    // A widened grant keeps its jobs and the barrier of its authorizations.
    let keep = keep_jobs && enabled;
    let minimum_epoch = if changed && !keep {
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
    if (changed && !keep) || !enabled {
        cancel_matching(
            state,
            |job| PrincipalIdentity::from(&job.lease.principal) == identity,
            ExecCancelReason::Revoked,
        );
    }
}

/// Ends the jobs of every principal an Exec-relevant restriction hits; a
/// narrowing of a screen capability alone leaves Exec jobs running.
fn restrict_state(state: &mut RegistryState, epoch: u64, restrictions: &RestrictionSet) {
    state.authorization_epoch = epoch;
    for (principal, policy) in &mut state.policies {
        if restrictions.affects_exec(&principal.0, &principal.1, &principal.3, &principal.5) {
            policy.minimum_epoch = epoch;
        }
    }
    cancel_matching(
        state,
        |job| {
            let principal = &job.lease.principal;
            restrictions.affects_exec(
                &principal.relation_kind,
                &principal.relation_id,
                &principal.public_key,
                &principal.node_id,
            )
        },
        ExecCancelReason::Revoked,
    );
}
