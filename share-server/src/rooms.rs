//! Room membership, proof partitions and owner-safe departures (FC4).

use std::sync::{Arc, Mutex};

use crate::access;
use crate::direct_presence::origin_matches;
use crate::limits::{
    validate_identifier, validate_presence, RetainError, MAX_ROOMS_PER_CLIENT, MAX_ROOM_MEMBERS,
};
use crate::state::{lock_state, send_all, State};
use crate::{send, Out, PeerPresence, Writer};

#[cfg(test)]
pub(super) fn join_room(
    id: u64,
    writer: &Writer,
    room: &str,
    presence: PeerPresence,
    state: &Arc<Mutex<State>>,
) {
    join_with_access(id, writer, room, presence, None, state);
}

pub(super) fn join_with_access(
    id: u64,
    writer: &Writer,
    room: &str,
    presence: PeerPresence,
    proof: Option<String>,
    state: &Arc<Mutex<State>>,
) {
    let result = validate_identifier("room id", room)
        .and_then(|_| validate_presence(&presence))
        .and_then(|_| {
            let mut state = lock_state(state);
            if presence.kind != "room" || presence.relation_id != room {
                return Err(RetainError::InvalidField("room presence"));
            }
            if !origin_matches(
                &state,
                id,
                &presence.device_id,
                &presence.public_key,
                &presence.node_id,
            ) {
                return Err(RetainError::Denied("registered identity"));
            }
            let client = &state.clients[&id];
            if !client.rooms.contains(room) && client.rooms.len() >= MAX_ROOMS_PER_CLIENT {
                return Err(RetainError::Limit("rooms"));
            }
            let partition = if client.identity.proven().is_some() {
                Some(
                    proof
                        .as_deref()
                        .and_then(access::proof_hash)
                        .ok_or(RetainError::Denied("room access proof"))?,
                )
            } else {
                None
            };
            let members = state.rooms.get(room);
            let existing = members.and_then(|members| members.get(&presence.device_id));
            if existing.is_none()
                && members.is_some_and(|members| members.len() >= MAX_ROOM_MEMBERS)
            {
                return Err(RetainError::Limit("room members"));
            }
            let visible = |device: &str| {
                access::visible(
                    partition.as_ref(),
                    state.room_access.get(&(room.into(), device.into())),
                )
            };
            let roster = Out::RoomRoster {
                room_id: room.into(),
                members: members
                    .into_iter()
                    .flat_map(|members| members.iter())
                    .filter(|(device, _)| *device != &presence.device_id && visible(device))
                    .map(|(_, (_, presence))| presence.clone())
                    .collect(),
            };
            if !crate::writer::outbound_fits(&roster) {
                return Err(RetainError::Limit("room roster bytes"));
            }
            let previous_partition = state
                .room_access
                .get(&(room.into(), presence.device_id.clone()));
            let changed = existing.is_none_or(|(_, previous)| previous != &presence)
                || previous_partition != partition.as_ref();
            let targets: Vec<Writer> = if changed {
                members
                    .into_iter()
                    .flat_map(|members| members.iter())
                    .filter(|(device, _)| *device != &presence.device_id && visible(device))
                    .filter_map(|(_, (id, _))| {
                        state.clients.get(id).map(|client| client.writer.clone())
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let previous = existing
                .map(|(id, _)| *id)
                .filter(|previous| *previous != id);
            if let Some(previous) = previous {
                if let Some(client) = state.clients.get_mut(&previous) {
                    client.rooms.remove(room);
                }
            }
            let membership = (room.to_string(), presence.device_id.clone());
            match partition {
                Some(partition) => {
                    state.room_access.insert(membership, partition);
                }
                None => {
                    state.room_access.remove(&membership);
                }
            }
            state
                .rooms
                .entry(room.into())
                .or_default()
                .insert(presence.device_id.clone(), (id, presence.clone()));
            if let Some(client) = state.clients.get_mut(&id) {
                client.rooms.insert(room.into());
            }
            Ok((roster, targets))
        });
    match result {
        Ok((roster, targets)) => {
            send(writer, &roster);
            let message = Out::RoomJoined {
                room_id: room.into(),
                presence,
            };
            for target in targets {
                target.offer(&message);
            }
        }
        Err(error) => {
            send(
                writer,
                &Out::Error {
                    scope: "room".into(),
                    msg: error.message(),
                },
            );
        }
    }
}

pub(super) fn leave_room(id: u64, room: &str, state: &Arc<Mutex<State>>) {
    let notifications = depart_locked(&mut lock_state(state), id, room);
    send_all(notifications);
}

pub(super) fn depart_locked(state: &mut State, id: u64, room: &str) -> Vec<(Writer, Out)> {
    let Some(device) = state.rooms.get(room).and_then(|members| {
        members
            .iter()
            .find_map(|(device, (owner, _))| (*owner == id).then(|| device.clone()))
    }) else {
        return Vec::new();
    };
    let partition = state.room_access.remove(&(room.into(), device.clone()));
    let mut targets = Vec::new();
    let empty = if let Some(members) = state.rooms.get_mut(room) {
        members.remove(&device);
        targets.extend(
            members
                .iter()
                .map(|(device, (id, _))| (device.clone(), *id)),
        );
        members.is_empty()
    } else {
        false
    };
    if empty {
        state.rooms.remove(room);
    }
    if let Some(client) = state.clients.get_mut(&id) {
        client.rooms.remove(room);
    }
    targets
        .into_iter()
        .filter(|(device, _)| {
            access::visible(
                partition.as_ref(),
                state.room_access.get(&(room.into(), device.clone())),
            )
        })
        .filter_map(|(_, id)| {
            state.clients.get(&id).map(|client| {
                (
                    client.writer.clone(),
                    Out::RoomLeft {
                        room_id: room.into(),
                        device_id: device.clone(),
                    },
                )
            })
        })
        .collect()
}
