use super::ipc_host::{configure_service, ShareHost};
use super::state::log;

impl ShareHost {
    pub(super) fn drain_events(&self) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => return,
        };
        let events: Vec<_> = state
            .service
            .as_ref()
            .map(|service| service.events.try_iter().collect())
            .unwrap_or_default();
        let retrying_profile_commit = state.pending_profiles_base.is_some();
        let retrying_direct_events = !state.pending_direct_events.is_empty();
        let retrying_legacy_events = !state.pending_legacy_events.is_empty();
        let retrying_legacy_answers = !state
            .profiles
            .legacy_answers_due(crate::share::core_now_secs())
            .is_empty();
        if events.is_empty()
            && !retrying_profile_commit
            && !retrying_direct_events
            && !retrying_legacy_events
            && !retrying_legacy_answers
        {
            return;
        }
        let pending_direct_events = std::mem::take(&mut state.pending_direct_events);
        let mut direct_events = Vec::new();
        let mut legacy_events = std::mem::take(&mut state.pending_legacy_events);
        let previous_profiles = state.profiles.clone();
        let commit_base = state
            .pending_profiles_base
            .take()
            .unwrap_or_else(|| previous_profiles.clone());
        let mut changed = false;
        for event in events {
            use crate::share::ShareEvent as Event;
            let mut ui_event = Some(event.clone());
            match event {
                Event::Status(status) => log(&format!("share: {status}")),
                Event::Error(error) => {
                    log(&format!("share error: {error}"));
                    state.signal_error = Some(error);
                }
                Event::ServerConnected => {
                    log("share signaling connected");
                    state.signal_connected = true;
                    state.signal_error = None;
                }
                Event::ServerDisconnected(error) => {
                    log(&format!("share signaling disconnected: {error}"));
                    state.signal_connected = false;
                    state.signal_error = Some(error);
                }
                Event::DirectSignal(event) => {
                    match state.identity.clone() {
                        Some(expected_identity) => {
                            if let Err(error) = super::ipc_host::direct_event_queue::enqueue(
                                &mut direct_events,
                                expected_identity,
                                event,
                            ) {
                                super::ipc_host::ui_events::push(
                                    &mut state.ui_events,
                                    crate::share::ShareEvent::Error(error),
                                );
                            }
                        }
                        None => super::ipc_host::ui_events::push(
                            &mut state.ui_events,
                            crate::share::ShareEvent::Error(
                                "Tracked-Direct-Event ohne lokale Share-Identitaet wurde verworfen"
                                    .into(),
                            ),
                        ),
                    }
                    ui_event = None;
                }
                Event::DirectAvailable {
                    lookup_id,
                    presence,
                } => {
                    if let Some(contact) = state
                        .profiles
                        .direct_contacts
                        .iter_mut()
                        .find(|contact| contact.lookup_id == lookup_id)
                    {
                        if !contact.expected_node_id.trim().is_empty()
                            && contact.expected_node_id != presence.node_id
                        {
                            contact.status = crate::share::ShareStatus::IdentityConflict;
                            contact.last_error = Some("Iroh NodeId passt nicht zum Code".into());
                            changed = true;
                            continue;
                        }
                        if contact.expected_node_id.trim().is_empty() {
                            contact.expected_node_id = presence.node_id.clone();
                        }
                        contact.remote_device_id = Some(presence.device_id.clone());
                        contact.remote_public_key = Some(presence.public_key.clone());
                        contact.last_seen = Some(crate::share::core_now_secs());
                        contact.status =
                            if contact.access_state == crate::share::DirectAccessState::Accepted {
                                crate::share::ShareStatus::Available
                            } else {
                                crate::share::ShareStatus::WaitingForAccess
                            };
                        contact.last_error = None;
                        contact.presence = Some(presence);
                        changed = true;
                    }
                }
                Event::DirectOffline { lookup_id } => {
                    if let Some(contact) = state
                        .profiles
                        .direct_contacts
                        .iter_mut()
                        .find(|contact| contact.lookup_id == lookup_id)
                    {
                        contact.status = crate::share::ShareStatus::Offline;
                        changed = true;
                    }
                }
                Event::DirectAccessRequest {
                    lookup_id,
                    presence,
                } => {
                    // Authentication happened before this event was emitted.
                    // Keep the raw event out of the UI/IPC backlog; the
                    // separately typed durable legacy ledger is canonical.
                    if let Err(error) = super::ipc_host::legacy_events::enqueue(
                        &mut legacy_events,
                        lookup_id,
                        presence,
                    ) {
                        super::ipc_host::ui_events::push(
                            &mut state.ui_events,
                            crate::share::ShareEvent::Error(error),
                        );
                    }
                    ui_event = None;
                }
                Event::DirectAccessAccepted {
                    lookup_id,
                    requester_device_id,
                    accepted,
                    presence,
                    msg,
                } => {
                    let Some(local_device_id) = state
                        .identity
                        .as_ref()
                        .map(|identity| identity.device_id.clone())
                    else {
                        super::ipc_host::ui_events::push(
                            &mut state.ui_events,
                            crate::share::ShareEvent::Error(
                                "Share-Identitaet ist nicht verfuegbar".into(),
                            ),
                        );
                        continue;
                    };
                    if requester_device_id != local_device_id {
                        continue;
                    }
                    if let Some(contact) = state
                        .profiles
                        .direct_contacts
                        .iter_mut()
                        .find(|contact| contact.lookup_id == lookup_id)
                    {
                        if accepted {
                            contact.access_state = crate::share::DirectAccessState::Accepted;
                            contact.accepted_at = Some(crate::share::core_now_secs());
                            if let Some(presence) = presence {
                                contact.remote_device_id = Some(presence.device_id.clone());
                                contact.remote_public_key = Some(presence.public_key.clone());
                                contact.accepted_public_key = Some(presence.public_key.clone());
                                if contact.expected_node_id.trim().is_empty() {
                                    contact.expected_node_id = presence.node_id.clone();
                                }
                                contact.presence = Some(presence);
                            }
                            contact.status = crate::share::ShareStatus::Available;
                            contact.last_error = None;
                        } else {
                            contact.access_state = crate::share::DirectAccessState::Ignored;
                            contact.status = crate::share::ShareStatus::Failed(
                                msg.unwrap_or_else(|| "Freigabe abgelehnt".into()),
                            );
                        }
                        changed = true;
                    }
                }
                Event::RoomRoster { room_id, members } => {
                    let Some(local_device) = state
                        .identity
                        .as_ref()
                        .map(|identity| identity.device_id.clone())
                    else {
                        continue;
                    };
                    if let Some(room) = state
                        .profiles
                        .rooms
                        .iter_mut()
                        .find(|room| room.room_id == room_id)
                    {
                        room.status = crate::share::ShareStatus::Available;
                        room.last_seen = Some(crate::share::core_now_secs());
                        for presence in members {
                            if presence.device_id != local_device {
                                upsert_room_member(room, presence);
                            }
                        }
                        changed = true;
                    }
                }
                Event::RoomJoined { room_id, presence } => {
                    let Some(local_device) = state
                        .identity
                        .as_ref()
                        .map(|identity| identity.device_id.clone())
                    else {
                        continue;
                    };
                    if let Some(room) = state
                        .profiles
                        .rooms
                        .iter_mut()
                        .find(|room| room.room_id == room_id)
                    {
                        if presence.device_id != local_device {
                            upsert_room_member(room, presence);
                            changed = true;
                        }
                    }
                }
                Event::RoomLeft { room_id, device_id } => {
                    if let Some(room) = state
                        .profiles
                        .rooms
                        .iter_mut()
                        .find(|room| room.room_id == room_id)
                    {
                        if let Some(member) = room
                            .members
                            .iter_mut()
                            .find(|member| member.device_id == device_id)
                        {
                            member.status = crate::share::ShareStatus::Offline;
                            changed = true;
                        }
                    }
                }
            }
            if let Some(event) = ui_event {
                super::ipc_host::ui_events::push(&mut state.ui_events, event);
            }
        }
        if changed || retrying_profile_commit {
            let worker_profiles = state.profiles.clone();
            match crate::share::ShareProfiles::mutate_persisted(
                Some(super::ipc_host::default_home()),
                |latest| {
                    super::ipc_host::profile_merge::merge_worker_updates(
                        latest,
                        &commit_base,
                        &worker_profiles,
                    );
                    Ok(())
                },
            ) {
                Err(error) => {
                    state.pending_profiles_base = Some(commit_base);
                    state.last_reload = std::time::Instant::now();
                    super::ipc_host::ui_events::push(
                        &mut state.ui_events,
                        crate::share::ShareEvent::Error(format!(
                            "Share-Status konnte nicht gespeichert werden; Wiederholung vorgemerkt: {error}"
                        )),
                    );
                }
                Ok(committed) => {
                    state.profiles = committed;
                    state.pending_profiles_base = None;
                }
            }
            if state.pending_profiles_base.is_none() {
                if let Some(service) = &state.service {
                    if let Err(error) = configure_service(service, &state.profiles) {
                        super::ipc_host::ui_events::push(
                            &mut state.ui_events,
                            crate::share::ShareEvent::Error(format!(
                                "Share-Konfiguration konnte nicht zugestellt werden: {error}"
                            )),
                        );
                    }
                }
            }
        }
        let direct_batch = super::ipc_host::direct_event_schedule::process_tick(
            pending_direct_events,
            direct_events,
            state.pending_profiles_base.is_some(),
            super::ipc_host::direct_event_persistence::persist_all,
        );
        state.pending_direct_events = direct_batch.pending;
        for error in direct_batch.errors {
            super::ipc_host::ui_events::push(
                &mut state.ui_events,
                crate::share::ShareEvent::Error(error),
            );
        }
        if let Some(committed) = direct_batch.committed {
            state.profiles = committed;
            if let Some(service) = &state.service {
                if let Err(error) = configure_service(service, &state.profiles) {
                    super::ipc_host::ui_events::push(
                        &mut state.ui_events,
                        crate::share::ShareEvent::Error(format!(
                            "Direkt-Lifecycle konnte nicht zugestellt werden: {error}"
                        )),
                    );
                }
            }
        }
        if !state.pending_direct_events.is_empty() {
            state.last_reload = std::time::Instant::now();
        }
        if state.pending_profiles_base.is_some() && !legacy_events.is_empty() {
            state.pending_legacy_events = legacy_events;
            legacy_events = Vec::new();
        }
        if !legacy_events.is_empty() {
            let batch = super::ipc_host::legacy_events::persist_all(legacy_events);
            state.pending_legacy_events = batch.retry;
            for error in batch.rejected {
                super::ipc_host::ui_events::push(
                    &mut state.ui_events,
                    crate::share::ShareEvent::Error(error),
                );
            }
            if let Some(committed) = batch.committed {
                state.profiles = committed;
                if let Some(service) = &state.service {
                    if let Err(error) = configure_service(service, &state.profiles) {
                        super::ipc_host::ui_events::push(
                            &mut state.ui_events,
                            crate::share::ShareEvent::Error(format!(
                                "Legacy-Anfragen konnten nicht zugestellt werden: {error}"
                            )),
                        );
                    }
                }
            }
            if !state.pending_legacy_events.is_empty() {
                state.last_reload = std::time::Instant::now();
            }
        }
        drop(state);
        self.flush_legacy_answers();
    }
}

fn upsert_room_member(room: &mut crate::share::RoomProfile, presence: crate::share::PeerPresence) {
    if let Some(member) = room
        .members
        .iter_mut()
        .find(|member| member.device_id == presence.device_id)
    {
        if member.public_key != presence.public_key
            || (!member.node_id.is_empty() && member.node_id != presence.node_id)
        {
            member
                .exec
                .reset_for_identity_change(crate::share::core_now_secs());
            member.status = crate::share::ShareStatus::IdentityConflict;
            return;
        }
        member.device_name = presence.device_name.clone();
        member.fingerprint = presence.fingerprint.clone();
        member.candidates = presence.candidates.clone();
        member.node_id = presence.node_id.clone();
        member.relay_url = presence.relay_url.clone();
        member.last_seen = Some(crate::share::core_now_secs());
        member.status = crate::share::ShareStatus::Available;
        member.presence = Some(presence);
    } else {
        room.members.push(crate::share::RoomMember {
            device_id: presence.device_id.clone(),
            device_name: presence.device_name.clone(),
            fingerprint: presence.fingerprint.clone(),
            public_key: presence.public_key.clone(),
            node_id: presence.node_id.clone(),
            relay_url: presence.relay_url.clone(),
            candidates: presence.candidates.clone(),
            last_seen: Some(crate::share::core_now_secs()),
            status: crate::share::ShareStatus::Available,
            blocked: false,
            exec: crate::share::ExecGrant::default(),
            presence: Some(presence),
        });
    }
}
