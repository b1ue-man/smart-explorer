//! Commands the worker applies locally, connected or not: Exec grants,
//! request ledger sync, the Direct online switch and runtime relation data.
use std::io;
use std::sync::{Arc, Mutex};

use super::super::backend::ShareIrohNode;
use super::super::configuration_runtime::schedule_current;
use super::super::core::{eio, now_secs};
use super::super::direct_ledger::DirectRequestEntry;
use super::super::direct_request_tombstone::DirectRequestTombstone;
use super::super::exec_grant_runtime::{self, ExecGrantMutation};
use super::super::exec_policy::ExecGrant;
use super::super::exec_types::ExecPrincipal;
use super::super::types::{ExecGrantTarget, RelationRuntime, ShareAuthState};

pub(super) fn mutate_exec_grant(
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    target: ExecGrantTarget,
    enabled: bool,
) -> io::Result<ExecGrantMutation> {
    let mutation =
        exec_grant_runtime::mutate(auth, iroh.exec_registry(), target, enabled, now_secs())?;
    iroh.recheck_exec_connections();
    schedule_current(auth, iroh)?;
    // Idle `/3` connections whose grant is now off close at once; connections
    // with running streams stay alive just long enough to deliver their
    // signed-off Revoked terminal status (`exec_host_connections.rs`).
    Ok(mutation)
}

pub(super) fn apply_persisted_exec_grant(
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    target: ExecGrantTarget,
    principal: ExecPrincipal,
    policy: ExecGrant,
) -> io::Result<ExecGrantMutation> {
    let mutation = exec_grant_runtime::apply_exact_with_access(
        auth,
        iroh.exec_registry(),
        Some(iroh.access_registry()),
        target,
        principal,
        policy,
    )?;
    iroh.recheck_exec_connections();
    schedule_current(auth, iroh)?;
    Ok(mutation)
}

pub(super) fn sync_direct_requests(
    auth: &Arc<Mutex<ShareAuthState>>,
    direct_requests: Vec<DirectRequestEntry>,
    direct_request_tombstones: Vec<DirectRequestTombstone>,
) -> io::Result<()> {
    auth.lock()
        .map_err(|_| eio("Share-State gesperrt"))
        .map(|mut state| {
            state.direct_requests = direct_requests;
            state.direct_request_tombstones = direct_request_tombstones;
        })
}

pub(super) fn set_direct_online(
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    online: bool,
) -> io::Result<String> {
    let transition = iroh.begin_runtime_transition()?;
    let (lookup_id, changed, restrictions) = {
        let mut state = auth.lock().map_err(|_| eio("Share-State gesperrt"))?;
        let changed = state.direct_online != online;
        let mut candidate = state.clone();
        candidate.direct_online = online;
        let restrictions =
            super::super::relation_rights::authorization_restrictions(&state, &candidate);
        if changed {
            let next_epoch = if restrictions.is_empty() {
                state.authorization_epoch
            } else {
                state
                    .authorization_epoch
                    .checked_add(1)
                    .ok_or_else(|| eio("Share authorization epoch exhausted"))?
            };
            candidate.authorization_epoch = next_epoch;
            exec_grant_runtime::apply_configuration_transition(
                &state,
                &candidate,
                next_epoch,
                iroh.exec_registry(),
            )?;
            let now = now_secs();
            iroh.access_registry().recheck(&candidate, now);
            iroh.exec_host.recheck(&candidate, now);
            *state = candidate;
        }
        (
            state.identity.direct_lookup_id.clone(),
            changed,
            restrictions,
        )
    };
    let invalidation = if !restrictions.is_empty() {
        iroh.invalidate_restrictions(&restrictions).map(|_| ())
    } else {
        Ok(())
    };
    drop(transition);
    if changed {
        schedule_current(auth, iroh)?;
    }
    invalidation?;
    Ok(lookup_id)
}

/// FA3: runtime data never passes through a configuration transition, so a
/// presence or LAN change cannot invalidate sessions or wait for a repair.
pub(super) fn apply_relation_runtime(
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    update: &RelationRuntime,
) -> io::Result<()> {
    let changed = auth
        .lock()
        .map_err(|_| eio("Share-State gesperrt"))?
        .apply_runtime(update);
    if changed {
        // New presences or LAN sightings: the direct retries look again.
        iroh.peer_addresses.changed();
        schedule_current(auth, iroh)?;
    }
    Ok(())
}
