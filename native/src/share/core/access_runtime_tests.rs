use super::*;
use crate::share::access_constraints::{
    AccessConstraints, AppId, AppLaunchConstraints, AppLaunchRequest, LoginAccessConstraints,
    LoginAccessRequest, LoginOperation, NodeId, TcpDirection, TcpForwardRequest, TcpForwardRule,
    TcpHost, TcpPort,
};
use crate::share::access_grant::{AccessGrantChange, Capability, CapabilityDecision};
use crate::share::exec_policy::ExecGrant;
use crate::share::exec_registry::{ExecRegistry, ExecRegistryLimits};
use crate::share::identity::ShareIdentity;
use crate::share::types::{DirectGrant, DirectGrantState, ExecGrantTarget};

fn identity(name: &str, byte: u8) -> ShareIdentity {
    let secret = iroh::SecretKey::from_bytes(&[byte; 32]);
    let key = secret.public().to_string();
    ShareIdentity {
        device_id: name.into(),
        device_name: name.into(),
        direct_lookup_id: name.into(),
        fingerprint: crate::share::core::public_fingerprint(key.as_bytes()),
        public_key: key.clone(),
        node_id: key,
        iroh_secret: secret,
        direct_secret: [byte; 32],
    }
}
fn app(name: &str, session: &str, user: &str) -> AccessRequest {
    AccessRequest::AppLaunch(AppLaunchRequest {
        app_id: AppId::parse(name).unwrap(),
        session: SessionId::parse(session).unwrap(),
        user: UserId::parse(user).unwrap(),
    })
}
fn constraints(state: &ShareAuthState, principal: &ProvenPrincipal) -> AccessConstraints {
    AccessConstraints {
        app_launch: Some(AppLaunchConstraints {
            app_ids: vec![
                AppId::parse("editor").unwrap(),
                AppId::parse("viewer").unwrap(),
            ],
            sessions: vec![
                SessionId::parse("session-a").unwrap(),
                SessionId::parse("session-b").unwrap(),
            ],
            users: vec![
                UserId::parse("alice").unwrap(),
                UserId::parse("bob").unwrap(),
            ],
            valid_until: None,
        }),
        login_access: Some(LoginAccessConstraints {
            sessions: vec![SessionId::parse("session-a").unwrap()],
            users: vec![UserId::parse("alice").unwrap()],
            operations: vec![LoginOperation::Unlock],
            valid_until: None,
        }),
        tcp_forward: vec![TcpForwardRule {
            initiator: NodeId::parse(&principal.node_id).unwrap(),
            target: NodeId::parse(&state.identity.node_id).unwrap(),
            direction: TcpDirection::PeerToHost,
            host: TcpHost::Loopback,
            ports: vec![
                TcpPort::try_from(3389).unwrap(),
                TcpPort::try_from(5900).unwrap(),
            ],
            valid_until: None,
        }],
    }
}
fn fixture() -> (Arc<Mutex<ShareAuthState>>, ProvenPrincipal) {
    let host = identity("host", 3);
    let peer = identity("peer", 5);
    let principal = ProvenPrincipal {
        relation_kind: "direct".into(),
        relation_id: host.direct_lookup_id.clone(),
        device_id: peer.device_id.clone(),
        device_name: peer.device_name.clone(),
        public_key: peer.public_key.clone(),
        fingerprint: peer.fingerprint.clone(),
        node_id: peer.node_id.clone(),
    };
    let mut state = ShareAuthState {
        identity: host,
        direct_secret: vec![7; 32],
        default_direct_exports: Default::default(),
        direct_contacts: Vec::new(),
        direct_grants: Vec::new(),
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 1,
    };
    let mut policy = ExecGrant::default();
    policy
        .apply_access_change(
            &AccessGrantChange {
                capabilities: Capability::ALL
                    .into_iter()
                    .map(|capability| (capability, CapabilityDecision::Allow))
                    .collect(),
                constraints: Some(constraints(&state, &principal)),
                ..Default::default()
            },
            now_secs(),
        )
        .unwrap();
    state.direct_grants.push(DirectGrant {
        device_id: peer.device_id,
        device_name: peer.device_name,
        public_key: peer.public_key,
        fingerprint: peer.fingerprint,
        node_id: peer.node_id,
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: policy,
        write: true,
    });
    (Arc::new(Mutex::new(state)), principal)
}
fn admit(
    host: &NodeAccess,
    auth: &Mutex<ShareAuthState>,
    principal: &ProvenPrincipal,
    request: AccessRequest,
) -> AccessHandle {
    host.admit(auth, principal, request, None, |_| Ok(()))
        .unwrap()
        .1
}
fn replace_constraints(state: &mut ShareAuthState, constraints: AccessConstraints) {
    state.direct_grants[0]
        .exec
        .apply_access_change(
            &AccessGrantChange {
                constraints: Some(constraints),
                ..Default::default()
            },
            now_secs(),
        )
        .unwrap();
}
fn tcp(state: &ShareAuthState, principal: &ProvenPrincipal, port: u16) -> AccessRequest {
    AccessRequest::TcpForward(TcpForwardRequest {
        initiator: NodeId::parse(&principal.node_id).unwrap(),
        target: NodeId::parse(&state.identity.node_id).unwrap(),
        direction: TcpDirection::PeerToHost,
        host: TcpHost::Loopback,
        resolved_ip: "127.0.0.1".parse().unwrap(),
        port: TcpPort::try_from(port).unwrap(),
    })
}

#[test]
fn p18d_targeted_recheck_preserves_unrelated_app_user_port_and_old_rights() {
    let (auth, principal) = fixture();
    let host = NodeAccess::new();
    let removed = admit(
        &host,
        &auth,
        &principal,
        app("editor", "session-a", "alice"),
    );
    let kept = admit(&host, &auth, &principal, app("viewer", "session-b", "bob"));
    let tcp_a = tcp(&auth.lock().unwrap(), &principal, 3389);
    let tcp_b = tcp(&auth.lock().unwrap(), &principal, 5900);
    let removed_port = admit(&host, &auth, &principal, tcp_a);
    let kept_port = admit(&host, &auth, &principal, tcp_b);
    let mut state = auth.lock().unwrap();
    let before = state.clone();
    let mut next = constraints(&state, &principal);
    next.app_launch.as_mut().unwrap().app_ids.remove(0);
    next.app_launch.as_mut().unwrap().users.remove(0);
    next.tcp_forward[0].ports.remove(0);
    replace_constraints(&mut state, next);
    assert_eq!(host.registry.recheck(&state, now_secs()), 2);
    assert_eq!(
        removed.reason(),
        Some(AccessRevocation::PermissionWithdrawn)
    );
    assert_eq!(
        removed_port.reason(),
        Some(AccessRevocation::PermissionWithdrawn)
    );
    assert_eq!(kept.reason(), None);
    assert_eq!(kept_port.reason(), None);
    for capability in [
        Capability::ExecCommands,
        Capability::ExecTerminal,
        Capability::ScreenView,
        Capability::ScreenControl,
    ] {
        assert_eq!(
            before.direct_grants[0]
                .exec
                .decision_at(capability, now_secs()),
            state.direct_grants[0]
                .exec
                .decision_at(capability, now_secs())
        );
    }
    assert!(state.direct_grants[0].write);
    assert_eq!(state.default_direct_exports, before.default_direct_exports);
}

#[test]
fn p18d_persisted_widening_keeps_lease_narrowing_revokes_before_publication() {
    let (auth, principal) = fixture();
    let host = NodeAccess::new();
    let handle = admit(
        &host,
        &auth,
        &principal,
        app("editor", "session-a", "alice"),
    );
    let registry = ExecRegistry::new(ExecRegistryLimits::default());
    crate::share::exec_grant_runtime::seed_registry(&auth, &registry).unwrap();
    let target = ExecGrantTarget::Direct {
        device_id: principal.device_id.clone(),
        public_key: principal.public_key.clone(),
        fingerprint: principal.fingerprint.clone(),
        node_id: principal.node_id.clone(),
    };
    let mut next = auth.lock().unwrap().clone();
    let mut widened = constraints(&next, &principal);
    widened
        .app_launch
        .as_mut()
        .unwrap()
        .app_ids
        .push(AppId::parse("browser").unwrap());
    replace_constraints(&mut next, widened.clone());
    let apply = |policy| {
        crate::share::exec_grant_runtime::apply_exact_with_access(
            &auth,
            &registry,
            Some(host.registry()),
            target.clone(),
            (&principal).into(),
            policy,
        )
    };
    apply(next.direct_grants[0].exec.clone()).unwrap();
    assert_eq!(handle.reason(), None);
    widened.app_launch.as_mut().unwrap().app_ids.remove(0);
    replace_constraints(&mut next, widened);
    apply(next.direct_grants[0].exec.clone()).unwrap();
    assert_eq!(handle.reason(), Some(AccessRevocation::PermissionWithdrawn));
    assert_eq!(
        crate::share::access_authorize::request_for(
            &auth.lock().unwrap(),
            &principal,
            &app("editor", "session-a", "alice"),
            now_secs()
        )
        .decision,
        CapabilityDecision::Off
    );
}

#[test]
fn p18d_session_user_and_relation_changes_end_only_bound_operations() {
    let (auth, principal) = fixture();
    let host = NodeAccess::new();
    let a = admit(
        &host,
        &auth,
        &principal,
        app("editor", "session-a", "alice"),
    );
    let b = admit(&host, &auth, &principal, app("editor", "session-b", "bob"));
    let login = admit(
        &host,
        &auth,
        &principal,
        AccessRequest::LoginAccess(LoginAccessRequest {
            session: SessionId::parse("session-a").unwrap(),
            user: UserId::parse("alice").unwrap(),
            operation: LoginOperation::Unlock,
        }),
    );
    let forward = tcp(&auth.lock().unwrap(), &principal, 3389);
    let forward = admit(&host, &auth, &principal, forward);
    assert_eq!(
        host.invalidate_session(&auth, &SessionId::parse("session-a").unwrap(), || {
            assert!(auth.try_lock().is_err());
            assert_eq!(a.reason(), Some(AccessRevocation::SessionChanged));
        })
        .unwrap()
        .0,
        2
    );
    assert_eq!(a.reason(), Some(AccessRevocation::SessionChanged));
    assert_eq!(login.reason(), Some(AccessRevocation::SessionChanged));
    assert_eq!(b.reason(), None);
    assert_eq!(forward.reason(), None);
    assert_eq!(
        host.invalidate_user(&auth, &UserId::parse("alice").unwrap(), || ())
            .unwrap()
            .0,
        0
    );
    assert_eq!(
        host.invalidate_user(&auth, &UserId::parse("bob").unwrap(), || ())
            .unwrap()
            .0,
        1
    );
    assert_eq!(b.reason(), Some(AccessRevocation::UserChanged));
    let mut state = auth.lock().unwrap();
    state.direct_online = false;
    host.registry.recheck(&state, now_secs());
    assert_eq!(forward.reason(), Some(AccessRevocation::RelationEnded));
}

#[test]
fn p18d_admission_holds_auth_through_registration_and_resource_publication() {
    let (auth, principal) = fixture();
    let host = NodeAccess::new();
    let result = host
        .admit(
            &auth,
            &principal,
            app("editor", "session-a", "alice"),
            None,
            |handle| {
                assert!(auth.try_lock().is_err());
                assert!(host.lifecycle.try_lock().is_err());
                assert_eq!(handle.reason(), None);
                Ok(42)
            },
        )
        .unwrap();
    assert_eq!(result.0, 42);
    host.stop();
    assert_eq!(result.1.reason(), Some(AccessRevocation::HostStopped));
    let denied = host.admit(
        &auth,
        &principal,
        app("editor", "session-a", "alice"),
        None,
        |_| -> Result<(), AccessRevocation> { panic!("stopped host admitted a resource") },
    );
    assert!(matches!(denied, Err(AccessRevocation::HostStopped)));
}

#[test]
fn p18d_poisoned_auth_denies_without_panic_or_resource_publication() {
    let (auth, principal) = fixture();
    let poisoned = auth.clone();
    let _ = std::thread::spawn(move || {
        let _state = poisoned.lock().unwrap();
        panic!("poison isolated test state");
    })
    .join();
    let host = NodeAccess::new();
    let result = host.admit(
        &auth,
        &principal,
        app("editor", "session-a", "alice"),
        None,
        |_| -> Result<(), AccessRevocation> { panic!("poisoned state admitted a resource") },
    );
    assert!(matches!(result, Err(AccessRevocation::Unavailable)));
}

#[path = "access_runtime_lifecycle_tests.rs"]
mod lifecycle;

#[test]
fn p18d_deadline_wakes_without_screen_and_later_admission_wakes_idle_timer() {
    lifecycle::p18d_deadline_wakes_without_screen_and_later_admission_wakes_idle_timer();
}

#[test]
fn p18d_account_and_earlier_deadline_changes_revoke_only_affected_leases() {
    lifecycle::p18d_account_and_earlier_deadline_changes_revoke_only_affected_leases();
}

#[test]
fn p18d_failed_persisted_identity_check_keeps_live_authority_unchanged() {
    lifecycle::p18d_failed_persisted_identity_check_keeps_live_authority_unchanged();
}

#[test]
fn p18d_room_login_is_denied_even_with_an_explicit_matching_room_grant() {
    lifecycle::p18d_room_login_is_denied_even_with_an_explicit_matching_room_grant();
}
