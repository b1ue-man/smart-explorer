use std::cell::{Cell, RefCell};

use crate::share::{
    DirectEnvelopeKind, DirectPeerIdentity, DirectRequestId, DirectSignalEvent, ShareIdentity,
    ShareProfiles,
};

use super::direct_event_persistence::persist_with_loaded_identity;
use super::direct_event_queue::PendingDirectEvent;
use super::direct_events::GroupPersistError;

#[test]
fn permanent_bad_request_does_not_block_independent_valid_request() {
    let identity = identity("lookup", 7);
    let bad = request_id(1);
    let good = request_id(2);
    let calls = RefCell::new(Vec::new());
    let batch = persist_with_loaded_identity(
        &identity,
        vec![
            pending(&identity, attempt(&bad, 1)),
            pending(&identity, attempt(&good, 2)),
            pending(&identity, attempt(&bad, 3)),
        ],
        |_, events| {
            let request_id = super::direct_event_queue::request_id(&events[0]);
            calls.borrow_mut().push((request_id.clone(), events.len()));
            if request_id == &bad {
                Err(GroupPersistError::Permanent(
                    "unknown direct request".into(),
                ))
            } else {
                Ok(ShareProfiles {
                    auto_connect: false,
                    ..ShareProfiles::default()
                })
            }
        },
    );

    assert_eq!(*calls.borrow(), [(bad, 2), (good, 1)]);
    assert!(batch
        .committed
        .is_some_and(|profiles| !profiles.auto_connect));
    assert!(batch.retry.is_empty());
    assert_eq!(batch.errors.len(), 1);
    assert!(batch.errors[0].contains("unknown direct request"));
}

#[test]
fn stale_generation_is_discarded_without_running_persistence() {
    let expected = identity("old-lookup", 7);
    let current = identity("new-lookup", 8);
    let called = Cell::new(false);
    let batch = persist_with_loaded_identity(
        &current,
        vec![pending(&expected, attempt(&request_id(3), 1))],
        |_, _| {
            called.set(true);
            Ok(ShareProfiles::default())
        },
    );

    assert!(!called.get());
    assert!(batch.committed.is_none());
    assert!(batch.retry.is_empty());
    assert_eq!(batch.errors.len(), 1);
    assert!(batch.errors[0].contains("dauerhaft verworfen"));
}

#[test]
fn repeated_transient_event_is_retained_only_once() {
    let identity = identity("lookup", 7);
    let event = attempt(&request_id(4), 1);
    let batch = persist_with_loaded_identity(
        &identity,
        vec![
            pending(&identity, event.clone()),
            pending(&identity, event.clone()),
        ],
        |_, _| Err(GroupPersistError::Retryable("storage unavailable".into())),
    );

    assert_eq!(batch.retry.len(), 1);
    assert_eq!(batch.retry[0].event, event);
    assert!(crate::share::with_matching_identity_generation(
        &identity,
        &batch.retry[0].expected_identity,
        |_| Ok(())
    )
    .is_ok());
    assert_eq!(batch.errors.len(), 1);
}

#[test]
fn stale_group_is_discarded_while_current_generation_group_progresses() {
    let stale = identity("old-lookup", 7);
    let current = identity("new-lookup", 8);
    let stale_id = request_id(5);
    let current_id = request_id(6);
    let calls = RefCell::new(Vec::new());
    let batch = persist_with_loaded_identity(
        &current,
        vec![
            pending(&stale, attempt(&stale_id, 1)),
            pending(&current, attempt(&current_id, 2)),
        ],
        |_, events| {
            calls
                .borrow_mut()
                .push(super::direct_event_queue::request_id(&events[0]).clone());
            Ok(ShareProfiles::default())
        },
    );

    assert_eq!(*calls.borrow(), [current_id]);
    assert!(batch.committed.is_some());
    assert!(batch.retry.is_empty());
    assert_eq!(batch.errors.len(), 1);
    assert!(batch.errors[0].contains("veralteten lokalen Identitaet"));
}

fn request_id(index: u64) -> DirectRequestId {
    DirectRequestId::parse(format!("00000000-0000-4000-8000-{index:012x}")).unwrap()
}

fn attempt(request_id: &DirectRequestId, at: i64) -> DirectSignalEvent {
    DirectSignalEvent::EnvelopeAttempted {
        request_id: request_id.clone(),
        envelope: DirectEnvelopeKind::Request,
        attempt_count: u32::try_from(at).unwrap(),
        at,
        failure: None,
    }
}

fn pending(identity: &ShareIdentity, event: DirectSignalEvent) -> PendingDirectEvent {
    PendingDirectEvent {
        expected_identity: identity.clone(),
        event,
    }
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
