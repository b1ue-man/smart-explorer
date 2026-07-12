use super::core::public_fingerprint;
use super::direct_lifecycle::{
    DirectDecisionDeliveryState, DirectDecisionState, DirectDeliveryState, DirectFailure,
    DirectLifecycleEvent, DirectRequestRecord,
};
use super::direct_lifecycle_error::DirectLifecycleError;
use super::direct_protocol::{
    DirectDecisionKind, DirectPeerIdentity, DirectRequestId, SignedDirectRequest,
};

const REQUEST_ID: &str = "123e4567-e89b-42d3-a456-426614174000";
const OTHER_ID: &str = "123e4567-e89b-42d3-b456-426614174001";
const SECRET: [u8; 32] = [0x44; 32];

fn request_id(value: &str) -> DirectRequestId {
    DirectRequestId::parse(value).unwrap()
}

fn request() -> SignedDirectRequest {
    let requester_key = iroh::SecretKey::from_bytes(&[1; 32]);
    let target_key = iroh::SecretKey::from_bytes(&[2; 32]);
    let target_public = target_key.public().to_string();
    SignedDirectRequest::sign_with_nonce(
        request_id(REQUEST_ID),
        "lookup-a",
        DirectPeerIdentity::from_secret("requester-a", "Requester", &requester_key),
        DirectPeerIdentity::pinned_target(
            target_public.clone(),
            public_fingerprint(target_public.as_bytes()),
        ),
        100,
        200,
        "request-nonce",
        Some("access please".into()),
        &SECRET,
        &requester_key,
    )
    .unwrap()
}

fn delivery(state: DirectDeliveryState, at: i64) -> DirectLifecycleEvent {
    DirectLifecycleEvent::Delivery {
        request_id: request_id(REQUEST_ID),
        state,
        at,
        failure: None,
    }
}

fn decision(kind: DirectDecisionKind, revision: u64, at: i64) -> DirectLifecycleEvent {
    DirectLifecycleEvent::Decision {
        request_id: request_id(REQUEST_ID),
        decision: kind,
        revision,
        at,
        message: None,
    }
}

fn decision_delivery(
    state: DirectDecisionDeliveryState,
    revision: u64,
    at: i64,
) -> DirectLifecycleEvent {
    DirectLifecycleEvent::DecisionDelivery {
        request_id: request_id(REQUEST_ID),
        state,
        revision,
        at,
        failure: None,
    }
}

#[test]
fn state_machine_codes_are_stable() {
    assert_eq!(DirectDeliveryState::Queued.code(), "queued");
    assert_eq!(DirectDeliveryState::ServerQueued.code(), "server_queued");
    assert_eq!(DirectDeliveryState::Received.code(), "received");
    assert_eq!(DirectDecisionState::Accepted.code(), "accepted");
    assert_eq!(DirectDecisionState::Revoked.code(), "revoked");
    assert_eq!(
        DirectDecisionDeliveryState::NotStarted.code(),
        "not_started"
    );
    assert_eq!(DirectDecisionDeliveryState::Received.code(), "received");
}

#[test]
fn persisted_record_round_trips_all_orthogonal_states() {
    let mut record = DirectRequestRecord::new(request());
    record
        .apply(delivery(DirectDeliveryState::Received, 120))
        .unwrap();
    record
        .apply(decision(DirectDecisionKind::Accepted, 1, 130))
        .unwrap();
    record
        .apply(decision_delivery(
            DirectDecisionDeliveryState::Received,
            1,
            140,
        ))
        .unwrap();

    let encoded = serde_json::to_string_pretty(&record).unwrap();
    let decoded: DirectRequestRecord = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, record);
    assert!(encoded.contains(r#""state": "received""#));
    assert!(encoded.contains(r#""state": "accepted""#));
}

#[test]
fn delivery_reducer_is_monotonic_idempotent_and_order_independent() {
    let mut record = DirectRequestRecord::new(request());
    assert!(record
        .apply(delivery(DirectDeliveryState::Delivered, 130))
        .unwrap());
    assert!(!record
        .apply(delivery(DirectDeliveryState::ServerQueued, 120))
        .unwrap());
    assert!(!record
        .apply(delivery(DirectDeliveryState::Delivered, 130))
        .unwrap());
    assert!(record
        .apply(delivery(DirectDeliveryState::Received, 140))
        .unwrap());
    assert!(!record
        .apply(delivery(DirectDeliveryState::Expired, 200))
        .unwrap());
    assert_eq!(record.delivery.state, DirectDeliveryState::Received);
    assert_eq!(record.delivery.changed_at, 140);
}

#[test]
fn expiry_and_failure_are_terminal_and_timestamp_checked() {
    let mut record = DirectRequestRecord::new(request());
    assert_eq!(
        record.apply(delivery(DirectDeliveryState::Expired, 199)),
        Err(DirectLifecycleError::InvalidTimestamp)
    );
    assert!(record
        .apply(delivery(DirectDeliveryState::Expired, 200))
        .unwrap());
    assert!(!record
        .apply(delivery(DirectDeliveryState::Received, 200))
        .unwrap());

    let mut record = DirectRequestRecord::new(request());
    assert_eq!(
        record.apply(delivery(DirectDeliveryState::Failed, 120)),
        Err(DirectLifecycleError::MissingFailure)
    );
    assert!(record
        .apply(DirectLifecycleEvent::Delivery {
            request_id: request_id(REQUEST_ID),
            state: DirectDeliveryState::Failed,
            at: 120,
            failure: Some(DirectFailure {
                code: "invalid_signature".into(),
                message: "target rejected the envelope".into(),
            }),
        })
        .unwrap());
    assert_eq!(record.delivery.state, DirectDeliveryState::Failed);
    assert_eq!(
        record.apply(delivery(DirectDeliveryState::Sent, 201)),
        Err(DirectLifecycleError::AfterExpiry)
    );
}

#[test]
fn accepted_decision_can_only_advance_to_higher_revision_revoke() {
    let mut record = DirectRequestRecord::new(request());
    assert!(record
        .apply(decision(DirectDecisionKind::Accepted, 1, 130))
        .unwrap());
    assert!(!record
        .apply(decision(DirectDecisionKind::Accepted, 1, 130))
        .unwrap());
    assert_eq!(
        record.apply(decision(DirectDecisionKind::Rejected, 1, 130)),
        Err(DirectLifecycleError::DecisionRevisionConflict)
    );
    assert!(record
        .apply(decision(DirectDecisionKind::Revoked, 2, 250))
        .unwrap());
    assert_eq!(record.decision.state, DirectDecisionState::Revoked);
    assert_eq!(record.decision.revision, 2);
    assert_eq!(
        record.decision_delivery.state,
        DirectDecisionDeliveryState::Queued
    );
    assert_eq!(record.decision_delivery.revision, 2);
    assert!(!record
        .apply(decision(DirectDecisionKind::Accepted, 1, 130))
        .unwrap());
}

#[test]
fn out_of_order_revoke_converges_without_resurrecting_acceptance() {
    let mut record = DirectRequestRecord::new(request());
    assert!(record
        .apply(decision(DirectDecisionKind::Revoked, 2, 250))
        .unwrap());
    assert!(!record
        .apply(decision(DirectDecisionKind::Accepted, 1, 130))
        .unwrap());
    assert_eq!(record.decision.state, DirectDecisionState::Revoked);
    assert_eq!(record.decision.revision, 2);
}

#[test]
fn decision_delivery_is_revision_scoped_and_order_independent() {
    let mut record = DirectRequestRecord::new(request());
    record
        .apply(decision(DirectDecisionKind::Accepted, 1, 130))
        .unwrap();
    assert!(record
        .apply(decision_delivery(
            DirectDecisionDeliveryState::Delivered,
            1,
            150,
        ))
        .unwrap());
    assert!(!record
        .apply(decision_delivery(
            DirectDecisionDeliveryState::ServerQueued,
            1,
            140,
        ))
        .unwrap());
    assert!(record
        .apply(decision_delivery(
            DirectDecisionDeliveryState::Received,
            1,
            160,
        ))
        .unwrap());
    assert_eq!(
        record.apply(decision_delivery(
            DirectDecisionDeliveryState::Received,
            2,
            170,
        )),
        Err(DirectLifecycleError::DecisionNotKnown)
    );
}

#[test]
fn reducer_rejects_events_for_another_request() {
    let mut record = DirectRequestRecord::new(request());
    let event = DirectLifecycleEvent::Delivery {
        request_id: request_id(OTHER_ID),
        state: DirectDeliveryState::Sent,
        at: 110,
        failure: None,
    };
    assert_eq!(record.apply(event), Err(DirectLifecycleError::WrongRequest));
}
