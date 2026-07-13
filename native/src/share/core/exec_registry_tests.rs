use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};

use super::*;
use crate::share::exec_types::ExecCommand;

fn principal(name: &str, node: &str) -> ExecPrincipal {
    ExecPrincipal {
        relation_kind: "direct".into(),
        relation_id: "relation".into(),
        device_id: "device".into(),
        device_name: name.into(),
        public_key: "public".into(),
        fingerprint: "fingerprint".into(),
        node_id: node.into(),
    }
}

fn start(byte: &str, command: &str) -> ExecStart {
    ExecStart {
        exec_id: ExecId::parse(byte.repeat(16)).unwrap(),
        command: ExecCommand::Shell {
            command: command.into(),
        },
        cwd: Some("/secret/cwd".into()),
        env: BTreeMap::from([("SECRET".into(), "not-persisted".into())]),
        timeout_ms: None,
        max_output_bytes: None,
    }
}

fn auth(revision: u64, epoch: u64, session: &str) -> ExecAuthorization {
    ExecAuthorization {
        policy_revision: revision,
        authorization_epoch: epoch,
        session_id: session.into(),
    }
}

fn limits(global: usize, peer: usize, history: usize, cache: usize) -> ExecRegistryLimits {
    ExecRegistryLimits {
        global_active: global,
        principal_active: peer,
        history,
        terminal_cache: cache,
    }
}

fn reserve(
    registry: &ExecRegistry,
    peer: ExecPrincipal,
    request: &ExecStart,
    authorization: ExecAuthorization,
) -> ExecReservation {
    match registry.prepare(peer, authorization, request, 10).unwrap() {
        ExecAdmission::Prepared(reservation) => reservation,
        other => panic!("unexpected admission: {other:?}"),
    }
}

fn terminal(id: &ExecId, kind: ExecTerminalKind) -> ExecTerminal {
    ExecTerminal {
        exec_id: id.clone(),
        kind,
        exit_code: None,
        signal: None,
        message: Some("private failure detail".into()),
        stdout_bytes: 3,
        stderr_bytes: 4,
        output_truncated: false,
    }
}

fn assert_error<T: std::fmt::Debug>(result: Result<T, ExecRegistryError>, want: ExecRegistryError) {
    assert_eq!(result.unwrap_err(), want);
}

#[test]
fn model_enforces_exact_authorization_deduplication_and_admission_limits() {
    let registry = ExecRegistry::new(limits(2, 1, 8, 8));
    let alice = principal("Alice", "node-a");
    let bob = principal("Bob", "node-b");
    let reduced = principal("Reduced", "node-r");
    registry.apply_authorization(&reduced, 7, 1, true).unwrap();
    registry.apply_authorization(&reduced, 7, 1, false).unwrap();
    assert_error(
        registry.apply_authorization(&reduced, 7, 1, true),
        ExecRegistryError::StaleAuthorization,
    );
    registry.apply_authorization(&alice, 4, 1, true).unwrap();
    registry.apply_authorization(&bob, 2, 2, true).unwrap();
    assert_error(
        registry.prepare(alice.clone(), auth(4, 2, ""), &start("00", "invalid"), 9),
        ExecRegistryError::InvalidAuthorization,
    );
    let mut invalid = start("00", "invalid");
    invalid.command = ExecCommand::Shell {
        command: "bad\0command".into(),
    };
    assert_error(
        registry.prepare(alice.clone(), auth(4, 2, "session"), &invalid, 9),
        ExecRegistryError::InvalidStart,
    );
    assert_error(
        registry.apply_authorization(&alice, 3, 2, true),
        ExecRegistryError::StaleAuthorization,
    );

    let first = start("01", "echo one");
    let first_reservation = reserve(&registry, alice.clone(), &first, auth(4, 2, "session-a"));
    assert!(first_reservation.cancellation.reason().is_none());
    match registry
        .prepare(alice.clone(), auth(4, 2, "new-session"), &first, 11)
        .unwrap()
    {
        ExecAdmission::AlreadyRunning(view) => {
            assert_eq!(view.state, ExecLifecycleState::Starting)
        }
        other => panic!("unexpected duplicate: {other:?}"),
    }
    let changed = start("01", "echo changed");
    assert_error(
        registry.prepare(alice.clone(), auth(4, 2, "session-a"), &changed, 11),
        ExecRegistryError::DuplicateMismatch,
    );
    assert_error(
        registry.prepare(bob.clone(), auth(2, 2, "session-b"), &first, 11),
        ExecRegistryError::DuplicateMismatch,
    );
    assert_error(
        registry.prepare(
            alice.clone(),
            auth(4, 2, "session-a"),
            &start("02", "two"),
            11,
        ),
        ExecRegistryError::PrincipalLimit,
    );
    reserve(
        &registry,
        bob,
        &start("03", "three"),
        auth(2, 2, "session-b"),
    );
    let carol = principal("Carol", "node-c");
    registry.apply_authorization(&carol, 1, 3, true).unwrap();
    assert_error(
        registry.prepare(carol, auth(1, 3, "session-c"), &start("04", "four"), 11),
        ExecRegistryError::GlobalLimit,
    );
    let changed_identity = principal("Alice", "node-replaced");
    assert_error(
        registry.prepare(
            changed_identity,
            auth(4, 3, "session-a"),
            &start("05", "five"),
            11,
        ),
        ExecRegistryError::NotAuthorized,
    );
}

#[test]
fn every_stop_reason_is_signalled_and_requires_matching_terminal_after_empty() {
    let cases = [
        (ExecCancelReason::User, ExecTerminalKind::Cancelled),
        (ExecCancelReason::Timeout, ExecTerminalKind::TimedOut),
        (ExecCancelReason::Revoked, ExecTerminalKind::Revoked),
        (
            ExecCancelReason::Disconnected,
            ExecTerminalKind::Disconnected,
        ),
        (
            ExecCancelReason::WorkerStopping,
            ExecTerminalKind::Cancelled,
        ),
    ];
    for (index, (reason, kind)) in cases.into_iter().enumerate() {
        let registry = ExecRegistry::new(limits(2, 2, 4, 4));
        let peer = principal("Peer", "node");
        registry.apply_authorization(&peer, 1, 1, true).unwrap();
        let request = start(&format!("{index:02x}"), "secret shell text");
        let reservation = reserve(&registry, peer, &request, auth(1, 1, "session"));
        registry
            .commit_start(&reservation.lease, || Ok(()))
            .unwrap();
        match reason {
            ExecCancelReason::WorkerStopping => assert_eq!(registry.cancel_all(reason), 1),
            _ => assert!(registry.cancel(&request.exec_id, reason)),
        }
        registry.cancel(&request.exec_id, ExecCancelReason::Revoked);
        assert_eq!(reservation.cancellation.reason(), Some(reason));
        assert_error(
            registry.record_terminal(
                &reservation.lease,
                terminal(&request.exec_id, kind.clone()),
                false,
                20,
            ),
            ExecRegistryError::ContainmentNotConfirmed,
        );
        assert_error(
            registry.record_terminal(
                &reservation.lease,
                terminal(&request.exec_id, ExecTerminalKind::Failed),
                true,
                20,
            ),
            ExecRegistryError::TerminalReasonMismatch,
        );
        let view = registry
            .record_terminal(
                &reservation.lease,
                terminal(&request.exec_id, kind),
                true,
                20,
            )
            .unwrap();
        assert_eq!(view.finished_at, Some(20));
        assert!(registry.active_views().is_empty());
    }
}

#[test]
fn history_and_terminal_cache_are_bounded_and_persisted_views_are_redacted() {
    let registry = ExecRegistry::new(limits(1, 1, 2, 1));
    let peer = principal("Peer", "node");
    registry.apply_authorization(&peer, 1, 1, true).unwrap();
    let mut requests = Vec::new();
    for byte in ["10", "11", "12"] {
        let request = start(byte, "token=super-secret");
        let reservation = reserve(&registry, peer.clone(), &request, auth(1, 1, "session"));
        registry
            .commit_start(&reservation.lease, || Ok(()))
            .unwrap();
        registry
            .record_terminal(
                &reservation.lease,
                terminal(&request.exec_id, ExecTerminalKind::Exited),
                true,
                30,
            )
            .unwrap();
        requests.push(request);
    }
    let history = registry.redacted_history();
    assert_eq!(history.len(), 2);
    let encoded = serde_json::to_string(&history).unwrap();
    assert!(!encoded.contains("super-secret"));
    assert!(!encoded.contains("/secret/cwd"));
    assert!(!encoded.contains("private failure detail"));
    assert!(matches!(
        registry.prepare(peer.clone(), auth(1, 1, "session"), &requests[2], 31),
        Ok(ExecAdmission::CachedTerminal(_))
    ));
    let changed = start("12", "different digest");
    assert_error(
        registry.prepare(peer.clone(), auth(1, 1, "session"), &changed, 31),
        ExecRegistryError::DuplicateMismatch,
    );
    assert!(matches!(
        registry.prepare(peer, auth(1, 1, "session"), &requests[0], 31),
        Ok(ExecAdmission::Prepared(_))
    ));
}

#[test]
fn revoke_and_launch_commit_have_two_atomic_orderings() {
    let peer = principal("Peer", "node");
    let before = ExecRegistry::new(limits(1, 1, 2, 2));
    before.apply_authorization(&peer, 1, 1, true).unwrap();
    let request = start("20", "never starts");
    let reservation = reserve(&before, peer.clone(), &request, auth(1, 1, "session"));
    before.apply_authorization(&peer, 2, 2, false).unwrap();
    let released = AtomicBool::new(false);
    assert_error(
        before.commit_start(&reservation.lease, || {
            released.store(true, Ordering::SeqCst);
            Ok(())
        }),
        ExecRegistryError::StaleAuthorization,
    );
    assert!(!released.load(Ordering::SeqCst));
    assert_eq!(
        reservation.cancellation.reason(),
        Some(ExecCancelReason::Revoked)
    );

    let after = Arc::new(ExecRegistry::new(limits(1, 1, 2, 2)));
    after.apply_authorization(&peer, 1, 1, true).unwrap();
    let reservation = reserve(
        &after,
        peer.clone(),
        &start("21", "starts"),
        auth(1, 1, "s"),
    );
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let runner_registry = after.clone();
    let runner_lease = reservation.lease.clone();
    let runner_entered = entered.clone();
    let runner_release = release.clone();
    let runner = std::thread::spawn(move || {
        runner_registry.commit_start(&runner_lease, || {
            runner_entered.wait();
            runner_release.wait();
            Ok(())
        })
    });
    entered.wait();
    let revoke_registry = after.clone();
    let revoke_peer = peer.clone();
    let revoke =
        std::thread::spawn(move || revoke_registry.apply_authorization(&revoke_peer, 2, 2, false));
    release.wait();
    runner.join().unwrap().unwrap();
    revoke.join().unwrap().unwrap();
    assert_eq!(
        reservation.cancellation.reason(),
        Some(ExecCancelReason::Revoked)
    );
}

#[test]
fn failed_platform_prepare_or_launch_releases_the_slot_without_running() {
    let registry = ExecRegistry::new(limits(1, 1, 4, 4));
    let peer = principal("Peer", "node");
    registry.apply_authorization(&peer, 1, 1, true).unwrap();
    let first = start("30", "first");
    let reservation = reserve(&registry, peer.clone(), &first, auth(1, 1, "session"));
    assert_error(
        registry.commit_start(&reservation.lease, || Err("barrier failed".into())),
        ExecRegistryError::LaunchFailed("barrier failed".into()),
    );
    assert_eq!(
        registry.active_views()[0].state,
        ExecLifecycleState::Cancelling
    );
    registry
        .fail_preparation(&reservation.lease, "platform prepare failed".into(), 40)
        .unwrap();
    let second = start("31", "second");
    let second_reservation = reserve(&registry, peer, &second, auth(1, 1, "session"));
    registry
        .fail_preparation(&second_reservation.lease, "never created".into(), 41)
        .unwrap();
    assert!(registry.active_views().is_empty());
}
