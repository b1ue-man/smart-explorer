use crate::share::{DirectRequestId, DirectSignalEvent, ShareIdentity};

pub(crate) const MAX_PENDING_DIRECT_EVENTS: usize = 256;

#[derive(Clone)]
pub(crate) struct PendingDirectEvent {
    pub expected_identity: ShareIdentity,
    pub event: DirectSignalEvent,
}

pub(crate) struct DirectEventGroup {
    pub expected_identity: ShareIdentity,
    pub request_id: DirectRequestId,
    pub events: Vec<DirectSignalEvent>,
}

pub(crate) struct GroupSelection {
    pub selected: Vec<PendingDirectEvent>,
    pub deferred: Vec<PendingDirectEvent>,
    pub selected_groups: usize,
}

pub(crate) fn enqueue(
    queue: &mut Vec<PendingDirectEvent>,
    expected_identity: ShareIdentity,
    event: DirectSignalEvent,
) -> Result<bool, String> {
    enqueue_pending(
        queue,
        PendingDirectEvent {
            expected_identity,
            event,
        },
    )
}

pub(crate) fn enqueue_pending(
    queue: &mut Vec<PendingDirectEvent>,
    pending: PendingDirectEvent,
) -> Result<bool, String> {
    if queue.iter().any(|queued| same_pending(queued, &pending)) {
        return Ok(false);
    }
    if queue.len() >= MAX_PENDING_DIRECT_EVENTS {
        return Err(format!(
            "Tracked-Direct-Event-Backlog ist voll (maximal {MAX_PENDING_DIRECT_EVENTS}); Event wurde verworfen"
        ));
    }
    queue.push(pending);
    Ok(true)
}

pub(crate) fn enqueue_group(
    queue: &mut Vec<PendingDirectEvent>,
    expected_identity: &ShareIdentity,
    events: Vec<DirectSignalEvent>,
) -> Result<(), String> {
    let mut additions = Vec::new();
    for event in events {
        let pending = PendingDirectEvent {
            expected_identity: expected_identity.clone(),
            event,
        };
        if !queue.iter().any(|queued| same_pending(queued, &pending))
            && !additions
                .iter()
                .any(|queued| same_pending(queued, &pending))
        {
            additions.push(pending);
        }
    }
    if queue.len().saturating_add(additions.len()) > MAX_PENDING_DIRECT_EVENTS {
        return Err(format!(
            "Tracked-Direct-Event-Backlog ist voll (maximal {MAX_PENDING_DIRECT_EVENTS}); Request-Gruppe wurde verworfen"
        ));
    }
    queue.extend(additions);
    Ok(())
}

pub(crate) fn grouped(events: Vec<PendingDirectEvent>) -> Vec<DirectEventGroup> {
    let mut groups: Vec<DirectEventGroup> = Vec::new();
    for pending in events {
        let request_id = request_id(&pending.event).clone();
        match groups.iter_mut().find(|group| {
            group.request_id == request_id
                && same_generation(&group.expected_identity, &pending.expected_identity)
        }) {
            Some(group) => group.events.push(pending.event),
            None => groups.push(DirectEventGroup {
                expected_identity: pending.expected_identity,
                request_id,
                events: vec![pending.event],
            }),
        }
    }
    groups
}

pub(crate) fn take_group_budget(
    events: Vec<PendingDirectEvent>,
    max_groups: usize,
) -> GroupSelection {
    let mut groups = grouped(events);
    let deferred_groups = groups.split_off(max_groups.min(groups.len()));
    let selected_groups = groups.len();
    GroupSelection {
        selected: flatten(groups),
        deferred: flatten(deferred_groups),
        selected_groups,
    }
}

pub(crate) fn flatten(groups: Vec<DirectEventGroup>) -> Vec<PendingDirectEvent> {
    let mut pending = Vec::new();
    for group in groups {
        for event in group.events {
            pending.push(PendingDirectEvent {
                expected_identity: group.expected_identity.clone(),
                event,
            });
        }
    }
    pending
}

pub(crate) fn same_pending(left: &PendingDirectEvent, right: &PendingDirectEvent) -> bool {
    left.event == right.event && same_generation(&left.expected_identity, &right.expected_identity)
}

fn same_generation(left: &ShareIdentity, right: &ShareIdentity) -> bool {
    crate::share::with_matching_identity_generation(left, right, |_| Ok(())).is_ok()
}

pub(crate) fn request_id(event: &DirectSignalEvent) -> &DirectRequestId {
    match event {
        DirectSignalEvent::RequestReceived { request, .. } => &request.request_id,
        DirectSignalEvent::RequestReceiptReceived { receipt, .. } => &receipt.request_id,
        DirectSignalEvent::DecisionReceived { decision, .. } => &decision.request_id,
        DirectSignalEvent::DecisionReceiptReceived { receipt, .. } => &receipt.request_id,
        DirectSignalEvent::EnvelopeAttempted { request_id, .. }
        | DirectSignalEvent::RelayAcknowledged { request_id, .. } => request_id,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        enqueue, grouped, take_group_budget, PendingDirectEvent, MAX_PENDING_DIRECT_EVENTS,
    };
    use crate::share::{
        DirectEnvelopeKind, DirectPeerIdentity, DirectRequestId, DirectSignalEvent, ShareIdentity,
    };

    #[test]
    fn queue_deduplicates_per_generation_and_enforces_its_hard_cap() {
        let mut queue = Vec::new();
        let identity = identity("lookup", 1);
        let first = event(0, 1);
        for _ in 0..MAX_PENDING_DIRECT_EVENTS * 2 {
            enqueue(&mut queue, identity.clone(), first.clone()).unwrap();
        }
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].event, first);

        for index in 1..MAX_PENDING_DIRECT_EVENTS {
            enqueue(&mut queue, identity.clone(), event(index, 1)).unwrap();
        }
        let error = enqueue(&mut queue, identity, event(MAX_PENDING_DIRECT_EVENTS, 1)).unwrap_err();
        assert!(error.contains("Backlog ist voll"));
        assert_eq!(queue.len(), MAX_PENDING_DIRECT_EVENTS);
    }

    #[test]
    fn identical_events_from_distinct_identity_generations_are_not_deduplicated() {
        let mut queue = Vec::new();
        let event = event(1, 1);
        enqueue(&mut queue, identity("old", 1), event.clone()).unwrap();
        enqueue(&mut queue, identity("new", 2), event).unwrap();
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn grouping_preserves_generation_request_and_event_order() {
        let identity = identity("lookup", 1);
        let groups = grouped(vec![
            pending(&identity, event(1, 1)),
            pending(&identity, event(2, 2)),
            pending(&identity, event(1, 3)),
        ]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].request_id, request_id(1));
        assert_eq!(groups[1].request_id, request_id(2));
        assert_eq!(event_times(&groups[0].events), [1, 3]);
        assert_eq!(event_times(&groups[1].events), [2]);
    }

    #[test]
    fn group_budget_defers_whole_later_requests() {
        let identity = identity("lookup", 1);
        let selection = take_group_budget(
            vec![
                pending(&identity, event(1, 1)),
                pending(&identity, event(2, 2)),
                pending(&identity, event(1, 3)),
                pending(&identity, event(3, 4)),
            ],
            2,
        );
        assert_eq!(selection.selected_groups, 2);
        assert_eq!(event_times_pending(&selection.selected), [1, 3, 2]);
        assert_eq!(event_times_pending(&selection.deferred), [4]);
    }

    fn pending(identity: &ShareIdentity, event: DirectSignalEvent) -> PendingDirectEvent {
        PendingDirectEvent {
            expected_identity: identity.clone(),
            event,
        }
    }

    fn event(index: usize, at: i64) -> DirectSignalEvent {
        DirectSignalEvent::EnvelopeAttempted {
            request_id: request_id(index),
            envelope: DirectEnvelopeKind::Request,
            attempt_count: u32::try_from(at).unwrap(),
            at,
            failure: None,
        }
    }

    fn request_id(index: usize) -> DirectRequestId {
        DirectRequestId::parse(format!("00000000-0000-4000-8000-{index:012x}")).unwrap()
    }

    fn event_times(events: &[DirectSignalEvent]) -> Vec<i64> {
        events
            .iter()
            .map(|event| match event {
                DirectSignalEvent::EnvelopeAttempted { at, .. } => *at,
                _ => unreachable!(),
            })
            .collect()
    }

    fn event_times_pending(events: &[PendingDirectEvent]) -> Vec<i64> {
        events
            .iter()
            .map(|pending| match &pending.event {
                DirectSignalEvent::EnvelopeAttempted { at, .. } => *at,
                _ => unreachable!(),
            })
            .collect()
    }

    fn identity(lookup_id: &str, direct_secret: u8) -> ShareIdentity {
        let iroh_secret = iroh::SecretKey::from_bytes(&[1; 32]);
        let peer = DirectPeerIdentity::from_secret("local-device", "Local", &iroh_secret);
        ShareIdentity {
            device_id: peer.device_id,
            device_name: peer.device_name,
            direct_lookup_id: lookup_id.into(),
            public_key: peer.public_key,
            fingerprint: peer.fingerprint,
            node_id: peer.node_id,
            iroh_secret,
            direct_secret: [direct_secret; 32],
        }
    }
}
