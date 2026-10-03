use super::ipc_host::{
    configure_service, reload_committed_profiles, update_runtime, ShareHost, ShareHostState,
};
use super::state::log;

#[path = "ipc_host_relation_events.rs"]
mod relation_events;

impl ShareHost {
    pub(super) fn drain_events(&self) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => return,
        };
        let mut events: Vec<_> = std::mem::take(&mut state.pending_lan_events);
        if let Some(service) = state.service.as_ref() {
            events.extend(service.events.try_iter());
        }
        let retrying_profile_commit = state.pending_profiles_base.is_some();
        let retrying_direct_events = !state.pending_direct_events.is_empty();
        let retrying_legacy_events = !state.pending_legacy_events.is_empty();
        let retrying_legacy_answers = !state
            .profiles
            .legacy_answers_due(crate::share::core_now_secs())
            .is_empty();
        retry_deferred_configuration(&mut state);
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
        let mut runtime_profiles_committed = false;
        let local_device_id = state
            .identity
            .as_ref()
            .map(|identity| identity.device_id.clone());
        let now = crate::share::core_now_secs();
        for event in events {
            use crate::share::ShareEvent as Event;
            let mut ui_event = Some(event.clone());
            match relation_events::apply(
                &mut state.profiles,
                local_device_id.as_deref(),
                &event,
                now,
            ) {
                relation_events::RelationEvent::Applied {
                    changed: event_changed,
                    forward,
                    error,
                } => {
                    changed |= event_changed;
                    if let Some(error) = error {
                        super::ipc_host::ui_events::push(
                            &mut state.ui_events,
                            crate::share::ShareEvent::Error(error),
                        );
                    }
                    if !forward {
                        continue;
                    }
                }
                relation_events::RelationEvent::Other => match event {
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
                    Event::RuntimeProfilesCommitted => {
                        runtime_profiles_committed = true;
                        ui_event = None;
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
                    Event::DirectAccessRequest {
                        lookup_id,
                        presence,
                    } => {
                        // Authentication happened before this event was
                        // emitted. Keep the raw event out of the UI/IPC
                        // backlog; the typed durable legacy ledger is
                        // canonical.
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
                    Event::Discovery(event) => state.discovery_offers.observe(&event),
                    // Relation events are applied above.
                    _ => {}
                },
            }
            if let Some(event) = ui_event {
                super::ipc_host::ui_events::push(&mut state.ui_events, event);
            }
        }
        let canonical_reloaded = if runtime_profiles_committed {
            match reload_committed_profiles(&mut state, &previous_profiles, changed) {
                Ok(()) => true,
                Err(error) => {
                    state.profiles_error = Some(error.clone());
                    state.last_reload = std::time::Instant::now();
                    super::ipc_host::ui_events::push(
                        &mut state.ui_events,
                        crate::share::ShareEvent::Error(format!(
                            "Verbindliche Share-Profile konnten nicht neu geladen werden: {error}"
                        )),
                    );
                    false
                }
            }
        } else {
            false
        };
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
        }
        if canonical_reloaded
            || state.pending_profiles_base.is_none() && (changed || retrying_profile_commit)
        {
            // FA3: presence, routes and newly seen members never cost a
            // configuration transition; anything else does.
            let configuration = canonical_reloaded
                || retrying_profile_commit
                || crate::share::profiles_differ_beyond_runtime(
                    &previous_profiles,
                    &state.profiles,
                );
            let result = match &state.service {
                Some(service) if configuration => Some(configure_service(service, &state.profiles)),
                Some(service) => Some(update_runtime(service, &state.profiles).map(|()| true)),
                None => None,
            };
            deliver_configuration(&mut state, result, "Share-Konfiguration");
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
            let result = state
                .service
                .as_ref()
                .map(|service| configure_service(service, &state.profiles));
            deliver_configuration(&mut state, result, "Direkt-Lifecycle");
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
                let result = state
                    .service
                    .as_ref()
                    .map(|service| configure_service(service, &state.profiles));
                deliver_configuration(&mut state, result, "Legacy-Anfragen");
            }
            if !state.pending_legacy_events.is_empty() {
                state.last_reload = std::time::Instant::now();
            }
        }
        drop(state);
        self.flush_legacy_answers();
    }
}

/// A configuration the worker could not take while a repair wrote its
/// relation is retried on the next tick instead of being dropped, so a
/// revocation always reaches the running worker (S66).
fn deliver_configuration(
    state: &mut ShareHostState,
    result: Option<Result<bool, String>>,
    what: &str,
) {
    match result {
        Some(Ok(true)) | None => {}
        Some(Ok(false)) => state.configure_deferred = true,
        Some(Err(error)) => super::ipc_host::ui_events::push(
            &mut state.ui_events,
            crate::share::ShareEvent::Error(format!(
                "{what} konnte nicht zugestellt werden: {error}"
            )),
        ),
    }
}

fn retry_deferred_configuration(state: &mut ShareHostState) {
    if !state.configure_deferred {
        return;
    }
    state.configure_deferred = false;
    let result = state
        .service
        .as_ref()
        .map(|service| configure_service(service, &state.profiles));
    deliver_configuration(state, result, "Share-Konfiguration");
}
