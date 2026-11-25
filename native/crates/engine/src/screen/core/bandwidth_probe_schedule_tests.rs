use super::*;

const S: u64 = 1_000_000;

fn facts(now_us: u64) -> ProbeFacts {
    ProbeFacts {
        now_us,
        route: Some((1, 0, None)),
        sendable: true,
        delivery: Delivery::Realtime,
        line: LineFacts::default(),
    }
}

/// Starts the probe `facts` ask for and answers it at once.
fn measured(schedule: &mut ProbeSchedule, facts: &ProbeFacts) -> ProbeReason {
    let (id, reason) = schedule.due(facts).expect("a probe is due");
    assert!(schedule.on_result(id));
    reason
}

/// User 2026-11-25 (Shadow's „Auto-detect“ and „Restart Auto-Detection“):
/// the host measures right after the opening, after a route change, on
/// „Neu messen“ and every five minutes; not while the route stops the application
/// or the session shows a preview.
#[test]
fn probe_task_schedule_opening_route_remeasure_and_periodic() {
    let mut schedule = ProbeSchedule::default();
    // Not before the route lets the application send.
    let paused = ProbeFacts {
        sendable: false,
        ..facts(0)
    };
    assert_eq!(schedule.due(&paused), None);
    assert_eq!(schedule.due(&facts(S)), Some((1, ProbeReason::Opening)));
    assert_eq!(schedule.running(), Some(1));
    assert!(schedule.on_result(1));
    assert_eq!(schedule.running(), None);
    // The route the session opened on is no change; nothing more is due.
    assert_eq!(schedule.due(&facts(2 * S)), None);
    // A new generation, a pause or another class is a change.
    for (index, route) in [
        (2, 0, None),
        (2, 1, None),
        (2, 1, Some(crate::share::route_state::RouteClass::Relay)),
    ]
    .into_iter()
    .enumerate()
    {
        let now = (3 + index as u64) * S;
        let moved = ProbeFacts {
            route: Some(route),
            ..facts(now)
        };
        assert_eq!(measured(&mut schedule, &moved), ProbeReason::RouteChange);
    }
    // A change seen while the route stops the application waits for it.
    let changed = ProbeFacts {
        route: Some((3, 1, None)),
        sendable: false,
        ..facts(6 * S)
    };
    assert_eq!(schedule.due(&changed), None);
    let back = ProbeFacts {
        route: Some((3, 1, None)),
        ..facts(7 * S)
    };
    assert_eq!(measured(&mut schedule, &back), ProbeReason::RouteChange);
    // „Neu messen“.
    schedule.request();
    let still = ProbeFacts {
        route: Some((3, 1, None)),
        ..facts(8 * S)
    };
    assert_eq!(measured(&mut schedule, &still), ProbeReason::Remeasure);
    // Every five minutes after the last start.
    let periodic = |now| ProbeFacts {
        route: Some((3, 1, None)),
        ..facts(now)
    };
    assert_eq!(schedule.due(&periodic(8 * S + PERIODIC_US - 1)), None);
    assert_eq!(
        measured(&mut schedule, &periodic(8 * S + PERIODIC_US)),
        ProbeReason::Periodic
    );
    // „Neu messen“ before the first probe names itself.
    let mut fresh = ProbeSchedule::default();
    fresh.request();
    assert_eq!(fresh.due(&facts(0)), Some((1, ProbeReason::Remeasure)));
}

/// When the line no longer carries the measured rate: under „Echtzeit“
/// three stale resets within 10 s, under „Zuverlässig“ an oldest picture
/// unacknowledged for more than 2 s that left after the last probe started.
/// The resets of the probe's own time count for nothing.
#[test]
fn probe_task_schedule_line_triggers_follow_the_delivery() {
    let mut schedule = ProbeSchedule::default();
    let at = |now, resets, delivery| ProbeFacts {
        delivery,
        line: LineFacts {
            stale_resets: resets,
            oldest_unacked_sent_us: None,
        },
        ..facts(now)
    };
    assert_eq!(
        measured(&mut schedule, &at(0, 0, Delivery::Realtime)),
        ProbeReason::Opening
    );
    // Three resets 12 s apart are too slow; three within 10 s ask.
    assert_eq!(schedule.due(&at(S, 1, Delivery::Realtime)), None);
    assert_eq!(schedule.due(&at(7 * S, 2, Delivery::Realtime)), None);
    assert_eq!(schedule.due(&at(13 * S, 3, Delivery::Realtime)), None);
    assert_eq!(
        measured(&mut schedule, &at(16 * S, 5, Delivery::Realtime)),
        ProbeReason::StaleResets
    );
    // Resets while a probe runs (it fills the line) count for nothing.
    schedule.request();
    let (id, _) = schedule.due(&at(17 * S, 5, Delivery::Realtime)).unwrap();
    assert_eq!(
        schedule.due(&at(17 * S + 500_000, 9, Delivery::Realtime)),
        None
    );
    assert!(schedule.on_result(id));
    assert_eq!(schedule.due(&at(18 * S, 9, Delivery::Realtime)), None);
    // „Zuverlässig“ never resets for staleness; resets there ask nothing.
    assert_eq!(schedule.due(&at(19 * S, 20, Delivery::Reliable)), None);

    // Its backlog asks: older than 2 s and sent after the last start.
    let mut reliable = ProbeSchedule::default();
    let backlog = |now, sent| ProbeFacts {
        delivery: Delivery::Reliable,
        line: LineFacts {
            stale_resets: 0,
            oldest_unacked_sent_us: sent,
        },
        ..facts(now)
    };
    assert_eq!(
        measured(&mut reliable, &backlog(10 * S, None)),
        ProbeReason::Opening
    );
    // Sent before that probe, or not 2 s old yet: nothing.
    assert_eq!(reliable.due(&backlog(13 * S, Some(9 * S))), None);
    assert_eq!(reliable.due(&backlog(13 * S, Some(12 * S))), None);
    // „Echtzeit“ gives such a picture up instead (its resets count there).
    let realtime = ProbeFacts {
        delivery: Delivery::Realtime,
        ..backlog(13 * S, Some(10 * S + 1))
    };
    assert_eq!(reliable.due(&realtime), None);
    assert_eq!(
        measured(&mut reliable, &backlog(13 * S, Some(10 * S + 1))),
        ProbeReason::Backlog
    );
    // The picture that asked is older than that probe now: no second one.
    assert_eq!(reliable.due(&backlog(16 * S, Some(10 * S + 1))), None);
}

/// Never two probes at once; one without an answer within 3 s is dropped
/// and its late answer ignored; „Neu messen“ while one runs is answered by
/// it.
#[test]
fn probe_task_schedule_no_overlap_and_timeout() {
    let mut schedule = ProbeSchedule::default();
    assert_eq!(schedule.due(&facts(0)), Some((1, ProbeReason::Opening)));
    schedule.request();
    let moved = ProbeFacts {
        route: Some((2, 0, None)),
        ..facts(S)
    };
    assert_eq!(schedule.due(&moved), None);
    assert_eq!(schedule.expire(RESULT_TIMEOUT_US), None);
    assert_eq!(schedule.expire(RESULT_TIMEOUT_US + 1), Some(1));
    assert_eq!(schedule.running(), None);
    assert!(!schedule.on_result(1));
    // The route change seen meanwhile follows; the request does not.
    let next = ProbeFacts {
        route: Some((2, 0, None)),
        ..facts(RESULT_TIMEOUT_US + 2)
    };
    assert_eq!(schedule.due(&next), Some((2, ProbeReason::RouteChange)));
    assert!(!schedule.on_result(1));
    assert!(!schedule.on_result(3));
    assert!(schedule.on_result(2));
    assert_eq!(schedule.due(&next), None);
}
