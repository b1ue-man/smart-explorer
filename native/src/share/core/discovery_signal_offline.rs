use std::io;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;

use super::core::eio;
use super::signal_commands::{run_offline_command, OfflineCommandRuntime};
use super::signal_connector::NegotiatedSignal;
use super::signal_worker::WorkerRuntime;

pub(super) enum ConnectionWait {
    Ready(io::Result<NegotiatedSignal>),
    Stopped,
}

pub(super) fn wait_for_connection(
    connector: &Receiver<io::Result<NegotiatedSignal>>,
    runtime: &mut WorkerRuntime<'_>,
) -> ConnectionWait {
    loop {
        super::signal_worker::drain_repair_completions(runtime);
        runtime.discovery.maintain_offline(runtime.events);
        if runtime.stopped_flag.load(std::sync::atomic::Ordering::Relaxed) {
            return ConnectionWait::Stopped;
        }
        match connector.try_recv() {
            Ok(result) => return ConnectionWait::Ready(result),
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                return ConnectionWait::Ready(Err(eio(
                    "Share-Verbindungsversuch wurde unerwartet beendet",
                )))
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {}
        }
        match runtime.commands.recv_timeout(Duration::from_millis(25)) {
            Ok(pending) => {
                if acknowledge_offline(pending, runtime) {
                    return ConnectionWait::Stopped;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                return ConnectionWait::Stopped
            }
        }
    }
}

pub(super) fn wait_offline_backoff(
    duration: Duration,
    runtime: &mut WorkerRuntime<'_>,
) -> bool {
    let deadline = Instant::now() + duration;
    loop {
        super::signal_worker::drain_repair_completions(runtime);
        runtime.discovery.maintain_offline(runtime.events);
        if runtime.stopped_flag.load(std::sync::atomic::Ordering::Relaxed) {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        let remaining = deadline.saturating_duration_since(now);
        match runtime
            .commands
            .recv_timeout(remaining.min(Duration::from_millis(50)))
        {
            Ok(pending) => {
                if acknowledge_offline(pending, runtime) {
                    return true;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return true,
        }
    }
}

fn acknowledge_offline(
    pending: super::types::PendingShareCmd,
    runtime: &mut WorkerRuntime<'_>,
) -> bool {
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
