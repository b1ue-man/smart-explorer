use std::collections::{HashMap, HashSet};

use super::{
    b64, direct_secret_account, IdentityPersistence, IdentityRepairAction, ShareIdentity,
    IDENTITY_KEY_ACCOUNT,
};

#[derive(Default)]
struct FakePersistence {
    identity: Option<String>,
    secrets: HashMap<String, String>,
    fail_identity_save: bool,
    fail_secret_save: bool,
    discard_secret_saves: bool,
    fail_secret_deletes: HashSet<String>,
}

impl IdentityPersistence for FakePersistence {
    fn load_identity(&mut self) -> Result<Option<String>, String> {
        Ok(self.identity.clone())
    }

    fn save_identity(&mut self, contents: &str) -> Result<(), String> {
        if self.fail_identity_save {
            Err("disk full".into())
        } else {
            self.identity = Some(contents.to_string());
            Ok(())
        }
    }

    fn load_secret(&mut self, account: &str) -> Result<Option<String>, String> {
        Ok(self.secrets.get(account).cloned())
    }

    fn save_secret(&mut self, account: &str, secret: &str) -> Result<(), String> {
        if self.fail_secret_save {
            return Err("secure store unavailable".into());
        }
        if !self.discard_secret_saves {
            self.secrets.insert(account.to_string(), secret.to_string());
        }
        Ok(())
    }

    fn delete_secret(&mut self, account: &str) -> Result<(), String> {
        if self.fail_secret_deletes.contains(account) {
            return Err("secure store refused deletion".into());
        }
        self.secrets.remove(account);
        Ok(())
    }
}

fn create_identity(storage: &mut FakePersistence, name: &str) -> ShareIdentity {
    ShareIdentity::load_or_create_with(name.into(), storage).unwrap()
}

#[test]
fn missing_persisted_direct_secret_never_produces_a_code() {
    let mut storage = FakePersistence::default();
    let identity = create_identity(&mut storage, "device");
    storage
        .secrets
        .remove(&direct_secret_account(&identity.direct_lookup_id));
    assert!(ShareIdentity::load_or_create_with("device".into(), &mut storage).is_err());
}

#[test]
fn failed_initial_metadata_write_cleans_up_new_secrets() {
    let mut storage = FakePersistence {
        fail_identity_save: true,
        ..FakePersistence::default()
    };
    assert!(ShareIdentity::load_or_create_with("device".into(), &mut storage).is_err());
    assert!(storage.secrets.is_empty());
    assert!(storage.identity.is_none());
}

#[test]
fn failed_rotation_secret_write_keeps_the_old_identity_and_code() {
    let mut storage = FakePersistence::default();
    let mut identity = create_identity(&mut storage, "device");
    let old_code = identity.direct_code();
    let old_lookup = identity.direct_lookup_id.clone();
    storage.fail_secret_save = true;
    assert!(identity.regenerate_direct_code_with(&mut storage).is_err());
    assert_eq!(identity.direct_code(), old_code);
    assert_eq!(identity.direct_lookup_id, old_lookup);
}

#[test]
fn failed_rotation_metadata_write_keeps_old_code_and_removes_candidate_secret() {
    let mut storage = FakePersistence::default();
    let mut identity = create_identity(&mut storage, "device");
    let old_code = identity.direct_code();
    let old_accounts = storage.secrets.clone();
    storage.fail_identity_save = true;
    assert!(identity.regenerate_direct_code_with(&mut storage).is_err());
    assert_eq!(identity.direct_code(), old_code);
    assert_eq!(storage.secrets, old_accounts);
}

#[test]
fn successful_rotation_returns_the_single_persisted_code() {
    let mut storage = FakePersistence::default();
    let mut identity = create_identity(&mut storage, "device");
    let old_account = direct_secret_account(&identity.direct_lookup_id);
    let outcome = identity.regenerate_direct_code_with(&mut storage).unwrap();
    assert_eq!(outcome.code, identity.direct_code());
    assert_eq!(outcome.cleanup_warning, None);
    assert!(!storage.secrets.contains_key(&old_account));
    assert!(storage
        .secrets
        .contains_key(&direct_secret_account(&identity.direct_lookup_id)));
}

#[test]
fn stale_rotation_uses_the_identity_created_by_full_repair() {
    let mut storage = FakePersistence::default();
    let mut stale = create_identity(&mut storage, "device");
    storage.secrets.remove(IDENTITY_KEY_ACCOUNT);
    let repaired = ShareIdentity::repair_missing_with("fallback".into(), &mut storage).unwrap();
    let repaired_code = repaired.identity.direct_code();
    let repaired_account = direct_secret_account(&repaired.identity.direct_lookup_id);

    let rotated = stale.regenerate_direct_code_with(&mut storage).unwrap();

    assert_eq!(stale.device_id, repaired.identity.device_id);
    assert_eq!(stale.device_name, repaired.identity.device_name);
    assert_eq!(stale.node_id, repaired.identity.node_id);
    assert_ne!(stale.direct_code(), repaired_code);
    assert_eq!(stale.direct_code(), rotated.code);
    assert!(!storage.secrets.contains_key(&repaired_account));
    let persisted = ShareIdentity::load_or_create_with("fallback".into(), &mut storage).unwrap();
    assert_eq!(persisted.device_id, repaired.identity.device_id);
    assert_eq!(persisted.direct_code(), stale.direct_code());
}

#[test]
fn stale_rename_preserves_a_newer_direct_code_rotation() {
    let mut storage = FakePersistence::default();
    let mut stale = create_identity(&mut storage, "old name");
    let mut current = stale.clone();
    let rotated = current.regenerate_direct_code_with(&mut storage).unwrap();

    stale
        .set_device_name_with("new name".into(), &mut storage)
        .unwrap();

    assert_eq!(stale.device_name, "new name");
    assert_eq!(stale.direct_code(), rotated.code);
    let persisted = ShareIdentity::load_or_create_with("fallback".into(), &mut storage).unwrap();
    assert_eq!(persisted.device_name, "new name");
    assert_eq!(persisted.direct_code(), rotated.code);
}

#[test]
fn healthy_identity_refuses_repair_without_mutation() {
    let mut storage = FakePersistence::default();
    create_identity(&mut storage, "device");
    let old_identity = storage.identity.clone();
    let old_secrets = storage.secrets.clone();

    let error = ShareIdentity::repair_missing_with("fallback".into(), &mut storage).unwrap_err();

    assert!(error.contains("vollstaendig"));
    assert_eq!(storage.identity, old_identity);
    assert_eq!(storage.secrets, old_secrets);
}

#[test]
fn repair_preflight_reports_missing_direct_secret_without_mutation() {
    let mut storage = FakePersistence::default();
    let identity = create_identity(&mut storage, "device");
    storage
        .secrets
        .remove(&direct_secret_account(&identity.direct_lookup_id));
    let old_identity = storage.identity.clone();
    let old_secrets = storage.secrets.clone();

    let action = ShareIdentity::repair_action_needed_with("fallback".into(), &mut storage).unwrap();

    assert_eq!(action, IdentityRepairAction::DirectCodeRotated);
    assert_eq!(storage.identity, old_identity);
    assert_eq!(storage.secrets, old_secrets);
}

#[test]
fn repair_preflight_reports_missing_iroh_secret_without_mutation() {
    let mut storage = FakePersistence::default();
    create_identity(&mut storage, "device");
    storage.secrets.remove(IDENTITY_KEY_ACCOUNT);
    let old_identity = storage.identity.clone();
    let old_secrets = storage.secrets.clone();

    let action = ShareIdentity::repair_action_needed_with("fallback".into(), &mut storage).unwrap();

    assert_eq!(action, IdentityRepairAction::IdentityReplaced);
    assert_eq!(storage.identity, old_identity);
    assert_eq!(storage.secrets, old_secrets);
}

#[test]
fn repair_preflight_refuses_a_healthy_identity_without_mutation() {
    let mut storage = FakePersistence::default();
    create_identity(&mut storage, "device");
    let old_identity = storage.identity.clone();
    let old_secrets = storage.secrets.clone();

    let error =
        ShareIdentity::repair_action_needed_with("fallback".into(), &mut storage).unwrap_err();

    assert!(error.contains("vollstaendig"));
    assert_eq!(storage.identity, old_identity);
    assert_eq!(storage.secrets, old_secrets);
}

#[test]
fn missing_direct_secret_rotates_only_the_direct_code() {
    let mut storage = FakePersistence::default();
    let original = create_identity(&mut storage, "Stored name");
    let old_lookup = original.direct_lookup_id.clone();
    let old_iroh = storage.secrets[IDENTITY_KEY_ACCOUNT].clone();
    storage.secrets.remove(&direct_secret_account(&old_lookup));
    storage.secrets.insert(
        "share:direct-contact:friend".into(),
        "contact-secret".into(),
    );
    storage
        .secrets
        .insert("share:room:team".into(), "room-secret".into());

    let repair = ShareIdentity::repair_missing_with("fallback".into(), &mut storage).unwrap();

    assert_eq!(repair.action, IdentityRepairAction::DirectCodeRotated);
    assert_eq!(repair.cleanup_warning, None);
    assert_eq!(repair.identity.device_id, original.device_id);
    assert_eq!(repair.identity.device_name, "Stored name");
    assert_eq!(repair.identity.node_id, original.node_id);
    assert_ne!(repair.identity.direct_lookup_id, old_lookup);
    assert_eq!(storage.secrets[IDENTITY_KEY_ACCOUNT], old_iroh);
    assert_eq!(
        storage.secrets["share:direct-contact:friend"],
        "contact-secret"
    );
    assert_eq!(storage.secrets["share:room:team"], "room-secret");
    let loaded = ShareIdentity::load_or_create_with("fallback".into(), &mut storage).unwrap();
    assert_eq!(loaded.direct_code(), repair.identity.direct_code());
}

#[test]
fn direct_repair_metadata_failure_rolls_back_candidate_secret() {
    let mut storage = FakePersistence::default();
    let original = create_identity(&mut storage, "device");
    storage
        .secrets
        .remove(&direct_secret_account(&original.direct_lookup_id));
    let old_identity = storage.identity.clone();
    let old_secrets = storage.secrets.clone();
    storage.fail_identity_save = true;

    assert!(ShareIdentity::repair_missing_with("fallback".into(), &mut storage).is_err());

    assert_eq!(storage.identity, old_identity);
    assert_eq!(storage.secrets, old_secrets);
}

#[test]
fn unverified_direct_secret_never_commits_repair_metadata() {
    let mut storage = FakePersistence::default();
    let original = create_identity(&mut storage, "device");
    storage
        .secrets
        .remove(&direct_secret_account(&original.direct_lookup_id));
    let old_identity = storage.identity.clone();
    let old_secrets = storage.secrets.clone();
    storage.discard_secret_saves = true;

    let error = ShareIdentity::repair_missing_with("fallback".into(), &mut storage).unwrap_err();

    assert!(error.contains("nicht im sicheren Speicher behalten"));
    assert_eq!(storage.identity, old_identity);
    assert_eq!(storage.secrets, old_secrets);
}

#[test]
fn missing_iroh_secret_replaces_the_whole_device_identity() {
    let mut storage = FakePersistence::default();
    let original = create_identity(&mut storage, "Stored name");
    let old_direct_account = direct_secret_account(&original.direct_lookup_id);
    storage.secrets.remove(IDENTITY_KEY_ACCOUNT);
    storage.secrets.insert(
        "share:direct-contact:friend".into(),
        "contact-secret".into(),
    );
    storage
        .secrets
        .insert("share:room:team".into(), "room-secret".into());

    let repair = ShareIdentity::repair_missing_with("fallback".into(), &mut storage).unwrap();

    assert_eq!(repair.action, IdentityRepairAction::IdentityReplaced);
    assert_eq!(repair.cleanup_warning, None);
    assert_eq!(repair.identity.device_name, "Stored name");
    assert_ne!(repair.identity.device_id, original.device_id);
    assert_ne!(repair.identity.node_id, original.node_id);
    assert_ne!(repair.identity.direct_lookup_id, original.direct_lookup_id);
    assert!(!storage.secrets.contains_key(&old_direct_account));
    assert!(storage.secrets.contains_key(IDENTITY_KEY_ACCOUNT));
    assert_eq!(
        storage.secrets["share:direct-contact:friend"],
        "contact-secret"
    );
    assert_eq!(storage.secrets["share:room:team"], "room-secret");
    let loaded = ShareIdentity::load_or_create_with("fallback".into(), &mut storage).unwrap();
    assert_eq!(loaded.device_id, repair.identity.device_id);
    assert_eq!(loaded.direct_code(), repair.identity.direct_code());
}

#[test]
fn replacement_metadata_failure_restores_the_pre_repair_secret_set() {
    let mut storage = FakePersistence::default();
    create_identity(&mut storage, "device");
    storage.secrets.remove(IDENTITY_KEY_ACCOUNT);
    let old_identity = storage.identity.clone();
    let old_secrets = storage.secrets.clone();
    storage.fail_identity_save = true;

    assert!(ShareIdentity::repair_missing_with("fallback".into(), &mut storage).is_err());

    assert_eq!(storage.identity, old_identity);
    assert_eq!(storage.secrets, old_secrets);
}

#[test]
fn replacement_reports_old_direct_cleanup_failure_after_commit() {
    let mut storage = FakePersistence::default();
    let original = create_identity(&mut storage, "device");
    let old_direct_account = direct_secret_account(&original.direct_lookup_id);
    storage.secrets.remove(IDENTITY_KEY_ACCOUNT);
    storage
        .fail_secret_deletes
        .insert(old_direct_account.clone());

    let repair = ShareIdentity::repair_missing_with("fallback".into(), &mut storage).unwrap();

    assert_eq!(repair.action, IdentityRepairAction::IdentityReplaced);
    assert!(repair.cleanup_warning.is_some());
    assert!(storage.secrets.contains_key(&old_direct_account));
    assert!(storage
        .secrets
        .contains_key(&direct_secret_account(&repair.identity.direct_lookup_id)));
}

#[test]
fn mismatched_iroh_secret_is_not_treated_as_a_missing_direct_secret() {
    let mut storage = FakePersistence::default();
    let original = create_identity(&mut storage, "device");
    storage
        .secrets
        .remove(&direct_secret_account(&original.direct_lookup_id));
    let replacement = iroh::SecretKey::from_bytes(&[0xa5; 32]);
    storage
        .secrets
        .insert(IDENTITY_KEY_ACCOUNT.into(), b64(&replacement.to_bytes()));
    let old_identity = storage.identity.clone();
    let old_secrets = storage.secrets.clone();

    let error = ShareIdentity::repair_missing_with("fallback".into(), &mut storage).unwrap_err();

    assert!(error.contains("passt nicht"));
    assert_eq!(storage.identity, old_identity);
    assert_eq!(storage.secrets, old_secrets);
}

#[test]
fn corrupt_secret_is_not_treated_as_missing() {
    let mut storage = FakePersistence::default();
    let identity = create_identity(&mut storage, "device");
    let account = direct_secret_account(&identity.direct_lookup_id);
    storage.secrets.insert(account, "not-base64".into());
    let old_identity = storage.identity.clone();
    let old_secrets = storage.secrets.clone();

    let error = ShareIdentity::repair_missing_with("fallback".into(), &mut storage).unwrap_err();

    assert!(error.contains("ungueltig"));
    assert_eq!(storage.identity, old_identity);
    assert_eq!(storage.secrets, old_secrets);
}
