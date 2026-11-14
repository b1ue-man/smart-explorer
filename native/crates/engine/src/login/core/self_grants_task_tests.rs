//! E21 grant channel: own account only, revocation, network record.
use super::*;
use crate::login::core::{
    authority::Authority,
    machine::MachineIdentity,
    model::{DesktopKind, Operation, SessionBinding, SignedRequest, VERSION},
};

fn key(seed: u8) -> String {
    iroh::SecretKey::from_bytes(&[seed; 32])
        .public()
        .to_string()
}

fn device(seed: u8, valid_until: Option<u64>) -> SteeringGrant {
    SteeringGrant {
        peer_id: format!("device-{seed}"),
        public_key: key(seed),
        valid_until,
    }
}

fn admin_grant(seed: u8, users: &[&str]) -> PeerGrant {
    PeerGrant {
        peer_id: format!("device-{seed}"),
        public_key: key(seed),
        target_users: users.iter().map(|user| user.to_string()).collect(),
        not_before: 0,
        expires_at: 4_000_000_000,
        rights: ConsoleAbilities {
            before_login: true,
            locked: true,
            uac: false,
            sas: false,
            console: true,
        }
        .rights(),
    }
}

fn policy(grants: Vec<PeerGrant>) -> Policy {
    let identity = MachineIdentity::generate().expect("machine identity");
    Policy {
        format: VERSION,
        revision: 7,
        machine_id: identity.machine_id.clone(),
        machine_public_key: identity.public_key.clone(),
        gateway_identity: "991".into(),
        grants,
    }
}

const LINUX: ConsoleAbilities = ConsoleAbilities {
    before_login: true,
    locked: true,
    uac: false,
    sas: false,
    console: true,
};
const NOW: u64 = 1_800_000_000;

#[test]
fn login_selbst_task_grant_only_for_own_account() {
    let start = policy(Vec::new());
    let merged = merge(
        &start,
        &Ledger::default(),
        "1000",
        &[device(1, Some(NOW + 3600)), device(2, None)],
        &LINUX,
        NOW,
    )
    .expect("merge");
    let next = merged.policy.expect("policy changed");
    assert_eq!(next.revision, start.revision + 1);
    assert_eq!(next.grants.len(), 2);
    for grant in &next.grants {
        assert_eq!(grant.target_users, vec!["1000".to_string()]);
        assert!(grant.rights.login && grant.rights.screen_view && grant.rights.screen_control);
        assert!(grant.rights.before_login && grant.rights.locked);
        assert!(!grant.rights.uac && !grant.rights.sas);
    }
    let timed = next.grants.iter().find(|grant| grant.peer_id == "device-1");
    assert_eq!(timed.map(|grant| grant.expires_at), Some(NOW + 3600));
    let open = next.grants.iter().find(|grant| grant.peer_id == "device-2");
    assert_eq!(open.map(|grant| grant.expires_at), Some(NOW + OPEN_END));
    assert_eq!(merged.ledger_after.entries.len(), 2);
    assert!(merged
        .ledger_after
        .entries
        .iter()
        .all(|entry| entry.account == "1000"));
    // The same request again changes nothing: no revision step, no session end.
    let again = merge(
        &next,
        &merged.ledger_after,
        "1000",
        &[device(1, Some(NOW + 3600)), device(2, None)],
        &LINUX,
        NOW + 60,
    )
    .expect("merge again");
    assert!(again.policy.is_none());
    // An expired steering right never becomes a login grant.
    let expired = merge(
        &start,
        &Ledger::default(),
        "1000",
        &[device(3, Some(NOW))],
        &LINUX,
        NOW,
    )
    .expect("merge expired");
    assert!(expired.policy.is_none());
}

#[test]
fn login_selbst_task_foreign_account_rejected() {
    // The wire has no field for an account: naming another one is refused.
    let named = r#"{"kind":"grants","format":1,"grants":[],"target_users":["0"]}"#;
    assert!(serde_json::from_str::<SelfRequest>(named).is_err());
    let device_named = format!(
        r#"{{"kind":"grants","format":1,"grants":[{{"peer_id":"d","public_key":"{}","valid_until":null,"target_users":["0"]}}]}}"#,
        key(4)
    );
    assert!(serde_json::from_str::<SelfRequest>(&device_named).is_err());
    // Grants of other accounts and administrative grants stay untouched,
    // also an administrative grant that names only this account.
    let start = policy(vec![admin_grant(5, &["1001"]), admin_grant(6, &["1000"])]);
    let merged = merge(&start, &Ledger::default(), "1000", &[], &LINUX, NOW).expect("merge");
    assert!(merged.policy.is_none());
    // A device another grant already names keeps that grant.
    let taken = merge(
        &start,
        &Ledger::default(),
        "1000",
        &[device(5, None)],
        &LINUX,
        NOW,
    )
    .expect("merge taken");
    assert!(taken.policy.is_none());
    assert!(taken.ledger_after.entries.is_empty());
    // An account without a console session (SSH only) adds no device,
    // but keeps and may remove the grants it already has.
    let remote = ConsoleAbilities {
        console: false,
        ..LINUX
    };
    let none = merge(
        &start,
        &Ledger::default(),
        "1000",
        &[device(8, None)],
        &remote,
        NOW,
    )
    .expect("merge without console");
    assert!(none.policy.is_none());
    let granted = merge(
        &start,
        &Ledger::default(),
        "1000",
        &[device(8, None)],
        &LINUX,
        NOW,
    )
    .expect("merge at the console");
    let granted_policy = granted.policy.expect("granted");
    let kept = merge(
        &granted_policy,
        &granted.ledger_after,
        "1000",
        &[device(8, None)],
        &remote,
        NOW + 1,
    )
    .expect("keep without console");
    assert!(kept.policy.is_none(), "an existing grant stays unchanged");
    let removed = merge(
        &granted_policy,
        &granted.ledger_after,
        "1000",
        &[],
        &remote,
        NOW + 2,
    )
    .expect("remove without console");
    assert!(removed
        .policy
        .is_some_and(|policy| policy.grants.len() == 2));
    // A caller without a valid account is refused before any change.
    for account in ["", "1000/..", "S-1-5-21 1", &"9".repeat(257)] {
        assert!(merge(&start, &Ledger::default(), account, &[], &LINUX, NOW).is_err());
    }
    // A forged key is refused as a whole request.
    let mut forged = device(7, None);
    forged.public_key = "not-a-key".into();
    assert!(merge(&start, &Ledger::default(), "1000", &[forged], &LINUX, NOW).is_err());
    let flood: Vec<SteeringGrant> = (0..=MAX_SELF_GRANTS as u8)
        .map(|seed| device(seed, None))
        .collect();
    assert!(merge(&start, &Ledger::default(), "1000", &flood, &LINUX, NOW).is_err());
}

#[test]
fn login_selbst_task_revocation_removes_grant_and_ends_session() {
    let start = policy(vec![admin_grant(9, &["1001"])]);
    let granted = merge(
        &start,
        &Ledger::default(),
        "1000",
        &[device(1, None)],
        &LINUX,
        NOW,
    )
    .expect("grant");
    let granted_policy = granted.policy.expect("granted policy");
    let binding = SessionBinding {
        session_id: "c1".into(),
        user_id: None,
        session_generation: "g".into(),
        desktop_generation: "d".into(),
        desktop: DesktopKind::X11Greeter,
    };
    let authority = Authority::new(
        &granted_policy,
        "boot".into(),
        binding.clone(),
        "device-1".into(),
        "1000".into(),
        "nonce".into(),
        1,
        NOW,
    );
    let revoked = merge(
        &granted_policy,
        &granted.ledger_after,
        "1000",
        &[],
        &LINUX,
        NOW + 5,
    )
    .expect("revoke");
    let revoked_policy = revoked.policy.expect("revoked policy");
    assert!(revoked_policy
        .grants
        .iter()
        .all(|grant| grant.peer_id != "device-1"));
    assert_eq!(
        revoked_policy.grants.len(),
        1,
        "the other account's grant stays"
    );
    assert!(revoked.ledger_after.entries.is_empty());
    assert_eq!(
        revoked.ledger_before.entries.len(),
        1,
        "written before the policy"
    );
    // The running session's next operation is refused: new revision.
    let mut running = authority;
    let request = SignedRequest {
        challenge: running.challenge().clone(),
        peer_id: "device-1".into(),
        target_user: "1000".into(),
        operation: Operation::Start,
        signature: String::new(),
    };
    let denial = running
        .authorize(&request, &revoked_policy, &binding, NOW + 6)
        .expect_err("revoked session must end");
    assert_eq!(denial.state, "session_changed");
    // A fresh challenge on the new policy finds no grant for the device.
    let mut fresh = Authority::new(
        &revoked_policy,
        "boot".into(),
        binding.clone(),
        "device-1".into(),
        "1000".into(),
        "nonce2".into(),
        2,
        NOW + 6,
    );
    let request = SignedRequest {
        challenge: fresh.challenge().clone(),
        ..request
    };
    let denial = fresh
        .authorize(&request, &revoked_policy, &binding, NOW + 7)
        .expect_err("no grant left");
    assert_eq!(denial.state, "right_missing");
}

#[test]
fn login_selbst_task_network_follows_changed_server() {
    let direct = GatewayNetwork {
        format: VERSION,
        use_default_relays: false,
        relay_urls: Vec::new(),
        relay_pins: Vec::new(),
        allow_plaintext_relays: false,
    };
    let relayed = GatewayNetwork {
        relay_urls: vec!["https://relay.example.org/".into()],
        relay_pins: vec![[7u8; 32]],
        ..direct.clone()
    };
    relayed.validate().expect("valid network");
    assert!(network_differs(None, &direct));
    assert!(network_differs(Some(&direct), &relayed));
    assert!(!network_differs(Some(&relayed), &relayed));
    let request = SelfRequest::Network {
        network: relayed.clone(),
    };
    let wire = serde_json::to_string(&request).expect("encode");
    assert_eq!(
        serde_json::from_str::<SelfRequest>(&wire).expect("decode"),
        request
    );
    // An account without an own grant may not change the machine's relays.
    let ledger = Ledger {
        format: SELF_FORMAT,
        entries: vec![LedgerEntry {
            account: "1000".into(),
            peer_id: "device-1".into(),
            public_key: key(1),
        }],
    };
    assert!(ledger.holds("1000"));
    assert!(!ledger.holds("1001"));
}

#[test]
fn login_selbst_task_device_named_twice_keeps_one_grant() {
    // A device steers through a Direct grant and a room membership with
    // different ends: the broker refuses a request that names a device
    // twice, so the list keeps one entry with the later end.
    let both = vec![
        device(1, Some(NOW + 60)),
        device(1, Some(NOW + 3600)),
        device(2, Some(NOW + 3600)),
        device(2, None),
        device(3, Some(NOW + 60)),
        device(3, Some(NOW + 60)),
    ];
    let one = one_per_device(both);
    assert_eq!(
        one,
        vec![
            device(1, Some(NOW + 3600)),
            device(2, None),
            device(3, Some(NOW + 60))
        ]
    );
    let merged = merge(
        &policy(Vec::new()),
        &Ledger::default(),
        "1000",
        &one,
        &LINUX,
        NOW,
    )
    .expect("one entry per device is a valid request");
    assert_eq!(merged.policy.expect("policy changed").grants.len(), 3);
    // The list as the runtime noted it would have been refused as a whole.
    assert!(merge(
        &policy(Vec::new()),
        &Ledger::default(),
        "1000",
        &[device(1, Some(NOW + 60)), device(1, Some(NOW + 3600))],
        &LINUX,
        NOW,
    )
    .is_err());
}
