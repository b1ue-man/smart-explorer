//! Daemon side of an access-right change (D.2.3; the IPC request
//! `AccessIpcRequest::MutateGrant` of PRM-M3 calls it): the journal,
//! persistence and worker hand-off of an Exec-grant change
//! (`exec_grant_journal.rs`) for a change of account, capabilities, program
//! list or end. A grant that allows anything is persisted before the worker
//! applies it; one that allows nothing is applied first.
use super::ipc_host::exec_grant_journal::ExecGrantPersistResult;
use super::ipc_host::ShareHost;
use crate::share::access_grant::{AccessGrantChange, AccountProfile, Capability};

impl ShareHost {
    /// Applies `change` to the grant of `target`. `expected_revision` is the
    /// revision the surface showed (`None`: whatever is current); another
    /// one is refused as a concurrent change. Returns the persistence result
    /// and the grant as requested; `None` when an earlier change is still
    /// being recovered and nothing new was prepared.
    pub(crate) fn mutate_access_grant(
        &self,
        target: crate::share::ExecGrantTarget,
        change: &AccessGrantChange,
        expected_revision: Option<u64>,
    ) -> Result<(ExecGrantPersistResult, Option<crate::share::ExecGrant>), String> {
        // Before anything is locked or written: a refused change leaves the
        // grant exactly as it is.
        validate_for_this_host(change)?;
        let mut requested = None;
        let result = self.mutate_grant_with(|profiles, identity| {
            let prepared = crate::share::ExecGrantMutation::prepare_access(
                profiles,
                identity,
                target,
                change,
                expected_revision,
                crate::share::core_now_secs(),
            )?;
            requested = Some(prepared.0.policy.clone());
            Ok(prepared)
        })?;
        Ok((result, requested))
    }
}

/// Another account than the worker's needs a host that can switch to it
/// (today a Linux worker running as root).
fn validate_for_this_host(change: &AccessGrantChange) -> Result<(), String> {
    // A per-user daemon cannot establish the administratively protected
    // machine authority needed before login or on the secure desktop.
    if change.capabilities.get(&Capability::LoginAccess).is_some_and(|decision| decision.is_on()) {
        return Err("login_access must be granted through the protected machine policy, not the user daemon".into());
    }
    if let Some(constraints) = &change.constraints { constraints.validate()?; }
    let other_account = change
        .account
        .as_ref()
        .is_some_and(|account| *account != AccountProfile::Own);
    if other_account && !crate::share::exec_run_as_supported() {
        return Err(
            "this host runs commands only as the Smart Explorer user (Standardnutzer); \
             another account needs a Linux worker running as root"
                .into(),
        );
    }
    Ok(())
}
