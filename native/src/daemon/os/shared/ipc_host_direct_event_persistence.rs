use crate::share::{DirectSignalEvent, ShareIdentity, ShareProfiles};

use super::direct_event_queue::{enqueue_group, grouped, DirectEventGroup, PendingDirectEvent};
use super::direct_events::{persist_group, GroupPersistError};

pub(crate) struct DirectPersistBatch {
    pub committed: Option<ShareProfiles>,
    pub retry: Vec<PendingDirectEvent>,
    pub errors: Vec<String>,
}

pub(crate) fn persist_all(events: Vec<PendingDirectEvent>) -> DirectPersistBatch {
    let default_name = events
        .first()
        .map(|pending| pending.expected_identity.device_name.clone())
        .unwrap_or_else(super::default_device_name);
    let retry_events = events.clone();
    let mut events = Some(events);
    let mut completed = None;
    let result = ShareIdentity::with_current_locked(default_name, |current| {
        completed = Some(persist_with_loaded_identity(
            current,
            events.take().unwrap_or_default(),
            persist_group,
        ));
        Ok(())
    });
    match result {
        Ok(()) => completed.unwrap_or_else(empty_batch),
        Err(error) => retry_batch(
            retry_events,
            format!("Lokale Share-Identitaet konnte nicht gesperrt werden: {error}"),
        ),
    }
}

pub(super) fn persist_with_loaded_identity<F>(
    current: &ShareIdentity,
    events: Vec<PendingDirectEvent>,
    mut persist: F,
) -> DirectPersistBatch
where
    F: FnMut(&ShareIdentity, &[DirectSignalEvent]) -> Result<ShareProfiles, GroupPersistError>,
{
    let mut batch = empty_batch();
    for group in grouped(events) {
        let mut outcome = None;
        let generation = crate::share::with_matching_identity_generation(
            &group.expected_identity,
            current,
            |locked| {
                outcome = Some(persist(locked, &group.events));
                Ok(())
            },
        );
        if let Err(error) = generation {
            reject_group(&mut batch, group, error);
            continue;
        }
        let Some(outcome) = outcome else {
            retry_group(
                &mut batch,
                group,
                "Identitaetspruefung wurde nicht ausgefuehrt".into(),
            );
            continue;
        };
        match outcome {
            Ok(committed) => batch.committed = Some(committed),
            Err(GroupPersistError::Permanent(error)) => batch.errors.push(format!(
                "Tracked-Direct-Request {} wurde dauerhaft verworfen: {error}",
                group.request_id
            )),
            Err(GroupPersistError::Retryable(error)) => retry_group(&mut batch, group, error),
        }
    }
    batch
}

fn retry_batch(events: Vec<PendingDirectEvent>, error: String) -> DirectPersistBatch {
    let mut batch = empty_batch();
    for group in grouped(events) {
        let request_id = group.request_id.clone();
        match enqueue_group(&mut batch.retry, &group.expected_identity, group.events) {
            Ok(()) => batch.errors.push(format!(
                "Tracked-Direct-Request {request_id} wartet auf Wiederholung: {error}"
            )),
            Err(queue_error) => batch.errors.push(format!(
                "Tracked-Direct-Request {request_id} wurde verworfen: {queue_error}"
            )),
        }
    }
    batch
}

fn retry_group(batch: &mut DirectPersistBatch, group: DirectEventGroup, error: String) {
    let request_id = group.request_id.clone();
    match enqueue_group(
        &mut batch.retry,
        &group.expected_identity,
        group.events,
    ) {
        Ok(()) => batch.errors.push(format!(
            "Tracked-Direct-Request {request_id} konnte nicht gespeichert werden; Wiederholung vorgemerkt: {error}"
        )),
        Err(queue_error) => batch.errors.push(format!(
            "Tracked-Direct-Request {request_id} wurde verworfen: {queue_error}"
        )),
    }
}

fn reject_group(batch: &mut DirectPersistBatch, group: DirectEventGroup, error: String) {
    batch.errors.push(format!(
        "Tracked-Direct-Request {} gehoert zu einer veralteten lokalen Identitaet und wurde dauerhaft verworfen: {error}",
        group.request_id
    ));
}

fn empty_batch() -> DirectPersistBatch {
    DirectPersistBatch {
        committed: None,
        retry: Vec::new(),
        errors: Vec::new(),
    }
}
