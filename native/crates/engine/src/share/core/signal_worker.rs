//! The Share signal worker: connects to the Share server, keeps the
//! connection (normal or idle mode) and runs commands, offline as well.
//! It waits for events (commands, data, power changes, probes, repair
//! completions, timers) instead of polling.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::Receiver;

use super::backend::ShareIrohNode;
use super::direct_reciprocal_coordinator::{
    DirectReciprocalCoordinator, DirectRepairCompletionReceiver,
};
use super::discovery_signal_commands::DiscoverySignalRuntime;
use super::discovery_signal_offline::{
    wait_for_connection, wait_offline_backoff, ConnectionWait, OfflineWait,
};
use super::discovery_signal_port::DiscoveryExchangePort;
use super::identity::ShareIdentity;
use super::power::{ProbeOutcome, CONNECT_HOLD_MS};
use super::signal_connector::spawn_connect;
use super::tracked_signal_sender::AttemptCounters;
use super::types::{PendingShareCmd, ShareAuthState, ShareEvent};

#[path = "signal_connected.rs"]
mod connected;
#[path = "signal_idle.rs"]
mod idle;
#[path = "signal_publish.rs"]
mod publish;
#[path = "signal_readiness.rs"]
mod readiness;
#[path = "signal_schedule.rs"]
mod schedule;
#[path = "signal_power.rs"]
pub(super) mod worker_power;

pub(super) use self::publish::{publish_all, send_direct_answer, send_direct_request};
use self::worker_power::WorkerPower;

const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// A session that served this long resets the reconnect backoff; a server
/// that drops every new session right away is not hammered (S53).
const HEALTHY_SESSION: Duration = Duration::from_secs(60);
/// Longest wait of a worker without server when nothing happens.
const SERVERLESS_WAIT: Duration = Duration::from_secs(30);

#[allow(clippy::too_many_arguments)]
pub(super) fn worker(
    server: String,
    identity: ShareIdentity,
    iroh: Arc<ShareIrohNode>,
    auth: Arc<Mutex<ShareAuthState>>,
    commands: Receiver<PendingShareCmd>,
    events: crossbeam_channel::Sender<ShareEvent>,
    stopped_flag: Arc<AtomicBool>,
    discovery_port: Box<dyn DiscoveryExchangePort>,
    reciprocal: Arc<DirectReciprocalCoordinator>,
    repair_completions: DirectRepairCompletionReceiver,
) {
    let mut direct_requests_sent = HashSet::new();
    let mut tracked_attempts = AttemptCounters::new();
    let mut discovery = DiscoverySignalRuntime::with_port(discovery_port);
    let mut power = WorkerPower::subscribe(&iroh);
    let _reciprocal_guard = reciprocal;
    let mut runtime = WorkerRuntime {
        auth: &auth,
        iroh: &iroh,
        commands: &commands,
        events: &events,
        stopped_flag: &stopped_flag,
        direct_requests_sent: &mut direct_requests_sent,
        tracked_attempts: &mut tracked_attempts,
        discovery: &mut discovery,
        repair_completions: &repair_completions,
        power: &mut power,
    };
    // Without a signaling server the worker stays in offline mode: Direct
    // peers are still reachable through local-network presence, and every
    // command is acknowledged by the offline runtime.
    if server.trim().is_empty() {
        let _ = events.send(ShareEvent::Status(
            "Kein Share-Server konfiguriert: Direktgeraete nur ueber das lokale Netz".into(),
        ));
        serverless(&mut runtime);
        return;
    }
    connect_loop(&server, &identity, &mut runtime);
}

pub(super) struct WorkerRuntime<'a> {
    pub(super) auth: &'a Arc<Mutex<ShareAuthState>>,
    pub(super) iroh: &'a ShareIrohNode,
    pub(super) commands: &'a Receiver<PendingShareCmd>,
    pub(super) events: &'a crossbeam_channel::Sender<ShareEvent>,
    pub(super) stopped_flag: &'a AtomicBool,
    pub(super) direct_requests_sent: &'a mut HashSet<String>,
    pub(super) tracked_attempts: &'a mut AttemptCounters,
    pub(super) discovery: &'a mut DiscoverySignalRuntime,
    pub(super) repair_completions: &'a DirectRepairCompletionReceiver,
    pub(super) power: &'a mut WorkerPower,
}

impl WorkerRuntime<'_> {
    pub(super) fn stopped(&self) -> bool {
        self.stopped_flag.load(Ordering::Relaxed)
    }
}

pub(super) fn drain_repair_completions(runtime: &WorkerRuntime<'_>) {
    if runtime.repair_completions.drain() {
        let _ = runtime.events.send(ShareEvent::RuntimeProfilesCommitted);
    }
}

/// LAN-only operation: a probe has nothing to reconnect and is answered
/// once the network change and the idle sweep went through.
fn serverless(runtime: &mut WorkerRuntime<'_>) {
    loop {
        match wait_offline_backoff(SERVERLESS_WAIT, runtime) {
            OfflineWait::Stopped => return,
            OfflineWait::Probe => runtime.power.answer(ProbeOutcome {
                ok: true,
                reconnected: false,
            }),
            OfflineWait::Elapsed => {}
        }
    }
}

fn connect_loop(server: &str, identity: &ShareIdentity, runtime: &mut WorkerRuntime<'_>) {
    let mut backoff = Duration::from_secs(1);
    let mut tuning = idle::KeepaliveTuning::default();
    while !runtime.stopped() {
        // The CPU may sleep through the backoff, never through an attempt
        // (the hold is a no-op outside low power).
        runtime.power.hold(CONNECT_HOLD_MS);
        let connector = match spawn_connect(server.to_string(), identity.clone()) {
            Ok(connector) => connector,
            Err(error) => {
                let _ = runtime.events.send(ShareEvent::ServerDisconnected(format!(
                    "Share-Verbindungsversuch konnte nicht starten: {error}"
                )));
                runtime.power.answer(ProbeOutcome::default());
                if !backoff_wait(&mut backoff, runtime) {
                    return;
                }
                continue;
            }
        };
        let negotiated = match wait_for_connection(&connector, runtime) {
            ConnectionWait::Stopped => return,
            ConnectionWait::Ready(result) => result,
        };
        match negotiated {
            Ok(negotiated) => {
                let started = std::time::Instant::now();
                if connected::run(negotiated, runtime, &mut tuning) {
                    return;
                }
                if started.elapsed() >= HEALTHY_SESSION {
                    backoff = Duration::from_secs(1);
                }
            }
            Err(error) => {
                let _ = runtime.events.send(ShareEvent::ServerDisconnected(format!(
                    "Share-Server nicht erreichbar: {error}"
                )));
                runtime.power.answer(ProbeOutcome::default());
            }
        }
        if runtime.stopped() || !backoff_wait(&mut backoff, runtime) {
            return;
        }
    }
}

/// Waits out the reconnect backoff; a probe ends it early and keeps the
/// backoff. Returns false once the worker stops.
fn backoff_wait(backoff: &mut Duration, runtime: &mut WorkerRuntime<'_>) -> bool {
    match wait_offline_backoff(*backoff, runtime) {
        OfflineWait::Stopped => false,
        OfflineWait::Probe => true,
        OfflineWait::Elapsed => {
            *backoff = (*backoff * 2).min(MAX_BACKOFF);
            true
        }
    }
}

#[cfg(test)]
#[path = "signal_worker_tests.rs"]
mod android_background_task_worker_tests;
