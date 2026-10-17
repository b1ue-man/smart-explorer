//! The Exec protocol of access rights (D.2.2): what a host of this version
//! tells before a start, that clients of 0.5.180 to 0.5.182 never see the
//! new frame and survive a long confirmation, that a new client works with
//! an old host, and the refusal texts with their next step (C6).
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::{mpsc, watch};

use super::*;
use crate::share::access_grant::{
    AccessGrantChange, AccountProfile, Capability, CapabilityDecision, Expiry,
};
use crate::share::core::{hmac_proof, public_fingerprint, verify_hmac};
use crate::share::exec_auth::{authorize_client_hello_in, transcript};
use crate::share::exec_client::{run_stream_with_transport, ExecClientEvent};
use crate::share::exec_client_active;
use crate::share::exec_client_messages::{
    account_helper_unavailable, account_line, account_not_allowed, approval_cancelled,
    approval_denied, approval_pending_event, approval_status_line, approval_unavailable,
    approval_unavailable_notifications, client_failure_message, permission_denied,
    right_not_granted, ACCOUNT_HELPER_UNAVAILABLE, ACCOUNT_NOT_ALLOWED, APPROVAL_UNAVAILABLE,
    OLD_PERMISSION_DENIED_MESSAGE, PERMISSION_DENIED, RIGHT_NOT_GRANTED,
};
use crate::share::exec_frame_reader::client_frames;
use crate::share::exec_heartbeat::{ExecHeartbeatPolicy, EXEC_HEARTBEAT_POLICY};
use crate::share::exec_protocol::{
    recv_client_frame, recv_client_hello, recv_hello_result, recv_server_frame, send_client_frame,
    send_hello_ok, send_server_frame, send_server_hello, ClientFrame, ExecClientHello,
    ExecServerHello, ServerFrame, ServerProtocolState,
};
use crate::share::exec_server::{start_grant, wait_for_approval, ApprovalWait, ClientStream};
use crate::share::exec_terminal::host_features;
use crate::share::exec_types::{
    ExecAuthorization, ExecCommand, ExecId, ExecProviderStatus, ExecStart, ExecTerminal,
    ExecTerminalKind, EXEC_CAPABILITY, EXEC_PROTOCOL_VERSION,
};
use crate::share::identity::ShareIdentity;
use crate::share::types::{
    DirectGrant, DirectGrantState, PeerEndpoint, PeerPresence, ShareAuthState, ShareScope,
};
use crate::share::{ExecTerminalRequest, ExecTerminalSize};

use crate::share::access_grant::CapabilityDecision::{Allow, Ask, Off};

const NOW: i64 = 1_760_000_000;
const SECRET: &[u8] = b"relation-secret";

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

fn state(host: &ShareIdentity, client: &ShareIdentity, exec: ExecGrant) -> ShareAuthState {
    ShareAuthState {
        identity: host.clone(),
        direct_secret: SECRET.to_vec(),
        default_direct_exports: Default::default(),
        direct_contacts: Vec::new(),
        direct_grants: vec![DirectGrant {
            device_id: client.device_id.clone(),
            device_name: client.device_name.clone(),
            public_key: client.public_key.clone(),
            fingerprint: client.fingerprint.clone(),
            node_id: client.node_id.clone(),
            state: DirectGrantState::Accepted,
            updated_at: 1,
            exec,
            write: false,
        }],
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 1,
    }
}

fn server_hello(host: &ShareIdentity) -> ExecServerHello {
    ExecServerHello::new(
        "challenge-0123456789-abcdef".into(),
        host.device_id.clone(),
        host.public_key.clone(),
        host.fingerprint.clone(),
        host.node_id.clone(),
    )
}

fn client_hello(
    server: &ExecServerHello,
    host: &ShareIdentity,
    client: &ShareIdentity,
) -> ExecClientHello {
    let mut hello = ExecClientHello {
        protocol_version: EXEC_PROTOCOL_VERSION,
        capability: EXEC_CAPABILITY.into(),
        relation_kind: "direct".into(),
        relation_id: host.direct_lookup_id.clone(),
        device_id: client.device_id.clone(),
        device_name: client.device_name.clone(),
        public_key: client.public_key.clone(),
        fingerprint: client.fingerprint.clone(),
        node_id: client.node_id.clone(),
        client_nonce: "client-nonce-0123456789abc".into(),
        proof: String::new(),
        client_features: Vec::new(),
    };
    hello.proof = hmac_proof(SECRET, &transcript(server, &hello));
    hello
}

fn provider() -> ExecProviderStatus {
    ExecProviderStatus {
        available: true,
        provider: "test".into(),
        detail: "ready".into(),
        elevated: false,
        user_label: "tester".into(),
    }
}

fn change(decisions: &[(Capability, CapabilityDecision)]) -> AccessGrantChange {
    AccessGrantChange {
        capabilities: decisions.iter().copied().collect(),
        ..AccessGrantChange::default()
    }
}

fn start(byte: &str, command: ExecCommand) -> ExecStart {
    let terminal = (command == ExecCommand::Login).then(|| ExecTerminalRequest {
        term: "xterm-256color".into(),
        size: ExecTerminalSize { cols: 80, rows: 24 },
    });
    ExecStart {
        exec_id: ExecId::parse(byte.repeat(16)).unwrap(),
        command,
        cwd: None,
        env: BTreeMap::new(),
        timeout_ms: None,
        max_output_bytes: None,
        terminal,
    }
}

fn shell(byte: &str) -> ExecStart {
    start(
        byte,
        ExecCommand::Shell {
            command: "ls -la".into(),
        },
    )
}

#[test]
fn perm_task_hello_ok_access_view() {
    let mut grant = ExecGrant::default();
    let remote = AccessGrantChange {
        account: Some(AccountProfile::Remote),
        expiry: Some(Expiry::At(1_760_728_800)),
        ..change(&[
            (Capability::ExecCommands, Allow),
            (Capability::ExecTerminal, Ask),
        ])
    };
    grant.apply_access_change(&remote, 1).unwrap();
    let view = ExecAccessView::of(&grant, "alice");
    let view_text = serde_json::to_string(&view).unwrap();
    assert_eq!(
        view_text,
        r#"{"account_label":"se-remote","account_profile":"remote","commands":"allow","terminal":"ask","programs_restricted":false,"valid_until":1760728800}"#
    );
    let mut features = host_features(true);
    features.extend(host_access_features().map(String::from));
    assert_eq!(
        features,
        ["exec_terminal_v1", "exec_access_v1", "exec_approval_v1"]
    );
    let hello = ExecHelloOk {
        authorization: ExecAuthorization {
            policy_revision: 1,
            authorization_epoch: 2,
            session_id: "session-0123456789".into(),
        },
        provider: provider(),
        features,
    };
    // `access` is the last key of the hello every older client reads.
    let plain = serde_json::to_string(&hello).unwrap();
    let announced = serde_json::to_string(&HelloOkWithAccess {
        hello: &hello,
        access: Some(&view),
    })
    .unwrap();
    assert_eq!(
        announced,
        format!("{},\"access\":{view_text}}}", &plain[..plain.len() - 1])
    );
    let without = HelloOkWithAccess {
        hello: &hello,
        access: None,
    };
    assert_eq!(serde_json::to_string(&without).unwrap(), plain);
    let old: ExecHelloOk = serde_json::from_str(&announced).unwrap();
    assert_eq!(old, hello);
    let received: ReceivedHelloOk = serde_json::from_str(&announced).unwrap();
    assert_eq!(received.hello, hello);
    assert_eq!(received.access.as_ref(), Some(&view));
    // A view this client cannot read is left out; the handshake goes on.
    let newer = announced.replace(r#""terminal":"ask""#, r#""terminal":"later""#);
    let received: ReceivedHelloOk = serde_json::from_str(&newer).unwrap();
    assert!(received.access.is_none());
    assert_eq!(received.hello, hello);
    // On the wire.
    run(async {
        let (mut writer, mut reader) = tokio::io::duplex(64 * 1024);
        let wire = HelloOkWithAccess {
            hello: &hello,
            access: Some(&view),
        };
        send_hello_ok(&mut writer, &wire).await.unwrap();
        let received = recv_hello_result::<_, ReceivedHelloOk>(&mut reader)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.access, Some(view.clone()));
    });
    assert_eq!(client_features(), ["exec_approval_v1", "exec_access_v1"]);
    assert!(wants_approval_frames(&client_features()));
    assert!(!wants_approval_frames(&[]));
}

#[test]
fn perm_task_error_texts_have_next_step() {
    let device = "Laptop";
    let with_next_step = [
        (PERMISSION_DENIED, permission_denied(device)),
        (
            RIGHT_NOT_GRANTED,
            right_not_granted(device, Capability::ExecTerminal),
        ),
        (
            RIGHT_NOT_GRANTED,
            right_not_granted(device, Capability::ExecCommands),
        ),
        (APPROVAL_UNAVAILABLE, approval_unavailable(device)),
        (
            APPROVAL_UNAVAILABLE,
            approval_unavailable_notifications(device),
        ),
        (ACCOUNT_NOT_ALLOWED, account_not_allowed(device, "deploy")),
        (
            ACCOUNT_HELPER_UNAVAILABLE,
            account_helper_unavailable(device, "deploy", "not installed"),
        ),
    ];
    for (code, text) in &with_next_step {
        assert!(text.contains(device), "{code}: {text}");
        assert!(text.contains("Next step"), "{code}: {text}");
    }
    assert!(right_not_granted(device, Capability::ExecTerminal).contains("no terminal"));
    assert!(right_not_granted(device, Capability::ExecCommands).contains("no commands"));
    assert!(account_not_allowed(device, "deploy").contains("se share exec accounts allow deploy"));
    // A decision of the host user names the device.
    assert!(approval_denied(device).starts_with("Laptop:"));
    assert!(approval_cancelled(device).starts_with("Laptop:"));
    // A client replaces only the bare text of hosts up to 0.5.182.
    assert_eq!(
        client_failure_message(PERMISSION_DENIED, OLD_PERMISSION_DENIED_MESSAGE, device),
        permission_denied(device)
    );
    assert_eq!(
        client_failure_message(PERMISSION_DENIED, "custom", device),
        "custom"
    );
    assert_eq!(client_failure_message(RIGHT_NOT_GRANTED, "x", device), "x");
    // The waiting line and its JSON event.
    assert_eq!(
        approval_status_line(device, Some(ApprovalSurface::Window)),
        "Waiting for confirmation on Laptop ... (Ctrl+C cancels)"
    );
    assert!(approval_status_line(device, Some(ApprovalSurface::CliOnly))
        .ends_with("(on the device only with `se share exec approvals`)"));
    assert_eq!(
        approval_pending_event(device, Some(ApprovalSurface::Notification)),
        serde_json::json!({"event": "approval_pending", "device": "Laptop", "surface": "notification"})
    );
}

#[test]
fn perm_task_terminal_off_rejects_login() {
    let host = identity("host", "Host", 3);
    let client = identity("client", "Client", 5);
    let mut grant = ExecGrant::default();
    let commands_only = change(&[
        (Capability::ExecCommands, Allow),
        (Capability::ExecTerminal, Off),
    ]);
    grant.apply_access_change(&commands_only, 1).unwrap();
    let auth = Mutex::new(state(&host, &client, grant.clone()));
    let server = server_hello(&host);
    let hello = client_hello(&server, &host, &client);
    let authorized = {
        let state = auth.lock().unwrap();
        authorize_client_hello_in(&server, &hello, &client.node_id, &state, NOW).unwrap()
    };
    // The client learns it before any start.
    let view = ExecAccessView::of(&authorized.grant, "alice");
    assert_eq!((view.commands, view.terminal), (Allow, Off));

    let login = start("a1", ExecCommand::Login);
    let refused = start_grant(&auth, &authorized, &login, "Host", NOW)
        .err()
        .expect("a terminal is refused");
    assert_eq!(refused.code, RIGHT_NOT_GRANTED);
    assert_eq!(
        refused.message,
        right_not_granted("Host", Capability::ExecTerminal)
    );
    // Commands still start, at the grant's current revision.
    let decided = match start_grant(&auth, &authorized, &shell("a2"), "Host", NOW) {
        Ok(decided) => decided,
        Err(error) => panic!("commands were refused: {error:?}"),
    };
    assert_eq!(decided.capability, Capability::ExecCommands);
    assert_eq!(decided.authorization.policy_revision, grant.policy_revision);

    // A program list allows no terminal either (C1).
    auth.lock().unwrap().direct_grants[0].exec = ExecGrant {
        enabled: true,
        policy_revision: grant.policy_revision,
        programs: vec!["uptime".into()],
        ..ExecGrant::default()
    };
    let refused = start_grant(&auth, &authorized, &login, "Host", NOW).err();
    assert_eq!(
        refused.map(|error| error.code),
        Some(RIGHT_NOT_GRANTED.into())
    );

    // A terminal on `ask` waits for the host user; this host cannot ask yet.
    let mut asking = grant.clone();
    asking
        .apply_access_change(&change(&[(Capability::ExecTerminal, Ask)]), 2)
        .unwrap();
    auth.lock().unwrap().direct_grants[0].exec = asking;
    let refused = start_grant(&auth, &authorized, &login, "Host", NOW).err();
    assert_eq!(
        refused.map(|error| error.code),
        Some(APPROVAL_UNAVAILABLE.into())
    );

    // Switched off after the handshake: `permission_denied`, as at it.
    auth.lock().unwrap().direct_grants[0].exec = ExecGrant::default();
    let refused = start_grant(&auth, &authorized, &shell("a3"), "Host", NOW).err();
    assert_eq!(
        refused.map(|error| error.code),
        Some(PERMISSION_DENIED.into())
    );
}

#[path = "exec_access_stream_tests.rs"]
mod stream;
