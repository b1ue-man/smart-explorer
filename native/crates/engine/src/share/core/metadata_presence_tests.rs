//! Metadata presence (decision of 2026-11-14 on E4 of plan
//! `docs/plaene/2026-11-14-verbindungen-je-zweck`). Remote suite only
//! (AGENTS.md).
use super::*;
use crate::share::keepalive::yield_engagement::{hold_time, PeerHold, NAVIGATION_GAP};
use crate::share::wire::{Ctrl, FsRequest};

const MS: Duration = Duration::from_millis(1);

fn register() -> &'static MetadataPresence {
    Box::leak(Box::new(MetadataPresence::new()))
}

fn device(seed: u8) -> EndpointId {
    iroh::SecretKey::from_bytes(&[seed; 32]).public()
}

#[test]
fn verbindungen_task_metadata_presence_runs_until_the_guard_drops() {
    let register = register();
    let peer = device(1);
    let activity = register.activity(peer);
    assert!(!activity.running());
    assert_eq!(activity.last_end(), None, "nothing ended yet");
    let first = register.begin(peer, OperationOrigin::User);
    let second = register.begin(peer, OperationOrigin::User);
    assert!(first.is_some() && second.is_some());
    assert!(activity.running(), "starts when the operation is sent");
    let before_end = Instant::now();
    drop(first);
    assert!(activity.running(), "the second operation still runs");
    drop(second);
    assert!(!activity.running());
    let ended = activity.last_end().expect("the last end is recorded");
    assert!(ended >= before_end && ended <= Instant::now());
}

#[test]
fn verbindungen_task_metadata_presence_job_origin_sets_nothing() {
    let register = register();
    let peer = device(2);
    assert!(register.begin(peer, OperationOrigin::Job).is_none());
    let activity = register.activity(peer);
    assert!(!activity.running());
    assert_eq!(activity.last_end(), None);
    assert!(begin_for_node(&peer.to_string(), OperationOrigin::Job).is_none());
    assert!(begin_for_node("kein-knoten", OperationOrigin::User).is_none());
    assert_eq!(OperationOrigin::default(), OperationOrigin::Job);
}

#[test]
fn verbindungen_task_metadata_presence_is_per_device() {
    let register = register();
    let (browsed, other) = (device(3), device(4));
    let browsed_activity = register.activity(browsed);
    let other_activity = register.activity(other);
    let _browsing = register.begin(browsed, OperationOrigin::User);
    let now = Instant::now();
    let (mut to_browsed, mut to_other) = (PeerHold::default(), PeerHold::default());
    to_browsed.refresh(Some(&*browsed_activity), now, NAVIGATION_GAP);
    to_other.refresh(Some(&*other_activity), now, NAVIGATION_GAP);
    assert!(to_browsed.engaged());
    assert!(!to_other.engaged(), "another device is not browsed");
    let mut unbound = PeerHold::default();
    unbound.refresh(None, now, NAVIGATION_GAP);
    assert!(
        !unbound.engaged(),
        "a connection without a device never holds"
    );
}

#[test]
fn verbindungen_task_metadata_presence_holds_after_the_last_operation() {
    let register = register();
    let peer = device(5);
    let activity = register.activity(peer);
    drop(register.begin(peer, OperationOrigin::User));
    let ended = activity.last_end().expect("ended");
    let hold = Duration::from_secs(3);
    let mut state = PeerHold::default();
    state.refresh(Some(&*activity), ended + hold - MS, hold);
    assert!(state.engaged(), "within the hold");
    state.refresh(Some(&*activity), ended + hold + MS, hold);
    assert!(!state.engaged(), "the hold is over");
    // A new operation starts the presence again at once.
    let _again = register.begin(peer, OperationOrigin::User);
    state.refresh(Some(&*activity), ended + hold + 2 * MS, hold);
    assert!(state.engaged());
}

#[test]
fn verbindungen_task_metadata_presence_hold_latches_the_loaded_round_trip() {
    let register = register();
    let peer = device(6);
    let activity = register.activity(peer);
    drop(register.begin(peer, OperationOrigin::User));
    let ended = activity.last_end().expect("ended");
    let mut state = PeerHold::default();
    // Taken while the queue was full: 17.5 s; the drained path later asks
    // for only 3 s, which must not cut the first hold short.
    state.refresh(Some(&*activity), ended + MS, Duration::from_millis(17_500));
    state.refresh(
        Some(&*activity),
        ended + Duration::from_secs(5),
        NAVIGATION_GAP,
    );
    assert!(state.engaged());
    state.refresh(
        Some(&*activity),
        ended + Duration::from_millis(17_499),
        NAVIGATION_GAP,
    );
    assert!(state.engaged());
    state.refresh(
        Some(&*activity),
        ended + Duration::from_millis(17_501),
        NAVIGATION_GAP,
    );
    assert!(!state.engaged());
}

#[test]
fn verbindungen_task_metadata_presence_hold_time_from_the_drain() {
    let target = Duration::from_millis(25);
    // `wan` of M3: R 2.5 s, B 50 ms: ⌈log2(2500 / 75)⌉ = 6 cuts, plus one
    // round trip before the first: 7 × 2.5 s.
    assert_eq!(
        hold_time(Duration::from_millis(2_500), Some(50 * MS), target),
        Duration::from_millis(17_500)
    );
    // `lan` of M3: R 170 ms, B 2 ms: 4 × 170 ms = 680 ms, below the gap.
    assert_eq!(
        hold_time(Duration::from_millis(170), Some(2 * MS), target),
        NAVIGATION_GAP
    );
    // A queue at the target needs no cut; without a base delay yet the
    // target alone is the goal.
    assert_eq!(hold_time(30 * MS, Some(10 * MS), target), NAVIGATION_GAP);
    assert_eq!(
        hold_time(Duration::from_secs(4), None, target),
        Duration::from_secs(4 * 9)
    );
}

#[test]
fn verbindungen_task_metadata_presence_origin_on_the_wire() {
    let ctrl = Ctrl::Fs {
        req: FsRequest::Stat { path: "/a".into() },
        lease: None,
    };
    let user = fs_ctrl_payload(&ctrl, OperationOrigin::User).expect("user payload");
    let text = String::from_utf8(user.clone()).expect("JSON");
    assert!(text.contains("\"origin\":\"user\""), "{text}");
    assert_eq!(origin_of(&user), OperationOrigin::User);
    // A host before the field reads the same request.
    match serde_json::from_slice::<Ctrl>(&user).expect("older host parses") {
        Ctrl::Fs {
            req: FsRequest::Stat { path },
            lease: None,
        } => assert_eq!(path, "/a"),
        other => panic!("unexpected {other:?}"),
    }
    let job = fs_ctrl_payload(&ctrl, OperationOrigin::Job).expect("job payload");
    assert_eq!(job, serde_json::to_vec(&ctrl).expect("plain"), "unchanged");
    assert_eq!(origin_of(&job), OperationOrigin::Job);
    assert_eq!(
        origin_of(br#"{"c":"fs","origin":"spaeter"}"#),
        OperationOrigin::Job
    );
    assert_eq!(origin_of(b"kein json"), OperationOrigin::Job);
}
