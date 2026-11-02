//! PRM-M3: a start on „Fragen“ waits for the host user and starts only on
//! „Diesmal erlauben“ or „Immer erlauben“; „Ablehnen“, a cancel of the
//! requesting device and a narrowed grant refuse it; open requests (Exec and
//! screen requests of S4) are not limited (R15,
//! `access_approval_limits_tests.rs`); without a graphical session the
//! request is shown nowhere but `cli_only`.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use super::*;
use crate::share::access_approval::{ApprovalDecision, ApprovalId, ApprovalView, ConsentNotifier};
use crate::share::access_audit::access_line;
use crate::share::access_grant::AccessGrantChange;
use crate::share::access_grant::CapabilityDecision::{Allow, Ask};
use crate::share::core::public_fingerprint;
use crate::share::exec_access_view::{client_features, ApprovalSurface};
use crate::share::exec_audit::{self, StartAccess};
use crate::share::exec_client_messages::{approval_denied, APPROVAL_DENIED};
use crate::share::exec_frame_reader::client_frames;
use crate::share::exec_protocol::{recv_server_frame, send_client_frame, ClientFrame, ServerFrame};
use crate::share::exec_registry::{ExecAdmission, ExecRegistry, ExecRegistryLimits};
use crate::share::exec_types::{ExecCommand, ExecId, ExecPrincipal};
use crate::share::identity::ShareIdentity;
use crate::share::types::{DirectGrant, DirectGrantState};

const NOW: i64 = 1_760_000_000;
const POLICY: ExecHeartbeatPolicy = ExecHeartbeatPolicy {
    interval: Duration::from_millis(50),
    peer_timeout: Duration::from_secs(5),
    write_timeout: Duration::from_secs(1),
};

fn run<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn identity(device_id: &str, name: &str, seed: u8) -> ShareIdentity {
    let secret = iroh::SecretKey::from_bytes(&[seed; 32]);
    let node_id = secret.public().to_string();
    ShareIdentity {
        device_id: device_id.into(),
        device_name: name.into(),
        direct_lookup_id: format!("lookup-{device_id}"),
        public_key: node_id.clone(),
        fingerprint: public_fingerprint(node_id.as_bytes()),
        node_id,
        iroh_secret: secret,
        direct_secret: [seed; 32],
    }
}

fn change(decisions: &[(Capability, CapabilityDecision)]) -> AccessGrantChange {
    AccessGrantChange {
        capabilities: decisions.iter().copied().collect(),
        ..AccessGrantChange::default()
    }
}

fn shell(byte: &str, text: &str) -> ExecStart {
    ExecStart {
        exec_id: ExecId::parse(byte.repeat(16)).unwrap(),
        command: ExecCommand::Shell {
            command: text.into(),
        },
        cwd: None,
        env: Default::default(),
        timeout_ms: None,
        max_output_bytes: None,
        terminal: None,
    }
}

/// Answers with `surface` and records what it was told to withdraw.
struct TestConsent {
    surface: Option<ApprovalSurface>,
    withdrawn: Mutex<Vec<ApprovalId>>,
}

impl ConsentNotifier for TestConsent {
    fn show(&self, _request: &ApprovalView) -> Option<ApprovalSurface> {
        self.surface
    }

    fn withdraw(&self, id: &ApprovalId) {
        self.withdrawn.lock().unwrap().push(id.clone());
    }
}

/// A host whose device „Laptop“ has the grant `grant`.
struct Host {
    auth: Mutex<ShareAuthState>,
    authorized: AuthorizedExecPeer,
    registry: Arc<ExecRegistry>,
    service: ApprovalService,
    consent: Arc<TestConsent>,
    log: Mutex<Vec<Value>>,
}

impl Host {
    fn new(decisions: &[(Capability, CapabilityDecision)]) -> Self {
        let host = identity("host", "Host", 3);
        let client = identity("laptop", "Laptop", 5);
        let mut grant = ExecGrant::default();
        grant.apply_access_change(&change(decisions), NOW).unwrap();
        let principal = ExecPrincipal {
            relation_kind: "direct".into(),
            relation_id: host.direct_lookup_id.clone(),
            device_id: client.device_id.clone(),
            device_name: client.device_name.clone(),
            public_key: client.public_key.clone(),
            fingerprint: client.fingerprint.clone(),
            node_id: client.node_id.clone(),
        };
        let registry = Arc::new(ExecRegistry::new(ExecRegistryLimits::default()));
        registry
            .apply_authorization(&principal, grant.policy_revision, 1, true)
            .unwrap();
        let consent = Arc::new(TestConsent {
            surface: Some(ApprovalSurface::Window),
            withdrawn: Mutex::new(Vec::new()),
        });
        let authorized = AuthorizedExecPeer {
            principal,
            authorization: ExecAuthorization {
                policy_revision: grant.policy_revision,
                authorization_epoch: 1,
                session_id: "session-0123456789".into(),
            },
            grant: grant.clone(),
        };
        let state = ShareAuthState {
            identity: host,
            direct_secret: b"relation-secret".to_vec(),
            default_direct_exports: Default::default(),
            direct_contacts: Vec::new(),
            direct_grants: vec![DirectGrant {
                device_id: client.device_id.clone(),
                device_name: "Laptop (gespeichert)".into(),
                public_key: client.public_key.clone(),
                fingerprint: client.fingerprint.clone(),
                node_id: client.node_id.clone(),
                state: DirectGrantState::Accepted,
                updated_at: 1,
                exec: grant,
                write: false,
            }],
            rooms: Vec::new(),
            direct_requests: Vec::new(),
            direct_request_tombstones: Vec::new(),
            seen_nonces: Default::default(),
            direct_online: true,
            authorization_epoch: 1,
        };
        Self {
            auth: Mutex::new(state),
            authorized,
            registry,
            service: ApprovalService::new(consent.clone()),
            consent,
            log: Mutex::new(Vec::new()),
        }
    }

    fn device(&self) -> &str {
        &self.authorized.principal.device_id
    }

    fn change_grant(&self, decisions: &[(Capability, CapabilityDecision)]) -> u64 {
        let mut state = self.auth.lock().unwrap();
        let grant = &mut state.direct_grants[0].exec;
        grant
            .apply_access_change(&change(decisions), NOW + 1)
            .unwrap();
        grant.policy_revision
    }

    /// A start of the device that runs (an earlier session).
    fn running(&self, start: &ExecStart) -> crate::share::exec_registry::ExecReservation {
        let admission = self.registry.prepare(
            self.authorized.principal.clone(),
            self.authorized.authorization.clone(),
            start,
            NOW,
        );
        let reservation = match admission {
            Ok(ExecAdmission::Prepared(reservation)) => reservation,
            other => panic!("the earlier session did not start: {other:?}"),
        };
        self.registry
            .commit_start(&reservation.lease, || Ok(()))
            .unwrap();
        reservation
    }

    fn events(&self) -> Vec<String> {
        let log = self.log.lock().unwrap();
        log.iter()
            .map(|line| line["event"].as_str().unwrap().to_string())
            .collect()
    }
}

enum Act {
    Answer(ApprovalDecision),
    /// What the daemon's `Decide` does for „Immer erlauben“.
    Always,
    Cancel,
    Grant(Vec<(Capability, CapabilityDecision)>),
}

type Decided = Result<ApprovedStart, Refusal>;

/// Runs `decide_start` while `act` happens on the host or the client side;
/// returns the decision, the frames the client got and the request.
fn drive(
    host: &Host,
    start: &ExecStart,
    features: &[String],
    act: Act,
) -> (Decided, Vec<ServerFrame>, ApprovalView) {
    run(async {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (mut client_read, mut client_write) = tokio::io::split(client);
        let (server_read, mut server_write) = tokio::io::split(server);
        let mut stream = ClientStream::new(client_frames(server_read));
        let mut protocol = ServerProtocolState::default();
        let first = ClientFrame::Start {
            start: start.clone(),
            digest: start.digest().unwrap(),
        };
        protocol.accept(&first).unwrap();
        let sink = |entry: AccessEntry<'_>| host.log.lock().unwrap().push(access_line(&entry));
        let approval = StartApproval {
            service: &host.service,
            auth: &host.auth,
            authorized: &host.authorized,
            start,
            host: "Host",
            client_features: features,
            own_label: "alice",
            audit: &sink,
            grant_poll: Duration::from_millis(50),
        };
        let decision = decide_start(
            &approval,
            &mut server_write,
            &mut stream,
            &mut protocol,
            POLICY,
        );
        let actor = async {
            let view = loop {
                if let Some(view) = host
                    .service
                    .pending()
                    .into_iter()
                    .find(|v| v.surface.is_some())
                {
                    break view;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            };
            match act {
                Act::Answer(answer) => host.service.decide(&view.id, answer).unwrap(),
                Act::Always => {
                    let revision = host.change_grant(&[(view.capability, Allow)]);
                    let principal = &host.authorized.principal;
                    host.registry
                        .apply_widened_authorization(principal, revision, 1)
                        .unwrap();
                    let _ = host.service.decide(&view.id, ApprovalDecision::Always);
                }
                Act::Cancel => {
                    let cancel = ClientFrame::Cancel {
                        exec_id: start.exec_id.clone(),
                    };
                    send_client_frame(&mut client_write, &cancel).await.unwrap();
                }
                Act::Grant(decisions) => {
                    host.change_grant(&decisions);
                }
            }
            view
        };
        let (decided, view) = tokio::join!(decision, actor);
        drop(server_write);
        let mut frames = Vec::new();
        let read = Duration::from_millis(300);
        while let Ok(Ok(frame)) =
            tokio::time::timeout(read, recv_server_frame(&mut client_read)).await
        {
            frames.push(frame);
        }
        (decided.unwrap(), frames, view)
    })
}

fn approved(decided: Decided) -> ApprovedStart {
    match decided {
        Ok(approved) => approved,
        Err(refusal) => panic!("refused: {}: {}", refusal.error.code, refusal.error.message),
    }
}

fn refused(decided: Decided) -> Refusal {
    match decided {
        Ok(_) => panic!("the start was allowed"),
        Err(refusal) => refusal,
    }
}

#[test]
fn perm_task_ask_once_starts_and_logs() {
    let host = Host::new(&[(Capability::ExecCommands, Ask)]);
    let start = shell("a1", "echo top-secret-token");
    let (decided, frames, view) = drive(
        &host,
        &start,
        &client_features(),
        Act::Answer(ApprovalDecision::Once),
    );
    let approved = approved(decided);
    assert_eq!(approved.approval, ApprovalResult::Once);
    assert_eq!(approved.decided.capability, Capability::ExecCommands);
    // The client of this version learned where the host asks.
    let pending = ExecApprovalPending::new(
        start.exec_id.clone(),
        Capability::ExecCommands,
        ApprovalSurface::Window,
    );
    assert_eq!(frames, [ServerFrame::ApprovalPending(pending)]);
    // The request showed the stored name and the command; it is gone now.
    assert_eq!(view.device_name, "Laptop (gespeichert)");
    assert_eq!(view.account_label.as_deref(), Some("alice"));
    assert!(
        matches!(&view.command, Some(ApprovalCommand::Shell { text }) if text.contains("top-secret"))
    );
    assert!(host.service.pending().is_empty());
    assert!(host.consent.withdrawn.lock().unwrap().contains(&view.id));
    // It starts: the registry admits it at the grant's revision.
    let admission = host.registry.prepare(
        host.authorized.principal.clone(),
        approved.decided.authorization.clone(),
        &start,
        NOW,
    );
    assert!(matches!(admission, Ok(ExecAdmission::Prepared(_))));
    // It is logged: request and answer, and the start carries the answer.
    assert_eq!(host.events(), ["approval_requested", "approval_granted"]);
    let granted = host.log.lock().unwrap()[1].clone();
    assert_eq!(granted["approval"], "once");
    assert_eq!(granted["kind"], "exec");
    assert_eq!(granted["approval_id"], view.id.as_str());
    let access = StartAccess {
        capability: approved.decided.capability,
        account_profile: approved.decided.grant.access_grant().account.key(),
        approval: approved.approval.key(),
    };
    exec_audit::note_start(&start.exec_id, access.clone());
    assert_eq!(
        exec_audit::for_tests::noted(&start.exec_id, true),
        Some(access)
    );
    assert_eq!(exec_audit::for_tests::noted(&start.exec_id, true), None);
}

#[test]
fn perm_task_ask_always_sets_allow_keeps_session() {
    let host = Host::new(&[(Capability::ExecCommands, Ask)]);
    let earlier = host.running(&shell("b0", "sleep 60"));
    let start = shell("b1", "uptime");
    let (decided, _, _) = drive(&host, &start, &client_features(), Act::Always);
    let approved = approved(decided);
    assert_eq!(approved.approval, ApprovalResult::Always);
    // The grant allows now, at the next revision; the start passes as allowed.
    let revision = host.auth.lock().unwrap().direct_grants[0]
        .exec
        .policy_revision;
    assert_eq!(approved.decided.authorization.policy_revision, revision);
    assert_eq!(
        approved
            .decided
            .grant
            .decision_at(Capability::ExecCommands, NOW),
        Allow
    );
    assert!(start_grant(&host.auth, &host.authorized, &start, "Host", NOW).is_ok());
    // The widening keeps the session that already runs.
    assert_eq!(earlier.cancellation.reason(), None);
    assert_eq!(host.registry.active_views().len(), 1);
    let admission = host.registry.prepare(
        host.authorized.principal.clone(),
        approved.decided.authorization,
        &start,
        NOW,
    );
    assert!(matches!(admission, Ok(ExecAdmission::Prepared(_))));
    assert_eq!(host.events(), ["approval_requested", "approval_granted"]);
    assert_eq!(host.log.lock().unwrap()[1]["approval"], "always");
}

#[test]
fn perm_task_ask_deny_returns_approval_denied() {
    let host = Host::new(&[(Capability::ExecCommands, Ask)]);
    let start = shell("c1", "rm -rf /tmp/x");
    let (decided, frames, _) = drive(
        &host,
        &start,
        &client_features(),
        Act::Answer(ApprovalDecision::Deny),
    );
    let refusal = refused(decided);
    assert_eq!(refusal.error.code, APPROVAL_DENIED);
    assert_eq!(refusal.error.message, approval_denied("Host"));
    assert!(!refusal.start_refused);
    assert_eq!(frames.len(), 1);
    assert_eq!(host.events(), ["approval_requested", "approval_denied"]);
    assert_eq!(host.log.lock().unwrap()[1]["approval"], "denied");
    assert!(host.service.pending().is_empty());
}

#[test]
fn perm_task_ask_client_cancel_withdraws_prompt() {
    let host = Host::new(&[(Capability::ExecCommands, Ask)]);
    let start = shell("d1", "make deploy");
    let (decided, _, view) = drive(&host, &start, &client_features(), Act::Cancel);
    let refusal = refused(decided);
    assert_eq!(refusal.error.code, APPROVAL_CANCELLED);
    // The window and the lists lose the request at once.
    assert!(host.service.pending().is_empty());
    assert_eq!(*host.consent.withdrawn.lock().unwrap(), [view.id.clone()]);
    assert_eq!(host.events(), ["approval_requested", "approval_cancelled"]);
    assert_eq!(host.log.lock().unwrap()[1]["approval"], "cancelled");
    // An answer after the cancel finds nothing to allow.
    assert!(host
        .service
        .decide(&view.id, ApprovalDecision::Once)
        .is_err());
}

#[path = "access_approval_limits_tests.rs"]
mod limits;
