//! The relay events of the session switches (plan 2026-11-14 „Bildkette“
//! P1): `set_decoders` in `accepted` and `RELAY_RATE_CEILING`.
use super::*;

/// P1: the relay names `set_decoders_v1` in `accepted` (absent from an
/// older worker: `false`) and what became of a rate changed during the
/// session (`RELAY_RATE_CEILING`).
#[test]
fn bildkette_task_proto_relay_events_of_the_session_switches() {
    use super::super::ipc_protocol::screen::{RateCeilingState, RELAY_RATE_CEILING};
    let hello = serde_json::json!({"kind":"accepted","path":"direct",
        "hello":{"authorization":{"view":"allow","control":"allow"},
        "platform":"linux","approval":"not_required"}});
    let older = serde_json::to_vec(&hello).unwrap();
    match decode_event(RELAY_HELLO, &older).unwrap() {
        StreamEvent::Accepted { set_decoders, .. } => assert!(!set_decoders),
        other => panic!("unexpected {other:?}"),
    }
    let mut newer = hello;
    newer["set_decoders"] = serde_json::json!(true);
    match decode_event(RELAY_HELLO, &serde_json::to_vec(&newer).unwrap()).unwrap() {
        StreamEvent::Accepted { set_decoders, .. } => assert!(set_decoders),
        other => panic!("unexpected {other:?}"),
    }
    for state in [
        RateCeilingState::Automatic,
        RateCeilingState::Applied,
        RateCeilingState::UnsupportedByHost,
    ] {
        let json = crate::screen::core::wire::to_json(&state).unwrap();
        assert_eq!(json, format!("\"{}\"", state.key()).into_bytes());
        match decode_event(RELAY_RATE_CEILING, &json).unwrap() {
            StreamEvent::RateCeiling(read) => assert_eq!(read, state),
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(decode_event(RELAY_RATE_CEILING, b"\"faster\"").is_err());
    for tag in [
        RELAY_HELLO,
        RELAY_HOST,
        RELAY_RECORD,
        RELAY_BROKEN,
        RELAY_CLOCK,
        RELAY_END,
    ] {
        assert_ne!(tag, RELAY_RATE_CEILING);
    }
    // A live change: applied for a host with `rate_ceiling_v1`, the
    // host's own regulation for an old one, automatic for 0.
    assert_eq!(
        RateCeilingState::of(Some(8_000_000), true),
        RateCeilingState::Applied
    );
    assert_eq!(
        RateCeilingState::of(Some(8_000_000), false),
        RateCeilingState::UnsupportedByHost
    );
    assert_eq!(
        RateCeilingState::of(None, false),
        RateCeilingState::Automatic
    );
}
