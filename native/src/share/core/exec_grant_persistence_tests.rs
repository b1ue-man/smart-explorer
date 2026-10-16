use super::*;
use crate::share::types::{DirectGrant, ExecGrantTarget};

#[test]
fn persisted_enable_uses_exact_revision_and_is_idempotent() {
    let (identity, mut profiles, target) = direct_fixture();
    let (mutation, expected) =
        ExecGrantMutation::prepare_persisted(&mut profiles, &identity, target, true, None, 17)
            .unwrap();

    assert_eq!(expected, 0);
    assert!(mutation.policy.enabled);
    assert_eq!(mutation.policy.policy_revision, 1);
    mutation
        .apply_persisted_cas(&mut profiles, &identity, expected)
        .unwrap();
    mutation
        .apply_persisted_cas(&mut profiles, &identity, expected)
        .unwrap();
    assert_eq!(profiles.direct_grants[0].exec, mutation.policy);
}

#[test]
fn concurrent_revision_change_is_not_overwritten() {
    let (identity, mut profiles, target) = direct_fixture();
    let (mutation, expected) =
        ExecGrantMutation::prepare_persisted(&mut profiles, &identity, target, true, None, 17)
            .unwrap();
    profiles.direct_grants[0]
        .exec
        .set_runtime_enabled(false, 18)
        .unwrap();

    let error = mutation
        .apply_persisted_cas(&mut profiles, &identity, expected)
        .unwrap_err();
    assert!(error.contains("revision changed concurrently"));
    assert!(!profiles.direct_grants[0].exec.enabled);
}

#[test]
fn pending_enable_is_masked_without_advancing_its_revision() {
    let (identity, mut profiles, target) = direct_fixture();
    let (mutation, expected) =
        ExecGrantMutation::prepare_persisted(&mut profiles, &identity, target, true, None, 17)
            .unwrap();
    mutation
        .apply_persisted_cas(&mut profiles, &identity, expected)
        .unwrap();

    mutation
        .mask_pending_policy(&mut profiles, &identity, expected)
        .unwrap();
    assert!(!profiles.direct_grants[0].exec.enabled);
    assert_eq!(profiles.direct_grants[0].exec.policy_revision, 1);
}

#[test]
fn luecken_task_settings_change_run_as_and_programs_in_one_revision() {
    let (identity, mut profiles, target) = direct_fixture();
    let settings = ExecGrantSettings {
        run_as: crate::share::ExecRunAs::Account {
            name: "deploy".into(),
        },
        // An absolute path of this platform (`/usr/bin/uptime` is not one on Windows).
        programs: vec![std::env::temp_dir()
            .join("uptime")
            .to_string_lossy()
            .into_owned()],
    };
    let (mutation, expected) = ExecGrantMutation::prepare_persisted(
        &mut profiles,
        &identity,
        target.clone(),
        true,
        Some(&settings),
        17,
    )
    .unwrap();
    assert_eq!(mutation.policy.run_as, settings.run_as);
    assert_eq!(mutation.policy.programs, settings.programs);
    assert_eq!(mutation.policy.policy_revision, expected + 1);
    mutation
        .apply_persisted_cas(&mut profiles, &identity, expected)
        .unwrap();
    // A later enable/disable without settings keeps them.
    let (disabled, _) = ExecGrantMutation::prepare_persisted(
        &mut profiles,
        &identity,
        target.clone(),
        false,
        None,
        18,
    )
    .unwrap();
    assert_eq!(disabled.policy.run_as, settings.run_as);
    let invalid = ExecGrantSettings {
        run_as: crate::share::ExecRunAs::Account { name: "-rf".into() },
        programs: Vec::new(),
    };
    assert!(ExecGrantMutation::prepare_persisted(
        &mut profiles,
        &identity,
        target,
        true,
        Some(&invalid),
        19
    )
    .is_err());
}

#[test]
fn luecken_task_profiles_without_run_as_port_to_the_standard_user_unchanged() {
    let legacy = serde_json::json!({
        "enabled": true,
        "policy_revision": 4,
        "changed_at": 9,
        "source_request_id": null,
        "source_decision_revision": null
    });
    let grant: ExecGrant = serde_json::from_value(legacy).unwrap();
    assert!(grant.enabled);
    assert_eq!(grant.policy_revision, 4);
    assert_eq!(grant.run_as, crate::share::ExecRunAs::Standard);
    assert!(grant.programs.is_empty());
    let written = serde_json::to_value(&grant).unwrap();
    assert_eq!(written["run_as"], serde_json::json!({"kind": "standard"}));
    assert!(written.get("programs").is_none());
}

fn direct_fixture() -> (ShareIdentity, ShareProfiles, ExecGrantTarget) {
    let local = identity("local", "lookup-local", 11);
    let peer = identity("peer", "lookup-peer", 29);
    let target = ExecGrantTarget::Direct {
        device_id: peer.device_id.clone(),
        public_key: peer.public_key.clone(),
        fingerprint: peer.fingerprint.clone(),
        node_id: peer.node_id.clone(),
    };
    let mut profiles = ShareProfiles::default();
    profiles.direct_grants.push(DirectGrant {
        device_id: peer.device_id,
        device_name: peer.device_name,
        public_key: peer.public_key,
        fingerprint: peer.fingerprint,
        node_id: peer.node_id,
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: ExecGrant::default(),
        write: false,
    });
    (local, profiles, target)
}

fn identity(device_id: &str, lookup_id: &str, key_byte: u8) -> ShareIdentity {
    let secret = iroh::SecretKey::from_bytes(&[key_byte; 32]);
    let public_key = secret.public().to_string();
    ShareIdentity {
        device_id: device_id.into(),
        device_name: device_id.into(),
        direct_lookup_id: lookup_id.into(),
        fingerprint: crate::share::core::public_fingerprint(public_key.as_bytes()),
        node_id: public_key.clone(),
        public_key,
        iroh_secret: secret,
        direct_secret: [7; 32],
    }
}
