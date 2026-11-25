//! Direct retries in relay fallback (2026-11-25): schedule, triggers, end of
//! the retries and the in-place admission of a late direct path.
use super::super::super::route_diagnostics::RetryCause;
use super::super::super::route_state::{RouteClass, RoutePhase};
use super::super::{RouteCandidate, RoutePolicy};
use super::*;
use std::time::Duration;

const SAME: Candidates = Candidates { local: 1, peer: 1 };
const BASE: &str =
    "Absolute Direktfrist abgelaufen; im aktuellen Versuch kein validierter Direktpfad";

/// Arms at `now` and runs every due attempt until `until`; returns the
/// attempt times.
fn run_backoff(schedule: &mut RetrySchedule, from: u64, until: u64) -> Vec<u64> {
    let mut attempts = Vec::new();
    let mut now = from;
    while now <= until {
        if schedule.due(now) {
            schedule.started(now, SAME);
            attempts.push(now);
        }
        now += 1_000;
    }
    attempts
}

#[test]
fn route_retry_task_backoff_doubles_to_sixty_seconds_and_never_stops() {
    let mut schedule = RetrySchedule::default();
    assert!(schedule.follow(0, true, false, SAME));
    assert!(!schedule.follow(1_000, true, false, SAME), "armed once");
    assert_eq!(schedule.wait_ms(0), Some(5_000));
    assert_eq!(
        schedule.reason(BASE).as_deref(),
        Some(
            "Relay als Rückfall; neuer Direktversuch in 5 s (Versuch 1); Absolute Direktfrist \
             abgelaufen; im aktuellen Versuch kein validierter Direktpfad"
        )
    );
    assert!(!schedule.due(4_999));
    // 5, then 10, 20, 40 and every 60 s after the previous attempt.
    let attempts = run_backoff(&mut schedule, 0, 600_000);
    assert_eq!(
        attempts,
        [
            5_000, 15_000, 35_000, 75_000, 135_000, 195_000, 255_000, 315_000, 375_000, 435_000,
            495_000, 555_000
        ]
    );
    assert_eq!(schedule.attempts(), 12);
    assert_eq!(schedule.last_cause(), Some(RetryCause::Backoff));
    assert_eq!(schedule.wait_ms(555_000), Some(60_000));
    assert!(schedule
        .reason(BASE)
        .is_some_and(|reason| reason
            .starts_with("Relay als Rückfall; neuer Direktversuch in 60 s (Versuch 13)")));
}

#[test]
fn route_retry_task_address_changes_start_an_attempt_at_once() {
    let mut schedule = RetrySchedule::default();
    schedule.follow(0, true, false, SAME);
    // Unchanged candidates never bring an attempt forward.
    assert!(!schedule.observe_candidates(2_000, SAME));
    assert!(!schedule.due(2_000));
    // This device's addresses changed: the attempt starts at once.
    let local = Candidates { local: 2, peer: 1 };
    assert!(schedule.observe_candidates(2_000, local));
    assert!(schedule.due(2_000));
    assert_eq!(schedule.started(2_000, local), 1);
    assert_eq!(schedule.last_cause(), Some(RetryCause::LocalAddresses));
    // The backoff restarts at 5 s after a triggered attempt.
    assert_eq!(schedule.wait_ms(2_000), Some(FIRST_RETRY_MS));
    // The peer republished other addresses half a second later: the next
    // attempt waits for the one-second gap, not for the backoff.
    let peer = Candidates { local: 2, peer: 3 };
    assert!(schedule.observe_candidates(2_500, peer));
    assert!(!schedule.due(2_500));
    assert_eq!(schedule.wait_ms(2_500), Some(500));
    assert!(schedule.due(3_000));
    schedule.started(3_000, peer);
    assert_eq!(schedule.last_cause(), Some(RetryCause::PeerAddresses));
    assert_eq!(schedule.attempts(), 2);
    // Without further changes the backoff continues: 5 s, then 10 s.
    assert!(!schedule.observe_candidates(4_000, peer));
    assert_eq!(run_backoff(&mut schedule, 4_000, 20_000), [8_000, 18_000]);
}

#[test]
fn route_retry_task_direct_admission_ends_the_retries() {
    let mut schedule = RetrySchedule::default();
    // A server connection (no fallback this side can retry) never arms.
    assert!(!schedule.follow(0, false, false, SAME));
    assert_eq!(schedule.reason(BASE), None);
    schedule.follow(0, true, false, SAME);
    run_backoff(&mut schedule, 0, 40_000);
    assert_eq!(schedule.attempts(), 3);
    // A late direct path was admitted: no retry and no retry reason.
    assert!(schedule.follow(40_000, false, true, SAME));
    assert!(!schedule.armed());
    assert!(!schedule.due(1_000_000));
    assert_eq!(schedule.wait_ms(40_000), None);
    assert_eq!(schedule.reason(BASE), None);
    assert!(!schedule.observe_candidates(41_000, Candidates { local: 9, peer: 9 }));
    // The direct path failed after 10 s: the previous backoff resumes.
    assert!(schedule.follow(50_000, true, false, SAME));
    assert_eq!(schedule.attempts(), 3);
    assert_eq!(schedule.wait_ms(50_000), Some(40_000));
    // A direct path that held longer than a minute restarts the backoff.
    schedule.follow(60_000, false, true, SAME);
    schedule.follow(200_000, true, false, SAME);
    assert_eq!(schedule.attempts(), 0);
    assert_eq!(schedule.wait_ms(200_000), Some(FIRST_RETRY_MS));
    // A new network generation keeps nothing.
    let revision = schedule.revision();
    schedule.reset();
    assert!(!schedule.armed());
    assert!(schedule.revision() > revision);
}

fn path(key: &str, class: RouteClass, validated: bool) -> RouteCandidate {
    RouteCandidate {
        key: key.into(),
        snapshot_key: key.into(),
        class,
        validated,
        validation_pending: false,
        rtt: Some(Duration::from_millis(1)),
        preferred: false,
    }
}

#[test]
fn route_retry_task_path_of_a_late_retry_is_admitted_in_place() {
    // Collection, response and validation windows of 30 ms: the direct phase
    // ends at 90 ms.
    let mut policy = RoutePolicy::new(7, Duration::from_millis(10), Duration::from_millis(10));
    let mut schedule = RetrySchedule::default();
    let mut paths = vec![path("relay", RouteClass::Relay, true)];
    for now in [0, 30, 90] {
        assert_eq!(policy.observe(now, &paths, None), Some("relay".into()));
    }
    let status = policy.status().clone();
    assert_eq!(status.phase, RoutePhase::RelayFallback);
    assert!(status.application_allowed);
    assert!(policy.decided(90));
    assert!(schedule.follow(90, true, false, SAME));
    run_backoff(&mut schedule, 90, 300_000);
    assert!(schedule.attempts() > 4);
    // Minutes later a retry validated a direct path on the same connection,
    // one that was not among the candidates frozen in the direct phase.
    assert_eq!(policy.budget().frozen_candidates, 0);
    paths.push(path("direct", RouteClass::Direct, true));
    assert_eq!(policy.observe(300_000, &paths, None), Some("direct".into()));
    let admitted = policy.status().clone();
    assert_eq!(admitted.phase, RoutePhase::Direct);
    assert_eq!(admitted.class, Some(RouteClass::Direct));
    assert!(admitted.application_allowed);
    assert_eq!(admitted.generation, status.generation);
    // In place: the switch is no interruption of the route.
    assert_eq!(admitted.pause_epoch, status.pause_epoch);
    assert_eq!(
        admitted.reason,
        "LANversuche beendet; validierter Direktpfad"
    );
    assert!(schedule.follow(300_000, false, true, SAME));
    assert_eq!(schedule.reason(&admitted.reason), None);
    // An unvalidated direct candidate never takes the payload off the relay.
    let mut fresh = RoutePolicy::new(8, Duration::from_millis(10), Duration::from_millis(10));
    let pending = [
        path("relay", RouteClass::Relay, true),
        RouteCandidate {
            validation_pending: true,
            ..path("direct", RouteClass::Direct, false)
        },
    ];
    for now in [0, 30, 90, 300_000] {
        assert_eq!(fresh.observe(now, &pending, None), Some("relay".into()));
    }
    assert_eq!(fresh.status().phase, RoutePhase::RelayFallback);
}
