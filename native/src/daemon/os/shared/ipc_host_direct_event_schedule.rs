use crate::share::ShareProfiles;

use super::direct_event_persistence::DirectPersistBatch;
use super::direct_event_queue::{
    enqueue_pending, same_pending, take_group_budget, PendingDirectEvent, MAX_PENDING_DIRECT_EVENTS,
};

pub(crate) const MAX_DIRECT_REQUEST_GROUPS_PER_TICK: usize = 16;

pub(crate) struct DirectTickBatch {
    pub committed: Option<ShareProfiles>,
    pub pending: Vec<PendingDirectEvent>,
    pub errors: Vec<String>,
}

pub(crate) fn process_tick<F>(
    pending: Vec<PendingDirectEvent>,
    incoming: Vec<PendingDirectEvent>,
    profile_commit_blocked: bool,
    mut persist: F,
) -> DirectTickBatch
where
    F: FnMut(Vec<PendingDirectEvent>) -> DirectPersistBatch,
{
    if profile_commit_blocked {
        return merge_while_blocked(pending, incoming);
    }

    let first = take_group_budget(pending, MAX_DIRECT_REQUEST_GROUPS_PER_TICK);
    let first_seen = first.selected.clone();
    let first_groups = first.selected_groups;
    let first_batch = persist_nonempty(first.selected, &mut persist);
    let mut errors = first_batch.errors;
    let first_retry = first_batch.retry;

    let mut accepted_new = Vec::new();
    for event in incoming {
        if contains(&first_seen, &event)
            || contains(&first.deferred, &event)
            || contains(&first_retry, &event)
        {
            continue;
        }
        if let Err(error) = enqueue_pending(&mut accepted_new, event) {
            errors.push(format!("Neues Tracked-Direct-Event: {error}"));
        }
    }

    let remaining_budget = MAX_DIRECT_REQUEST_GROUPS_PER_TICK.saturating_sub(first_groups);
    let second = take_group_budget(accepted_new, remaining_budget);
    let second_batch = persist_nonempty(second.selected, &mut persist);
    errors.extend(second_batch.errors);

    let mut final_pending = Vec::new();
    merge_pending(
        &mut final_pending,
        first.deferred,
        &mut errors,
        "Zurueckgestelltes Tracked-Direct-Event",
    );
    merge_pending(
        &mut final_pending,
        second.deferred,
        &mut errors,
        "Neues zurueckgestelltes Tracked-Direct-Event",
    );
    // New retryable work stays ahead of already-attempted old retries. This
    // prevents one full relation from indefinitely monopolizing the backlog.
    merge_pending(
        &mut final_pending,
        second_batch.retry,
        &mut errors,
        "Neuer Tracked-Direct-Retry",
    );
    merge_pending(
        &mut final_pending,
        first_retry,
        &mut errors,
        "Alter Tracked-Direct-Retry",
    );
    debug_assert!(final_pending.len() <= MAX_PENDING_DIRECT_EVENTS);

    DirectTickBatch {
        committed: second_batch.committed.or(first_batch.committed),
        pending: final_pending,
        errors,
    }
}

fn merge_while_blocked(
    mut pending: Vec<PendingDirectEvent>,
    incoming: Vec<PendingDirectEvent>,
) -> DirectTickBatch {
    let mut errors = Vec::new();
    for event in incoming {
        if let Err(error) = enqueue_pending(&mut pending, event) {
            errors.push(error);
        }
    }
    DirectTickBatch {
        committed: None,
        pending,
        errors,
    }
}

fn persist_nonempty<F>(events: Vec<PendingDirectEvent>, persist: &mut F) -> DirectPersistBatch
where
    F: FnMut(Vec<PendingDirectEvent>) -> DirectPersistBatch,
{
    if events.is_empty() {
        DirectPersistBatch {
            committed: None,
            retry: Vec::new(),
            errors: Vec::new(),
        }
    } else {
        persist(events)
    }
}

fn contains(events: &[PendingDirectEvent], event: &PendingDirectEvent) -> bool {
    events.iter().any(|queued| same_pending(queued, event))
}

fn merge_pending(
    queue: &mut Vec<PendingDirectEvent>,
    events: Vec<PendingDirectEvent>,
    errors: &mut Vec<String>,
    context: &str,
) {
    for event in events {
        if let Err(error) = enqueue_pending(queue, event) {
            errors.push(format!("{context}: {error}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::{process_tick, MAX_DIRECT_REQUEST_GROUPS_PER_TICK};
    use crate::share::{
        DirectEnvelopeKind, DirectPeerIdentity, DirectRequestId, DirectSignalEvent, ShareIdentity,
        ShareProfiles,
    };

    use super::super::direct_event_persistence::DirectPersistBatch;
    use super::super::direct_event_queue::{grouped, PendingDirectEvent};

    #[test]
    fn tick_processes_at_most_the_fixed_request_group_budget() {
        let identity = identity();
        let pending: Vec<_> = (0..MAX_DIRECT_REQUEST_GROUPS_PER_TICK + 4)
            .map(|index| event(&identity, index, 1))
            .collect();
        let calls = RefCell::new(Vec::new());
        let result = process_tick(pending, Vec::new(), false, |events| {
            calls.borrow_mut().push(grouped(events).len());
            success()
        });

        assert_eq!(*calls.borrow(), [MAX_DIRECT_REQUEST_GROUPS_PER_TICK]);
        assert_eq!(result.pending.len(), 4);
    }

    #[test]
    fn full_bad_relation_is_discarded_and_new_good_relation_progresses_same_tick() {
        let identity = identity();
        let pending: Vec<_> = (0..super::MAX_PENDING_DIRECT_EVENTS)
            .map(|at| event(&identity, 1, i64::try_from(at + 1).unwrap()))
            .collect();
        let good = request_id(2);
        let calls = RefCell::new(Vec::new());
        let result = process_tick(pending, vec![event(&identity, 2, 1)], false, |events| {
            let request_id = grouped(events)[0].request_id.clone();
            calls.borrow_mut().push(request_id.clone());
            if request_id == good {
                success()
            } else {
                DirectPersistBatch {
                    committed: None,
                    retry: Vec::new(),
                    errors: vec!["bad A discarded".into()],
                }
            }
        });

        assert_eq!(*calls.borrow(), [request_id(1), good]);
        assert!(result.committed.is_some());
        assert!(result.pending.is_empty());
    }

    #[test]
    fn full_retryable_relation_does_not_block_new_good_relation_in_remaining_budget() {
        let identity = identity();
        let retried = request_id(1);
        let good = request_id(2);
        let pending: Vec<_> = (0..super::MAX_PENDING_DIRECT_EVENTS)
            .map(|at| event(&identity, 1, i64::try_from(at + 1).unwrap()))
            .collect();
        let calls = RefCell::new(Vec::new());
        let result = process_tick(pending, vec![event(&identity, 2, 1)], false, |events| {
            let request_id = grouped(events.clone())[0].request_id.clone();
            calls.borrow_mut().push(request_id.clone());
            if request_id == retried {
                DirectPersistBatch {
                    committed: None,
                    retry: events,
                    errors: vec!["retry A".into()],
                }
            } else {
                success()
            }
        });

        assert_eq!(*calls.borrow(), [retried.clone(), good]);
        assert!(result.committed.is_some());
        assert_eq!(result.pending.len(), super::MAX_PENDING_DIRECT_EVENTS);
        let groups = grouped(result.pending);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].request_id, retried);
        assert!(!result.errors.iter().any(|error| error.contains("Backlog")));
    }

    #[test]
    fn retry_is_rotated_behind_new_work_and_not_reprocessed_in_one_tick() {
        let identity = identity();
        let retried = request_id(1);
        let good = request_id(2);
        let calls = RefCell::new(Vec::new());
        let result = process_tick(
            vec![event(&identity, 1, 1)],
            vec![event(&identity, 2, 1)],
            false,
            |events| {
                let request_id = grouped(events.clone())[0].request_id.clone();
                calls.borrow_mut().push(request_id.clone());
                if request_id == retried {
                    DirectPersistBatch {
                        committed: None,
                        retry: events,
                        errors: vec!["retry A".into()],
                    }
                } else {
                    success()
                }
            },
        );

        assert_eq!(*calls.borrow(), [retried, good]);
        assert_eq!(result.pending.len(), 1);
        assert_eq!(grouped(result.pending)[0].request_id, request_id(1));
    }

    fn success() -> DirectPersistBatch {
        DirectPersistBatch {
            committed: Some(ShareProfiles::default()),
            retry: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn event(identity: &ShareIdentity, request: usize, at: i64) -> PendingDirectEvent {
        PendingDirectEvent {
            expected_identity: identity.clone(),
            event: DirectSignalEvent::EnvelopeAttempted {
                request_id: request_id(request),
                envelope: DirectEnvelopeKind::Request,
                attempt_count: u32::try_from(at).unwrap(),
                at,
                failure: None,
            },
        }
    }

    fn request_id(index: usize) -> DirectRequestId {
        DirectRequestId::parse(format!("00000000-0000-4000-8000-{index:012x}")).unwrap()
    }

    fn identity() -> ShareIdentity {
        let iroh_secret = iroh::SecretKey::from_bytes(&[1; 32]);
        let peer = DirectPeerIdentity::from_secret("local-device", "Local", &iroh_secret);
        ShareIdentity {
            device_id: peer.device_id,
            device_name: peer.device_name,
            direct_lookup_id: "lookup".into(),
            public_key: peer.public_key,
            fingerprint: peer.fingerprint,
            node_id: peer.node_id,
            iroh_secret,
            direct_secret: [7; 32],
        }
    }
}
