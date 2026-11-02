use super::{
    finish_pending_cleanup_with, load_or_replace_unpaired, with_matching_identity_generation,
    IdentityPersistence, IdentityRepairAction, ShareIdentity,
};
use crate::share::{DirectGrant, DirectGrantState};
use std::collections::HashMap;

/// The identity file stays, its key is gone from the secure store (a fresh
/// Windows installation, 2026-10-22).
fn identity_without_key() -> (SecretPersistence, String) {
    let mut storage = SecretPersistence::default();
    let created = ShareIdentity::load_or_create_with("Local".into(), &mut storage).unwrap();
    assert!(storage
        .secrets
        .remove("share:identity:iroh_secret")
        .is_some());
    (storage, created.node_id)
}

#[test]
fn windows_direct_task_unpaired_identity_without_key_is_replaced() {
    let (mut storage, old_node) = identity_without_key();
    let replaced = load_or_replace_unpaired("Local".into(), &mut storage, || true).unwrap();
    assert_ne!(replaced.node_id, old_node);
    // The new identity loads like any other afterwards.
    let loaded = ShareIdentity::load_or_create_with("Local".into(), &mut storage).unwrap();
    assert_eq!(loaded.node_id, replaced.node_id);
}

#[test]
fn windows_direct_task_paired_identity_without_key_names_the_repair() {
    let (mut storage, _) = identity_without_key();
    let kept = storage.identity.clone();
    let error = load_or_replace_unpaired("Local".into(), &mut storage, || false).unwrap_err();
    assert!(error.contains("fehlt im sicheren Speicher"), "{error}");
    assert!(error.contains("se share identity --repair"), "{error}");
    assert_eq!(storage.identity, kept, "pairings keep the identity file");
}

/// An identity file and a secure store kept in memory.
#[derive(Default)]
struct SecretPersistence {
    identity: Option<String>,
    secrets: HashMap<String, String>,
}

impl IdentityPersistence for SecretPersistence {
    fn load_identity(&mut self) -> Result<Option<String>, String> {
        Ok(self.identity.clone())
    }

    fn save_identity(&mut self, contents: &str) -> Result<(), String> {
        self.identity = Some(contents.to_string());
        Ok(())
    }

    fn load_secret(&mut self, account: &str) -> Result<Option<String>, String> {
        Ok(self.secrets.get(account).cloned())
    }

    fn save_secret(&mut self, account: &str, secret: &str) -> Result<(), String> {
        self.secrets.insert(account.to_string(), secret.to_string());
        Ok(())
    }

    fn delete_secret(&mut self, account: &str) -> Result<(), String> {
        self.secrets.remove(account);
        Ok(())
    }
}

#[test]
fn stale_generation_never_runs_an_acceptance_mutation() {
    let stale = identity("old-lookup", 7);
    let current = identity("new-lookup", 8);
    let mut grants = Vec::new();
    let result = with_matching_identity_generation(&stale, &current, |_| {
        grants.push(DirectGrant {
            device_id: "peer".into(),
            device_name: "Peer".into(),
            public_key: "peer-key".into(),
            fingerprint: "peer-fingerprint".into(),
            node_id: "peer-node".into(),
            state: DirectGrantState::Accepted,
            updated_at: 1,
            exec: Default::default(),
            write: false,
        });
        Ok(())
    });
    assert!(result.unwrap_err().contains("changed concurrently"));
    assert!(grants.is_empty());
}

#[test]
fn a_device_rename_does_not_change_the_security_generation() {
    let expected = identity("lookup", 7);
    let mut current = expected.clone();
    current.device_name = "Renamed".into();
    assert_eq!(
        with_matching_identity_generation(&expected, &current, |locked| {
            Ok(locked.device_name.clone())
        })
        .unwrap(),
        "Renamed"
    );
}

#[test]
fn cleanup_marker_survives_cleanup_and_marker_write_failures() {
    let identity = identity("lookup", 7);
    let mut storage = MarkerPersistence::default();
    identity
        .save_with_pending_cleanup(&mut storage, IdentityRepairAction::IdentityReplaced)
        .unwrap();

    let mut cleanup_calls = 0;
    assert!(finish_pending_cleanup_with(&identity, &mut storage, |_| {
        cleanup_calls += 1;
        Err::<(), _>("profile store unavailable".into())
    })
    .is_err());
    assert_eq!(
        ShareIdentity::pending_cleanup_action_with(&mut storage).unwrap(),
        Some(IdentityRepairAction::IdentityReplaced)
    );

    storage.fail_save = true;
    assert!(finish_pending_cleanup_with(&identity, &mut storage, |_| {
        cleanup_calls += 1;
        Ok(())
    })
    .is_err());
    storage.fail_save = false;
    assert!(finish_pending_cleanup_with(&identity, &mut storage, |_| {
        cleanup_calls += 1;
        Ok(())
    })
    .unwrap()
    .is_some());
    assert_eq!(cleanup_calls, 3);
    assert_eq!(
        ShareIdentity::pending_cleanup_action_with(&mut storage).unwrap(),
        None
    );
}

#[derive(Default)]
struct MarkerPersistence {
    identity: Option<String>,
    fail_save: bool,
}

impl IdentityPersistence for MarkerPersistence {
    fn load_identity(&mut self) -> Result<Option<String>, String> {
        Ok(self.identity.clone())
    }

    fn save_identity(&mut self, contents: &str) -> Result<(), String> {
        if self.fail_save {
            return Err("metadata unavailable".into());
        }
        self.identity = Some(contents.to_string());
        Ok(())
    }

    fn load_secret(&mut self, _account: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn save_secret(&mut self, _account: &str, _secret: &str) -> Result<(), String> {
        Ok(())
    }

    fn delete_secret(&mut self, _account: &str) -> Result<(), String> {
        Ok(())
    }
}

fn identity(lookup_id: &str, direct_secret: u8) -> ShareIdentity {
    let iroh_secret = iroh::SecretKey::from_bytes(&[1; 32]);
    let public = iroh_secret.public().to_string();
    ShareIdentity {
        device_id: "local-device".into(),
        device_name: "Local".into(),
        direct_lookup_id: lookup_id.into(),
        public_key: public.clone(),
        fingerprint: super::super::core::public_fingerprint(public.as_bytes()),
        node_id: public,
        iroh_secret,
        direct_secret: [direct_secret; 32],
    }
}
