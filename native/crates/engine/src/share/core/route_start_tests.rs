//! Start bridge decisions (Z-R2) on the platform-independent route policy.
use super::super::RoutePolicy;
use super::*;

fn path(key: &str, class: RouteClass, validated: bool) -> RouteCandidate {
    RouteCandidate {
        key: key.into(),
        snapshot_key: key.into(),
        class,
        validated,
        validation_pending: !validated,
        rtt: Some(Duration::from_millis(1)),
        preferred: false,
    }
}

/// Collection, response and validation windows of 30 ms each: direct phase
/// ends at 90 ms without NAT round facts.
fn policy() -> RoutePolicy {
    RoutePolicy::new(3, Duration::from_millis(10), Duration::from_millis(10))
}

fn relay() -> Option<String> {
    Some("relay".into())
}

#[test]
fn route_start_task_relay_carries_payload_before_direct_admission() {
    let mut policy = policy();
    let paths = [
        path("lan", RouteClass::Lan, false),
        path("direct", RouteClass::Direct, false),
        path("relay", RouteClass::Relay, true),
    ];
    // First observation, still collecting candidates: payload may start.
    assert_eq!(policy.observe(0, &paths, None), relay());
    let status = policy.status();
    assert!(status.application_allowed);
    assert_eq!(status.phase, RoutePhase::RelayStart);
    assert_eq!(status.class, Some(RouteClass::Relay));
    assert_eq!(status.reason, RELAY_START_REASON);
    assert_eq!(status.label(), "über Relay, Direktweg wird aufgebaut");
    assert_eq!(
        progress_hint(status),
        Some("über Relay, Direktweg wird aufgebaut")
    );
}

#[test]
fn route_start_task_moves_to_direct_in_place_and_closes_the_bridge() {
    let mut policy = policy();
    let mut paths = vec![
        path("direct", RouteClass::Direct, false),
        path("relay", RouteClass::Relay, true),
    ];
    assert_eq!(policy.observe(0, &paths, None), relay());
    paths[0].validated = true;
    paths[0].validation_pending = false;
    assert_eq!(policy.observe(5, &paths, None), Some("direct".into()));
    let status = policy.status().clone();
    assert_eq!(status.phase, RoutePhase::Direct);
    assert!(status.application_allowed);
    // A switch of the admitted path is no interruption of the policy state.
    assert_eq!(status.pause_epoch, 0);
    assert_eq!(progress_hint(&status), Some("direkt"));
    // The direct path needs a fresh proof before the direct phase ended:
    // no new payload over the relay, the route pauses instead.
    paths[0].validated = false;
    paths[0].validation_pending = true;
    assert_eq!(policy.observe(40, &paths, None), None);
    assert_eq!(policy.status().phase, RoutePhase::Paused);
    assert_eq!(policy.status().pause_epoch, 1);
    paths[0].validated = true;
    paths[0].validation_pending = false;
    assert_eq!(policy.observe(50, &paths, None), Some("direct".into()));
    // After the finite direct phase, the relay is the marked fallback again.
    paths[0].validated = false;
    assert_eq!(policy.observe(90, &paths, None), relay());
    assert_eq!(policy.status().phase, RoutePhase::RelayFallback);
    assert_eq!(
        progress_hint(policy.status()),
        Some("über Relay: die Bandbreite begrenzt der Relay-Server")
    );
}

#[test]
fn route_start_task_direct_before_lan_end_then_lan_upgrade() {
    let mut policy = policy();
    let mut paths = vec![
        path("lan", RouteClass::Lan, false),
        path("direct", RouteClass::Direct, true),
        path("relay", RouteClass::Relay, true),
    ];
    // A validated direct path is used at once, before the LAN checks end.
    assert_eq!(policy.observe(0, &paths, None), Some("direct".into()));
    assert_eq!(policy.status().phase, RoutePhase::Direct);
    assert_eq!(
        policy.status().reason,
        "Validierter Direktpfad; LANkandidaten werden weiter geprüft"
    );
    paths[0].validated = true;
    assert_eq!(policy.observe(10, &paths, None), Some("lan".into()));
    assert_eq!(policy.status().phase, RoutePhase::Lan);
    assert_eq!(progress_hint(policy.status()), Some("direkt (LAN/WLAN)"));
    assert_eq!(policy.status().pause_epoch, 0);
}

#[test]
fn route_start_task_relay_only_peer_keeps_one_uninterrupted_route() {
    let mut policy = policy();
    let paths = [path("relay", RouteClass::Relay, true)];
    let mut revisions = Vec::new();
    for now in [0, 10, 29, 30, 60, 89] {
        assert_eq!(policy.observe(now, &paths, None), relay());
        assert_eq!(policy.status().phase, RoutePhase::RelayStart);
        revisions.push(policy.status().revision);
    }
    // Only the first observation changed the state.
    assert!(revisions.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(policy.observe(90, &paths, None), relay());
    let status = policy.status();
    assert_eq!(status.phase, RoutePhase::RelayFallback);
    assert!(status.application_allowed);
    assert_eq!(status.class, Some(RouteClass::Relay));
    assert_eq!(status.pause_epoch, 0, "the fallback mark never pauses");
    assert_eq!(status.label(), "über Relay (Rückfall)");
    assert_eq!(policy.budget().direct_deadline_ms, 90);
    assert_eq!(policy.observe(5_000, &paths, None), relay());
    assert_eq!(policy.status().phase, RoutePhase::RelayFallback);
}

#[test]
fn route_start_task_refused_first_direct_grant_keeps_the_bridge() {
    let mut policy = policy();
    let mut paths = vec![
        path("direct", RouteClass::Direct, true),
        path("relay", RouteClass::Relay, true),
    ];
    assert_eq!(policy.observe(0, &paths, None), Some("direct".into()));
    // The QUIC gate refused the stale direct observation.
    policy.gate_rejected();
    assert_eq!(policy.status().phase, RoutePhase::Paused);
    paths[0].validated = false;
    paths[0].validation_pending = true;
    assert_eq!(policy.observe(5, &paths, None), relay());
    assert_eq!(policy.status().phase, RoutePhase::RelayStart);
}

#[test]
fn route_start_task_new_generation_opens_a_new_bridge() {
    let mut old = policy();
    let mut paths = vec![
        path("direct", RouteClass::Direct, true),
        path("relay", RouteClass::Relay, true),
    ];
    assert_eq!(old.observe(0, &paths, None), Some("direct".into()));
    // A confirmed network change starts a new generation and policy.
    let mut next = RoutePolicy::new(4, Duration::from_millis(10), Duration::from_millis(10));
    next.resuming();
    paths[0].validated = false;
    paths[0].validation_pending = true;
    assert_eq!(next.observe(0, &paths, None), relay());
    assert_eq!(next.status().phase, RoutePhase::RelayStart);
    assert_eq!(next.status().generation, 4);
}

#[test]
fn route_start_task_hints_and_log_lines() {
    let mut status = RouteStatus::initial(1);
    assert_eq!(progress_hint(&status), None);
    status.application_allowed = true;
    status.class = Some(RouteClass::Relay);
    status.phase = RoutePhase::RelayStart;
    assert_eq!(Mark::of(&status), Some(Mark::RelayStart));
    status.phase = RoutePhase::Paused;
    status.application_allowed = false;
    assert_eq!(Mark::of(&status), None);
    assert_eq!(progress_hint(&status), None);
    assert_eq!(
        route_line("B", Mark::RelayStart, None, 0),
        "Route zu B: über Relay, Direktweg wird aufgebaut"
    );
    let switched = Some(Duration::from_millis(420));
    assert_eq!(
        route_line("B", Mark::Direct, switched, 3 * 1024 * 1024),
        "Route zu B: direkt nach 420 ms über Relay (3072 KiB Nutzdaten); neue Daten nur noch direkt"
    );
    assert_eq!(
        route_line("B", Mark::Lan, None, 0),
        "Route zu B: direkt (LAN/WLAN)"
    );
    let fallback = route_line("B", Mark::RelayFallback, None, 0);
    assert!(fallback.contains("Rückfall"));
}

#[test]
fn route_start_task_bridge_state_machine() {
    let open = Progress {
        lan_done: false,
        direct_done: false,
        expired: false,
    };
    let done = Progress {
        lan_done: true,
        direct_done: true,
        expired: true,
    };
    let mut start = StartPhase::default();
    assert!(start.bridge_open(open));
    assert!(!start.bridge_open(done));
    let only_relay = [path("relay", RouteClass::Relay, true)];
    let chosen = start.choose(&only_relay, open).map(|choice| choice.phase);
    assert_eq!(chosen, Some(RoutePhase::RelayStart));
    let direct = [path("direct", RouteClass::Direct, true)];
    assert!(start.choose(&direct, open).is_some());
    assert!(!start.bridge_open(open));
    // r-weg-2: no LAN/direct path of the connection is validated or being
    // validated any more, so the relay carries payload as the marked fallback
    // before the direct phase ends.
    let lost = start
        .choose(&only_relay, open)
        .map(|choice| (choice.phase, choice.reason));
    assert_eq!(lost, Some((RoutePhase::RelayFallback, DIRECT_LOST_REASON)));
    let fallback = start.choose(&only_relay, done).map(|choice| choice.phase);
    assert_eq!(fallback, Some(RoutePhase::RelayFallback));
    // A direct path that only re-proves itself keeps the bridge closed: the
    // relay does not carry payload while the proof is pending.
    let mut proving = StartPhase::default();
    assert!(proving.choose(&direct, open).is_some());
    let pending = [
        path("direct", RouteClass::Direct, false),
        path("relay", RouteClass::Relay, true),
    ];
    assert!(pending[0].validation_pending);
    assert!(proving.choose(&pending, open).is_none());
}

/// A direct path that is neither validated nor being validated.
fn vanished(key: &str) -> RouteCandidate {
    let mut vanished = path(key, RouteClass::Direct, false);
    vanished.validation_pending = false;
    vanished
}

#[test]
fn route_start_task_lost_direct_path_falls_back_to_relay_until_a_direct_path_returns() {
    // E5: the admitted direct path is no longer validated or pending.
    let mut policy = policy();
    let mut paths = vec![
        path("direct", RouteClass::Direct, true),
        path("relay", RouteClass::Relay, true),
    ];
    assert_eq!(policy.observe(0, &paths, None), Some("direct".into()));
    paths[0] = vanished("direct");
    assert_eq!(policy.observe(10, &paths, None), relay());
    let status = policy.status().clone();
    assert_eq!(status.phase, RoutePhase::RelayFallback);
    assert_eq!(status.class, Some(RouteClass::Relay));
    assert!(status.application_allowed);
    assert_eq!(status.reason, DIRECT_LOST_REASON);
    assert_eq!(status.pause_epoch, 0, "the fallback never pauses payload");
    assert_eq!(
        progress_hint(&status),
        Some("über Relay: die Bandbreite begrenzt der Relay-Server")
    );
    // E6: a direct path that is being validated again does not take the
    // relay away from the transfer; only a validated one moves it back.
    paths[0].validation_pending = true;
    assert_eq!(policy.observe(20, &paths, None), relay());
    assert_eq!(policy.status().phase, RoutePhase::RelayFallback);
    assert!(policy.status().application_allowed);
    paths[0].validated = true;
    paths[0].validation_pending = false;
    assert_eq!(policy.observe(30, &paths, None), Some("direct".into()));
    assert_eq!(policy.status().phase, RoutePhase::Direct);
    assert_eq!(policy.status().pause_epoch, 0);
    // E5, second form: the direct path is gone from the connection's paths.
    let mut policy = policy_after_direct();
    assert_eq!(
        policy.observe(10, &[path("relay", RouteClass::Relay, true)], None),
        relay()
    );
    assert_eq!(policy.status().phase, RoutePhase::RelayFallback);
    assert_eq!(policy.status().reason, DIRECT_LOST_REASON);
    assert!(policy.status().application_allowed);
    assert_eq!(policy.status().pause_epoch, 0);
}

/// A policy whose first observation chose a validated direct path.
fn policy_after_direct() -> RoutePolicy {
    let mut policy = policy();
    let paths = [
        path("direct", RouteClass::Direct, true),
        path("relay", RouteClass::Relay, true),
    ];
    assert_eq!(policy.observe(0, &paths, None), Some("direct".into()));
    policy
}

#[test]
fn route_start_task_refused_relay_fallback_pauses_until_the_direct_path_proves_itself() {
    // E8: the gate refuses the relay grant of the fallback; the relay is not
    // allowed again while the direct path is merely being validated.
    let mut policy = policy_after_direct();
    let mut paths = vec![vanished("direct"), path("relay", RouteClass::Relay, true)];
    assert_eq!(policy.observe(10, &paths, None), relay());
    policy.gate_rejected();
    assert_eq!(policy.status().phase, RoutePhase::Paused);
    assert!(!policy.status().application_allowed);
    assert_eq!(policy.status().pause_epoch, 1);
    paths[0].validation_pending = true;
    assert_eq!(policy.observe(15, &paths, None), None);
    assert_eq!(policy.status().phase, RoutePhase::Paused);
    assert!(!policy.status().application_allowed);
    assert_eq!(policy.status().pause_epoch, 1, "one pause, one epoch");
    paths[0].validated = true;
    paths[0].validation_pending = false;
    assert_eq!(policy.observe(16, &paths, None), Some("direct".into()));
    assert_eq!(policy.status().phase, RoutePhase::Direct);
}

#[test]
fn route_start_task_validated_lan_or_direct_path_never_yields_to_relay() {
    // E9: for every state of the LAN and direct paths, at every time of the
    // finite direct phase, and with or without an earlier loss, a validated
    // LAN or direct path is the choice, never the relay.
    #[derive(Clone, Copy)]
    enum State {
        Absent,
        Pending,
        Validated,
    }
    let make = |key: &str, class, state| match state {
        State::Absent => None,
        State::Pending => Some(path(key, class, false)),
        State::Validated => Some(path(key, class, true)),
    };
    let states = [State::Absent, State::Pending, State::Validated];
    for earlier_loss in [false, true] {
        for lan in states {
            for direct in states {
                for now in [0u64, 10, 29, 30, 60, 89, 90, 500] {
                    let mut policy = policy();
                    if earlier_loss {
                        assert_eq!(
                            policy.observe(
                                0,
                                &[
                                    path("direct", RouteClass::Direct, true),
                                    path("relay", RouteClass::Relay, true)
                                ],
                                None
                            ),
                            Some("direct".into())
                        );
                        assert_eq!(
                            policy.observe(1, &[path("relay", RouteClass::Relay, true)], None),
                            relay()
                        );
                    }
                    let mut paths: Vec<RouteCandidate> = [
                        make("lan", RouteClass::Lan, lan),
                        make("direct", RouteClass::Direct, direct),
                    ]
                    .into_iter()
                    .flatten()
                    .collect();
                    paths.push(path("relay", RouteClass::Relay, true));
                    let chosen = policy.observe(now.max(1), &paths, None);
                    let validated =
                        matches!(lan, State::Validated) || matches!(direct, State::Validated);
                    if validated {
                        assert_ne!(chosen, relay(), "loss {earlier_loss}, now {now}");
                        let expected = if matches!(lan, State::Validated) {
                            "lan"
                        } else {
                            "direct"
                        };
                        assert_eq!(chosen, Some(expected.into()));
                        assert_ne!(policy.status().class, Some(RouteClass::Relay));
                    }
                }
            }
        }
    }
}
