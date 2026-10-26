//! Existing deadline/identity/account lifecycle regression bodies.
use super::*;

pub(super) fn p18d_deadline_wakes_without_screen_and_later_admission_wakes_idle_timer() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let (auth, principal) = fixture();
        let host = NodeAccess::new();
        let watcher = tokio::spawn(access_expiry(Arc::downgrade(&auth), host.registry.clone()));
        // Let the watcher subscribe with an empty registry first.
        tokio::task::yield_now().await;
        let mut state = auth.lock().unwrap();
        let mut next = constraints(&state, &principal);
        next.app_launch.as_mut().unwrap().valid_until = Some(now_secs() + 2);
        replace_constraints(&mut state, next);
        drop(state);
        let handle = admit(
            &host,
            &auth,
            &principal,
            app("editor", "session-a", "alice"),
        );
        let mut stop = handle.revoked();
        tokio::time::timeout(Duration::from_secs(4), stop.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(handle.reason(), Some(AccessRevocation::Expired));
        host.stop();
        tokio::time::timeout(Duration::from_secs(1), watcher)
            .await
            .unwrap()
            .unwrap();
    });
}

pub(super) fn p18d_account_and_earlier_deadline_changes_revoke_only_affected_leases() {
    let (auth, principal) = fixture();
    let host = NodeAccess::new();
    let app_handle = admit(
        &host,
        &auth,
        &principal,
        app("editor", "session-a", "alice"),
    );
    let request = tcp(&auth.lock().unwrap(), &principal, 3389);
    let tcp_handle = admit(&host, &auth, &principal, request);
    let mut state = auth.lock().unwrap();
    state.direct_grants[0]
        .exec
        .apply_access_change(
            &AccessGrantChange {
                account: Some(crate::share::access_grant::AccountProfile::Remote),
                ..Default::default()
            },
            now_secs(),
        )
        .unwrap();
    host.registry.recheck(&state, now_secs());
    assert_eq!(app_handle.reason(), Some(AccessRevocation::AccountChanged));
    assert_eq!(tcp_handle.reason(), None);
    let mut next = constraints(&state, &principal);
    next.tcp_forward[0].valid_until = Some(now_secs() + 3600);
    replace_constraints(&mut state, next);
    host.registry.recheck(&state, now_secs());
    assert_eq!(tcp_handle.reason(), Some(AccessRevocation::EarlierDeadline));
}

pub(super) fn p18d_failed_persisted_identity_check_keeps_live_authority_unchanged() {
    let (auth, principal) = fixture();
    let host = NodeAccess::new();
    let handle = admit(
        &host,
        &auth,
        &principal,
        app("editor", "session-a", "alice"),
    );
    let before = auth.lock().unwrap().clone();
    let registry = ExecRegistry::new(ExecRegistryLimits::default());
    let target = ExecGrantTarget::Direct {
        device_id: principal.device_id.clone(),
        public_key: principal.public_key.clone(),
        fingerprint: principal.fingerprint.clone(),
        node_id: principal.node_id.clone(),
    };
    let mut wrong = principal.clone();
    wrong.node_id = "wrong-node".into();
    let result = crate::share::exec_grant_runtime::apply_exact_with_access(
        &auth,
        &registry,
        Some(host.registry()),
        target,
        (&wrong).into(),
        ExecGrant::default(),
    );
    assert!(result.is_err());
    assert_eq!(handle.reason(), None);
    let state = auth.lock().unwrap();
    assert_eq!(state.direct_grants[0].exec, before.direct_grants[0].exec);
    assert_eq!(state.authorization_epoch, before.authorization_epoch);
}

pub(super) fn p18d_room_login_is_denied_even_with_an_explicit_matching_room_grant() {
    use crate::share::types::{RoomMember, RoomProfile, ShareStatus};
    let (auth, mut principal) = fixture();
    let mut state = auth.lock().unwrap();
    let policy = state.direct_grants[0].exec.clone();
    state.rooms.push(RoomProfile {
        id: "room-profile".into(),
        name: "room".into(),
        room_id: "room-relation".into(),
        auto_join: true,
        last_seen: None,
        status: ShareStatus::Waiting,
        members: vec![RoomMember {
            device_id: principal.device_id.clone(),
            device_name: principal.device_name.clone(),
            public_key: principal.public_key.clone(),
            fingerprint: principal.fingerprint.clone(),
            node_id: principal.node_id.clone(),
            relay_url: String::new(),
            candidates: Vec::new(),
            last_seen: None,
            status: ShareStatus::Waiting,
            blocked: false,
            exec: policy,
            presence: None,
            relation: Default::default(),
        }],
        exports: Default::default(),
        policy: crate::share::RoomPolicy::new_room(),
    });
    principal.relation_kind = "room".into();
    principal.relation_id = "room-relation".into();
    assert!(crate::share::access_authorize::principal_grant(&state, &principal).is_some());
    drop(state);
    let request = AccessRequest::LoginAccess(LoginAccessRequest {
        session: SessionId::parse("session-a").unwrap(),
        user: UserId::parse("alice").unwrap(),
        operation: LoginOperation::Unlock,
    });
    let host = NodeAccess::new();
    let result = host.admit(&auth, &principal, request, None, |_| Ok(()));
    assert!(matches!(result, Err(AccessRevocation::PermissionWithdrawn)));
    assert!(host
        .admit(
            &auth,
            &principal,
            app("editor", "session-a", "alice"),
            None,
            |_| Ok(())
        )
        .is_ok());
}
