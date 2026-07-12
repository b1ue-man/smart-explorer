use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;

use super::backend::ShareIrohNode;
use super::core::{eio, hmac_proof, now_secs, presence_payload, random_token};
use super::identity::ShareIdentity;
use super::profiles::ShareProfiles;
use super::signal_commands::{
    run_connected_command, run_offline_command, ConnectedCommandRuntime, OfflineCommandRuntime,
};
use super::signal_connection::{send_line, SignalConnection};
use super::signal_connector::{spawn_connect, NegotiatedSignal};
use super::tracked_signal_dispatch::dispatch_server_line;
use super::tracked_signal_sender::{send_pending_tracked, AttemptCounters};
use super::types::{
    DirectAccessState, DirectContact, PeerPresence, PendingShareCmd, ShareAuthState, ShareEvent,
};
use super::wire::ClientMsg;

pub(super) fn worker(
    server: String,
    identity: ShareIdentity,
    iroh: Arc<ShareIrohNode>,
    auth: Arc<Mutex<ShareAuthState>>,
    commands: Receiver<PendingShareCmd>,
    events: crossbeam_channel::Sender<ShareEvent>,
    stopped_flag: Arc<AtomicBool>,
) {
    let mut stopped = false;
    let mut backoff = Duration::from_secs(1);
    let mut direct_requests_sent = HashSet::new();
    let mut tracked_attempts = AttemptCounters::new();
    let mut runtime = WorkerRuntime {
        auth: &auth,
        iroh: &iroh,
        commands: &commands,
        events: &events,
        stopped_flag: &stopped_flag,
        direct_requests_sent: &mut direct_requests_sent,
        tracked_attempts: &mut tracked_attempts,
    };
    while !stopped && !stopped_flag.load(Ordering::Relaxed) {
        let connector = match spawn_connect(server.clone(), identity.clone()) {
            Ok(connector) => connector,
            Err(error) => {
                let _ = events.send(ShareEvent::ServerDisconnected(format!(
                    "Share-Verbindungsversuch konnte nicht starten: {error}"
                )));
                if wait_offline_backoff(backoff, &mut runtime) {
                    break;
                }
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };
        let negotiated = match wait_for_connection(&connector, &mut runtime) {
            ConnectionWait::Stopped => break,
            ConnectionWait::Ready(result) => result,
        };
        match negotiated {
            Ok(negotiated) => {
                backoff = Duration::from_secs(1);
                stopped = run_connected(negotiated, &mut runtime);
            }
            Err(error) => {
                let _ = events.send(ShareEvent::ServerDisconnected(format!(
                    "Share-Server nicht erreichbar: {error}"
                )));
            }
        }
        if !stopped && !stopped_flag.load(Ordering::Relaxed) {
            stopped = wait_offline_backoff(backoff, &mut runtime);
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    }
}

struct WorkerRuntime<'a> {
    auth: &'a Arc<Mutex<ShareAuthState>>,
    iroh: &'a ShareIrohNode,
    commands: &'a Receiver<PendingShareCmd>,
    events: &'a crossbeam_channel::Sender<ShareEvent>,
    stopped_flag: &'a AtomicBool,
    direct_requests_sent: &'a mut HashSet<String>,
    tracked_attempts: &'a mut AttemptCounters,
}

enum ConnectionWait {
    Ready(io::Result<NegotiatedSignal>),
    Stopped,
}

fn wait_for_connection(
    connector: &Receiver<io::Result<NegotiatedSignal>>,
    runtime: &mut WorkerRuntime<'_>,
) -> ConnectionWait {
    loop {
        if runtime.stopped_flag.load(Ordering::Relaxed) {
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

fn wait_offline_backoff(duration: Duration, runtime: &mut WorkerRuntime<'_>) -> bool {
    let deadline = Instant::now() + duration;
    loop {
        if runtime.stopped_flag.load(Ordering::Relaxed) {
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

fn acknowledge_offline(pending: PendingShareCmd, runtime: &mut WorkerRuntime<'_>) -> bool {
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
    };
    let outcome = run_offline_command(pending.command, &mut command_runtime);
    let _ = pending
        .acknowledgement
        .send(outcome.result.map_err(|error| error.to_string()));
    if outcome.should_stop {
        runtime.stopped_flag.store(true, Ordering::Relaxed);
    }
    outcome.should_stop
}

fn run_connected(mut negotiated: NegotiatedSignal, runtime: &mut WorkerRuntime<'_>) -> bool {
    let tracked_direct = negotiated.capabilities.tracked_direct;
    let _ = runtime.events.send(ShareEvent::ServerConnected);
    let _ = runtime.events.send(ShareEvent::Status(format!(
        "Share-Server verbunden ({}, tracked_direct={tracked_direct})",
        negotiated.transport
    )));
    if let Err(error) = publish_all(
        &mut negotiated.connection,
        runtime.auth,
        runtime.iroh,
        runtime.direct_requests_sent,
        tracked_direct,
    ) {
        let _ = runtime.events.send(ShareEvent::ServerDisconnected(format!(
            "Share-Presence konnte nicht sicher erzeugt werden: {error}"
        )));
        return false;
    }
    if tracked_direct
        && send_pending_tracked(
            &mut negotiated.connection,
            runtime.auth,
            runtime.events,
            runtime.tracked_attempts,
        )
        .is_err()
    {
        let _ = runtime.events.send(ShareEvent::ServerDisconnected(
            "Direct-Outbox konnte nicht gesendet werden".into(),
        ));
        return false;
    }

    let mut last_heartbeat = Instant::now();
    let mut last_publish = Instant::now();
    let mut last_tracked_send = Instant::now();
    let mut stopped = false;
    loop {
        if runtime.stopped_flag.load(Ordering::Relaxed) {
            stopped = true;
            break;
        }
        while let Ok(pending) = runtime.commands.try_recv() {
            if Instant::now() > pending.expires_at {
                let _ = pending.acknowledgement.send(Err(
                    "Share-Kommando ist vor der Verarbeitung abgelaufen".into(),
                ));
                continue;
            }
            let mut command_runtime = ConnectedCommandRuntime {
                signal: &mut negotiated.connection,
                auth: runtime.auth,
                iroh: runtime.iroh,
                direct_requests_sent: runtime.direct_requests_sent,
                tracked_direct,
                events: runtime.events,
                tracked_attempts: runtime.tracked_attempts,
            };
            let outcome = run_connected_command(pending.command, &mut command_runtime);
            let _ = pending
                .acknowledgement
                .send(outcome.result.map_err(|error| error.to_string()));
            if outcome.published {
                last_publish = Instant::now();
            }
            if outcome.should_stop {
                stopped = true;
                runtime.stopped_flag.store(true, Ordering::Relaxed);
                break;
            }
        }
        if stopped {
            break;
        }
        if last_heartbeat.elapsed() >= Duration::from_secs(20) {
            if send_line(&mut negotiated.connection, &ClientMsg::Heartbeat).is_err() {
                break;
            }
            last_heartbeat = Instant::now();
        }
        if last_publish.elapsed() >= Duration::from_secs(60) {
            if let Err(error) = publish_all(
                &mut negotiated.connection,
                runtime.auth,
                runtime.iroh,
                runtime.direct_requests_sent,
                tracked_direct,
            ) {
                let _ = runtime.events.send(ShareEvent::Error(format!(
                    "Share-Presence konnte nicht erneuert werden: {error}"
                )));
                break;
            }
            last_publish = Instant::now();
        }
        if tracked_direct && last_tracked_send.elapsed() >= Duration::from_secs(2) {
            if send_pending_tracked(
                &mut negotiated.connection,
                runtime.auth,
                runtime.events,
                runtime.tracked_attempts,
            )
            .is_err()
            {
                break;
            }
            last_tracked_send = Instant::now();
        }
        match negotiated.connection.read_message() {
            Ok(Some(line)) => {
                dispatch_server_line(line.trim(), tracked_direct, runtime.auth, runtime.events)
            }
            Ok(None) => break,
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut => {}
            Err(_) => break,
        }
    }
    let _ = runtime
        .events
        .send(ShareEvent::ServerDisconnected("Signaling getrennt".into()));
    stopped
}

pub(super) fn publish_all(
    stream: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    direct_requests_sent: &mut HashSet<String>,
    tracked_direct: bool,
) -> io::Result<()> {
    let state = auth
        .lock()
        .map_err(|_| eio("Share-State gesperrt"))?
        .clone();
    if state.direct_online {
        let direct = build_presence(
            "direct",
            &state.identity.direct_lookup_id,
            &state.identity,
            &state.direct_secret,
            iroh,
        )?;
        send_line(stream, &ClientMsg::PublishDirect { presence: direct })?;
    }
    for contact in state
        .direct_contacts
        .iter()
        .filter(|contact| contact.auto_connect)
    {
        send_line(
            stream,
            &ClientMsg::WatchDirect {
                lookup_id: contact.lookup_id.clone(),
            },
        )?;
        if !tracked_direct
            && contact.access_state == DirectAccessState::Pending
            && !direct_requests_sent.contains(&contact.id)
        {
            send_direct_request_locked(stream, &state, contact, iroh)?;
            direct_requests_sent.insert(contact.id.clone());
        }
    }
    for room in state.rooms.iter().filter(|room| room.auto_join) {
        if let Some(secret) = ShareProfiles::room_secret_checked(room).map_err(eio)? {
            let presence = build_presence("room", &room.room_id, &state.identity, &secret, iroh)?;
            send_line(
                stream,
                &ClientMsg::JoinRoom {
                    room_id: room.room_id.clone(),
                    presence,
                },
            )?;
        }
    }
    Ok(())
}

pub(super) fn send_direct_request(
    stream: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    contact_id: &str,
) -> io::Result<()> {
    let state = auth
        .lock()
        .map_err(|_| eio("Share-State gesperrt"))?
        .clone();
    let contact = state
        .direct_contacts
        .iter()
        .find(|contact| contact.id == contact_id)
        .ok_or_else(|| eio("Direktgeraet nicht gefunden"))?;
    send_direct_request_locked(stream, &state, contact, iroh)
}

fn send_direct_request_locked(
    stream: &mut SignalConnection,
    state: &ShareAuthState,
    contact: &DirectContact,
    iroh: &ShareIrohNode,
) -> io::Result<()> {
    let secret = ShareProfiles::direct_secret_checked(contact)
        .map_err(eio)?
        .ok_or_else(|| eio("Direkt-Secret fehlt"))?;
    let request = build_presence("direct", &contact.lookup_id, &state.identity, &secret, iroh)?;
    send_line(
        stream,
        &ClientMsg::RequestDirect {
            lookup_id: contact.lookup_id.clone(),
            presence: request,
        },
    )
}

pub(super) fn send_direct_answer(
    stream: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    lookup_id: String,
    requester: PeerPresence,
    accepted: bool,
) -> io::Result<()> {
    let state = auth
        .lock()
        .map_err(|_| eio("Share-State gesperrt"))?
        .clone();
    let presence = Some(build_presence(
        "direct",
        &lookup_id,
        &state.identity,
        &state.direct_secret,
        iroh,
    )?);
    send_line(
        stream,
        &ClientMsg::DirectAccessAccepted {
            lookup_id,
            requester_device_id: requester.device_id,
            accepted,
            presence,
            msg: None,
        },
    )
}

pub(super) fn build_presence(
    kind: &str,
    relation_id: &str,
    identity: &ShareIdentity,
    secret: &[u8],
    iroh: &ShareIrohNode,
) -> io::Result<PeerPresence> {
    let candidates = iroh.candidates();
    let relay_url = iroh.relay_url().to_string();
    let expires_at = now_secs() + 300;
    let nonce = random_token(12).map_err(eio)?;
    let payload = presence_payload(
        kind,
        relation_id,
        &identity.device_id,
        &identity.public_key,
        &identity.node_id,
        &relay_url,
        &candidates,
        expires_at,
        &nonce,
    );
    Ok(PeerPresence {
        kind: kind.to_string(),
        relation_id: relation_id.to_string(),
        device_id: identity.device_id.clone(),
        device_name: identity.device_name.clone(),
        public_key: identity.public_key.clone(),
        fingerprint: identity.fingerprint.clone(),
        node_id: identity.node_id.clone(),
        relay_url,
        candidates,
        expires_at,
        nonce,
        proof: hmac_proof(secret, &payload),
    })
}
