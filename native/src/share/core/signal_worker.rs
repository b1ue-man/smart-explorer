use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;

use super::authorization_policy::configuration_changed;
use super::backend::ShareIrohNode;
use super::core::{eio, hmac_proof, now_secs, presence_payload, random_token};
use super::identity::ShareIdentity;
use super::profiles::ShareProfiles;
use super::signal_auth::handle_server_msg;
use super::signal_connection::{send_line, SignalConnection};
use super::system::lan_ips;
use super::types::{
    DirectAccessState, DirectContact, PeerPresence, PendingShareCmd, ShareAuthState, ShareCmd,
    ShareEvent,
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
    while !stopped && !stopped_flag.load(Ordering::Relaxed) {
        match SignalConnection::connect(&server) {
            Ok(mut signal) => {
                let transport = signal.label().to_string();
                if let Err(error) = send_hello(&mut signal, &identity) {
                    let _ = events.send(ShareEvent::ServerDisconnected(error.to_string()));
                    std::thread::sleep(backoff);
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                    continue;
                }
                let _ = events.send(ShareEvent::ServerConnected);
                let _ = events.send(ShareEvent::Status(format!(
                    "Share-Server verbunden ({transport})"
                )));
                backoff = Duration::from_secs(1);
                if let Err(error) =
                    publish_all(&mut signal, &auth, &iroh, &mut direct_requests_sent)
                {
                    let _ = events.send(ShareEvent::ServerDisconnected(format!(
                        "Share-Presence konnte nicht sicher erzeugt werden: {error}"
                    )));
                    std::thread::sleep(backoff);
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                    continue;
                }
                let mut last_heartbeat = Instant::now();
                let mut last_publish = Instant::now();
                loop {
                    if stopped_flag.load(Ordering::Relaxed) {
                        stopped = true;
                        break;
                    }
                    while let Ok(pending) = commands.try_recv() {
                        if Instant::now() > pending.expires_at {
                            let _ = pending.acknowledgement.send(Err(
                                "Share-Kommando ist vor der Verarbeitung abgelaufen".into(),
                            ));
                            continue;
                        }
                        let (result, should_stop, published) = run_command(
                            pending.command,
                            &mut signal,
                            &auth,
                            &iroh,
                            &mut direct_requests_sent,
                        );
                        let result = result.map_err(|error| error.to_string());
                        let _ = pending.acknowledgement.send(result);
                        if published {
                            last_publish = Instant::now();
                        }
                        if should_stop {
                            stopped = true;
                            stopped_flag.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                    if stopped {
                        break;
                    }
                    if last_heartbeat.elapsed() >= Duration::from_secs(20) {
                        if send_line(&mut signal, &ClientMsg::Heartbeat).is_err() {
                            break;
                        }
                        last_heartbeat = Instant::now();
                    }
                    if last_publish.elapsed() >= Duration::from_secs(60) {
                        if let Err(error) =
                            publish_all(&mut signal, &auth, &iroh, &mut direct_requests_sent)
                        {
                            let _ = events.send(ShareEvent::Error(format!(
                                "Share-Presence konnte nicht erneuert werden: {error}"
                            )));
                            break;
                        }
                        last_publish = Instant::now();
                    }
                    match signal.read_message() {
                        Ok(Some(line)) => handle_server_msg(line.trim(), &auth, &events),
                        Ok(None) => break,
                        Err(error)
                            if error.kind() == io::ErrorKind::WouldBlock
                                || error.kind() == io::ErrorKind::TimedOut => {}
                        Err(_) => break,
                    }
                }
                let _ = events.send(ShareEvent::ServerDisconnected("Signaling getrennt".into()));
            }
            Err(error) => {
                let _ = events.send(ShareEvent::ServerDisconnected(format!(
                    "Share-Server nicht erreichbar: {error}"
                )));
            }
        }
        if !stopped && !stopped_flag.load(Ordering::Relaxed) {
            std::thread::sleep(backoff);
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    }
}

fn run_command(
    command: ShareCmd,
    signal: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    direct_requests_sent: &mut HashSet<String>,
) -> (io::Result<()>, bool, bool) {
    match command {
        ShareCmd::Configure {
            direct,
            direct_grants,
            rooms,
            default_direct_exports,
        } => {
            let configured =
                auth.lock()
                    .map_err(|_| eio("Share-State gesperrt"))
                    .map(|mut state| {
                        let changed = configuration_changed(
                            &state,
                            &direct,
                            &direct_grants,
                            &rooms,
                            &default_direct_exports,
                        );
                        state.direct_contacts = direct;
                        state.direct_grants = direct_grants;
                        state.rooms = rooms;
                        state.default_direct_exports = default_direct_exports;
                        changed
                    });
            let result = configured.and_then(|changed| {
                if changed {
                    iroh.invalidate_sessions()?;
                    direct_requests_sent.clear();
                }
                publish_all(signal, auth, iroh, direct_requests_sent)
            });
            (result, false, true)
        }
        ShareCmd::Refresh => (
            publish_all(signal, auth, iroh, direct_requests_sent),
            false,
            true,
        ),
        ShareCmd::SetDirectOnline { online } => {
            let lookup_id =
                auth.lock()
                    .map_err(|_| eio("Share-State gesperrt"))
                    .map(|mut state| {
                        let changed = state.direct_online != online;
                        state.direct_online = online;
                        (state.identity.direct_lookup_id.clone(), changed)
                    });
            let result = lookup_id.and_then(|(lookup_id, changed)| {
                if changed {
                    iroh.invalidate_sessions()?;
                }
                if online {
                    publish_all(signal, auth, iroh, direct_requests_sent)
                } else {
                    send_line(signal, &ClientMsg::UnpublishDirect { lookup_id })
                }
            });
            (result, false, online)
        }
        ShareCmd::Stop => (Ok(()), true, false),
        ShareCmd::LeaveRoom { room_id } => (
            send_line(signal, &ClientMsg::LeaveRoom { room_id }),
            false,
            false,
        ),
        ShareCmd::RequestDirect { contact_id } => {
            let result = send_direct_request(signal, auth, iroh, &contact_id);
            if result.is_ok() {
                direct_requests_sent.insert(contact_id);
            }
            (result, false, false)
        }
        ShareCmd::AnswerDirectRequest {
            lookup_id,
            presence,
            accepted,
        } => (
            send_direct_answer(signal, auth, iroh, lookup_id, presence, accepted),
            false,
            false,
        ),
    }
}

fn send_hello(stream: &mut SignalConnection, identity: &ShareIdentity) -> io::Result<()> {
    send_line(
        stream,
        &ClientMsg::Hello {
            protocol_version: 3,
            device_id: identity.device_id.clone(),
            device_name: identity.device_name.clone(),
            listen_port: 0,
            lan: lan_ips(),
            public_key: identity.public_key.clone(),
            fingerprint: identity.fingerprint.clone(),
        },
    )
}

fn publish_all(
    stream: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    direct_requests_sent: &mut HashSet<String>,
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
        if contact.access_state == DirectAccessState::Pending
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

fn send_direct_request(
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

fn send_direct_answer(
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
