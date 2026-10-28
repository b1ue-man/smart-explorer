//! Task tests of the review fixes of the batch „Verbinden ist die
//! Einrichtung“ (review B, 2026-10-28) on the Share side: steering ends by
//! time (S1), ended feeds keep nothing (B3), old GUIs read the tunnel and
//! route forms (C1), feed services tolerate new fields (C2). Prefix
//! `einrichtung_task_`.
use crate::share::access_grant::{AccessGrant, Capability, CapabilityDecision};
use crate::share::access_grant_wire::{AccessRecord, KeptParts};
use crate::share::configuration_runtime::{next_steering_end, steering_devices};
use crate::share::power::test_support::{auth_state, identity};
use crate::share::route_state::{RoutePhase, RouteStatus};
use crate::share::tcp_forward::TunnelView;
use crate::share::types::{DirectGrant, DirectGrantState};
use crate::share::ExecGrant;

fn steering_grant(valid_until: Option<i64>) -> ExecGrant {
    let mut grant = AccessGrant::default();
    grant.set(Capability::ScreenControl, CapabilityDecision::Allow);
    grant.valid_until = valid_until;
    ExecGrant {
        access: Some(AccessRecord::Known {
            grant,
            kept: KeptParts::default(),
        }),
        ..ExecGrant::default()
    }
}

/// S1: a steering grant that ends by its own `valid_until` is an event of
/// the access-expiry loop: it wakes at that instant, and the device leaves
/// the steering set then (which rotates se-managed RDP credentials).
#[test]
fn einrichtung_task_review_steering_end_is_an_event_at_its_instant() {
    let host = identity("review host").expect("host identity");
    let viewer = identity("review viewer").expect("viewer identity");
    let mut state = auth_state(&host);
    state.direct_grants.push(DirectGrant {
        device_id: viewer.device_id.clone(),
        device_name: viewer.device_name.clone(),
        public_key: viewer.public_key.clone(),
        fingerprint: viewer.fingerprint.clone(),
        node_id: viewer.node_id.clone(),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: steering_grant(Some(1_000)),
        write: true,
    });
    assert_eq!(next_steering_end(&state, 900), Some(1_000));
    assert_eq!(steering_devices(&state, 999).len(), 1);
    assert!(steering_devices(&state, 1_000).is_empty());
    assert_eq!(next_steering_end(&state, 1_000), None);
    // A grant without an end needs no wake-up of its own.
    state.direct_grants[0].exec = steering_grant(None);
    assert_eq!(next_steering_end(&state, 900), None);
    assert_eq!(steering_devices(&state, 5_000).len(), 1);
}

/// B3: once a device feed ended, its window listing is not served as
/// current (CLI and MCP ask the device itself instead).
#[test]
fn einrichtung_task_review_ended_feed_serves_no_window_list() {
    use crate::screen::core::family_protocol::{FamilyListing, FAMILY_PROTOCOL};
    use crate::share::peer_feed::PeerFeedView;
    let listing = FamilyListing {
        version: FAMILY_PROTOCOL,
        entries: Vec::new(),
        limitations: Vec::new(),
    };
    let view = |live: bool| PeerFeedView {
        device_id: "device".into(),
        public_key: "key".into(),
        live,
        families: Some(listing.clone()),
        ..PeerFeedView::default()
    };
    let families = |live: bool| {
        let snapshot = crate::daemon::ShareWorkerSnapshot {
            peer_feeds: vec![view(live)],
            ..Default::default()
        };
        crate::desktop::core::tiles::families_of(&snapshot, "device", "key")
    };
    assert!(families(false).is_none());
    assert_eq!(families(true), Some(listing.clone()));
}

/// C1: a tunnel without a fixed end and a route in the relay start phase
/// travel in forms a 0.5.188 GUI parses, and this build reads them back.
#[test]
fn einrichtung_task_review_old_guis_parse_tunnels_and_routes() {
    let mut route = RouteStatus::initial(7);
    route.phase = RoutePhase::RelayStart;
    let wire = serde_json::to_value(&route).expect("route");
    assert_eq!(wire["phase"], "relay_fallback");
    assert_eq!(wire["relay_start"], true);
    let back: RouteStatus = serde_json::from_value(wire).expect("route back");
    assert_eq!(back.phase, RoutePhase::RelayStart);
    route.phase = RoutePhase::Direct;
    let wire = serde_json::to_value(&route).expect("route");
    assert!(wire.get("relay_start").is_none());

    let tunnel = serde_json::json!({
        "id": "0123456789abcdef0123456789abcdef", "role": "listener",
        "remote_node": "n", "initiator_node": "i", "target_node": "t",
        "direction": "peer_to_host", "relation_kind": "direct", "relation_id": "r",
        "destination": {"host": {"kind": "loopback"}, "port": 8080},
        "bound_target": null, "listener": null, "started_at": 1,
        "expires_at": i64::MAX, "active_connections": 0, "socket_read_bytes": 0,
        "socket_written_bytes": 0, "closing": false, "last_error": null
    });
    let view: TunnelView = serde_json::from_value(tunnel.clone()).expect("tunnel");
    assert_eq!(view.expires_at, None);
    let written = serde_json::to_value(&view).expect("tunnel");
    assert_eq!(
        written["expires_at"],
        i64::MAX,
        "a number, as 0.5.188 reads it"
    );
    let mut null = tunnel;
    null["expires_at"] = serde_json::Value::Null;
    let view: TunnelView = serde_json::from_value(null).expect("null end");
    assert_eq!(view.expires_at, None);
}

/// C2: a service entry of a later host with a field this build does not
/// know still parses, so one new field cannot empty the whole feed.
#[test]
fn einrichtung_task_review_feed_service_tolerates_new_fields() {
    use crate::share::loopback_services::LoopbackService;
    let entry = serde_json::json!({
        "port": 8080, "scope": "loopback_only", "owner": {"kind": "system"},
        "process": "python3", "pid": 42, "uid": "1000"
    });
    let service: LoopbackService = serde_json::from_value(entry).expect("service");
    assert_eq!(service.port, 8080);
    service.validate().expect("bounded");
}
