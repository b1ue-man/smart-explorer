//! Presence, watches, room joins and direct requests the worker publishes
//! on the signal connection.

use std::collections::HashSet;
use std::io;
use std::sync::{Arc, Mutex};

use crate::share::backend::ShareIrohNode;
use crate::share::core::eio;
use crate::share::profiles::ShareProfiles;
use crate::share::signal_connection::{send_line, SignalConnection};
use crate::share::signal_presence::build_presence;
use crate::share::types::{DirectAccessState, DirectContact, ShareAuthState};
use crate::share::wire::ClientMsg;

pub(in crate::share) fn publish_all(
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

pub(in crate::share) fn send_direct_request(
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

pub(in crate::share) fn send_direct_answer(
    stream: &mut SignalConnection,
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    lookup_id: String,
    requester_device_id: String,
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
            requester_device_id,
            accepted,
            presence,
            msg: None,
        },
    )
}
