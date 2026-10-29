//! The S1 rights of screen sessions between two real Iroh nodes (SCR-M2):
//! refusals without a right and for control with a view-only right, the
//! confirmations („Diesmal erlauben“, „Immer erlauben“, „Ablehnen“, the
//! viewer withdrawing), the end of a running session by a narrowed right and
//! at the end of its grant, and the entries of the access log.
use super::*;

#[test]
fn screen_task_permission_denied_without_right() {
    let pair = ScreenPair::new(Off, Off);
    assert_eq!(
        refused(&pair, &test_picture(SessionMode::View)),
        "permission_denied"
    );
    assert_eq!(
        refused(&pair, &test_picture(SessionMode::Probe)),
        "permission_denied"
    );
    // A device without any grant learns nothing more.
    pair.host_auth.lock().unwrap().direct_grants.clear();
    assert_eq!(
        refused(&pair, &test_picture(SessionMode::View)),
        "permission_denied"
    );
}

#[test]
fn screen_task_view_only_control_right_not_granted() {
    let pair = ScreenPair::new(Allow, Off);
    assert_eq!(
        refused(&pair, &test_picture(SessionMode::Control)),
        "right_not_granted"
    );
    let probe = pair
        .open(&test_picture(SessionMode::Probe), |_| {})
        .unwrap();
    assert_eq!(probe.hello.authorization.view, Allow);
    assert_eq!(probe.hello.authorization.control, Off);
    probe.close();
}

#[test]
fn screen_task_ask_once_session() {
    let pair = ScreenPair::new(Ask, Off);
    let answer = pair.answer_next(|service, request| {
        assert_eq!(request.capability, Capability::ScreenView);
        assert_eq!(request.device_name, "screen viewer");
        service.decide(&request.id, ApprovalDecision::Once).unwrap();
    });
    let mut pending = Vec::new();
    let session = pair
        .open(&test_picture(SessionMode::View), |capability| {
            pending.push(capability)
        })
        .unwrap();
    assert!(answer.join().unwrap());
    assert_eq!(pending, [Capability::ScreenView]);
    assert_eq!(session.hello.approval, ApprovalKind::Once);
    session.close();
    // „Diesmal erlauben“ leaves the right on ask.
    let state = pair.host_auth.lock().unwrap().clone();
    let grant = &state.direct_grants[0].exec;
    assert_eq!(grant.decision_at(Capability::ScreenView, now_secs()), Ask);
}

#[test]
fn screen_task_ask_always_sets_allow() {
    let pair = ScreenPair::new(Ask, Off);
    let auth = pair.host_auth.clone();
    // As the daemon's `Decide` does: the grant first, then the answer.
    let answer = pair.answer_next(move |service, request| {
        auth.lock().unwrap().direct_grants[0].exec = screen_grant(Allow, Off, None);
        service
            .decide(&request.id, ApprovalDecision::Always)
            .unwrap();
    });
    let first = pair.open(&test_picture(SessionMode::View), |_| {}).unwrap();
    assert!(answer.join().unwrap());
    assert_eq!(first.hello.approval, ApprovalKind::Always);
    first.close();
    let mut pending = Vec::new();
    let second = pair
        .open(&test_picture(SessionMode::View), |capability| {
            pending.push(capability)
        })
        .unwrap();
    assert!(pending.is_empty(), "allow asks nobody");
    assert_eq!(second.hello.approval, ApprovalKind::NotRequired);
    second.close();
}

#[test]
fn screen_task_ask_deny_approval_denied() {
    let pair = ScreenPair::new(Ask, Off);
    let answer = pair.answer_next(|service, request| {
        service.decide(&request.id, ApprovalDecision::Deny).unwrap();
    });
    assert_eq!(
        refused(&pair, &test_picture(SessionMode::View)),
        "approval_denied"
    );
    assert!(answer.join().unwrap());
    let registry = pair.host.0.screen_registry().clone();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !registry.is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(registry.is_empty(), "a refused session leaves no entry");
}

#[test]
fn screen_task_viewer_cancel_withdraws_approval() {
    let pair = ScreenPair::new(Ask, Off);
    *pair.host.0.screen.audit.write().unwrap() = capture;
    let link = pair.link().unwrap();
    let connection = link.connection.clone();
    let opened = pair.viewer.0.block_on(async {
        let build = |server: &_| link.client_hello(server, &test_picture(SessionMode::View));
        // The viewer gives up as soon as the host asks its user.
        let cancel = |_| connection.close(VarInt::from_u32(0), b"viewer cancelled");
        handshake(link.connection.clone(), build, cancel).await
    });
    assert!(opened.is_err());
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !pair.approvals.pending().is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        pair.approvals.pending().is_empty(),
        "the request was withdrawn"
    );
    let events: Vec<Value> = audit_of(pair.viewer_id())
        .into_iter()
        .map(|line| line["event"].clone())
        .collect();
    assert!(
        events.contains(&Value::from("approval_requested")),
        "{events:?}"
    );
    assert!(
        events.contains(&Value::from("approval_cancelled")),
        "{events:?}"
    );
}

#[test]
fn screen_task_narrowing_closes_session() {
    let pair = ScreenPair::new(Allow, Off);
    let mut session = pair.open(&test_picture(SessionMode::View), |_| {}).unwrap();
    let streaming = wait_state(
        &pair,
        &mut session,
        SessionState::Streaming,
        Duration::from_secs(10),
    );
    assert!(streaming.is_some(), "the session streams");
    // As `configuration_runtime.rs` does: restrictions under the auth lock,
    // the deny barrier, then the new state.
    {
        let mut state = pair.host_auth.lock().unwrap();
        let mut candidate = state.clone();
        candidate.direct_grants[0].exec = screen_grant(Off, Off, None);
        let restrictions = authorization_restrictions(&state, &candidate);
        assert!(!restrictions.is_empty());
        candidate.authorization_epoch += 1;
        let epoch = candidate.authorization_epoch;
        pair.host
            .0
            .invalidate_restrictions_at(&restrictions, epoch)
            .unwrap();
        *state = candidate;
    }
    let ended = wait_state(
        &pair,
        &mut session,
        SessionState::Ended,
        Duration::from_secs(10),
    );
    let reason = ended.and_then(|report| report.reason);
    assert_eq!(reason.as_deref(), Some("capability_narrowed"));
    session.close();
}

#[test]
fn screen_task_grant_expired_closes_session() {
    let end = now_secs() + 3;
    let pair = ScreenPair::with_server(fixture::NO_RELAY, Allow, Off, Some(end)).unwrap();
    *pair.host.0.screen.audit.write().unwrap() = capture;
    let mut session = pair.open(&test_picture(SessionMode::View), |_| {}).unwrap();
    // Every other host frame is kept: a session that ends for another
    // reason names its error frame in the failure message.
    let (ended, frames) = pair.viewer.0.block_on(async {
        let mut frames = Vec::new();
        let wait = async {
            loop {
                tokio::select! {
                    frame = session.host.recv() => match frame {
                        Some(Ok(HostFrame::State(report)))
                            if report.state == SessionState::Ended =>
                        {
                            return Some(report);
                        }
                        Some(Ok(HostFrame::Stats(_))) => {}
                        Some(Ok(other)) => frames.push(format!("{other:?}")),
                        Some(Err(error)) => {
                            frames.push(format!("stream: {error}"));
                            return None;
                        }
                        None => return None,
                    },
                    _ = session.pictures.recv() => {}
                }
            }
        };
        let ended = tokio::time::timeout(Duration::from_secs(20), wait)
            .await
            .ok()
            .flatten();
        (ended, frames)
    });
    let reason = ended.and_then(|report| report.reason);
    assert_eq!(reason.as_deref(), Some("grant_expired"), "{frames:#?}");
    assert!(now_secs() >= end, "not before the end of the grant");
    session.close();
    // The host logs `session_ended` right after it sent the last `STATE`;
    // the viewer may be faster than that.
    let ended_of = || -> Vec<Value> {
        audit_of(pair.viewer_id())
            .into_iter()
            .filter(|line| line["event"] == "session_ended")
            .collect()
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut ended_lines = ended_of();
    while ended_lines.is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        ended_lines = ended_of();
    }
    assert!(
        ended_lines.iter().any(|line| line["reason"]
            .as_str()
            .is_some_and(|text| text.starts_with("grant_expired"))),
        "{ended_lines:?}"
    );
}

#[test]
fn screen_task_audit_screen_entries() {
    let pair = ScreenPair::new(Allow, Off);
    *pair.host.0.screen.audit.write().unwrap() = capture;
    let _ = bench_on(&pair, 1);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut lines = audit_of(pair.viewer_id());
    while !lines.iter().any(|line| line["event"] == "session_ended")
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(50));
        lines = audit_of(pair.viewer_id());
    }
    let started = lines
        .iter()
        .find(|line| line["event"] == "session_started")
        .expect("session_started");
    assert_eq!(started["kind"], "screen");
    assert_eq!(started["mode"], "view");
    assert_eq!(started["capability"], "screen_view");
    assert_eq!(started["approval"], "not_required");
    assert_eq!(started["relation"], "direct");
    let ended = lines
        .iter()
        .find(|line| line["event"] == "session_ended")
        .expect("session_ended");
    let reason = ended["reason"].as_str().unwrap_or_default();
    assert!(reason.starts_with("viewer_closed; "), "{reason}");
    assert!(reason.ends_with(" bytes"), "{reason}");
}
