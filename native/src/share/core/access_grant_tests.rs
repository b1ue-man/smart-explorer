//! Access rights at runtime (C1, C5): which change narrows, what a narrowing
//! and the end of a grant end, and that one decision lets a device run an
//! ordinary command such as `ls` (K1).
use std::sync::{Arc, Mutex};

use super::*;
use crate::share::access_authorize::{capability_for, exec_connection_allowed, start_capability};
use crate::share::access_grant_runtime::{
    expired_restrictions, keeps_exec_jobs, next_expiry, GrantEndWatch,
};
use crate::share::core::{hmac_proof, now_secs, public_fingerprint};
use crate::share::exec_access_view::ExecAccessView;
use crate::share::exec_auth::{authorize_client_hello_in, transcript, ExecRejection};
use crate::share::exec_policy::ExecGrant;
use crate::share::exec_protocol::{ExecClientHello, ExecServerHello};
use crate::share::exec_registry::{
    ExecAdmission, ExecCancelReason, ExecRegistry, ExecRegistryLimits, ExecReservation,
};
use crate::share::exec_types::{
    ExecAuthorization, ExecCommand, ExecId, ExecPrincipal, ExecStart, EXEC_CAPABILITY,
    EXEC_PROTOCOL_VERSION,
};
use crate::share::identity::ShareIdentity;
use crate::share::relation_proof::ProvenPrincipal;
use crate::share::relation_rights::{authorization_restrictions, RestrictionReason};
use crate::share::types::{DirectGrant, DirectGrantState, ExecGrantTarget, ShareAuthState};
use crate::share::{ExecGrantMutation, ShareProfiles};

use super::Capability::{ExecCommands, ExecTerminal, ScreenControl, ScreenView};
use super::CapabilityDecision::{Allow, Ask, Off};

const NOW: i64 = 1_760_000_000;
const SECRET: &[u8] = b"direct-secret";

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

fn direct_grant(client: &ShareIdentity, exec: ExecGrant) -> DirectGrant {
    DirectGrant {
        device_id: client.device_id.clone(),
        device_name: client.device_name.clone(),
        public_key: client.public_key.clone(),
        fingerprint: client.fingerprint.clone(),
        node_id: client.node_id.clone(),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec,
        write: false,
    }
}

fn state(host: &ShareIdentity, client: &ShareIdentity, exec: ExecGrant) -> ShareAuthState {
    ShareAuthState {
        identity: host.clone(),
        direct_secret: SECRET.to_vec(),
        default_direct_exports: Default::default(),
        direct_contacts: Vec::new(),
        direct_grants: vec![direct_grant(client, exec)],
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 1,
    }
}

fn hellos(host: &ShareIdentity, client: &ShareIdentity) -> (ExecServerHello, ExecClientHello) {
    let server = ExecServerHello::new(
        "challenge-0123456789-abcdef".into(),
        host.device_id.clone(),
        host.public_key.clone(),
        host.fingerprint.clone(),
        host.node_id.clone(),
    );
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
    hello.proof = hmac_proof(SECRET, &transcript(&server, &hello));
    (server, hello)
}

fn principal(host: &ShareIdentity, client: &ShareIdentity) -> ExecPrincipal {
    ExecPrincipal {
        relation_kind: "direct".into(),
        relation_id: host.direct_lookup_id.clone(),
        device_id: client.device_id.clone(),
        device_name: client.device_name.clone(),
        public_key: client.public_key.clone(),
        fingerprint: client.fingerprint.clone(),
        node_id: client.node_id.clone(),
    }
}

/// A grant made through one access change from switched off.
fn granted(decisions: &[(Capability, CapabilityDecision)]) -> ExecGrant {
    let mut grant = ExecGrant::default();
    let change = AccessGrantChange {
        capabilities: decisions.iter().copied().collect(),
        ..AccessGrantChange::default()
    };
    grant.apply_access_change(&change, 1).unwrap();
    grant
}

fn start(command: ExecCommand) -> ExecStart {
    ExecStart {
        exec_id: ExecId::generate().unwrap(),
        command,
        cwd: None,
        env: Default::default(),
        timeout_ms: None,
        max_output_bytes: None,
        terminal: None,
    }
}

fn reserve(
    registry: &ExecRegistry,
    principal: &ExecPrincipal,
    revision: u64,
    epoch: u64,
) -> ExecReservation {
    let authorization = ExecAuthorization {
        policy_revision: revision,
        authorization_epoch: epoch,
        session_id: format!("session-{revision}-{epoch}"),
    };
    let start = start(ExecCommand::Shell {
        command: "sleep 60".into(),
    });
    match registry.prepare(principal.clone(), authorization, &start, NOW) {
        Ok(ExecAdmission::Prepared(reservation)) => reservation,
        _ => panic!("the start was not reserved"),
    }
}

#[test]
fn perm_task_narrowing_matrix() {
    // Every pair of decisions of one capability: only a sinking rank narrows.
    for capability in Capability::ALL {
        for old in [Off, Ask, Allow] {
            for new in [Off, Ask, Allow] {
                let before = AccessGrant::default().with(capability, old);
                let after = AccessGrant::default().with(capability, new);
                let narrowed = after.narrowing_against(&before, false);
                let expected = if new.rank() < old.rank() {
                    vec![capability]
                } else {
                    Vec::new()
                };
                assert_eq!(narrowed, expected, "{capability:?}: {old:?} -> {new:?}");
            }
        }
    }
    let on = AccessGrant::default()
        .with(ExecCommands, Allow)
        .with(ExecTerminal, Ask)
        .with(ScreenView, Allow);
    // The account applies to Exec only; a capability that was off never narrows.
    let mut remote = on.clone();
    remote.account = AccountProfile::Remote;
    assert_eq!(
        remote.narrowing_against(&on, false),
        [ExecCommands, ExecTerminal]
    );
    // A changed program list narrows commands.
    assert_eq!(on.narrowing_against(&on, true), [ExecCommands]);
    // A new or earlier end narrows every capability that was on; a later or
    // removed end widens.
    let mut ending = on.clone();
    ending.valid_until = Some(NOW + 100);
    let mut earlier = on.clone();
    earlier.valid_until = Some(NOW + 50);
    let all_on = [ExecCommands, ExecTerminal, ScreenView];
    assert_eq!(ending.narrowing_against(&on, false), all_on);
    assert_eq!(earlier.narrowing_against(&ending, false), all_on);
    assert!(ending.narrowing_against(&earlier, false).is_empty());
    assert!(on.narrowing_against(&ending, false).is_empty());
    let screen_control = on.clone().with(ScreenControl, Allow);
    assert!(screen_control.narrowing_against(&on, false).is_empty());

    // A profile change: one restriction per narrowed capability, none for a
    // widening; file sessions of the device are not affected.
    let host = identity("host", "Host", 3);
    let client = identity("client", "Client", 5);
    let ask = granted(&[(ExecCommands, Ask), (ExecTerminal, Allow)]);
    let allow = granted(&[(ExecCommands, Allow), (ExecTerminal, Allow)]);
    let asking = state(&host, &client, ask.clone());
    let allowing = state(&host, &client, allow.clone());
    assert!(authorization_restrictions(&asking, &allowing).is_empty());
    let narrowed = authorization_restrictions(&allowing, &asking);
    let reasons: Vec<RestrictionReason> = narrowed.items().iter().map(|item| item.reason).collect();
    assert_eq!(
        reasons,
        [RestrictionReason::CapabilityNarrowed(ExecCommands)]
    );
    let (key, node) = (client.public_key.as_str(), client.node_id.as_str());
    assert!(narrowed.affects_exec("direct", "", key, node));
    assert!(narrowed.affects_capability("direct", "", key, node, ExecCommands));
    assert!(!narrowed.affects_capability("direct", "", key, node, ExecTerminal));
    assert!(!narrowed.affects_capability("direct", "", key, node, ScreenView));
    assert!(!narrowed.affects("direct", "", key, node));

    // The registry keeps a running job through a widening, binds the
    // connection's authorization to the new revision, and ends the job on a
    // narrowing.
    assert!(keeps_exec_jobs(&ask, &allow));
    assert!(!keeps_exec_jobs(&allow, &ask));
    let registry = ExecRegistry::new(ExecRegistryLimits::default());
    let principal = principal(&host, &client);
    registry
        .apply_authorization(&principal, 1, 1, true)
        .unwrap();
    let job = reserve(&registry, &principal, 1, 1);
    registry
        .apply_widened_authorization(&principal, 2, 2)
        .unwrap();
    assert_eq!(job.cancellation.reason(), None);
    let later = reserve(&registry, &principal, 2, 1);
    registry
        .apply_authorization(&principal, 3, 3, true)
        .unwrap();
    assert_eq!(job.cancellation.reason(), Some(ExecCancelReason::Revoked));
    assert_eq!(later.cancellation.reason(), Some(ExecCancelReason::Revoked));
}

#[test]
fn perm_task_expiry_ends_sessions() {
    let host = identity("host", "Host", 3);
    let client = identity("client", "Client", 5);
    let end = NOW + 3_600;
    let mut grant = granted(&[(ExecCommands, Allow), (ExecTerminal, Allow)]);
    let until = AccessGrantChange {
        expiry: Some(Expiry::At(end)),
        ..AccessGrantChange::default()
    };
    grant.apply_access_change(&until, 2).unwrap();
    let state = state(&host, &client, grant.clone());
    let proven = ProvenPrincipal::from(&principal(&host, &client));

    // Every check applies the end: decisions, handshake and the next end.
    assert_eq!(
        capability_for(&state, &proven, ExecCommands, end - 1).decision,
        Allow
    );
    assert_eq!(
        capability_for(&state, &proven, ExecCommands, end).decision,
        Off
    );
    assert!(exec_connection_allowed(&grant, end - 1));
    assert!(!exec_connection_allowed(&grant, end));
    let (server, hello) = hellos(&host, &client);
    assert!(authorize_client_hello_in(&server, &hello, &client.node_id, &state, end - 1).is_ok());
    assert_eq!(
        authorize_client_hello_in(&server, &hello, &client.node_id, &state, end).err(),
        Some(ExecRejection::NotAuthorized)
    );
    assert_eq!(next_expiry(&state, end - 10), Some(end));
    assert_eq!(next_expiry(&state, end), None);

    // The end restricts every capability of the device, not its files.
    assert!(expired_restrictions(&state, end - 10, end - 1).is_empty());
    let ended = expired_restrictions(&state, end - 1, end);
    let reasons: Vec<RestrictionReason> = ended.items().iter().map(|item| item.reason).collect();
    assert_eq!(reasons, [RestrictionReason::GrantExpired]);
    let (key, node) = (client.public_key.as_str(), client.node_id.as_str());
    for capability in Capability::ALL {
        assert!(ended.affects_capability("direct", "", key, node, capability));
    }
    assert!(!ended.affects("direct", "", key, node));

    // A running job ends as revoked.
    let registry = ExecRegistry::new(ExecRegistryLimits::default());
    let exec_principal = principal(&host, &client);
    registry
        .apply_authorization(&exec_principal, grant.policy_revision, 1, true)
        .unwrap();
    let job = reserve(&registry, &exec_principal, grant.policy_revision, 1);
    registry.restrict_authorization(1, &ended).unwrap();
    assert_eq!(job.cancellation.reason(), Some(ExecCancelReason::Revoked));

    // The watch of a running job at the host clock: an end that has come
    // ends the job; an end moved meanwhile keeps it with the new end.
    let now = now_secs();
    let watched = |end: i64| {
        let mut grant = grant.clone();
        let change = AccessGrantChange {
            expiry: Some(Expiry::At(end)),
            ..AccessGrantChange::default()
        };
        grant.apply_access_change(&change, 3).unwrap();
        Arc::new(Mutex::new(state.clone_with_exec(grant)))
    };
    let registry = ExecRegistry::new(ExecRegistryLimits::default());
    registry
        .apply_authorization(&exec_principal, grant.policy_revision + 1, 1, true)
        .unwrap();
    let job = reserve(&registry, &exec_principal, grant.policy_revision + 1, 1);
    let passed = GrantEndWatch::new(
        watched(now - 1),
        proven.clone(),
        ExecCommands,
        Some(now - 1),
    );
    assert!(passed
        .unwrap()
        .expire(&registry, &job.lease.exec_id)
        .is_none());
    assert_eq!(job.cancellation.reason(), Some(ExecCancelReason::Revoked));
    let moved = GrantEndWatch::new(watched(now + 3_600), proven, ExecCommands, Some(now - 1));
    let other = reserve(&registry, &exec_principal, grant.policy_revision + 1, 1);
    let next = moved.unwrap().expire(&registry, &other.lease.exec_id);
    assert_eq!(next.map(|watch| watch.end()), Some(now + 3_600));
    assert_eq!(other.cancellation.reason(), None);
}

/// The test state with another grant for the one device.
trait WithExec {
    fn clone_with_exec(&self, exec: ExecGrant) -> ShareAuthState;
}

impl WithExec for ShareAuthState {
    fn clone_with_exec(&self, exec: ExecGrant) -> ShareAuthState {
        let mut state = self.clone();
        state.direct_grants[0].exec = exec;
        state
    }
}

#[test]
fn perm_task_allow_own_ls_runs() {
    let host = identity("host", "Host", 3);
    let client = identity("client", "Client", 5);
    let mut profiles = ShareProfiles::default();
    profiles
        .direct_grants
        .push(direct_grant(&client, ExecGrant::default()));
    let target = ExecGrantTarget::Direct {
        device_id: client.device_id.clone(),
        public_key: client.public_key.clone(),
        fingerprint: client.fingerprint.clone(),
        node_id: client.node_id.clone(),
    };
    // One decision: the preset "Volle Rechte als mein Konto".
    let preset = AccessPreset::FullOwn.change(Vec::new(), false);
    let (mutation, replaced) = ExecGrantMutation::prepare_access(
        &mut profiles,
        &host,
        target.clone(),
        &preset,
        Some(0),
        NOW,
    )
    .unwrap();
    assert_eq!(replaced, 0);
    let grant = mutation.policy;
    // The same grant every older surface makes by enabling.
    let (enabled, _) =
        ExecGrantMutation::prepare_persisted(&mut profiles, &host, target, true, None, NOW)
            .unwrap();
    assert_eq!(enabled.policy, grant);
    let access = grant.access_grant();
    assert_eq!(access.account, AccountProfile::Own);
    assert_eq!(access.decision(ExecCommands), Allow);
    assert_eq!(access.decision(ExecTerminal), Allow);
    assert!(grant.programs.is_empty() && access.valid_until.is_none());

    // The device connects, and `ls` as a program or a shell command is
    // allowed without any per-command permission and without a program check.
    let state = state(&host, &client, grant.clone());
    let (server, hello) = hellos(&host, &client);
    let authorized = authorize_client_hello_in(&server, &hello, &client.node_id, &state, NOW)
        .expect("the allowed device is admitted");
    let proven = ProvenPrincipal::from(&authorized.principal);
    let search = crate::share::exec_platform::program_search();
    for command in [
        ExecCommand::Argv {
            program: "ls".into(),
            args: vec!["-la".into()],
        },
        ExecCommand::Shell {
            command: "ls -la".into(),
        },
    ] {
        let start = start(command);
        let capability = start_capability(&start.command);
        assert_eq!(capability, ExecCommands);
        assert_eq!(
            capability_for(&state, &proven, capability, NOW).decision,
            Allow
        );
        assert_eq!(grant.authorize_start(&start, &search), Ok(None));
    }
    // Commands run as the worker's own account: there is nothing to switch.
    assert!(crate::share::exec_platform::resolve_run_as(&grant.run_as)
        .unwrap()
        .is_none());
    let view = ExecAccessView::of(&grant, "alice");
    assert_eq!(view.account_label, "alice");
    assert_eq!((view.commands, view.terminal), (Allow, Allow));
    assert!(!view.programs_restricted);
}
