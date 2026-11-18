//! Finding 4 of the measurement of 2026-11-18 (0.5.200): a locked host
//! pauses its capture with a state the viewer shows, logs only changes and
//! spaces its rebuilds.
use super::*;

/// The lock is found by the session flag even while the input desktop
/// reads as the user's; the order and the laziness of the questions stay;
/// a pause is logged once when it begins and once when it ends; failing
/// rebuilds wait 1 s, 2 s, 4 s, then 5 s and are logged once per streak.
#[test]
fn rollout_task_lock_pauses_and_rebuilds_back_off() {
    // Locked: `HostLocked`, the desktop is not asked.
    let answers = |active: bool, locked: bool, secure: bool| {
        let mut asked = Vec::new();
        let state = pause_state(|question| {
            asked.push(question);
            match question {
                PauseQuestion::SessionActive => active,
                PauseQuestion::SessionLocked => locked,
                PauseQuestion::SecureDesktop => secure,
            }
        });
        (state, asked)
    };
    use PauseQuestion::*;
    // The lock flag wins although the desktop reads as the user's.
    assert_eq!(
        answers(true, true, false),
        (
            Some(SessionState::HostLocked),
            vec![SessionActive, SessionLocked]
        )
    );
    assert_eq!(
        answers(false, true, true),
        (Some(SessionState::SessionInactive), vec![SessionActive])
    );
    assert_eq!(
        answers(true, false, true),
        (
            Some(SessionState::SecureDesktop),
            vec![SessionActive, SessionLocked, SecureDesktop]
        )
    );
    assert_eq!(answers(true, false, false).0, None);
    // Every pause opens the capture anew when it ends (picture without a
    // new connection).
    for state in [
        SessionState::HostLocked,
        SessionState::SecureDesktop,
        SessionState::SessionInactive,
    ] {
        assert!(renews_after(state));
    }
    assert!(!renews_after(SessionState::WindowMinimized));
    // `WTSINFOEX_LEVEL1_W::SessionFlags`.
    assert_eq!(session_lock_flag(1, 0), Some(true));
    assert_eq!(session_lock_flag(1, 1), Some(false));
    assert_eq!(session_lock_flag(1, u32::MAX), None);
    assert_eq!(session_lock_flag(2, 0), None);
    // One line when the pause begins, none while it lasts, one at its end.
    let mut log = PauseLog::new();
    assert_eq!(log.line(None), None);
    let begun = log.line(Some(SessionState::HostLocked));
    assert_eq!(
        begun.as_deref(),
        Some("screen capture: paused (host_locked)")
    );
    for _ in 0..50 {
        assert_eq!(log.line(Some(SessionState::HostLocked)), None);
    }
    let ended = log.line(None);
    assert_eq!(
        ended.as_deref(),
        Some("screen capture: pause (host_locked) ended")
    );
    // Backoff of the rebuilds.
    let waits: Vec<u64> = (1..=6)
        .map(|failures| RebuildBackoff::wait_after(failures).as_secs())
        .collect();
    assert_eq!(waits, [1, 2, 4, 5, 5, 5]);
    let start = Instant::now();
    let mut backoff = RebuildBackoff::default();
    assert!(backoff.due(start));
    assert!(backoff.failed(start), "the first failure is logged");
    assert!(!backoff.due(start + Duration::from_millis(200)));
    assert!(backoff.due(start + BACKOFF_FIRST));
    assert!(!backoff.failed(start + BACKOFF_FIRST), "later ones are not");
    assert!(!backoff.due(start + BACKOFF_FIRST + Duration::from_secs(1)));
    assert_eq!(backoff.succeeded(), 2);
    assert!(backoff.due(start));
    assert_eq!(backoff.succeeded(), 0);
}
