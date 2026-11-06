//! Route samples of an admitted path while application bytes move: at most one
//! per two seconds, recorded into the pinned pair's route history.
use super::super::route_diagnostics::{milliseconds, RouteMeasurement};
use super::super::route_state::RouteStatus;
use super::Entry;
use iroh::endpoint::Connection;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(super) struct PreviousSample {
    at: Instant,
    tx: u64,
    rx: u64,
    generation: u64,
    path: String,
    class: Option<super::super::route_state::RouteClass>,
}

pub(super) fn record_measurement(
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
    super::super::route_history::schedule_record(
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
