//! Keep confirmed network preparation separate from route-choice deadlines.

use super::{Entry, RoutePhase, RouteStatus};
use iroh::endpoint::ApplicationPathPolicySnapshot;

/// Update under the watch write lock so concurrent cancellation stays terminal.
pub(super) fn pause(entry: &Entry) -> bool {
    let reason = "Netzwechsel erkannt; Netzwerkbereitschaft wird abgewartet";
    entry.status.send_if_modified(|status| {
        if status.phase == RoutePhase::Cancelled
            || (status.phase == RoutePhase::Paused
                && !status.application_allowed
                && status.class.is_none()
                && status.reason == reason)
        {
            return false;
        }
        if status.application_allowed {
            status.pause_epoch = status.pause_epoch.saturating_add(1);
        }
        status.application_allowed = false;
        status.class = None;
        status.phase = RoutePhase::Paused;
        status.reason = reason.into();
        status.revision = status.revision.saturating_add(1);
        true
    });
    if entry.status.borrow().phase == RoutePhase::Cancelled {
        return false;
    }
    if let Ok(mut clock) = entry.clock.lock() {
        if clock.1.is_none() {
            clock.1 = Some(tokio::time::Instant::now());
        }
    }
    true
}

/// An old Offline decision cannot end a wait after its transport epoch changed.
pub(super) fn offline_is_outdated(
    status: &RouteStatus,
    transport: &ApplicationPathPolicySnapshot,
) -> bool {
    transport.network_change_in_progress
        || transport.requires_new_generation
        || transport.generation != status.generation
}

#[cfg(test)]
mod tests {
    use super::super::{cancelled, registry, wait_application_route, RouteBudget, RoutePolicy};
    use super::*;
    use crate::share::route_policy::batches_route_task_tests::{pair, runtime};
    use std::{
        io::ErrorKind,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    use tokio::sync::watch;

    struct Registered(usize);

    impl Drop for Registered {
        fn drop(&mut self) {
            if let Ok(mut entries) = registry().lock() {
                entries.remove(&self.0);
            }
        }
    }

    #[test]
    fn batches_task_b5_stale_offline_keeps_network_wait_deadline_cancel_and_close() {
        runtime().block_on(async {
            let (left, right, sender, _receiver) = pair().await;
            // A genuine prior RoutePolicy decision; only the watch delivery is
            // held old. The current transport epoch below is produced by Iroh.
            let generation = sender.application_path_policy().generation;
            let mut old = RoutePolicy::new(
                generation,
                Duration::from_millis(1),
                Duration::from_millis(1),
            );
            let collection = old.budget().collection_ms;
            let _ = old.observe(collection, &[], None);
            let deadline = old.budget().direct_deadline_ms;
            let _ = old.observe(deadline, &[], None);
            assert_eq!(old.status().phase, RoutePhase::Offline);
            let (status, _) = watch::channel(old.status().clone());
            let entry = Arc::new(Entry {
                id: sender.stable_id(),
                protocol: String::from_utf8_lossy(sender.alpn()).into_owned(),
                handle: sender.weak_handle(),
                local: left.id().to_string(),
                remote: sender.remote_id().to_string(),
                pair: None,
                status,
                budget: Mutex::new(RouteBudget::default()),
                clock: Mutex::new((Duration::ZERO, Some(tokio::time::Instant::now()))),
            });
            assert!(registry()
                .lock()
                .unwrap()
                .insert(entry.id, entry.clone())
                .is_none());
            let _registered = Registered(entry.id);
            let wait = Duration::from_secs(10);
            tokio::time::timeout(wait, left.network_change_detected(true))
                .await
                .unwrap();
            assert!(sender.application_path_policy().requires_new_generation);
            assert!(
                tokio::time::timeout(Duration::from_millis(50), wait_application_route(&sender, None))
                    .await
                    .is_err(),
                "old Offline must not win over the current network epoch"
            );
            let generation = sender.application_path_policy().generation.checked_add(1).unwrap();
            sender
                .update_application_path_policy(iroh::endpoint::ApplicationPathUpdate::BeginGeneration(generation))
                .unwrap();
            assert_ne!(sender.application_path_policy().generation, old.status().generation);
            assert!(
                tokio::time::timeout(Duration::from_millis(50), wait_application_route(&sender, None))
                    .await
                    .is_err(),
                "Offline from a prior generation must stay outdated"
            );
            let error = wait_application_route(&sender, Some(Instant::now()))
                .await
                .unwrap_err();
            assert_eq!(error.kind(), ErrorKind::TimedOut);
            cancelled(&sender);
            let error = tokio::time::timeout(wait, wait_application_route(&sender, None))
                .await
                .unwrap()
                .unwrap_err();
            assert_eq!(error.kind(), ErrorKind::NotConnected);
            assert_eq!(error.to_string(), "Verbindungsaufbau oder Sitzung abgebrochen");
            assert!(
                !pause(&entry),
                "network waiting must not overwrite cancellation"
            );
            sender.close(0u32.into(), b"fixture complete");
            tokio::time::timeout(wait, sender.closed()).await.unwrap();
            let error = wait_application_route(&sender, None).await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::NotConnected);
            assert_eq!(error.to_string(), "Share-Verbindung geschlossen");
            left.close().await;
            right.close().await;
        });
    }
}
