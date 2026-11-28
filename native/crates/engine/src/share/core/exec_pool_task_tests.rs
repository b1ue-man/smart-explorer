//! B3 of plan `docs/plaene/2026-11-14-verbindungen-je-zweck` (E10, D4):
//! `share-exec/3` <-> `/2` negotiation with per-peer `/2` memory, connection
//! pooling with stream multiplexing and retry-once-before-first-byte, per-job
//! authorization and host connection closure on revocation, and 60 s
//! `ControlPersist` idle expiry. Remote suite only (`AGENTS.md`).
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iroh::endpoint::VarInt;

use super::pool::ExecPoolKey;
use super::test_fixture::{
    argv_start, enabled_grant, make_identity, run_to_terminal, start_client_node, ExecLoopbackHost,
};
use crate::share::access_grant::{AccessGrantChange, Capability, CapabilityDecision};
use crate::share::core::now_secs;
use crate::share::exec_client::{ExecClientEvent, ExecClientFailureKind};
use crate::share::exec_client_messages::{PERMISSION_DENIED, RIGHT_NOT_GRANTED};
use crate::share::exec_grant_runtime::{apply_exact_with_access, mutate};
use crate::share::exec_job::test_runner::{TestExecPlan, TestExecRunner};
use crate::share::exec_policy::ExecGrant;
use crate::share::exec_protocol::{EXEC_ALPN, EXEC_ALPN_V3};
use crate::share::exec_terminal::{ExecTerminalRequest, ExecTerminalSize};
use crate::share::exec_types::{ExecCommand, ExecId, ExecPrincipal, ExecStart, ExecTerminalKind};
use crate::share::keepalive::transport_purpose::overrides::{hold, ProfileOverrides};
use crate::share::types::ExecGrantTarget;

fn terminal_start() -> ExecStart {
    ExecStart {
        exec_id: ExecId::generate().expect("exec id"),
        command: ExecCommand::Login,
        cwd: None,
        env: BTreeMap::new(),
        timeout_ms: Some(15_000),
        max_output_bytes: None,
        terminal: Some(ExecTerminalRequest {
            term: "xterm-256color".into(),
            size: ExecTerminalSize { cols: 80, rows: 24 },
        }),
        shell: None,
        share_cwd: None,
        elevated: false,
    }
}

fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    predicate()
}

#[test]
fn verbindungen_task_exec_negotiates_v3_and_remembers_v2_peers() {
    let client_id = make_identity("b3-neg-client").expect("client identity");
    let legacy_host = {
        let _legacy = hold(ProfileOverrides {
            legacy_exec_host: true,
            ..ProfileOverrides::default()
        });
        ExecLoopbackHost::start_for_client(&client_id, enabled_grant()).expect("legacy host")
    };
    let _plan = hold(ProfileOverrides::default());
    let modern_host =
        ExecLoopbackHost::start_for_client(&client_id, enabled_grant()).expect("modern host");
    let client = start_client_node(&client_id).expect("client node");

    let modern_key = ExecPoolKey::of(&client_id, &modern_host.endpoint);
    let (_, term) = run_to_terminal(
        &client.0,
        &modern_host.endpoint,
        &client_id,
        argv_start("true").expect("start"),
    )
    .expect("modern v3 job");
    assert_eq!(term.kind, ExecTerminalKind::Exited);
    let pooled = client
        .0
        .exec_pool
        .pooled_connection(&modern_key)
        .expect("modern host connection pooled");
    assert_eq!(pooled.alpn(), EXEC_ALPN_V3);
    assert!(!client
        .0
        .exec_pool
        .is_legacy_v2(&modern_host.identity.node_id));

    let legacy_key = ExecPoolKey::of(&client_id, &legacy_host.endpoint);
    assert!(!client
        .0
        .exec_pool
        .is_legacy_v2(&legacy_host.identity.node_id));
    let (_, first_v2) = run_to_terminal(
        &client.0,
        &legacy_host.endpoint,
        &client_id,
        argv_start("true").expect("start"),
    )
    .expect("first legacy v2 job");
    assert_eq!(first_v2.kind, ExecTerminalKind::Exited);
    assert!(client
        .0
        .exec_pool
        .is_legacy_v2(&legacy_host.identity.node_id));
    assert!(client.0.exec_pool.pooled_connection(&legacy_key).is_none());

    // Remembered `/2` peer: subsequent connects dial `/2` directly, while the
    // modern host remains pooled on `/3`.
    let (_, second_v2) = run_to_terminal(
        &client.0,
        &legacy_host.endpoint,
        &client_id,
        argv_start("true").expect("start"),
    )
    .expect("second legacy v2 job");
    assert_eq!(second_v2.kind, ExecTerminalKind::Exited);
    let direct = client
        .0
        .connect_exec(&legacy_host.endpoint)
        .expect("direct legacy connect");
    assert_eq!(direct.alpn(), EXEC_ALPN);
    direct.close(VarInt::from_u32(0), b"done");
}

#[test]
fn verbindungen_task_exec_reuses_one_connection_and_retries_only_before_first_byte() {
    let _plan = hold(ProfileOverrides::default());
    let client_id = make_identity("b3-pool-client").expect("client identity");
    let host = ExecLoopbackHost::start_for_client(&client_id, enabled_grant()).expect("host");
    let client = start_client_node(&client_id).expect("client");
    let key = ExecPoolKey::of(&client_id, &host.endpoint);

    let runs = Arc::new(AtomicUsize::new(0));
    let runs_hook = Arc::clone(&runs);
    host.node
        .0
        .exec_host
        .set_test_runner(Some(TestExecRunner::new(move |_| {
            runs_hook.fetch_add(1, Ordering::AcqRel);
            Ok(TestExecPlan::Immediate {
                stdout: b"ok".to_vec(),
                stderr: Vec::new(),
                exit_code: 0,
            })
        })));

    // 1. Two sequential jobs and two concurrent jobs share one `/3` connection.
    let (auth1, term1) = run_to_terminal(
        &client.0,
        &host.endpoint,
        &client_id,
        argv_start("one").expect("start 1"),
    )
    .expect("job 1");
    let conn1 = client
        .0
        .exec_pool
        .pooled_connection(&key)
        .expect("pooled 1");
    let (auth2, term2) = run_to_terminal(
        &client.0,
        &host.endpoint,
        &client_id,
        argv_start("two").expect("start 2"),
    )
    .expect("job 2");
    let conn2 = client
        .0
        .exec_pool
        .pooled_connection(&key)
        .expect("pooled 2");
    assert_eq!(term1.stdout_bytes, 2);
    assert_eq!(term2.stdout_bytes, 2);
    assert_eq!(conn1.stable_id(), conn2.stable_id());
    assert_ne!(
        auth1.expect("auth 1").session_id,
        auth2.expect("auth 2").session_id
    );
    assert_eq!(host.node.0.exec_host.open_count(), 1);

    std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            run_to_terminal(
                &client.0,
                &host.endpoint,
                &client_id,
                argv_start("par-a").expect("par a"),
            )
        });
        let b = scope.spawn(|| {
            run_to_terminal(
                &client.0,
                &host.endpoint,
                &client_id,
                argv_start("par-b").expect("par b"),
            )
        });
        assert!(a.join().expect("join a").is_ok());
        assert!(b.join().expect("join b").is_ok());
    });
    assert_eq!(
        client
            .0
            .exec_pool
            .pooled_connection(&key)
            .expect("still pooled")
            .stable_id(),
        conn1.stable_id()
    );
    assert_eq!(runs.load(Ordering::Acquire), 4);

    // 2. Closed pooled connection before any byte of the job is sent: evicted
    // and retried once on a new connection.
    conn1.close(VarInt::from_u32(0x4558), b"drop before next job");
    let (_, term3) = run_to_terminal(
        &client.0,
        &host.endpoint,
        &client_id,
        argv_start("three").expect("start 3"),
    )
    .expect("job 3 retried on fresh connection");
    assert_eq!(term3.kind, ExecTerminalKind::Exited);
    let conn3 = client
        .0
        .exec_pool
        .pooled_connection(&key)
        .expect("fresh pooled connection");
    assert_ne!(conn3.stable_id(), conn1.stable_id());
    assert_eq!(runs.load(Ordering::Acquire), 5);

    // 3. Connection lost AFTER `Start` was sent (`bytes_written > 0`): must
    // NOT retry automatically (`runs` increments only once, `start_may_have_been_sent` is true).
    let job4_started = Arc::new(AtomicBool::new(false));
    let started_hook = Arc::clone(&job4_started);
    let runs_hook2 = Arc::clone(&runs);
    host.node
        .0
        .exec_host
        .set_test_runner(Some(TestExecRunner::new(move |_| {
            runs_hook2.fetch_add(1, Ordering::AcqRel);
            Ok(TestExecPlan::HoldUntilStopped {
                started: Arc::clone(&started_hook),
            })
        })));
    let client_node = Arc::clone(&client.0);
    let endpoint = host.endpoint.clone();
    let identity = client_id.clone();
    let job4 = std::thread::spawn(move || {
        run_to_terminal(
            &client_node,
            &endpoint,
            &identity,
            argv_start("four").expect("start 4"),
        )
    });
    assert!(wait_until(Duration::from_secs(10), || {
        job4_started.load(Ordering::Acquire)
    }));
    conn3.close(VarInt::from_u32(0x4558), b"drop after start sent");
    let err = job4
        .join()
        .expect("join job 4")
        .expect_err("mid-flight disconnect fails");
    assert!(err.start_may_have_been_sent, "{err:?}");
    assert_eq!(runs.load(Ordering::Acquire), 6, "job 4 must not be retried");
}

#[test]
fn verbindungen_task_exec_checks_rights_per_job_and_closes_on_revocation() {
    let _plan = hold(ProfileOverrides::default());
    let client_id = make_identity("b3-rights-client").expect("client identity");
    let mut initial = ExecGrant::default();
    initial
        .apply_access_change(
            &AccessGrantChange {
                capabilities: BTreeMap::from([
                    (Capability::ExecCommands, CapabilityDecision::Allow),
                    (Capability::ExecTerminal, CapabilityDecision::Allow),
                ]),
                ..AccessGrantChange::default()
            },
            now_secs(),
        )
        .expect("initial grant");
    let host = ExecLoopbackHost::start_for_client(&client_id, initial.clone()).expect("host");
    let client = start_client_node(&client_id).expect("client");
    let key = ExecPoolKey::of(&client_id, &host.endpoint);

    // Job 1 (terminal) succeeds on `/3` with initial policy_revision.
    let (auth1, term1) = run_to_terminal(&client.0, &host.endpoint, &client_id, terminal_start())
        .expect("initial terminal job");
    assert_eq!(term1.kind, ExecTerminalKind::Exited);
    let rev1 = auth1.expect("auth 1").policy_revision;
    let conn1 = client.0.exec_pool.pooled_connection(&key).expect("pooled");
    assert_eq!(host.node.0.exec_host.open_count(), 1);

    // Narrow ONLY `Terminal` to `Off`, keeping `Commands` `Allow`.
    let target = ExecGrantTarget::Direct {
        device_id: client_id.device_id.clone(),
        public_key: client_id.public_key.clone(),
        fingerprint: client_id.fingerprint.clone(),
        node_id: client_id.node_id.clone(),
    };
    let principal = ExecPrincipal {
        relation_kind: "direct".into(),
        relation_id: host.identity.direct_lookup_id.clone(),
        device_id: client_id.device_id.clone(),
        device_name: client_id.device_name.clone(),
        public_key: client_id.public_key.clone(),
        fingerprint: client_id.fingerprint.clone(),
        node_id: client_id.node_id.clone(),
    };
    let mut commands_only = initial;
    commands_only
        .apply_access_change(
            &AccessGrantChange {
                capabilities: BTreeMap::from([(Capability::ExecTerminal, CapabilityDecision::Off)]),
                ..AccessGrantChange::default()
            },
            now_secs(),
        )
        .expect("narrow terminal");
    apply_exact_with_access(
        &host.auth,
        host.node.0.exec_registry(),
        Some(host.node.0.access_registry()),
        target.clone(),
        principal,
        commands_only,
    )
    .expect("apply narrowed grant");
    host.node.0.recheck_exec_connections();
    assert_eq!(host.node.0.exec_host.open_count(), 1);

    // Job 2 (terminal) on the SAME `/3` connection is refused per-job with `RIGHT_NOT_GRANTED`.
    let denied = run_to_terminal(&client.0, &host.endpoint, &client_id, terminal_start())
        .expect_err("terminal must be denied on next job");
    assert_eq!(denied.kind, ExecClientFailureKind::Remote);
    assert_eq!(denied.code, RIGHT_NOT_GRANTED);

    // Job 3 (commands) still reuses the SAME `/3` connection and sees the new policy_revision.
    let (auth3, term3) = run_to_terminal(
        &client.0,
        &host.endpoint,
        &client_id,
        argv_start("true").expect("cmd start"),
    )
    .expect("commands still allowed");
    assert_eq!(term3.kind, ExecTerminalKind::Exited);
    assert!(auth3.expect("auth 3").policy_revision > rev1);
    assert_eq!(
        client
            .0
            .exec_pool
            .pooled_connection(&key)
            .expect("same pooled connection")
            .stable_id(),
        conn1.stable_id()
    );

    // Revoke Exec completely while a job is running: delivers `Revoked` and
    // then closes the `/3` connection on the host; a subsequent job fails with `permission_denied`.
    let running_started = Arc::new(AtomicBool::new(false));
    let started_hook = Arc::clone(&running_started);
    host.node
        .0
        .exec_host
        .set_test_runner(Some(TestExecRunner::new(move |_| {
            Ok(TestExecPlan::HoldUntilStopped {
                started: Arc::clone(&started_hook),
            })
        })));
    let mut session = client
        .0
        .start_exec(
            host.endpoint.clone(),
            client_id.clone(),
            argv_start("hold").expect("hold start"),
        )
        .expect("start hold");
    assert!(wait_until(Duration::from_secs(10), || {
        running_started.load(Ordering::Acquire)
    }));
    mutate(
        &host.auth,
        host.node.0.exec_registry(),
        target,
        false,
        now_secs(),
    )
    .expect("revoke exec");
    host.node.0.recheck_exec_connections();
    let revoked_terminal = loop {
        match session.next_event().expect("revoked event") {
            ExecClientEvent::Terminal(term) => break term,
            ExecClientEvent::Failed(err) => panic!("expected Revoked terminal, got {err:?}"),
            _ => {}
        }
    };
    session.finish().expect("finish revoked session");
    assert_eq!(revoked_terminal.kind, ExecTerminalKind::Revoked);
    assert!(wait_until(Duration::from_secs(5), || {
        host.node.0.exec_host.open_count() == 0 && conn1.close_reason().is_some()
    }));

    let after_revoke = run_to_terminal(
        &client.0,
        &host.endpoint,
        &client_id,
        argv_start("denied").expect("after revoke"),
    )
    .expect_err("revoked peer must fail auth");
    assert_eq!(after_revoke.kind, ExecClientFailureKind::Authentication);
    assert_eq!(after_revoke.code, PERMISSION_DENIED);
}

#[test]
fn verbindungen_task_exec_pool_closes_connection_after_idle_timeout() {
    let _plan = hold(ProfileOverrides::default());
    let client_id = make_identity("b3-idle-client").expect("client identity");
    let host = ExecLoopbackHost::start_for_client(&client_id, enabled_grant()).expect("host");
    let client = start_client_node(&client_id).expect("client");
    client
        .0
        .exec_pool
        .set_idle_timeout_for_test(Duration::from_millis(160));
    let key = ExecPoolKey::of(&client_id, &host.endpoint);

    run_to_terminal(
        &client.0,
        &host.endpoint,
        &client_id,
        argv_start("first").expect("first"),
    )
    .expect("first job");
    let conn1 = client
        .0
        .exec_pool
        .pooled_connection(&key)
        .expect("pooled after first job");
    std::thread::sleep(Duration::from_millis(80));

    // A second job before the idle deadline reuses the connection and resets the timer.
    run_to_terminal(
        &client.0,
        &host.endpoint,
        &client_id,
        argv_start("second").expect("second"),
    )
    .expect("second job");
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        conn1.close_reason().is_none(),
        "idle timer must reset from the end of the second job"
    );

    // After the idle window elapses without further jobs, the connection closes on both ends.
    assert!(wait_until(Duration::from_secs(3), || {
        client.0.exec_pool.pooled_connection(&key).is_none()
            && conn1.close_reason().is_some()
            && host.node.0.exec_host.open_count() == 0
    }));
}
