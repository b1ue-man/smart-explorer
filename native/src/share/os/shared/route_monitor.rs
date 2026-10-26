//! One policy actor per QUIC connection. Weak handles preserve normal connection lifetime.
use super::route_adapter::RouteAdapter;
use super::route_diagnostics::{milliseconds, RouteBudget, RouteDiagnostics, RouteMeasurement};
use super::route_policy::RoutePolicy;
use super::route_state::{RemoteRouteGuarantee, RoutePhase, RouteStatus};
use iroh::endpoint::{ApplicationPathUpdate, Connection, WeakConnectionHandle};
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

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
    });
    entries.insert(connection.stable_id(), entry.clone());
    tokio::spawn(run(connection.stable_id(), entry, hints));
    Ok(())
}

pub(crate) fn subscribe(connection: &Connection) -> Option<watch::Receiver<RouteStatus>> {
    registry()
        .lock()
        .ok()?
        .get(&connection.stable_id())
        .map(|entry| entry.status.subscribe())
}

pub(super) fn connection(id: usize) -> Option<Connection> {
    registry().lock().ok()?.get(&id)?.handle.upgrade()
}

pub(crate) fn snapshot(connection: &Connection) -> Option<RouteStatus> {
    subscribe(connection).map(|status| {
        let current = status.borrow().clone();
        current
    })
}

pub(crate) fn cancelled(connection: &Connection) {
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
        }
    }
}

/// Call only after the existing application identity/proof has been verified.
pub(crate) fn authenticated_remote_capability(connection: &Connection, supported: bool) {
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

pub(crate) fn diagnostics(local: &str, remote: Option<&str>) -> Vec<RouteDiagnostics> {
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
        })
        .collect()
}

/// Transport identity has already been checked by Iroh; caller preserves its own
/// explicit absolute deadline and cancellation select while awaiting this future.
pub(crate) async fn wait_application_route(
    connection: &Connection,
    deadline: Option<Instant>,
) -> io::Result<RouteStatus> {
    let mut receiver =
        subscribe(connection).ok_or_else(|| failed("Verbindung hat keinen Routemonitor"))?;
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
        if matches!(status.phase, RoutePhase::Offline | RoutePhase::Cancelled) {
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

async fn run(id: usize, entry: Arc<Entry>, hints: Vec<SocketAddr>) {
    let mut origin = Instant::now();
    let mut adapter = RouteAdapter::default();
    let mut policy = None;
    let mut previous_sample = None;
    let mut previous_path = None;
    loop {
        let Some(connection) = entry.handle.upgrade() else {
            break;
        };
        if connection.close_reason().is_some() {
            break;
        }
        let mut snapshot = connection.application_path_policy();
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
            origin = Instant::now();
            policy = None;
            adapter = RouteAdapter::default();
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
        previous_path = current_path;
        if status != old {
            entry.status.send_replace(status.clone());
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
        record_measurement(&entry, &connection, &status, &mut previous_sample);
        // Do not retain a strong connection while idle; cancellation/drop still closes it.
        drop(connection);
        tokio::time::sleep(TICK).await;
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

struct PreviousSample {
    at: Instant,
    tx: u64,
    rx: u64,
    generation: u64,
    path: String,
    class: Option<super::route_state::RouteClass>,
}

fn record_measurement(
    entry: &Entry,
    connection: &Connection,
    status: &RouteStatus,
    previous: &mut Option<PreviousSample>,
) {
    let now = Instant::now();
    let snapshot = connection.application_path_policy();
    let Some(path) = snapshot.paths.iter().find(|path| path.data_allowed) else {
        *previous = None;
        return;
    };
    let current = PreviousSample {
        at: now,
        tx: path.application_tx_bytes,
        rx: path.application_rx_bytes,
        generation: status.generation,
        path: format!("{}/{}", path.token.path_id(), path.token.path_generation()),
        class: status.class,
    };
    if previous
        .as_ref()
        .is_some_and(|previous| previous.at.elapsed() < Duration::from_secs(2))
    {
        return;
    }
    let Some(before) = previous.replace(current) else {
        return;
    };
    let current = previous.as_ref().unwrap_or(&before);
    let Some(pair) = entry.pair.as_deref() else {
        return;
    };
    let Some(class) = before.class.filter(|class| {
        Some(*class) == status.class
            && before.generation == status.generation
            && before.path == current.path
    }) else {
        return;
    };
    if current.tx == before.tx && current.rx == before.rx {
        return;
    }
    let rtt = connection.rtt(path.token.path_id());
    super::route_history::schedule_record(
        pair.into(),
        RouteMeasurement {
            observed_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(milliseconds)
                .unwrap_or(0),
            generation: status.generation,
            class,
            rtt_us: rtt.map(|rtt| u64::try_from(rtt.as_micros()).unwrap_or(u64::MAX)),
            application_tx_bytes: current.tx.saturating_sub(before.tx),
            application_rx_bytes: current.rx.saturating_sub(before.rx),
            elapsed_ms: milliseconds(now.saturating_duration_since(before.at)),
        },
    );
}
