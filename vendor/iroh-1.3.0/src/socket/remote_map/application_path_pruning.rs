// Smart Explorer A7 patch (2026-11-06); see vendor/SMART_EXPLORER_ROUTE_POLICY.md.
//! Release redundant IP candidates once the app's route decision holds.
//!
//! Upstream keeps one IP path plus relay/custom paths per connection. A
//! policy-enabled client keeps the admitted path, the selected path and every
//! relay/custom path, and releases the other IP paths through Noq's atomic
//! `ReleaseCandidate` mutation. Before any admission (collection, network
//! change, fresh proofs of a new generation) every candidate stays open.

use std::pin::Pin;

use n0_future::Stream;
use noq::{ApplicationPathPolicySnapshot, ApplicationPathToken, ApplicationPathUpdate};
use noq_proto::PathId;
use smallvec::SmallVec;
use tracing::trace;

use super::{ConnId, ConnectionState, transports};

/// Change signals of all policy-enabled client connections of one remote.
pub(super) type PolicyChanges = Pin<Box<dyn Stream<Item = ConnId> + Send + Sync>>;

/// Yields `conn_id` whenever the event-driven policy facts of `conn` differ
/// from the ones last yielded (first poll included), and ends when the
/// connection is gone or closed. An idle connection yields nothing.
pub(super) fn policy_changes(conn_id: ConnId, conn: &noq::Connection) -> PolicyChanges {
    let state = (
        conn.application_path_policy_changes(),
        conn.weak_handle(),
        None,
    );
    Box::pin(futures_util::stream::unfold(
        state,
        move |(notify, handle, last): (_, noq::WeakConnectionHandle, Option<u64>)| async move {
            loop {
                // A Notified receives every notify_waiters() after its creation,
                // so a change during the comparison below wakes this wait.
                let changed = notify.notified();
                let current = {
                    let conn = handle.upgrade()?;
                    if conn.close_reason().is_some() {
                        return None;
                    }
                    conn.application_path_policy_fingerprint()
                };
                if last != Some(current) {
                    drop(changed);
                    return Some((conn_id, (notify, handle, Some(current))));
                }
                changed.await;
            }
        },
    ))
}

/// Releases every redundant IP candidate of `state` that the current decision
/// allows and returns the released PathIds. Closing uses ordinary QUIC path
/// abandonment; the peer and the Abandoned event clean up as for upstream.
pub(super) fn release_redundant_paths(
    conn: &noq::Connection,
    state: &ConnectionState,
    selected: &transports::FourTuple,
) -> SmallVec<[PathId; 8]> {
    let mut released = SmallVec::new();
    if !conn.application_path_policy_enabled() || !conn.side().is_client() {
        return released;
    }
    let mut tried = SmallVec::<[PathId; 8]>::new();
    // Each release advances the policy revision, so every attempt uses a fresh
    // observation. Every attempt excludes its path: at most one per known path.
    for _ in 0..state.paths.len() {
        let snapshot = conn.application_path_policy();
        let Some(token) = next_candidate(&snapshot, state, selected, &tried) else {
            break;
        };
        tried.push(token.path_id());
        // Noq rechecks token, current grant and side under its mutex, so a
        // concurrent Allow of this path (new revision) refuses the release.
        match conn.update_application_path_policy(ApplicationPathUpdate::ReleaseCandidate(token)) {
            Ok(()) => {
                trace!(path_id = %token.path_id(), "releasing redundant direct candidate");
                released.push(token.path_id());
            }
            Err(error) => {
                trace!(path_id = %token.path_id(), ?error, "keeping direct candidate");
            }
        }
    }
    released
}

fn next_candidate(
    snapshot: &ApplicationPathPolicySnapshot,
    state: &ConnectionState,
    selected: &transports::FourTuple,
    tried: &[PathId],
) -> Option<ApplicationPathToken> {
    let observed = move |id: PathId| {
        snapshot
            .paths
            .iter()
            .find(|path| path.token.path_id() == id)
    };
    // The app's decision: the one path that currently carries application data.
    let admitted = snapshot
        .paths
        .iter()
        .find(|path| path.data_allowed)?
        .token
        .path_id();
    let selected_id = state
        .paths
        .iter()
        .find(|(_, addr)| *addr == selected)
        .map(|(id, _)| *id);
    // An upgrade target still proving keeps the alternatives until its proof
    // ends: it validates (then this runs again), or it fails and is replaced.
    if let Some(id) = selected_id
        && id != admitted
        && !observed(id).is_some_and(|path| path.validated)
    {
        return None;
    }
    state
        .paths
        .iter()
        .filter(|(id, addr)| {
            addr.is_ip() && **id != admitted && Some(**id) != selected_id && !tried.contains(*id)
        })
        .filter_map(|(id, _)| observed(*id))
        .map(|path| path.token)
        .min_by_key(|token| token.path_id())
}
