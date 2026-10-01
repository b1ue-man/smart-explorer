//! One connected signal session: waits for events instead of polling,
//! follows the process power state with the server's idle mode (V1) and
//! answers probes.

use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::Select;

use super::idle::{IdleLink, KeepaliveTuning};
use super::readiness::SignalReadiness;
use super::schedule::{SignalMode, SignalSchedule};
use super::{drain_repair_completions, WorkerRuntime};
use crate::share::configuration_runtime::RuntimeConfiguration;
use crate::share::core::now_secs;
use crate::share::power::clock::Now;
use crate::share::power::{ProbeOutcome, SignalPowerStatus};
use crate::share::signal_connection::SignalConnection;
use crate::share::signal_connector::NegotiatedSignal;
use crate::share::signal_handshake::SignalCapabilities;
use crate::share::tracked_signal_dispatch::{dispatch_server_line, SignalDispatchOutcome};
use crate::share::tracked_signal_sender::send_pending_tracked;
use crate::share::types::{PendingShareCmd, ShareAuthState, ShareEvent};
use crate::share::wire::IdleServerMsg;

#[path = "signal_session.rs"]
mod upkeep;

/// Wait bound when no timer is armed (never in practice).
const MAX_EVENT_WAIT: Duration = Duration::from_secs(300);

/// How a session ended.
enum End {
    Stopped,
    /// The worker decided to reconnect (silent server, failed probe, ...).
    Reconnect,
    /// Reading reported the end of the stream or a transport error.
    ReadEnded,
    CommandFailed,
}

enum Woken {
    Readable,
    Command(PendingShareCmd),
    Event,
    CommandsClosed,
    ReadinessLost,
}

/// Runs the session until it ends; returns whether the worker stops.
pub(super) fn run(
    mut negotiated: NegotiatedSignal,
    runtime: &mut WorkerRuntime<'_>,
    tuning: &mut KeepaliveTuning,
) -> bool {
    let capabilities = negotiated.capabilities;
    let connection = &mut negotiated.connection;
    let _ = runtime.events.send(ShareEvent::ServerConnected);
    let _ = runtime.events.send(ShareEvent::Status(format!(
        "Share-Server verbunden ({}, tracked_direct={}, discovery_exchange={})",
        negotiated.transport, capabilities.tracked_direct, capabilities.discovery_exchange,
    )));
    if let Err(error) =
        runtime
            .discovery
            .connected(connection, capabilities.discovery_exchange, runtime.events)
    {
        let _ = runtime.events.send(ShareEvent::ServerDisconnected(format!(
            "Discovery-Signaling konnte nicht initialisiert werden: {error}"
        )));
        return setup_failed(runtime);
    }
    let now = runtime.power.now();
    let mut session = Session {
        capabilities,
        schedule: SignalSchedule::new(now),
        link: IdleLink::new(capabilities.idle_keepalive),
        readiness: None,
        published_routes: runtime.iroh.route_revision(),
        relay_was_connected: true,
    };
    if let Err(error) = session.publish(connection, runtime, now) {
        let _ = runtime.events.send(ShareEvent::ServerDisconnected(format!(
            "Share-Presence konnte nicht sicher erzeugt werden: {error}"
        )));
        return setup_failed(runtime);
    }
    if capabilities.tracked_direct
        && send_pending_tracked(
            connection,
            runtime.auth,
            runtime.iroh,
            runtime.events,
            runtime.tracked_attempts,
        )
        .is_err()
    {
        let _ = runtime.events.send(ShareEvent::ServerDisconnected(
            "Direct-Outbox konnte nicht gesendet werden".into(),
        ));
        return setup_failed(runtime);
    }
    // Without a watcher the session polls its socket like before.
    session.readiness = SignalReadiness::attach(connection).ok();
    runtime.power.claim_status(SignalPowerStatus {
        idle_supported: Some(capabilities.idle_keepalive),
        idle_active: false,
        keepalive_secs: None,
        last_server_contact_unix: Some(now.unix_secs()),
    });
    runtime.power.answer(ProbeOutcome {
        ok: true,
        reconnected: true,
    });

    let end = session.run(connection, runtime, tuning);

    if matches!(end, End::ReadEnded) {
        let now = runtime.power.now();
        let silent = now.wall_since(&session.schedule.last_inbound());
        tuning.transport_ended(
            session.link.active_secs(),
            i64::try_from(silent.as_millis()).unwrap_or(i64::MAX),
        );
    }
    runtime.power.update_status(|status| {
        status.idle_active = false;
        status.keepalive_secs = None;
    });
    drop(session);
    runtime.discovery.disconnected(runtime.events);
    let message = match end {
        End::CommandFailed => "Signaling-Kommando fehlgeschlagen",
        _ => "Signaling getrennt",
    };
    let _ = runtime
        .events
        .send(ShareEvent::ServerDisconnected(message.into()));
    matches!(end, End::Stopped)
}

/// A session that failed before it served: pending probes fail now, or the
/// next attempt would follow without backoff.
fn setup_failed(runtime: &mut WorkerRuntime<'_>) -> bool {
    runtime.discovery.disconnected(runtime.events);
    runtime.power.answer(ProbeOutcome::default());
    false
}

struct Session {
    capabilities: SignalCapabilities,
    schedule: SignalSchedule,
    link: IdleLink,
    readiness: Option<SignalReadiness>,
    published_routes: u64,
    relay_was_connected: bool,
}

impl Session {
    fn run(
        &mut self,
        connection: &mut SignalConnection,
        runtime: &mut WorkerRuntime<'_>,
        tuning: &mut KeepaliveTuning,
    ) -> End {
        // The handshake may have left messages in the read buffers; the
        // first drain takes them and arms the watcher.
        let mut readable = self.readiness.is_some();
        let mut command = None;
        loop {
            drain_repair_completions(runtime);
            if runtime.stopped() {
                return End::Stopped;
            }
            let change = runtime.power.absorb(runtime.iroh);
            if change.left_low_power {
                self.schedule.heartbeat_soon();
                runtime.discovery.refresh_list_soon();
            }
            readable |= self.take_ready();
            if let Some(end) = self.receive(&mut readable, connection, runtime, tuning) {
                return end;
            }
            if let Some(end) = self.follow_power(connection, runtime, tuning) {
                return end;
            }
            let commands = runtime.commands;
            for pending in command.take().into_iter().chain(commands.try_iter()) {
                if let Some(end) = self.command(pending, connection, runtime) {
                    return end;
                }
            }
            let now = runtime.power.now();
            let mode = self.mode(runtime);
            if runtime.power.probe_pending() && !self.schedule.probing() {
                if let Some(end) = self.probe(connection, runtime, now, mode) {
                    return end;
                }
            }
            if let Some(end) = self.maintain(connection, runtime, now, mode) {
                return end;
            }
            if readable {
                continue;
            }
            match self.wait(runtime, self.next_wake(runtime, now, mode)) {
                Woken::Readable => readable = true,
                Woken::Command(pending) => command = Some(pending),
                Woken::Event => {}
                Woken::CommandsClosed => return End::Stopped,
                Woken::ReadinessLost => return End::ReadEnded,
            }
        }
    }

    fn take_ready(&self) -> bool {
        self.readiness
            .as_ref()
            .is_some_and(|readiness| readiness.ready().try_recv().is_ok())
    }

    /// Reads what arrived: everything available with a watcher, otherwise
    /// one message within the short poll like before.
    fn receive(
        &mut self,
        readable: &mut bool,
        connection: &mut SignalConnection,
        runtime: &mut WorkerRuntime<'_>,
        tuning: &mut KeepaliveTuning,
    ) -> Option<End> {
        if self.readiness.is_none() {
            return match connection.read_message() {
                Ok(Some(line)) => self.line(&line, connection, runtime, tuning),
                Ok(None) => Some(End::ReadEnded),
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.kind() == io::ErrorKind::TimedOut =>
                {
                    None
                }
                Err(_) => Some(End::ReadEnded),
            };
        }
        if !std::mem::take(readable) {
            return None;
        }
        let Ok(drained) = connection.drain_messages() else {
            return Some(End::ReadEnded);
        };
        for message in &drained.messages {
            if let Some(end) = self.line(message, connection, runtime, tuning) {
                return Some(end);
            }
        }
        if drained.closed || drained.error.is_some() {
            return Some(End::ReadEnded);
        }
        if drained.more {
            *readable = true;
        } else if let Some(readiness) = &self.readiness {
            readiness.rearm();
        }
        None
    }

    fn line(
        &mut self,
        line: &str,
        connection: &mut SignalConnection,
        runtime: &mut WorkerRuntime<'_>,
        tuning: &mut KeepaliveTuning,
    ) -> Option<End> {
        let now = runtime.power.now();
        self.schedule.inbound(now);
        runtime.power.update_status(|status| {
            status.last_server_contact_unix = Some(now.unix_secs());
        });
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        if let Ok(Some(message)) = IdleServerMsg::parse(line) {
            return self.idle_message(message, connection, runtime, tuning);
        }
        let mut configuration = RuntimeConfiguration {
            auth: runtime.auth,
            iroh: runtime.iroh,
            direct_requests_sent: runtime.direct_requests_sent,
        };
        match dispatch_server_line(
            line,
            self.capabilities.tracked_direct,
            self.capabilities.discovery_exchange,
            runtime.discovery,
            connection,
            runtime.auth,
            runtime.events,
            &mut configuration,
        ) {
            SignalDispatchOutcome::Pong => {
                if self.schedule.pong_received() {
                    runtime.power.answer(ProbeOutcome {
                        ok: true,
                        reconnected: false,
                    });
                }
                None
            }
            SignalDispatchOutcome::Continue => None,
            SignalDispatchOutcome::Reconnect => Some(End::Reconnect),
        }
    }

    fn idle_message(
        &mut self,
        message: IdleServerMsg,
        connection: &mut SignalConnection,
        runtime: &mut WorkerRuntime<'_>,
        tuning: &mut KeepaliveTuning,
    ) -> Option<End> {
        if !self.link.supported() {
            let _ = runtime.events.send(ShareEvent::Error(
                "Ruhemodus-Nachricht ohne ausgehandelte Faehigkeit verworfen".into(),
            ));
            return None;
        }
        match message {
            IdleServerMsg::IdleAck {
                idle,
                keepalive_secs,
            } => {
                if self.link.acknowledged(idle, keepalive_secs) {
                    let active = self.link.active_secs();
                    runtime.power.update_status(|status| {
                        status.idle_active = active.is_some();
                        status.keepalive_secs = active;
                    });
                    let _ = runtime.events.send(ShareEvent::Status(match active {
                        Some(secs) => format!("Share-Server-Ruhemodus aktiv (Keepalive {secs} s)"),
                        None => "Share-Server-Ruhemodus beendet".into(),
                    }));
                }
                None
            }
            IdleServerMsg::Keepalive => self.keepalive(connection, runtime, tuning),
        }
    }

    fn mode(&self, runtime: &WorkerRuntime<'_>) -> SignalMode {
        let low_power = runtime.power.low_power();
        SignalMode {
            low_power,
            idle_secs: self.link.active_secs(),
            tracked: self.capabilities.tracked_direct
                && (!low_power || tracked_outbox_pending(runtime.auth)),
        }
    }

    fn next_wake(&self, runtime: &WorkerRuntime<'_>, now: Now, mode: SignalMode) -> Duration {
        let mut wait = self.schedule.next_wake(now, mode).unwrap_or(MAX_EVENT_WAIT);
        if self.capabilities.discovery_exchange {
            if let Some(due) = runtime.discovery.next_due(list_refresh(runtime)) {
                wait = wait.min(due.saturating_duration_since(Instant::now()));
            }
        }
        wait
    }

    fn wait(&self, runtime: &WorkerRuntime<'_>, timeout: Duration) -> Woken {
        if self.readiness.is_none() {
            // Polling session: the next read waits instead.
            return Woken::Event;
        }
        let timeout = runtime.power.hub().clock().real_wait(timeout);
        let wake = runtime.iroh.signal_wake();
        let completions = runtime.repair_completions.receiver();
        let ready = self.readiness.as_ref().map(SignalReadiness::ready);
        let mut select = Select::new();
        let commands_index = select.recv(runtime.commands);
        let wake_index = select.recv(wake);
        let completions_index = completions.map(|receiver| select.recv(receiver));
        let ready_index = ready.map(|receiver| select.recv(receiver));
        let Ok(operation) = select.select_timeout(timeout) else {
            return Woken::Event;
        };
        let index = operation.index();
        if index == commands_index {
            return match operation.recv(runtime.commands) {
                Ok(pending) => Woken::Command(pending),
                Err(_) => Woken::CommandsClosed,
            };
        }
        if index == wake_index {
            let _ = operation.recv(wake);
            return Woken::Event;
        }
        if let (Some(receiver), Some(selected)) = (ready, ready_index) {
            if index == selected {
                return match operation.recv(receiver) {
                    Ok(()) => Woken::Readable,
                    Err(_) => Woken::ReadinessLost,
                };
            }
        }
        if let (Some(receiver), Some(_)) = (completions, completions_index) {
            // A disconnected channel is noticed by the next drain.
            let _ = operation.recv(receiver);
        }
        Woken::Event
    }
}

/// Discovery lists are refreshed for a visible app and during own
/// discovery activity, not for an idling device.
fn list_refresh(runtime: &WorkerRuntime<'_>) -> bool {
    !runtime.power.low_power() || runtime.discovery.has_activity()
}

fn tracked_outbox_pending(auth: &Arc<Mutex<ShareAuthState>>) -> bool {
    let Ok(state) = auth.lock() else {
        return true;
    };
    let now = now_secs();
    state
        .direct_requests
        .iter()
        .any(|entry| !entry.pending_outboxes(now).is_empty())
}
