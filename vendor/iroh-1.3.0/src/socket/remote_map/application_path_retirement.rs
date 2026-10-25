// Smart Explorer P28b/V3 patch (2026-10-25); see vendor/SMART_EXPLORER_ROUTE_POLICY.md.
//! Free one redundant IP candidate only after the bounded path budget is full.

use noq::ApplicationPathUpdate;
use tracing::trace;

use super::{ConnectionState, transports};

pub(super) fn retire_redundant_path(
    conn: &noq::Connection,
    state: &ConnectionState,
    selected: Option<&transports::FourTuple>,
) {
    if !conn.application_path_policy_enabled() { return; }
    let snapshot = conn.application_path_policy();
    retire_from_snapshot(conn, state, selected, &snapshot);
}

pub(super) fn retire_for_pending_nat(
    conn: &noq::Connection,
    state: &ConnectionState,
    selected: Option<&transports::FourTuple>,
) {
    if !conn.application_path_policy_enabled() { return; }
    let snapshot = conn.application_path_policy();
    if snapshot.nat_round.as_ref().is_some_and(|round| round.pending_path_opens > 0) {
        retire_from_snapshot(conn, state, selected, &snapshot);
    }
}

fn retire_from_snapshot(
    conn: &noq::Connection,
    state: &ConnectionState,
    selected: Option<&transports::FourTuple>,
    snapshot: &noq::ApplicationPathPolicySnapshot,
) {
    let candidate = snapshot.paths.iter()
        .filter(|path| path.retirement_allowed)
        .filter(|path| state.paths.get(&path.token.path_id()).is_some_and(|addr| {
            addr.is_ip() && Some(addr) != selected
        }))
        .max_by_key(|path| (path.pto, path.token.path_id()));
    if let Some(path) = candidate {
        // Noq rechecks token, grant, validation and surviving usable path in one
        // mutation. Snapshot + Path::close would race a concurrent policy grant.
        let result = conn.update_application_path_policy(
            ApplicationPathUpdate::RetireRedundantPath(path.token),
        );
        trace!(path_id = %path.token.path_id(), ?result, "retiring redundant IP candidate for path capacity");
    }
}
