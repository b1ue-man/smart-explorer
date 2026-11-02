use std::time::{Duration, Instant};

use super::*;

#[test]
fn unique_external_error_flood_is_fixed_memory_and_coalesced() {
    let reporter = ConnectionEventReporter::default();
    let (events, received) = channel();
    let now = Instant::now();

    let kinds = [
        ConnectionErrorKind::Accept,
        ConnectionErrorKind::FsConnection,
        ConnectionErrorKind::ExecConnection,
        ConnectionErrorKind::FsStream,
    ];
    for kind in kinds {
        for index in 0..2_500 {
            reporter.report_at(
                kind,
                &format!("attacker-controlled-{index}-{}", "x".repeat(1_024)),
                &events,
                now,
            );
        }
    }

    assert_eq!(received.len(), ConnectionErrorKind::COUNT);
    let buckets = reporter.buckets.lock().unwrap();
    assert_eq!(buckets.len(), ConnectionErrorKind::COUNT);
    for bucket in buckets.iter() {
        assert_eq!(bucket.pending, 2_499);
        assert!(bucket.latest_detail.len() <= MAX_DETAIL_BYTES);
    }
}

#[test]
fn full_event_queue_keeps_a_bounded_summary_until_delivery_recovers() {
    let reporter = ConnectionEventReporter::default();
    let (events, received) = channel();
    let now = Instant::now();
    for index in 0..SHARE_EVENT_CAPACITY {
        events
            .try_send(ShareEvent::Status(format!("queued-{index}")))
            .unwrap();
    }

    reporter.report_at(
        ConnectionErrorKind::FsConnection,
        "first rejected handshake",
        &events,
        now,
    );
    assert_eq!(received.len(), SHARE_EVENT_CAPACITY);

    received.recv().unwrap();
    reporter.report_at(
        ConnectionErrorKind::FsConnection,
        "retry after drain",
        &events,
        now,
    );
    assert_eq!(received.len(), SHARE_EVENT_CAPACITY);
    assert!(received.try_iter().any(|event| matches!(
        event,
        ShareEvent::Error(message)
            if message.contains("retry after drain")
                && message.contains("1 weitere gleichartige Fehler")
    )));
}

#[test]
fn report_window_emits_a_summary_and_leaves_room_for_recovery_events() {
    let reporter = ConnectionEventReporter::default();
    let (events, received) = channel();
    let now = Instant::now();
    reporter.report_at(ConnectionErrorKind::Accept, "first", &events, now);
    for index in 0..100 {
        reporter.report_at(
            ConnectionErrorKind::Accept,
            &format!("suppressed-{index}"),
            &events,
            now + Duration::from_secs(1),
        );
    }

    events.try_send(ShareEvent::ServerConnected).unwrap();
    reporter.report_at(
        ConnectionErrorKind::Accept,
        "after-window",
        &events,
        now + REPORT_INTERVAL,
    );

    let events: Vec<_> = received.try_iter().collect();
    assert_eq!(events.len(), 3);
    assert!(events
        .iter()
        .any(|event| matches!(event, ShareEvent::ServerConnected)));
    assert!(events.iter().any(|event| matches!(
        event,
        ShareEvent::Error(message)
            if message.contains("after-window")
                && message.contains("100 weitere gleichartige Fehler")
    )));
}
