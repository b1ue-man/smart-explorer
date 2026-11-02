//! B7 fixtures: real signatures and authorization; no broker/OS process starts.
use super::{
    authority::Authority,
    gateway_types::{ClientHello, GatewayAccess},
    model::*,
};

fn fixture(desktop: DesktopKind) -> (iroh::SecretKey, Policy, SessionBinding) {
    let key = iroh::SecretKey::from_bytes(&[17; 32]);
    let machine = iroh::SecretKey::from_bytes(&[29; 32]);
    let user_id = if matches!(&desktop, DesktopKind::BeforeLogin | DesktopKind::X11Greeter) {
        None
    } else {
        Some("user-1".into())
    };
    let binding = SessionBinding {
        session_id: "session-7".into(),
        user_id,
        session_generation: "logon-1".into(),
        desktop_generation: "desktop-1".into(),
        desktop,
    };
    let policy = Policy {
        format: VERSION,
        revision: 3,
        machine_id: format!("login:{}", machine.public()),
        machine_public_key: machine.public().to_string(),
        gateway_identity: "gateway-service".into(),
        grants: vec![PeerGrant {
            peer_id: "peer-1".into(),
            public_key: key.public().to_string(),
            target_users: vec!["user-1".into()],
            not_before: 90,
            expires_at: 200,
            rights: LoginRights {
                screen_view: true,
                screen_control: true,
                login: true,
                before_login: true,
                locked: true,
                uac: true,
                sas: true,
            },
        }],
    };
    policy.validate().unwrap();
    (key, policy, binding)
}

fn authority(policy: &Policy, binding: &SessionBinding) -> Authority {
    Authority::new(
        policy,
        "boot-1".into(),
        binding.clone(),
        "peer-1".into(),
        "user-1".into(),
        "ab".repeat(32),
        1,
        100,
    )
}

fn signed(authority: &Authority, key: &iroh::SecretKey, operation: Operation) -> SignedRequest {
    let mut request = SignedRequest {
        challenge: authority.challenge().clone(),
        peer_id: "peer-1".into(),
        target_user: "user-1".into(),
        operation,
        signature: String::new(),
    };
    request.sign(key).unwrap();
    request
}

#[test]
fn batches_login_signed_operation_is_consumed_before_effect_and_cannot_be_replayed() {
    let (key, policy, binding) = fixture(DesktopKind::Locked);
    let mut authority = authority(&policy, &binding);
    let request = signed(&authority, &key, Operation::Capture { frame_id: 7 });
    assert_eq!(
        authority.authorize(&request, &policy, &binding, 101),
        Ok(())
    );
    // Even if the privileged effect subsequently fails, no success callback is
    // needed to consume the exact signed operation.
    assert_eq!(
        authority
            .authorize(&request, &policy, &binding, 101)
            .unwrap_err()
            .state,
        "session_changed"
    );
}

#[test]
fn batches_login_signature_binds_operation_and_authenticated_transport_peer() {
    let (key, policy, binding) = fixture(DesktopKind::Locked);
    let mut authority = authority(&policy, &binding);
    let request = signed(&authority, &key, Operation::Capture { frame_id: 7 });
    let hello = ClientHello {
        format: VERSION,
        peer_id: "peer-1".into(),
        target_user: "user-1".into(),
        route_policy: VERSION,
    };
    assert!(hello
        .authorize(&policy, &key.public().to_string(), 101)
        .is_ok());
    let stranger = iroh::SecretKey::from_bytes(&[31; 32]);
    assert!(hello
        .authorize(&policy, &stranger.public().to_string(), 101)
        .is_err());
    assert!(hello
        .check_operation(&request, &key.public().to_string(), authority.challenge())
        .is_ok());
    let mut changed = request.clone();
    changed.operation = Operation::SecureAttention;
    assert!(hello
        .check_operation(&changed, &key.public().to_string(), authority.challenge())
        .is_err());
    assert_eq!(
        authority
            .authorize(&changed, &policy, &binding, 101)
            .unwrap_err()
            .state,
        "invalid_request"
    );
    let mut forged = request.clone();
    forged.sign(&stranger).unwrap();
    assert_eq!(
        authority
            .authorize(&forged, &policy, &binding, 101)
            .unwrap_err()
            .state,
        "invalid_request"
    );
    assert!(authority
        .authorize(&request, &policy, &binding, 101)
        .is_ok());
}

#[test]
fn batches_login_old_boot_policy_session_desktop_and_user_are_rejected() {
    let (key, policy, binding) = fixture(DesktopKind::Locked);
    for case in 0..9 {
        let mut authority = authority(&policy, &binding);
        let mut request = signed(&authority, &key, Operation::Start);
        let mut current_policy = policy.clone();
        let mut current_binding = binding.clone();
        match case {
            0 => request.challenge.boot_generation = "boot-2".into(),
            1 => request.challenge.sequence += 1,
            2 => request.challenge.nonce = "cd".repeat(32),
            3 => current_policy.revision += 1,
            4 => current_binding.session_generation = "logon-2".into(),
            5 => current_binding.desktop_generation = "desktop-2".into(),
            6 => current_binding.user_id = Some("user-2".into()),
            7 => request.peer_id = "peer-2".into(),
            8 => request.target_user = "user-2".into(),
            _ => unreachable!(),
        }
        request.sign(&key).unwrap();
        assert_eq!(
            authority
                .authorize(&request, &current_policy, &current_binding, 101)
                .unwrap_err()
                .state,
            "session_changed",
            "case {case}"
        );
    }
    let mut expired = authority(&policy, &binding);
    let request = signed(&expired, &key, Operation::Start);
    assert_eq!(
        expired
            .authorize(&request, &policy, &binding, 131)
            .unwrap_err()
            .state,
        "session_changed"
    );
}

#[test]
fn batches_login_each_desktop_and_control_right_is_independently_required() {
    for (desktop, operation, right) in [
        (DesktopKind::BeforeLogin, Operation::Start, 0),
        (DesktopKind::Locked, Operation::Start, 1),
        (DesktopKind::Uac, Operation::Start, 2),
        (DesktopKind::X11Greeter, Operation::Start, 0),
        (DesktopKind::Locked, Operation::ReleaseInputs, 3),
        (DesktopKind::Locked, Operation::SecureAttention, 4),
        (DesktopKind::Locked, Operation::Start, 5),
        (DesktopKind::Locked, Operation::Start, 6),
    ] {
        let (key, mut policy, binding) = fixture(desktop);
        let mut admitted = authority(&policy, &binding);
        let request = signed(&admitted, &key, operation.clone());
        assert!(admitted.authorize(&request, &policy, &binding, 101).is_ok());
        let rights = &mut policy.grants[0].rights;
        match right {
            0 => rights.before_login = false,
            1 => rights.locked = false,
            2 => rights.uac = false,
            3 => rights.screen_control = false,
            4 => rights.sas = false,
            5 => rights.screen_view = false,
            6 => rights.login = false,
            _ => unreachable!(),
        }
        let mut refused = authority(&policy, &binding);
        let request = signed(&refused, &key, operation);
        assert_eq!(
            refused
                .authorize(&request, &policy, &binding, 101)
                .unwrap_err()
                .state,
            "right_missing"
        );
    }
    for (desktop, state) in [
        (DesktopKind::WaylandGreeter, "platform_unsupported"),
        (DesktopKind::UserSession, "user_session_active"),
        (DesktopKind::Unavailable, "session_unavailable"),
    ] {
        let (key, policy, binding) = fixture(desktop);
        let mut authority = authority(&policy, &binding);
        let request = signed(&authority, &key, Operation::Start);
        assert_eq!(
            authority
                .authorize(&request, &policy, &binding, 101)
                .unwrap_err()
                .state,
            state
        );
    }
}

#[test]
fn batches_login_revoked_and_expired_grants_never_inherit_screen_permission() {
    let (key, policy, binding) = fixture(DesktopKind::Locked);
    for case in 0..4 {
        let mut authority = authority(&policy, &binding);
        let request = signed(&authority, &key, Operation::Start);
        let mut revoked = policy.clone();
        match case {
            0 => revoked.grants.clear(),
            1 => revoked.grants[0].expires_at = 101,
            2 => revoked.grants[0].not_before = 102,
            3 => revoked.grants[0].target_users = vec!["user-2".into()],
            _ => unreachable!(),
        }
        assert_eq!(
            authority
                .authorize(&request, &revoked, &binding, 101)
                .unwrap_err()
                .state,
            "right_missing"
        );
    }
}

#[test]
fn batches_login_presented_frame_cannot_cross_geometry_or_desktop_generation() {
    use super::viewer_guard::ViewerGuard;
    let (_, policy, binding) = fixture(DesktopKind::Locked);
    let authority = authority(&policy, &binding);
    let grant = &policy.grants[0];
    let access = GatewayAccess {
        peer_id: grant.peer_id.clone(),
        public_key: grant.public_key.clone(),
        target_user: "user-1".into(),
        policy_revision: policy.revision,
        not_before: grant.not_before,
        expires_at: grant.expires_at,
        rights: grant.rights.clone(),
    };
    let mut guard = ViewerGuard::default();
    guard.adopt(authority.challenge(), &access).unwrap();
    guard.focused = true;
    guard.control = true;
    let frame = guard.frame(1, 640, 480).unwrap();
    assert!(!guard.input_ready(&frame));
    assert!(guard.presented(&frame, guard.epoch));
    assert!(guard.input_ready(&frame));
    let old_epoch = guard.epoch;
    let resized = guard.frame(2, 800, 600).unwrap();
    assert!(!guard.input_ready(&frame));
    assert!(!guard.presented(&resized, old_epoch));
    assert!(guard.presented(&resized, guard.epoch));
    assert!(guard.input_ready(&resized));
    let mut changed = authority.challenge().clone();
    changed.binding.desktop_generation = "desktop-2".into();
    assert!(guard.adopt(&changed, &access).is_err());
    assert!(guard.closed);
    assert!(!guard.input_ready(&resized));
    assert!(guard.frame(3, 800, 600).is_err());
}
