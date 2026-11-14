//! Candidate-cap tests of `route_policy.rs` (p-fix-b, M17, finding r-weg U2):
//! LAN paths are never refused, refused candidates are counted, and a refused
//! round candidate keeps the initial round open.
use super::super::route_state::{RouteClass, RoutePhase};
use super::{RoundFacts, RouteCandidate, RoutePolicy};
use std::time::Duration;

fn candidate(key: &str, class: RouteClass) -> RouteCandidate {
    RouteCandidate {
        key: key.into(),
        snapshot_key: key.into(),
        class,
        validated: false,
        validation_pending: false,
        rtt: None,
        preferred: false,
    }
}

/// Collection window of 3 x 10 ms = 30 ms.
fn new_policy() -> RoutePolicy {
    RoutePolicy::new(1, Duration::from_millis(10), Duration::from_millis(10))
}

#[test]
fn unbelegt_task_route_cap_counts_drops() {
    let mut paths = vec![candidate("lan", RouteClass::Lan)];
    paths.extend((0..128usize).map(|i| candidate(&format!("d{i:03}"), RouteClass::Direct)));
    let mut policy = new_policy();
    policy.observe(0, &paths, None);
    assert!(policy.lan_candidates.contains("lan"));
    assert_eq!(policy.budget().dropped_candidates, 1);
}

#[test]
fn unbelegt_task_route_cap_keeps_lan_phase() {
    let mut paths: Vec<RouteCandidate> = (0..200usize)
        .map(|i| candidate(&format!("d{i:03}"), RouteClass::Direct))
        .collect();
    paths.push(candidate("lan", RouteClass::Lan));
    let mut policy = new_policy();
    policy.observe(0, &paths, None);
    policy.observe(30, &paths, None);
    assert!(policy.lan_candidates.contains("lan"));
    assert!(matches!(policy.status().phase, RoutePhase::CheckingLan));
}

#[test]
fn unbelegt_task_route_cap_keeps_round_open() {
    let round = RoundFacts {
        round: "initial".into(),
        candidates: (0..129usize).map(|i| format!("r{i:03}")).collect(),
        exhausted: true,
        response_deadline_ms: Some(0),
        pending_path_opens: 0,
        retry_intervals: Vec::new(),
        max_probe_attempts: 1,
    };
    let mut policy = new_policy();
    policy.observe(0, &[], Some(&round));
    policy.observe(60, &[], Some(&round));
    assert_eq!(policy.budget().dropped_candidates, 1);
    assert!(!policy.round_completed(60, Some(&round)));
    assert!(matches!(policy.status().phase, RoutePhase::CheckingDirect));
}

#[test]
fn unbelegt_task_route_cap_counts_each_refused_candidate_once() {
    // 128 direct paths fill the cap. One more remote arrives as two open paths,
    // as a hint and as a round candidate, all with its one snapshot key.
    let mut paths: Vec<RouteCandidate> = (0..128usize)
        .map(|i| candidate(&format!("candidate/d{i:03}"), RouteClass::Direct))
        .collect();
    for key in ["path/1", "path/2", "candidate/late"] {
        let mut late = candidate(key, RouteClass::Direct);
        late.snapshot_key = "candidate/late".into();
        paths.push(late);
    }
    let round = RoundFacts {
        round: "initial".into(),
        candidates: vec!["candidate/late".into()],
        exhausted: false,
        response_deadline_ms: None,
        pending_path_opens: 0,
        retry_intervals: Vec::new(),
        max_probe_attempts: 1,
    };
    let mut policy = new_policy();
    policy.observe(0, &paths, Some(&round));
    assert_eq!(policy.budget().dropped_candidates, 1);
    assert!(policy.round_refused);
}
