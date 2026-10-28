//! Start phase of a connection route (Z-R2, decided 2026-10-28 for "maximal
//! möglicher Datentransferspeed von Anfang zu Ende"): payload may use a
//! validated relay path at once while LAN and direct paths are still being
//! established, and moves to the first admitted LAN/direct path in place.
//!
//! The move is a change of the single admitted QUIC path (the vendored gate's
//! `Allow` replaces the grant atomically). Streams, their offsets and the
//! retransmission queues belong to the connection, not to a path, so no
//! request or file is restarted or sent twice by the application; bytes still
//! in flight on the relay arrive or are recovered by QUIC loss detection on
//! the admitted path. Once a LAN/direct path was admitted in this generation
//! the start bridge closes: the relay carries payload again only as the marked
//! fallback after the unchanged finite direct phase. A new network generation
//! starts a new policy and therefore a new start phase.
use std::time::{Duration, Instant};

use iroh::endpoint::Connection;

use super::super::route_state::{RouteClass, RoutePhase, RouteStatus};
use super::super::types::{PeerEndpoint, ShareEvent};
use super::RouteCandidate;

/// Route lines one connection may add to the worker log (flapping bound).
const MAX_LOG_LINES: usize = 16;
const LOG_LIMIT: &str = "weitere Routewechsel werden nicht protokolliert";

pub(super) const RELAY_START_REASON: &str =
    "Startphase: Nutzdaten über Relay, Direktweg wird aufgebaut; Wechsel ohne Neustart, sobald zugelassen";

/// One chosen path with the phase and reason the route status shows.
pub(super) struct Choice<'a> {
    pub(super) path: &'a RouteCandidate,
    pub(super) phase: RoutePhase,
    pub(super) reason: &'static str,
}

/// Facts of the finite LAN/direct phase that the selection needs.
#[derive(Clone, Copy, Debug)]
pub(super) struct Progress {
    pub(super) lan_done: bool,
    pub(super) direct_done: bool,
    pub(super) expired: bool,
}

/// Start-phase state of one route generation.
#[derive(Debug, Default)]
pub(super) struct StartPhase {
    /// A LAN/direct path was chosen in this generation: the bridge is closed.
    non_relay_chosen: bool,
    /// The bridge closed in the current observation (undone if the gate refuses).
    closed_now: bool,
}

impl StartPhase {
    /// Whether the relay may still carry payload as the start bridge.
    pub(super) fn bridge_open(&self, progress: Progress) -> bool {
        !progress.direct_done && !self.non_relay_chosen
    }

    /// The best validated path: LAN, then direct, then relay (as start bridge
    /// before the direct phase ends, as marked fallback after it).
    pub(super) fn choose<'a>(
        &mut self,
        paths: &'a [RouteCandidate],
        progress: Progress,
    ) -> Option<Choice<'a>> {
        self.closed_now = false;
        let bridge = self.bridge_open(progress);
        let rank = |class| match class {
            RouteClass::Lan => 0,
            RouteClass::Direct => 1,
            RouteClass::Relay => 2,
        };
        let path = paths
            .iter()
            .filter(|path| {
                path.validated
                    && match path.class {
                        RouteClass::Lan | RouteClass::Direct => true,
                        RouteClass::Relay => progress.direct_done || bridge,
                    }
            })
            .min_by_key(|p| (rank(p.class), p.rtt.unwrap_or(Duration::MAX), p.key.clone()))?;
        if path.class != RouteClass::Relay && !self.non_relay_chosen {
            self.non_relay_chosen = true;
            self.closed_now = true;
        }
        let (phase, reason) = match path.class {
            RouteClass::Lan => {
                (RoutePhase::Lan, "Validierter on-link-Pfad über physisches LAN/WLAN-Interface")
            }
            RouteClass::Direct if progress.lan_done => {
                (RoutePhase::Direct, "LANversuche beendet; validierter Direktpfad")
            }
            RouteClass::Direct => {
                (RoutePhase::Direct, "Validierter Direktpfad; LANkandidaten werden weiter geprüft")
            }
            RouteClass::Relay if !progress.direct_done => {
                (RoutePhase::RelayStart, RELAY_START_REASON)
            }
            RouteClass::Relay if progress.expired => (
                RoutePhase::RelayFallback,
                "Absolute Direktfrist abgelaufen; im aktuellen Versuch kein validierter Direktpfad",
            ),
            RouteClass::Relay => (
                RoutePhase::RelayFallback,
                "Initiale Proberunde, Antwortfrist und Pfadvalidierung beendet; kein validierter Direktpfad",
            ),
        };
        Some(Choice {
            path,
            phase,
            reason,
        })
    }

    /// The QUIC gate refused the path chosen in this observation: a refused
    /// first LAN/direct choice does not close the bridge.
    pub(super) fn rejected(&mut self) {
        if self.closed_now {
            self.non_relay_chosen = false;
            self.closed_now = false;
        }
    }
}

/// The note next to a transfer for this route state (progress and lists).
pub(in crate::share) fn progress_hint(status: &RouteStatus) -> Option<&'static str> {
    if !status.application_allowed {
        return None;
    }
    match (status.phase, status.class?) {
        (RoutePhase::RelayStart, RouteClass::Relay) => Some("über Relay, Direktweg wird aufgebaut"),
        (_, RouteClass::Relay) => Some("über Relay: die Bandbreite begrenzt der Relay-Server"),
        (_, RouteClass::Direct) => Some("direkt"),
        (_, RouteClass::Lan) => Some("direkt (LAN/WLAN)"),
    }
}

/// Whether payload of `connection` currently goes over the relay (start
/// bridge or fallback). Extra bulk connections never add relay load.
pub(in crate::share) fn carries_relay(connection: &Connection) -> bool {
    super::super::route_monitor::snapshot(connection)
        .is_some_and(|status| status.class == Some(RouteClass::Relay))
}

/// Application payload bytes (sent plus received) of the paths of
/// `connection` that still exist: `(relay, lan_or_direct)`.
pub(in crate::share) fn payload_bytes_by_class(connection: &Connection) -> (u64, u64) {
    let paths = connection.paths();
    let policy = connection.application_path_policy();
    let mut relay = 0u64;
    let mut other = 0u64;
    for observed in &policy.paths {
        let bytes = observed
            .application_tx_bytes
            .saturating_add(observed.application_rx_bytes);
        let is_relay = paths
            .iter()
            .find(|path| path.id() == observed.token.path_id())
            .is_some_and(|path| path.is_relay());
        if is_relay {
            relay = relay.saturating_add(bytes);
        } else {
            other = other.saturating_add(bytes);
        }
    }
    (relay, other)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mark {
    RelayStart,
    RelayFallback,
    Direct,
    Lan,
}

impl Mark {
    fn of(status: &RouteStatus) -> Option<Self> {
        if !status.application_allowed {
            return None;
        }
        Some(match (status.phase, status.class?) {
            (RoutePhase::RelayStart, RouteClass::Relay) => Self::RelayStart,
            (_, RouteClass::Relay) => Self::RelayFallback,
            (_, RouteClass::Direct) => Self::Direct,
            (_, RouteClass::Lan) => Self::Lan,
        })
    }
}

fn route_line(device: &str, mark: Mark, relay_for: Option<Duration>, relay_bytes: u64) -> String {
    let direct = match mark {
        Mark::Lan => "direkt (LAN/WLAN)",
        _ => "direkt",
    };
    match (mark, relay_for) {
        (Mark::RelayStart, _) => format!("Route zu {device}: über Relay, Direktweg wird aufgebaut"),
        (Mark::RelayFallback, _) => format!(
            "Route zu {device}: über Relay (Rückfall); kein Direktweg im aktuellen Versuch"
        ),
        (Mark::Direct | Mark::Lan, Some(relay_for)) => format!(
            "Route zu {device}: {direct} nach {} ms über Relay ({} KiB Nutzdaten); neue Daten nur noch direkt",
            relay_for.as_millis(),
            relay_bytes / 1024
        ),
        (Mark::Direct | Mark::Lan, None) => format!("Route zu {device}: {direct}"),
    }
}

/// Writes the route changes of one authenticated connection to the worker
/// log: "über Relay, Direktweg wird aufgebaut" → "direkt …". The task ends
/// with the connection's route monitor and holds no strong connection.
/// Must run inside the node runtime.
pub(in crate::share) fn log_route_changes(
    connection: &Connection,
    device: String,
    events: crossbeam_channel::Sender<ShareEvent>,
) {
    let Some(mut status) = super::super::route_monitor::subscribe(connection) else {
        return;
    };
    let handle = connection.weak_handle();
    tokio::spawn(async move {
        let mut last = Mark::of(&status.borrow_and_update());
        let mut relay_since = None;
        let mut lines = 0usize;
        if last == Some(Mark::RelayStart) {
            relay_since = Some(Instant::now());
            let _ = events.try_send(ShareEvent::Status(route_line(
                &device,
                Mark::RelayStart,
                None,
                0,
            )));
            lines += 1;
        }
        while status.changed().await.is_ok() {
            let mark = Mark::of(&status.borrow_and_update());
            let Some(mark) = mark.filter(|mark| Some(*mark) != last) else {
                continue;
            };
            let previous = last.replace(mark);
            let relay_for = match (previous, mark) {
                (Some(Mark::RelayStart | Mark::RelayFallback), Mark::Direct | Mark::Lan) => {
                    relay_since.take().map(|since: Instant| since.elapsed())
                }
                _ => None,
            };
            if matches!(mark, Mark::RelayStart | Mark::RelayFallback) && relay_since.is_none() {
                relay_since = Some(Instant::now());
            }
            if lines >= MAX_LOG_LINES {
                continue;
            }
            let relay_bytes = handle
                .upgrade()
                .map_or(0, |connection| payload_bytes_by_class(&connection).0);
            lines += 1;
            let line = if lines == MAX_LOG_LINES {
                format!("Route zu {device}: {LOG_LIMIT}")
            } else {
                route_line(&device, mark, relay_for, relay_bytes)
            };
            let _ = events.try_send(ShareEvent::Status(line));
        }
    });
}

impl super::super::node::ShareIrohNode {
    /// The route note of the cached session to `endpoint` (transfer progress).
    /// Called by `PeerBackend::transfer_hint` (`backend.rs`).
    pub(in crate::share) fn session_route_hint(&self, endpoint: &PeerEndpoint) -> Option<String> {
        let key = super::super::session::session_key(endpoint);
        let connection = self.sessions.lock().ok()?.get(&key).cloned()?;
        if connection.close_reason().is_some() {
            return None;
        }
        let status = super::super::route_monitor::snapshot(&connection)?;
        progress_hint(&status).map(str::to_string)
    }
}

#[cfg(test)]
#[path = "route_start_tests.rs"]
mod tests;

#[cfg(all(test, target_os = "linux"))]
#[path = "route_start_task_tests.rs"]
mod task_tests;

#[cfg(all(test, target_os = "linux"))]
#[path = "route_start_gate_task_tests.rs"]
mod gate_task_tests;

#[cfg(all(test, target_os = "linux"))]
#[path = "speed_task_share_start_phase_tests.rs"]
mod start_phase_bench;
