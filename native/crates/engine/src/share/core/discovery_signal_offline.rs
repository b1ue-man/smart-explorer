//! The worker without a server connection: waiting for a connection
//! attempt or the reconnect backoff. Both wait for events (commands, the
//! service's stop, power changes and probes, repair completions, the
//! attempt's result) instead of polling, at most until the next discovery
//! offer expires or 30 s pass.

use std::io;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Select};

use super::core::eio;
use super::signal_commands::{run_offline_command, OfflineCommandRuntime};
use super::signal_connector::NegotiatedSignal;
use super::signal_worker::{drain_repair_completions, WorkerRuntime};
use super::types::PendingShareCmd;

/// Longest offline wait without any event.
const OFFLINE_WAIT_CAP: Duration = Duration::from_secs(30);

pub(super) enum ConnectionWait {
    Ready(io::Result<NegotiatedSignal>),
    Stopped,
}

pub(super) enum OfflineWait {
    Stopped,
    /// A probe asks to connect now.
    Probe,
    Elapsed,
}

enum OfflineEvent {
    Connected(io::Result<NegotiatedSignal>),
    Command(PendingShareCmd),
    Stopped,
    Other,
}

pub(super) fn wait_for_connection(
    connector: &Receiver<io::Result<NegotiatedSignal>>,
    runtime: &mut WorkerRuntime<'_>,
) -> ConnectionWait {
    loop {
        // Probes wait for this attempt's result.
        if offline_turn(runtime) {
            return ConnectionWait::Stopped;
        }
        match wait_event(runtime, Some(connector), OFFLINE_WAIT_CAP) {
            OfflineEvent::Connected(result) => return ConnectionWait::Ready(result),
            OfflineEvent::Command(pending) => {
                if acknowledge_offline(pending, runtime) {
                    return ConnectionWait::Stopped;
                }
            }
            OfflineEvent::Stopped => return ConnectionWait::Stopped,
            OfflineEvent::Other => {}
        }
    }
}

pub(super) fn wait_offline_backoff(
    duration: Duration,
    runtime: &mut WorkerRuntime<'_>,
) -> OfflineWait {
    let deadline = runtime.power.now().mono + duration;
    loop {
        if offline_turn(runtime) {
            return OfflineWait::Stopped;
        }
        if runtime.power.probe_pending() {
            return OfflineWait::Probe;
        }
        let now = runtime.power.now().mono;
        if now >= deadline {
            return OfflineWait::Elapsed;
        }
        match wait_event(runtime, None, deadline.saturating_duration_since(now)) {
            OfflineEvent::Command(pending) => {
                if acknowledge_offline(pending, runtime) {
                    return OfflineWait::Stopped;
                }
            }
            OfflineEvent::Stopped => return OfflineWait::Stopped,
            OfflineEvent::Connected(_) | OfflineEvent::Other => {}
        }
    }
}

/// Work of every offline wake; returns whether the worker stops.
fn offline_turn(runtime: &mut WorkerRuntime<'_>) -> bool {
    drain_repair_completions(runtime);
    runtime.discovery.maintain_offline(runtime.events);
    runtime.power.absorb(runtime.iroh);
    runtime.stopped()
}

fn wait_event(
    runtime: &WorkerRuntime<'_>,
    connector: Option<&Receiver<io::Result<NegotiatedSignal>>>,
    limit: Duration,
) -> OfflineEvent {
    let mut timeout = limit.min(OFFLINE_WAIT_CAP);
    if let Some(due) = runtime.discovery.next_offline_due() {
        timeout = timeout.min(due.saturating_duration_since(Instant::now()));
    }
    let timeout = runtime.power.hub().clock().real_wait(timeout);
    let wake = runtime.iroh.signal_wake();
    let completions = runtime.repair_completions.receiver();
    let mut select = Select::new();
    let commands_index = select.recv(runtime.commands);
    let wake_index = select.recv(wake);
    let connector_index = connector.map(|receiver| select.recv(receiver));
    let completions_index = completions.map(|receiver| select.recv(receiver));
    let Ok(operation) = select.select_timeout(timeout) else {
        return OfflineEvent::Other;
    };
    let index = operation.index();
    if index == commands_index {
        return match operation.recv(runtime.commands) {
            Ok(pending) => OfflineEvent::Command(pending),
            Err(_) => OfflineEvent::Stopped,
        };
    }
    if index == wake_index {
        let _ = operation.recv(wake);
        return OfflineEvent::Other;
    }
    if let (Some(receiver), Some(selected)) = (connector, connector_index) {
        if index == selected {
            return OfflineEvent::Connected(operation.recv(receiver).unwrap_or_else(|_| {
                Err(eio("Share-Verbindungsversuch wurde unerwartet beendet"))
            }));
        }
    }
    if let (Some(receiver), Some(_)) = (completions, completions_index) {
        // A disconnected channel is noticed by the next drain.
        let _ = operation.recv(receiver);
    }
    OfflineEvent::Other
}

fn acknowledge_offline(pending: PendingShareCmd, runtime: &mut WorkerRuntime<'_>) -> bool {
    runtime.discovery.maintain_offline(runtime.events);
    if Instant::now() > pending.expires_at {
        let _ = pending.acknowledgement.send(Err(
            "Share-Kommando ist vor der Verarbeitung abgelaufen".into(),
        ));
        return false;
    }
    let mut command_runtime = OfflineCommandRuntime {
        auth: runtime.auth,
        iroh: runtime.iroh,
        direct_requests_sent: runtime.direct_requests_sent,
        events: runtime.events,
        discovery: runtime.discovery,
    };
    let outcome = run_offline_command(pending.command, &mut command_runtime);
    let _ = pending
        .acknowledgement
        .send(outcome.result.map_err(|error| error.to_string()));
    if outcome.should_stop {
        runtime
            .stopped_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    outcome.should_stop
}
