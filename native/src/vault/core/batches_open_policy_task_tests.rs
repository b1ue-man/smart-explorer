//! B1: persisted root consent and cleartext consent remain independent.
use super::*;
use super::super::registry_types::{LockSettings, SiteRecord};

fn binding() -> RootBinding {
    RootBinding {
        site: "0123456789abcdef0123456789abcdef".into(), vault: "vault-id".into(),
        location: "sftp://saved/Vault".into(), backend: "Sftp".into(),
        namespace: "server-key".into(), account: "account-a".into(),
        root: "/Vault".into(), object: Some("object-a".into()),
    }
}
fn record() -> SiteRecord {
    SiteRecord::new("0123456789abcdef0123456789abcdef", "Fixture", "/vault", "vault-id", 0, LockSettings::desktop())
}

#[test]
fn batches_task_vault_root_and_cleartext_consents_persist_independently() {
    let mut record = record();
    record.extra.push(("future-setting".into(), "keep".into()));
    let original = binding();
    let mut policy = OpenPolicy { trusted_root: Some(original.clone()), cleartext_copy: None };
    store_policy(&mut record, &policy).unwrap();
    assert_eq!(stored_policy(&record).unwrap(), policy);
    assert!(stored_policy(&record).unwrap().cleartext_copy.is_none());
    policy.trusted_root = None;
    policy.cleartext_copy = Some(original);
    store_policy(&mut record, &policy).unwrap();
    assert_eq!(stored_policy(&record).unwrap(), policy);
    assert!(record.extra.contains(&("future-setting".into(), "keep".into())));
    assert_eq!(record.extra.iter().filter(|(key, _)| key == POLICY_KEY).count(), 1);
    let mount = MountPolicy { binding: binding(), enforced: false, trusted: false, cleartext_copy: true, read_only: false };
    assert!(!mount.allowed(), "cleartext consent cannot authorize a mount");
    assert!(MountPolicy { enforced: true, ..mount.clone() }.allowed());
    assert!(MountPolicy { trusted: true, ..mount }.allowed());
}

#[test]
fn batches_task_vault_consent_keeps_every_identity_component_exact() {
    let original = binding();
    let mut record = record();
    store_policy(&mut record, &OpenPolicy { trusted_root: Some(original.clone()), cleartext_copy: Some(original.clone()) }).unwrap();
    let stored = stored_policy(&record).unwrap();
    for field in ["site", "vault", "location", "backend", "namespace", "account", "root", "object"] {
        let mut changed = serde_json::to_value(&original).unwrap();
        changed[field] = serde_json::json!(format!("{}-changed", changed[field].as_str().unwrap()));
        let changed: RootBinding = serde_json::from_value(changed).unwrap();
        assert_ne!(stored.trusted_root.as_ref(), Some(&changed), "{field}");
        assert_ne!(stored.cleartext_copy.as_ref(), Some(&changed), "{field}");
    }
    let mut missing_object = original;
    missing_object.object = None;
    assert_ne!(stored.trusted_root, Some(missing_object));
}

#[test]
fn batches_task_vault_legacy_missing_and_ambiguous_policy_never_grants_consent() {
    let mut record = record();
    assert_eq!(stored_policy(&record).unwrap(), OpenPolicy::default());
    record.extra.push((POLICY_KEY.into(), "{}".into()));
    assert_eq!(stored_policy(&record).unwrap(), OpenPolicy::default());
    record.extra.push((POLICY_KEY.into(), "{}".into()));
    assert!(stored_policy(&record).is_err());
    record.extra.truncate(1);
    record.extra[0].1 = "broken-json".into();
    assert!(stored_policy(&record).is_err());
}
