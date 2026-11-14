//! Test fixture of the route monitor: the route entry of one connection
//! without a policy actor. The test admits, pauses and resumes the route
//! itself, with the status and the pause clock the actor would write
//! (`run` in `route_monitor.rs`, `network_wait::pause`), so consumers of
//! `subscribe`, `wait_application_route` and `paused_duration` meet a route
//! pause at a moment the test chooses. Only for `#[cfg(test)]`.
use super::{registry, Entry, RouteBudget, RoutePhase, RouteStatus};
use iroh::endpoint::Connection;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{watch, Notify};

/// The registered route of one connection; dropping it removes the entry.
pub(crate) struct TestRoute {
    entry: Arc<Entry>,
}

impl TestRoute {
    /// Registers `connection` with an admitted route and a pause clock that
    /// stands still, as the actor leaves both after an admission.
    pub(crate) fn admitted(connection: &Connection, local: String) -> Self {
        let mut initial = RouteStatus::initial(connection.application_path_policy().generation);
        initial.application_allowed = true;
        initial.phase = RoutePhase::Direct;
        initial.reason = "Testroute zugelassen".into();
        let (status, _) = watch::channel(initial);
        let entry = Arc::new(Entry {
            id: connection.stable_id(),
            protocol: String::from_utf8_lossy(connection.alpn()).into_owned(),
            handle: connection.weak_handle(),
            local,
            remote: connection.remote_id().to_string(),
            pair: None,
            status,
            budget: Mutex::new(RouteBudget::default()),
            clock: Mutex::new((Duration::ZERO, None)),
            changes: connection.application_path_policy_changes(),
            wake: Notify::new(),
        });
        registry()
            .lock()
            .expect("route registry")
            .insert(entry.id, entry.clone());
        Self { entry }
    }

    /// The route pauses: application data is not admitted and the pause
    /// clock runs.
    pub(crate) fn pause(&self) {
        self.entry.status.send_modify(|status| {
            if status.application_allowed {
                status.pause_epoch = status.pause_epoch.saturating_add(1);
            }
            status.application_allowed = false;
            status.phase = RoutePhase::Paused;
            status.reason = "Testroute pausiert".into();
            status.revision = status.revision.saturating_add(1);
        });
        let mut clock = self.entry.clock.lock().expect("route clock");
        if clock.1.is_none() {
            clock.1 = Some(tokio::time::Instant::now());
        }
    }

    /// The route is admitted again: the pause ends and its length is added
    /// to the paused time.
    pub(crate) fn resume(&self) {
        {
            let mut clock = self.entry.clock.lock().expect("route clock");
            if let Some(since) = clock.1.take() {
                clock.0 = clock.0.saturating_add(since.elapsed());
            }
        }
        self.entry.status.send_modify(|status| {
            status.application_allowed = true;
            status.phase = RoutePhase::Direct;
            status.reason = "Testroute zugelassen".into();
            status.revision = status.revision.saturating_add(1);
        });
    }
}

impl Drop for TestRoute {
    fn drop(&mut self) {
        if let Ok(mut entries) = registry().lock() {
            entries.remove(&self.entry.id);
        }
    }
}
