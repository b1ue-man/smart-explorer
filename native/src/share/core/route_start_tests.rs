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
    assert!(start.choose(&only_relay, open).is_none());
    let fallback = start.choose(&only_relay, done).map(|choice| choice.phase);
    assert_eq!(fallback, Some(RoutePhase::RelayFallback));
}
