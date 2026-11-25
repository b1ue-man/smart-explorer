//! One policy actor per QUIC connection. Weak handles preserve normal connection lifetime.
//!
//! The actor observes every `TICK` only while its observation can change with time
//! alone: during the finite collection/direct phase of a generation and while
//! application bytes move (route samples, RTT order between validated paths).
//! Otherwise it sleeps until the transport reports changed path-policy facts
//! (`Connection::application_path_policy_changes`), a session waits for a route
//! (`wait_application_route`), the start is cancelled, the connection closes, or
//! (in relay fallback) a direct retry is due or addresses changed
//! (`route_monitor_retry.rs`), so an idle connection causes no periodic wake-ups.
//! The actor loop itself is `route_monitor_actor.rs`.
use super::route_diagnostics::{RouteBudget, RouteDiagnostics};
use super::route_policy::RoutePolicy;
use super::route_retry_port::DirectRetryPort;
use super::route_state::{RemoteRouteGuarantee, RoutePhase, RouteStatus};
use iroh::endpoint::{ApplicationPathUpdate, Connection, WeakConnectionHandle};
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{watch, Notify};

#[path = "route_monitor_actor.rs"]
mod actor;
#[path = "route_monitor_network_wait.rs"]
mod network_wait;
#[path = "route_monitor_retry.rs"]
mod retry;
#[path = "route_monitor_samples.rs"]
mod samples;
// Test fixture: a route whose admission a test sets (pauses of consumers).
#[cfg(test)]
#[path = "route_monitor_test_route.rs"]
pub(super) mod test_route;

const MAX_CONNECTIONS: usize = 256;
const TICK: Duration = Duration::from_millis(25);
static GENERATION: AtomicU64 = AtomicU64::new(1);
static REGISTRY: OnceLock<Mutex<HashMap<usize, Arc<Entry>>>> = OnceLock::new();

struct Entry {
    id: usize,
    protocol: String,
    handle: WeakConnectionHandle,
    local: String,
    remote: String,
    pair: Option<String>,
    status: watch::Sender<RouteStatus>,
    budget: Mutex<RouteBudget>,
    clock: Mutex<(Duration, Option<tokio::time::Instant>)>,
    /// The transport's signal for changed path-policy facts.
    changes: Arc<Notify>,
    /// Wakes an idle actor: a session waits for its route, or the start was cancelled.
    wake: Notify,
    /// Direct retries in relay fallback (`route_monitor_retry.rs`).
    retry: retry::EntryRetry,
}

fn registry() -> &'static Mutex<HashMap<usize, Arc<Entry>>> {
    REGISTRY.get_or_init(Mutex::default)
}
fn failed(message: &str) -> io::Error {
    io::Error::other(message)
}

fn next_generation(current: u64) -> io::Result<u64> {
    GENERATION
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |next| {
            next.max(current).checked_add(1)
        })
        .map(|previous| previous.max(current) + 1)
        .map_err(|_| failed("Routegeneration erschoepft"))
}

pub(super) fn start(
    connection: &Connection,
    local: String,
    pinned: bool,
    remote_support: bool,
    hints: Vec<SocketAddr>,
) -> io::Result<()> {
    start_with_retry(connection, local, pinned, remote_support, hints, None)
}

/// [`start`] with the port through which the actor of a new entry keeps
/// trying the direct path while the relay is the fallback.
pub(super) fn start_with_retry(
    connection: &Connection,
    local: String,
    pinned: bool,
    remote_support: bool,
    hints: Vec<SocketAddr>,
    retry_port: Option<Arc<dyn DirectRetryPort>>,
) -> io::Result<()> {
    let mut entries = registry()
        .lock()
        .map_err(|_| failed("Routemonitor gesperrt"))?;
    if let Some(entry) = entries.get(&connection.stable_id()) {
        if remote_support {
            entry
                .status
                .send_modify(|status| status.remote_guarantee = RemoteRouteGuarantee::BothSenders);
        }
        return Ok(());
    }
    entries.retain(|_, entry| entry.handle.upgrade().is_some());
    if entries.len() >= MAX_CONNECTIONS {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Routezulassung erschoepft",
        ));
    }
    let snapshot = connection.application_path_policy();
    if !snapshot.enabled {
        return Err(failed("QUIC-Nutzdatenpolicy ist nicht aktiv"));
    }
    let generation = next_generation(snapshot.generation)?;
    connection
        .update_application_path_policy(ApplicationPathUpdate::BeginGeneration(generation))
        .map_err(io::Error::other)?;
    let mut initial = RouteStatus::initial(generation);
    initial.remote_guarantee = if remote_support {
        RemoteRouteGuarantee::BothSenders
    } else {
        RemoteRouteGuarantee::LocalSenderOnly
    };
    let (status, _) = watch::channel(initial);
    let remote = connection.remote_id().to_string();
    let pair = pinned.then(|| super::route_history::pair_key(&local, &remote));
    let entry = Arc::new(Entry {
        id: connection.stable_id(),
        protocol: String::from_utf8_lossy(connection.alpn()).into_owned(),
        handle: connection.weak_handle(),
        local,
        remote,
        pair,
        status,
        budget: Mutex::new(RouteBudget::default()),
        clock: Mutex::new((Duration::ZERO, Some(tokio::time::Instant::now()))),
        changes: connection.application_path_policy_changes(),
        wake: Notify::new(),
        retry: retry::EntryRetry::new(retry_port),
    });
    entries.insert(connection.stable_id(), entry.clone());
    tokio::spawn(actor::run(connection.stable_id(), entry, hints));
    Ok(())
}

pub fn subscribe(connection: &Connection) -> Option<watch::Receiver<RouteStatus>> {
    registry()
        .lock()
        .ok()?
        .get(&connection.stable_id())
        .map(|entry| entry.status.subscribe())
}

pub(super) fn connection(id: usize) -> Option<Connection> {
    registry().lock().ok()?.get(&id)?.handle.upgrade()
}

pub fn snapshot(connection: &Connection) -> Option<RouteStatus> {
    subscribe(connection).map(|status| {
        let current = status.borrow().clone();
        current
    })
}

pub fn cancelled(connection: &Connection) {
    if let Ok(entries) = registry().lock() {
        if let Some(entry) = entries.get(&connection.stable_id()) {
            entry.status.send_modify(|status| {
                if status.application_allowed {
                    status.pause_epoch = status.pause_epoch.saturating_add(1);
                }
                status.application_allowed = false;
                status.class = None;
                status.phase = RoutePhase::Cancelled;
                status.reason = "Verbindungsaufbau oder Sitzung abgebrochen".into();
                status.revision = status.revision.saturating_add(1);
            });
            entry.wake.notify_one();
        }
    }
}

/// Lets an idle actor observe at once: a session is about to use the connection.
fn wake(connection: &Connection) {
    if let Ok(entries) = registry().lock() {
        if let Some(entry) = entries.get(&connection.stable_id()) {
            entry.wake.notify_one();
        }
    }
}

/// Call only after the existing application identity/proof has been verified.
pub fn authenticated_remote_capability(connection: &Connection, supported: bool) {
    if !supported {
        return;
    }
    if let Ok(entries) = registry().lock() {
        if let Some(entry) = entries.get(&connection.stable_id()) {
            entry
                .status
                .send_modify(|status| status.remote_guarantee = RemoteRouteGuarantee::BothSenders);
        }
    }
}

pub(super) fn paused_duration(connection: &Connection) -> Duration {
    let entry = registry()
        .lock()
        .ok()
        .and_then(|entries| entries.get(&connection.stable_id()).cloned());
    let Some(entry) = entry else {
        return Duration::ZERO;
    };
    let paused = entry
        .clock
        .lock()
        .map(|clock| {
            clock
                .0
                .saturating_add(clock.1.map_or(Duration::ZERO, |since| since.elapsed()))
        })
        .unwrap_or_default();
    paused
}

pub fn diagnostics(local: &str, remote: Option<&str>) -> Vec<RouteDiagnostics> {
    let Ok(entries) = registry().lock() else {
        return Vec::new();
    };
    entries
        .values()
        .filter(|entry| entry.local == local && remote.is_none_or(|remote| entry.remote == remote))
        .map(|entry| RouteDiagnostics {
            connection_id: entry.id,
            protocol: entry.protocol.clone(),
            status: entry.status.borrow().clone(),
            budget: entry
                .budget
                .lock()
                .map(|budget| budget.clone())
                .unwrap_or_default(),
            measurements: entry
                .pair
                .as_deref()
                .map(super::route_history::samples)
                .unwrap_or_default(),
            retry: entry.retry.facts(),
        })
        .collect()
}

/// Transport identity has already been checked by Iroh; caller preserves its own
/// explicit absolute deadline and cancellation select while awaiting this future.
pub async fn wait_application_route(
    connection: &Connection,
    deadline: Option<Instant>,
) -> io::Result<RouteStatus> {
    let mut receiver =
        subscribe(connection).ok_or_else(|| failed("Verbindung hat keinen Routemonitor"))?;
    wake(connection);
    loop {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Verbindungsaufbau/Pfadwechsel: Gesamtfrist abgelaufen",
            ));
        }
        let status = receiver.borrow().clone();
        if status.application_allowed && connection.application_data_allowed() {
            return Ok(status);
        }
        if connection.close_reason().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "Share-Verbindung geschlossen",
            ));
        }
        if status.phase == RoutePhase::Cancelled
            || (status.phase == RoutePhase::Offline
                && !network_wait::offline_is_outdated(
                    &status,
                    &connection.application_path_policy(),
                ))
        {
            return Err(io::Error::new(io::ErrorKind::NotConnected, status.reason));
        }
        let expires = async {
            if let Some(deadline) = deadline {
                tokio::time::sleep_until(deadline.into()).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            result = receiver.changed() => if result.is_err() { return Err(failed("Routemonitor beendet")); },
            _ = connection.closed() => return Err(io::Error::new(io::ErrorKind::NotConnected, "Share-Verbindung geschlossen")),
            _ = expires => return Err(io::Error::new(io::ErrorKind::TimedOut, "Verbindungsaufbau/Pfadwechsel: Gesamtfrist abgelaufen")),
        }
    }
}
