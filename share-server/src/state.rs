use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use super::limits::{
    validate_identifier, validate_presence, RetainError, SourceKey, MAX_REGISTERED_CLIENTS,
    MAX_REGISTERED_CLIENTS_PER_SOURCE, MAX_ROOMS_PER_CLIENT, MAX_ROOM_MEMBERS,
};
use super::writer::outbound_fits;
use super::{send, Out, PeerPresence, Writer};

#[derive(Clone)]
pub(super) struct Client {
    pub(super) writer: Writer,
    pub(super) source: SourceKey,
    pub(super) device_id: String,
    pub(super) capabilities: HashSet<String>,
    pub(super) direct_lookup_ids: HashSet<String>,
    pub(super) watched_lookup_ids: HashSet<String>,
    pub(super) rooms: HashSet<String>,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) next_id: u64,
    pub(super) clients: HashMap<u64, Client>,
    pub(super) direct: HashMap<String, (u64, PeerPresence)>,
    pub(super) watchers: HashMap<String, HashSet<u64>>,
    pub(super) rooms: HashMap<String, HashMap<String, (u64, PeerPresence)>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RegistrationError {
    Full,
    SourceFull,
    InvalidDeviceId,
    IdExhausted,
}

impl RegistrationError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Full => "server client limit reached",
            Self::SourceFull => "server source client limit reached",
            Self::InvalidDeviceId => "invalid or oversized device id",
            Self::IdExhausted => "server client id space exhausted",
        }
    }
}

pub(super) fn lock_state(state: &Arc<Mutex<State>>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(super) fn register_client(
    state: &Arc<Mutex<State>>,
    writer: Writer,
    source: SourceKey,
    device_id: String,
    capabilities: HashSet<String>,
) -> Result<u64, RegistrationError> {
    if validate_identifier("device id", &device_id).is_err() {
        return Err(RegistrationError::InvalidDeviceId);
    }
    let mut state = lock_state(state);
    if state.clients.len() >= MAX_REGISTERED_CLIENTS {
        return Err(RegistrationError::Full);
    }
    if source.has_internal_source_limit()
        && state
            .clients
            .values()
            .filter(|client| client.source == source)
            .count()
            >= MAX_REGISTERED_CLIENTS_PER_SOURCE
    {
        return Err(RegistrationError::SourceFull);
    }
    let id = state
        .next_id
        .checked_add(1)
        .ok_or(RegistrationError::IdExhausted)?;
    state.next_id = id;
    state.clients.insert(
        id,
        Client {
            writer,
            source,
            device_id,
            capabilities,
            direct_lookup_ids: HashSet::new(),
            watched_lookup_ids: HashSet::new(),
            rooms: HashSet::new(),
        },
    );
    Ok(id)
}

pub(super) fn join_room(
    id: u64,
    writer: &Writer,
    room_id: &str,
    presence: PeerPresence,
    state: &Arc<Mutex<State>>,
) {
    if let Err(error) =
        validate_identifier("room id", room_id).and_then(|_| validate_presence(&presence))
    {
        send_retain_error(writer, "room", error);
        return;
    }

    let result = {
        let mut state = lock_state(state);
        let Some(client) = state.clients.get(&id) else {
            return;
        };
        if client.device_id != presence.device_id {
            Err(RetainError::InvalidField("presence device id"))
        } else if !client.rooms.contains(room_id) && client.rooms.len() >= MAX_ROOMS_PER_CLIENT {
            Err(RetainError::Limit("rooms"))
        } else {
            let members = state.rooms.get(room_id);
            let existing = members.and_then(|members| members.get(&presence.device_id));
            if existing.is_none()
                && members.is_some_and(|members| members.len() >= MAX_ROOM_MEMBERS)
            {
                Err(RetainError::Limit("room members"))
            } else {
                let roster = members
                    .into_iter()
                    .flat_map(|members| members.iter())
                    .filter(|(device_id, _)| *device_id != &presence.device_id)
                    .map(|(_, (_, presence))| presence.clone())
                    .collect::<Vec<_>>();
                let roster = Out::RoomRoster {
                    room_id: room_id.to_string(),
                    members: roster,
                };
                if !outbound_fits(&roster) {
                    Err(RetainError::Limit("room roster bytes"))
                } else {
                    let target_ids = members
                        .into_iter()
                        .flat_map(|members| members.iter())
                        .filter(|(device_id, _)| *device_id != &presence.device_id)
                        .map(|(_, (client_id, _))| *client_id)
                        .collect::<Vec<_>>();
                    let changed = existing
                        .map(|(_, existing_presence)| existing_presence != &presence)
                        .unwrap_or(true);
                    let replaced_client = existing
                        .map(|(client_id, _)| *client_id)
                        .filter(|client_id| *client_id != id);
                    if let Some(replaced_client) = replaced_client {
                        if let Some(client) = state.clients.get_mut(&replaced_client) {
                            client.rooms.remove(room_id);
                        }
                    }
                    let members = state.rooms.entry(room_id.to_string()).or_default();
                    members.insert(presence.device_id.clone(), (id, presence.clone()));
                    if let Some(client) = state.clients.get_mut(&id) {
                        client.rooms.insert(room_id.to_string());
                    }
                    let targets = if changed {
                        target_ids
                            .into_iter()
                            .filter_map(|client_id| {
                                state
                                    .clients
                                    .get(&client_id)
                                    .map(|client| client.writer.clone())
                            })
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                    Ok((roster, targets))
                }
            }
        }
    };

    let (roster, targets) = match result {
        Ok(result) => result,
        Err(error) => {
            send_retain_error(writer, "room", error);
            return;
        }
    };
    send(writer, &roster);
    for target in targets {
        send(
            &target,
            &Out::RoomJoined {
                room_id: room_id.to_string(),
                presence: presence.clone(),
            },
        );
    }
}

pub(super) fn leave_room(id: u64, room_id: &str, state: &Arc<Mutex<State>>) {
    let notifications = {
        let mut state = lock_state(state);
        let mut target_ids = Vec::new();
        let mut remove_room = false;
        let mut departed_device = None;
        if let Some(members) = state.rooms.get_mut(room_id) {
            departed_device = members.iter().find_map(|(device_id, (client_id, _))| {
                (*client_id == id).then(|| device_id.clone())
            });
            if let Some(device_id) = &departed_device {
                members.remove(device_id);
                target_ids.extend(members.values().map(|(client_id, _)| *client_id));
                remove_room = members.is_empty();
            }
        }
        if remove_room {
            state.rooms.remove(room_id);
        }
        if let Some(client) = state.clients.get_mut(&id) {
            client.rooms.remove(room_id);
        }
        if let Some(device_id) = departed_device {
            target_ids
                .into_iter()
                .filter_map(|client_id| {
                    state.clients.get(&client_id).map(|client| {
                        (
                            client.writer.clone(),
                            Out::RoomLeft {
                                room_id: room_id.to_string(),
                                device_id: device_id.clone(),
                            },
                        )
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        }
    };
    send_all(notifications);
}

pub(super) fn cleanup(id: u64, state: &Arc<Mutex<State>>) {
    let notifications = {
        let mut state = lock_state(state);
        let Some(client) = state.clients.remove(&id) else {
            return;
        };
        let mut notifications = Vec::new();

        for lookup_id in client.direct_lookup_ids {
            if state.direct.get(&lookup_id).map(|(owner, _)| *owner) == Some(id) {
                state.direct.remove(&lookup_id);
                if let Some(watchers) = state.watchers.get(&lookup_id) {
                    for watcher_id in watchers {
                        if let Some(watcher) = state.clients.get(watcher_id) {
                            notifications.push((
                                watcher.writer.clone(),
                                Out::DirectOffline {
                                    lookup_id: lookup_id.clone(),
                                },
                            ));
                        }
                    }
                }
            }
        }

        for lookup_id in client.watched_lookup_ids {
            let remove_lookup = if let Some(watchers) = state.watchers.get_mut(&lookup_id) {
                watchers.remove(&id);
                watchers.is_empty()
            } else {
                false
            };
            if remove_lookup {
                state.watchers.remove(&lookup_id);
            }
        }

        for room_id in client.rooms {
            let mut target_ids = Vec::new();
            let mut departed_device = None;
            let remove_room = if let Some(members) = state.rooms.get_mut(&room_id) {
                departed_device = members.iter().find_map(|(device_id, (client_id, _))| {
                    (*client_id == id).then(|| device_id.clone())
                });
                if let Some(device_id) = &departed_device {
                    members.remove(device_id);
                    target_ids.extend(members.values().map(|(client_id, _)| *client_id));
                    members.is_empty()
                } else {
                    false
                }
            } else {
                false
            };
            if remove_room {
                state.rooms.remove(&room_id);
            }
            let Some(device_id) = departed_device else {
                continue;
            };
            for target_id in target_ids {
                if let Some(target) = state.clients.get(&target_id) {
                    notifications.push((
                        target.writer.clone(),
                        Out::RoomLeft {
                            room_id: room_id.clone(),
                            device_id: device_id.clone(),
                        },
                    ));
                }
            }
        }
        notifications
    };
    send_all(notifications);
}

fn send_retain_error(writer: &Writer, scope: &str, error: RetainError) {
    send(
        writer,
        &Out::Error {
            scope: scope.to_string(),
            msg: error.message(),
        },
    );
}

fn send_all(notifications: Vec<(Writer, Out)>) {
    for (writer, message) in notifications {
        send(&writer, &message);
    }
}
