//! Self-check: is QUIC address discovery (QAD, UDP 7842) reachable from
//! outside, per address family (TODO `DIREKT-INTERNET`, 2026-11-18)?
//!
//! A firewall that drops UDP 7842 leaves every device behind NAT without a
//! public candidate, so connections across the internet run over the relay;
//! the server itself saw nothing of it. Method, without any third-party
//! service: the server watches what arrives.
//!
//! * Evidence for "reachable": a QAD connection from a public source of that
//!   family completed its handshake (after the address-validating Retry, so
//!   the source really receives) within [`WINDOW`], or is still held. The
//!   vendored QAD server reports each one (`set_connection_observer`,
//!   `vendor/SMART_EXPLORER_QAD_LIMITS.md` point 7).
//! * Evidence for "not reachable": a device that announces `qad_status_v1`
//!   is connected over TLS from a public source of that family for at least
//!   [`GRACE`] and no such QAD connection exists. Such a device uses this
//!   server's TLS relay and probes QAD on it within seconds
//!   (`endpoint_routes.rs` `relay_mode` of the app); older apps are never
//!   counted against QAD.
//! * Otherwise the state is unknown (no device of that family yet).
//!
//! Rejected: a self-probe to the own public address. The server does not
//! know that address without an outside service, and a packet to an own
//! address is delivered locally, past a cloud firewall and usually past the
//! host firewall's loopback rule, so it would report "reachable" in exactly
//! the case it has to find.
//!
//! The verdict is logged on every change and sent to devices with
//! `qad_status_v1`: in `hello_ok` and as `qad_status` when it changes.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use iroh_relay::quic::QadConnectionEvent;
use serde::{Deserialize, Serialize};

use super::limits::SourceKey;
use super::state::{lock_state, State};
use super::{send, Out};

/// Hello capability: the device takes the verdict and probes QAD itself.
pub(super) const CAPABILITY: &str = "qad_status_v1";
/// A connected device has probed QAD well before this.
const GRACE: Duration = Duration::from_secs(120);
/// How long one completed QAD connection counts as evidence after it ended.
const WINDOW: Duration = Duration::from_secs(30 * 60);
/// How often the verdict is evaluated.
const TICK: Duration = Duration::from_secs(30);
/// Memory bound of the sources remembered after their connection ended;
/// held ones are bounded by the QAD connection limits.
const MAX_ENDED_SOURCES: usize = 4096;

/// The state of one address family.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Reach {
    Reachable,
    Unreachable,
    #[default]
    Unknown,
    /// QAD does not listen for this family.
    Off,
}

/// The verdict sent to devices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct QadReport {
    pub(super) ipv4: Reach,
    pub(super) ipv6: Reach,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    V4,
    V6,
}

struct QadSource {
    held: usize,
    last: Instant,
}

struct Witness {
    source: SourceKey,
    since: Instant,
}

#[derive(Default)]
struct Tracker {
    /// Families the QAD listener answers; both `false` while QAD is off.
    listening: (bool, bool),
    qad: HashMap<SourceKey, QadSource>,
    witnesses: HashMap<u64, Witness>,
    next_witness: u64,
    report: QadReport,
}

fn tracker() -> MutexGuard<'static, Tracker> {
    static TRACKER: OnceLock<Mutex<Tracker>> = OnceLock::new();
    TRACKER
        .get_or_init(|| Mutex::new(Tracker::new()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

impl Tracker {
    fn new() -> Self {
        Self {
            report: QadReport {
                ipv4: Reach::Off,
                ipv6: Reach::Off,
            },
            ..Self::default()
        }
    }

    fn evaluate(&mut self, now: Instant) -> QadReport {
        self.qad
            .retain(|_, entry| entry.held > 0 || now.duration_since(entry.last) <= WINDOW);
        let judge_family = |family: Family, listening: bool| {
            let qad_seen = self.qad.iter().any(|(source, entry)| {
                family_of(*source) == Some(family)
                    && (entry.held > 0 || now.duration_since(entry.last) <= WINDOW)
            });
            let witness_ready = self.witnesses.values().any(|witness| {
                family_of(witness.source) == Some(family)
                    && now.duration_since(witness.since) >= GRACE
            });
            judge(listening, qad_seen, witness_ready)
        };
        QadReport {
            ipv4: judge_family(Family::V4, self.listening.0),
            ipv6: judge_family(Family::V6, self.listening.1),
        }
    }
}

/// The verdict of one family from its evidence.
pub(super) fn judge(listening: bool, qad_seen: bool, witness_ready: bool) -> Reach {
    if !listening {
        Reach::Off
    } else if qad_seen {
        Reach::Reachable
    } else if witness_ready {
        Reach::Unreachable
    } else {
        Reach::Unknown
    }
}

/// The family of a public source; `None` for loopback, private, link-local,
/// shared (CGNAT) and unique-local sources and for a trusted proxy, which
/// prove nothing about reachability from the internet.
fn family_of(source: SourceKey) -> Option<Family> {
    match source {
        SourceKey::Ipv4(octets) => {
            let ip = std::net::Ipv4Addr::from(octets);
            let shared = octets[0] == 100 && (octets[1] & 0xc0) == 64;
            let local = ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || shared;
            (!local).then_some(Family::V4)
        }
        // Global unicast 2000::/3 only.
        SourceKey::Ipv6Prefix64(prefix) => ((prefix[0] & 0xe0) == 0x20).then_some(Family::V6),
        SourceKey::ExternallyLimitedProxy => None,
    }
}

/// The QAD listener runs at `address`; a `[::]` listener answers both
/// families (dual stack, `relay_qad.rs`).
pub(super) fn listening(address: std::net::SocketAddr) {
    let families = match address.ip() {
        IpAddr::V4(_) => (true, false),
        IpAddr::V6(ip) if ip.is_unspecified() => (true, true),
        IpAddr::V6(ip) => (ip.to_ipv4_mapped().is_some(), ip.to_ipv4_mapped().is_none()),
    };
    let mut tracker = tracker();
    tracker.listening = families;
    tracker.report = QadReport {
        ipv4: judge(families.0, false, false),
        ipv6: judge(families.1, false, false),
    };
    drop(tracker);
    iroh_relay::quic::set_connection_observer(observe);
    eprintln!(
        "se-share-server QAD self-check: reachability from outside is judged from devices \
         connecting over TLS (unknown until one connects from a public address)"
    );
}

/// The QAD server's report of one connection (vendored observer hook).
fn observe(source: IpAddr, event: QadConnectionEvent) {
    let key = SourceKey::from_ip(source);
    let now = Instant::now();
    let mut tracker = tracker();
    if !tracker.qad.contains_key(&key) && tracker.qad.len() >= MAX_ENDED_SOURCES {
        tracker
            .qad
            .retain(|_, entry| entry.held > 0 || now.duration_since(entry.last) <= WINDOW);
        if tracker.qad.len() >= MAX_ENDED_SOURCES {
            // Ended sources go first; one recent one per family is enough
            // evidence, and the next connection brings it back.
            tracker.qad.retain(|_, entry| entry.held > 0);
        }
    }
    let entry = tracker
        .qad
        .entry(key)
        .or_insert(QadSource { held: 0, last: now });
    match event {
        QadConnectionEvent::Established => {
            entry.held += 1;
            entry.last = now;
        }
        QadConnectionEvent::Closed => {
            entry.held = entry.held.saturating_sub(1);
            entry.last = now;
        }
    }
}

/// A device that counts as evidence while this guard lives.
pub(super) struct WitnessGuard(u64);

/// Registers a device connected over TLS from `source` that announced
/// [`CAPABILITY`].
pub(super) fn witness(source: SourceKey) -> WitnessGuard {
    let mut tracker = tracker();
    tracker.next_witness = tracker.next_witness.wrapping_add(1);
    let id = tracker.next_witness;
    tracker.witnesses.insert(
        id,
        Witness {
            source,
            since: Instant::now(),
        },
    );
    WitnessGuard(id)
}

impl Drop for WitnessGuard {
    fn drop(&mut self) {
        tracker().witnesses.remove(&self.0);
    }
}

/// The verdict last evaluated.
pub(super) fn current() -> QadReport {
    tracker().report
}

/// The verdict for `hello_ok` of a device with these negotiated capabilities.
pub(super) fn for_hello(capabilities: &HashSet<String>) -> Option<QadReport> {
    capabilities.contains(CAPABILITY).then(current)
}

/// Evaluates every [`TICK`]; a change is logged and sent to the devices that
/// announced [`CAPABILITY`].
pub(super) fn start(state: Arc<Mutex<State>>) {
    let spawned = std::thread::Builder::new()
        .name("qad-self-check".into())
        .spawn(move || loop {
            std::thread::sleep(TICK);
            let changed = {
                let mut tracker = tracker();
                let report = tracker.evaluate(Instant::now());
                let changed = report != tracker.report;
                tracker.report = report;
                changed.then_some(report)
            };
            if let Some(report) = changed {
                eprintln!("{}", log_line(report));
                broadcast(&state, report);
            }
        });
    if let Err(error) = spawned {
        eprintln!("se-share-server: QAD self-check not started: {error}");
    }
}

fn broadcast(state: &Arc<Mutex<State>>, report: QadReport) {
    let writers: Vec<_> = lock_state(state)
        .clients
        .values()
        .filter(|client| client.capabilities.contains(CAPABILITY))
        .map(|client| client.writer.clone())
        .collect();
    for writer in writers {
        send(&writer, &Out::QadStatus { qad: report });
    }
}

/// The log line of a verdict.
pub(super) fn log_line(report: QadReport) -> String {
    format!(
        "se-share-server QAD (UDP 7842) from outside: IPv4 {}, IPv6 {}",
        reach_text(report.ipv4),
        reach_text(report.ipv6)
    )
}

fn reach_text(reach: Reach) -> &'static str {
    match reach {
        Reach::Reachable => "reachable",
        Reach::Unreachable => {
            "NOT reachable (devices connected over TLS from outside, no QAD connection arrived; \
             open UDP 7842 in every firewall)"
        }
        Reach::Unknown => "unknown (no device from outside yet)",
        Reach::Off => "off",
    }
}

#[cfg(test)]
#[path = "qad_reach_tests.rs"]
mod qad_reach_tests;
