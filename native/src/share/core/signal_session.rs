//! Upkeep of a connected signal session: server keepalives, probes, the
//! `set_idle` that follows the power state, commands and timed maintenance.

use std::io;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::{list_refresh, End, Session};
use crate::share::power::clock::Now;
use crate::share::power::{ProbeOutcome, CONNECT_HOLD_MS, KEEPALIVE_HOLD_MS};
use crate::share::signal_commands::{run_connected_command, ConnectedCommandRuntime};
use crate::share::signal_connection::{send_line, SignalConnection};
use crate::share::signal_worker::idle::KeepaliveTuning;
use crate::share::signal_worker::schedule::SignalMode;
use crate::share::signal_worker::{publish_all, WorkerRuntime};
use crate::share::tracked_signal_sender::send_pending_tracked;
use crate::share::types::{PendingShareCmd, ShareAuthState, ShareCmd, ShareCmdResult, ShareEvent};
use crate::share::wire::ClientMsg;

impl Session {
    /// The server's keepalive: answer, renew presence, close quiet peer
    /// connections and check the home relay while the CPU is awake (K3).
    pub(super) fn keepalive(
        &mut self,
        connection: &mut SignalConnection,
        runtime: &mut WorkerRuntime<'_>,
        tuning: &mut KeepaliveTuning,
    ) -> Option<End> {
        runtime.power.hold(KEEPALIVE_HOLD_MS);
        if send_line(connection, &ClientMsg::KeepaliveAck).is_err() {
            return Some(End::Reconnect);
        }
        let now = runtime.power.now();
        if self.link.active_secs().is_some() {
            // A fallback to the default keepalive is sent by `follow_power`.
            tuning.alive(now.wall_ms);
        }
        if runtime.power.low_power() {
            runtime.iroh.sweep_idle_connections();
            self.check_home_relay(runtime);
        }
        self.renew_idle_presence(connection, runtime, now)
    }

    pub(super) fn renew_idle_presence(
        &mut self,
        connection: &mut SignalConnection,
        runtime: &mut WorkerRuntime<'_>,
        now: Now,
    ) -> Option<End> {
        let secs = self.link.active_secs()?;
        if !self.schedule.idle_presence_due(now, secs) {
            return None;
        }
        self.publish_or_reconnect(connection, runtime, now)
    }

    pub(super) fn check_home_relay(&mut self, runtime: &mut WorkerRuntime<'_>) {
        match runtime.iroh.home_relay_connected() {
            Some(false) => {
                if std::mem::replace(&mut self.relay_was_connected, false) {
                    runtime.power.hold(CONNECT_HOLD_MS);
                }
                runtime.iroh.notify_network_change();
            }
            Some(true) => self.relay_was_connected = true,
            None => {}
        }
    }

    /// Sends the `set_idle` the current power state needs, if any.
    pub(super) fn follow_power(
        &mut self,
        connection: &mut SignalConnection,
        runtime: &mut WorkerRuntime<'_>,
        tuning: &KeepaliveTuning,
    ) -> Option<End> {
        let message = self
            .link
            .wanted(runtime.power.low_power(), tuning.proposal())?;
        if send_line(connection, &message).is_err() {
            return Some(End::Reconnect);
        }
        self.link.sent(&message);
        if self.link.active_secs().is_none() {
            runtime.power.update_status(|status| {
                status.idle_active = false;
                status.keepalive_secs = None;
            });
        }
        None
    }

    pub(super) fn command(
        &mut self,
        pending: PendingShareCmd,
        connection: &mut SignalConnection,
        runtime: &mut WorkerRuntime<'_>,
    ) -> Option<End> {
        if Instant::now() > pending.expires_at {
            let _ = pending.acknowledgement.send(Err(
                "Share-Kommando ist vor der Verarbeitung abgelaufen".into(),
            ));
            return None;
        }
        // The background reload repeats unchanged profiles; republishing
        // them would wake the radio for nothing.
        if runtime.power.low_power() && unchanged_profiles(&pending.command, runtime.auth) {
            let _ = pending.acknowledgement.send(Ok(ShareCmdResult::Applied));
            return None;
        }
        let mut command_runtime = ConnectedCommandRuntime {
            signal: connection,
            auth: runtime.auth,
            iroh: runtime.iroh,
            direct_requests_sent: runtime.direct_requests_sent,
            tracked_direct: self.capabilities.tracked_direct,
            discovery_exchange: self.capabilities.discovery_exchange,
            discovery: runtime.discovery,
            events: runtime.events,
            tracked_attempts: runtime.tracked_attempts,
        };
        let outcome = run_connected_command(pending.command, &mut command_runtime);
        let _ = pending
            .acknowledgement
            .send(outcome.result.map_err(|error| error.to_string()));
        if outcome.published {
            self.schedule.published(runtime.power.now());
        }
        if outcome.should_reconnect {
            return Some(End::CommandFailed);
        }
        if outcome.should_stop {
            runtime
                .stopped_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
            return Some(End::Stopped);
        }
        None
    }

    /// Answers a probe: a server silent beyond its deadlines means a new
    /// connection; after a network change a heartbeat must come back within
    /// 10 s; otherwise the connection is fine as it is.
    pub(super) fn probe(
        &mut self,
        connection: &mut SignalConnection,
        runtime: &mut WorkerRuntime<'_>,
        now: Now,
        mode: SignalMode,
    ) -> Option<End> {
        if self.schedule.stale(now, mode) {
            let _ = runtime.events.send(ShareEvent::Status(
                "Share-Server-Verbindung ist nach dem Ruhezustand veraltet; neuer Aufbau".into(),
            ));
            // The probe stays pending and is answered after reconnecting.
            return Some(End::Reconnect);
        }
        if runtime.power.low_power() {
            self.check_home_relay(runtime);
            if let Some(end) = self.renew_idle_presence(connection, runtime, now) {
                return Some(end);
            }
        }
        if !runtime.power.probe_network_changed() {
            runtime.power.answer(ProbeOutcome {
                ok: true,
                reconnected: false,
            });
            return None;
        }
        if send_line(connection, &ClientMsg::Heartbeat).is_err() {
            return Some(End::Reconnect);
        }
        self.schedule.heartbeat_sent(now);
        self.schedule.probe_started(now);
        None
    }

    pub(super) fn maintain(
        &mut self,
        connection: &mut SignalConnection,
        runtime: &mut WorkerRuntime<'_>,
        now: Now,
        mode: SignalMode,
    ) -> Option<End> {
        if self.schedule.stale(now, mode) {
            let _ = runtime.events.send(ShareEvent::Error(
                "Share-Signaling ist verstummt; Verbindung wird neu aufgebaut".into(),
            ));
            return Some(End::Reconnect);
        }
        if self.schedule.pong_expired(now) {
            let _ = runtime.events.send(ShareEvent::Error(
                "Share-Signaling hat den Keepalive nicht beantwortet; Verbindung wird neu aufgebaut"
                    .into(),
            ));
            return Some(End::Reconnect);
        }
        if self.schedule.probe_expired(now) {
            let _ = runtime.events.send(ShareEvent::Status(
                "Share-Server antwortet nach dem Netzwechsel nicht; neuer Aufbau".into(),
            ));
            return Some(End::Reconnect);
        }
        if self.schedule.heartbeat_due(now, mode) {
            if send_line(connection, &ClientMsg::Heartbeat).is_err() {
                return Some(End::Reconnect);
            }
            self.schedule.heartbeat_sent(now);
        }
        if runtime.iroh.route_revision() != self.published_routes
            || self.schedule.presence_due(now, mode)
        {
            if let Some(end) = self.publish_or_reconnect(connection, runtime, now) {
                return Some(end);
            }
        }
        if self.schedule.tracked_due(now, mode) {
            if send_pending_tracked(
                connection,
                runtime.auth,
                runtime.iroh,
                runtime.events,
                runtime.tracked_attempts,
            )
            .is_err()
            {
                return Some(End::Reconnect);
            }
            self.schedule.tracked_sent(now);
        }
        let list_refresh = list_refresh(runtime);
        if self.capabilities.discovery_exchange
            && runtime
                .discovery
                .maintain_with(connection, runtime.events, list_refresh)
                .is_err()
        {
            return Some(End::Reconnect);
        }
        None
    }

    pub(super) fn publish(
        &mut self,
        connection: &mut SignalConnection,
        runtime: &mut WorkerRuntime<'_>,
        now: Now,
    ) -> io::Result<()> {
        let routes = runtime.iroh.route_revision();
        publish_all(
            connection,
            runtime.auth,
            runtime.iroh,
            runtime.direct_requests_sent,
            self.capabilities.tracked_direct,
        )?;
        self.published_routes = routes;
        self.schedule.published(now);
        Ok(())
    }

    pub(super) fn publish_or_reconnect(
        &mut self,
        connection: &mut SignalConnection,
        runtime: &mut WorkerRuntime<'_>,
        now: Now,
    ) -> Option<End> {
        let error = self.publish(connection, runtime, now).err()?;
        let _ = runtime.events.send(ShareEvent::Error(format!(
            "Share-Presence konnte nicht erneuert werden: {error}"
        )));
        Some(End::Reconnect)
    }
}

/// A `ConfigureProfiles` whose content the runtime state already holds:
/// applying it changes nothing, tears nothing down and republishes only.
fn unchanged_profiles(command: &ShareCmd, auth: &Arc<Mutex<ShareAuthState>>) -> bool {
    let ShareCmd::ConfigureProfiles { profiles } = command else {
        return false;
    };
    let Ok(state) = auth.lock() else {
        return false;
    };
    state.direct_contacts == profiles.direct_contacts
        && state.direct_grants == profiles.direct_grants
        && state.rooms == profiles.rooms
        && state.default_direct_exports == profiles.default_direct_exports
        && state.direct_requests == profiles.direct_requests
        && state.direct_request_tombstones == profiles.direct_request_tombstones
}
