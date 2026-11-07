//! The case of `every_tracked_wire_payload_uses_the_coordinated_tag` in
//! `direct_protocol_tests.rs` that holds the tracked wire tags and fields to
//! the schema of the share server.
use super::super::wire::{
    DirectRoute, DirectRouteOutcome, TrackedDirectClientMsg, TrackedDirectServerMsg,
    TRACKED_DIRECT_CAPABILITY,
};
use super::{request, REQUEST_ID};

pub(super) fn tracked_wire_tags_match_server_schema() {
    assert_eq!(TRACKED_DIRECT_CAPABILITY, "tracked_direct_v1");
    let request = request();
    let encoded = serde_json::to_value(TrackedDirectClientMsg::Request {
        request: Box::new(request.clone()),
        legacy_presence: None,
    })
    .unwrap();
    assert_eq!(encoded["t"], "submit_direct_request");
    assert_eq!(encoded["request"]["request_id"], REQUEST_ID);
    assert!(encoded["request"]["target"]["device_id"]
        .as_str()
        .unwrap()
        .is_empty());

    let ack = serde_json::json!({
        "t": "direct_route_ack",
        "request_id": REQUEST_ID,
        "route": "decision_receipt",
        "outcome": "target_offline"
    });
    assert_eq!(
        serde_json::from_value::<TrackedDirectServerMsg>(ack).unwrap(),
        TrackedDirectServerMsg::RouteAck {
            request_id: request.request_id,
            route: DirectRoute::DecisionReceipt,
            outcome: DirectRouteOutcome::TargetOffline,
        }
    );

    let legacy_ack = serde_json::json!({
        "t": "direct_route_ack",
        "request_id": REQUEST_ID,
        "route": "request",
        "outcome": "legacy_forwarded"
    });
    assert!(matches!(
        serde_json::from_value::<TrackedDirectServerMsg>(legacy_ack).unwrap(),
        TrackedDirectServerMsg::RouteAck {
            route: DirectRoute::Request,
            outcome: DirectRouteOutcome::LegacyForwarded,
            ..
        }
    ));
}
