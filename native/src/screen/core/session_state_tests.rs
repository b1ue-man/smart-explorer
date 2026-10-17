use super::*;

const ALL: [SessionState; 12] = [
    SessionState::Connecting,
    SessionState::WaitingForApproval,
    SessionState::WaitingForPortal,
    SessionState::Starting,
    SessionState::Streaming,
    SessionState::SecureDesktop,
    SessionState::WindowMinimized,
    SessionState::HostLocked,
    SessionState::SessionInactive,
    SessionState::NoDisplay,
    SessionState::Ended,
    SessionState::Unknown,
];

#[test]
fn screen_task_session_state_moves_and_keyframe_requests() {
    use SessionState::*;

    // A session asked under `Ask`, paused by the secure desktop, waiting for
    // a new portal dialog for control, then ended.
    let mut session = SessionTracker::new(0);
    let path = [
        WaitingForApproval,
        Starting,
        Streaming,
        SecureDesktop,
        Streaming,
        WaitingForPortal,
        Streaming,
        Ended,
    ];
    for (at, state) in path.into_iter().enumerate() {
        session.move_to(state, at as u64 + 1).unwrap();
    }
    assert_eq!((session.state(), session.since_us()), (Ended, 8));
    assert!(
        session.move_to(Streaming, 9).is_err(),
        "nothing leaves ended"
    );
    assert_eq!(session.state(), Ended);

    // Pictures only after starting; a repeated state keeps its time.
    let mut fresh = SessionTracker::new(5);
    assert!(fresh.move_to(Streaming, 6).is_err());
    assert!(fresh.move_to(SecureDesktop, 6).is_err());
    fresh.move_to(Connecting, 7).unwrap();
    assert_eq!(fresh.since_us(), 5);
    fresh.move_to(NoDisplay, 8).unwrap();
    assert!(fresh.move_to(Streaming, 9).is_err());
    fresh.move_to(Starting, 10).unwrap();

    // Every state may end; nothing moves into `Unknown`; after an unknown
    // state of a newer host anything may follow.
    for state in ALL {
        assert_eq!(state.can_move_to(Ended), state != Ended, "{state:?}");
        assert!(!state.can_move_to(Unknown));
        assert_eq!(
            serde_json::to_string(&state).unwrap(),
            format!("\"{}\"", state.key())
        );
    }
    assert!(Unknown.can_move_to(Streaming));
    assert_eq!(
        serde_json::from_str::<SessionState>("\"future_state\"").unwrap(),
        Unknown
    );
    let paused: Vec<SessionState> = ALL.into_iter().filter(|state| state.is_paused()).collect();
    assert_eq!(
        paused,
        [SecureDesktop, WindowMinimized, HostLocked, SessionInactive]
    );
    assert!(Ended.is_terminal() && !Streaming.is_terminal());
    assert_eq!(EndReason::CapabilityNarrowed.key(), "capability_narrowed");

    // At most one open key-frame request; a request may repeat after 1 s.
    let mut requests = KeyframeRequester::default();
    assert!(requests.should_request(0));
    assert!(requests.is_open());
    assert!(!requests.should_request(KEYFRAME_RETRY_US - 1));
    assert!(requests.should_request(KEYFRAME_RETRY_US));
    requests.on_keyframe();
    assert!(!requests.is_open());
    assert!(requests.should_request(KEYFRAME_RETRY_US + 1));
}
