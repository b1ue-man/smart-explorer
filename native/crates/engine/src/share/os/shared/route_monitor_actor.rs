//! The policy actor of one connection, registered by `route_monitor.rs`.
//!
//! It observes every `TICK` only while its observation can change with time
//! alone (finite collection/direct phase, application bytes moving); otherwise
//! it sleeps until the transport reports changed path-policy facts, a session
//! waits for its route, the start is cancelled, the connection closes or, in
//! relay fallback, the next direct retry is due or addresses changed
//! (`route_monitor_retry.rs`).
use super::super::route_adapter::RouteAdapter;
use super::super::route_diagnostics::milliseconds;
use super::retry::Runner;
use super::{
    network_wait, next_generation, registry, samples, Entry, RoutePhase, RoutePolicy, TICK,
};
use iroh::endpoint::ApplicationPathUpdate;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

pub(super) async fn run(id: usize, entry: Arc<Entry>, hints: Vec<SocketAddr>) {
    let mut origin = Instant::now();
    let mut adapter = RouteAdapter::default();
    let mut policy: Option<RoutePolicy> = None;
    let mut previous_sample = None;
    let mut previous_path = None;
    let mut previous_bytes = None;
    let mut retry = Runner::new(&entry.retry);
    // Registered once: every call adds a close listener to the connection.
    let closed = entry.handle.closed();
    tokio::pin!(closed);
    loop {
        // Registered before the observation, so a change during it is not lost.
        let changed = entry.changes.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let woken = entry.wake.notified();
        tokio::pin!(woken);
        woken.as_mut().enable();
        let Some(connection) = entry.handle.upgrade() else {
            break;
        };
        if connection.close_reason().is_some() {
            break;
        }
        let mut snapshot = connection.application_path_policy();
        if snapshot.network_change_in_progress {
            if !network_wait::pause(&entry) {
                break;
            }
            previous_sample = None;
            previous_path = None;
            drop(connection);
            // The transport signals the end of the network change.
            tokio::select! {
                _ = changed => {}
                _ = woken => {}
                _ = &mut closed => break,
            }
            continue;
        }
        let mut reset_policy = policy
            .as_ref()
            .is_some_and(|policy| policy.status().generation != snapshot.generation);
        if snapshot.requires_new_generation {
            let Ok(generation) = next_generation(snapshot.generation) else {
                break;
            };
            if connection
                .update_application_path_policy(ApplicationPathUpdate::BeginGeneration(generation))
                .is_err()
            {
                break;
            }
            snapshot = connection.application_path_policy();
            reset_policy = true;
        }
        if reset_policy {
            origin = Instant::now();
            policy = None;
            adapter = RouteAdapter::default();
            retry.reset();
        }
        let policy = policy.get_or_insert_with(|| {
            let mut policy = RoutePolicy::new(
                snapshot.generation,
                snapshot.initial_pto,
                snapshot.best_validated_pto,
            );
            let old = entry.status.borrow();
            if old.application_allowed || old.pause_epoch > 0 {
                policy.resuming();
            }
            policy
        });
        policy.include_validation_pto(
            snapshot
                .paths
                .iter()
                .map(|path| path.pto)
                .max()
                .unwrap_or(snapshot.initial_pto),
        );
        let observed = adapter.observe(&connection, &snapshot, origin, &hints);
        let chosen = policy.observe(
            milliseconds(origin.elapsed()),
            &observed.paths,
            observed.round.as_ref(),
        );
        if let Some(token) = chosen
            .as_ref()
            .and_then(|key| observed.tokens.get(key))
            .copied()
        {
            let already = snapshot
                .paths
                .iter()
                .any(|path| path.token.path_id() == token.path_id() && path.data_allowed);
            if !already
                && connection
                    .update_application_path_policy(ApplicationPathUpdate::Allow(token))
                    .is_err()
            {
                policy.gate_rejected();
            }
        } else if connection.application_data_allowed() {
            let _ = connection
                .update_application_path_policy(ApplicationPathUpdate::Block(snapshot.generation));
        }
        let mut status = policy.status().clone();
        let old = entry.status.borrow().clone();
        if old.phase == RoutePhase::Cancelled {
            break;
        }
        status.remote_guarantee = old.remote_guarantee;
        // In relay fallback the connection keeps trying the direct path.
        let now = milliseconds(origin.elapsed());
        retry
            .step(
                &entry.retry,
                &connection,
                &entry.remote,
                &hints,
                now,
                &mut status,
            )
            .await;
        let current_path = connection
            .application_path_policy()
            .paths
            .iter()
            .find(|path| path.data_allowed)
            .map(|path| format!("{}/{}", path.token.path_id(), path.token.path_generation()));
        let changed_path =
            previous_path.is_some() && current_path.is_some() && previous_path != current_path;
        status.pause_epoch = if status.generation != old.generation
            || (old.application_allowed && !status.application_allowed)
            || changed_path
        {
            old.pause_epoch.saturating_add(1)
        } else {
            old.pause_epoch
        };
        if status.phase == RoutePhase::Offline
            && network_wait::offline_is_outdated(&status, &connection.application_path_policy())
        {
            drop(connection);
            tokio::time::sleep(TICK).await;
            continue;
        }
        previous_path = current_path;
        entry.status.send_if_modified(|current| {
            if current.phase == RoutePhase::Cancelled {
                return false;
            }
            status.remote_guarantee = current.remote_guarantee;
            if *current == status {
                return false;
            }
            *current = status.clone();
            true
        });
        if entry.status.borrow().phase == RoutePhase::Cancelled {
            break;
        }
        if let Ok(mut clock) = entry.clock.lock() {
            match (status.application_allowed, clock.1) {
                (true, Some(since)) => {
                    clock.0 = clock.0.saturating_add(since.elapsed());
                    clock.1 = None;
                }
                (false, None) => clock.1 = Some(tokio::time::Instant::now()),
                _ => {}
            }
        }
        if let Ok(mut budget) = entry.budget.lock() {
            *budget = policy.budget().clone();
        }
        samples::record_measurement(&entry, &connection, &status, &mut previous_sample);
        let bytes = (snapshot.application_tx_bytes, snapshot.application_rx_bytes);
        let moving = previous_bytes.is_some_and(|previous| previous != bytes);
        previous_bytes = Some(bytes);
        let timed = !policy.decided(milliseconds(origin.elapsed()));
        // Do not retain a strong connection while idle; cancellation/drop still closes it.
        drop(connection);
        if timed || moving {
            tokio::time::sleep(TICK).await;
        } else {
            let now = milliseconds(origin.elapsed());
            tokio::select! {
                _ = changed => {}
                _ = woken => {}
                _ = &mut closed => break,
                () = retry.next_event(now) => {}
            }
        }
    }
    entry.status.send_modify(|status| {
        if status.application_allowed {
            status.pause_epoch = status.pause_epoch.saturating_add(1);
        }
        status.application_allowed = false;
        status.class = None;
        if status.phase != RoutePhase::Cancelled {
            status.phase = RoutePhase::Offline;
            status.reason = "Share-Verbindung geschlossen".into();
        }
        status.revision = status.revision.saturating_add(1);
    });
    if let Ok(mut entries) = registry().lock() {
        entries.remove(&id);
    }
}
