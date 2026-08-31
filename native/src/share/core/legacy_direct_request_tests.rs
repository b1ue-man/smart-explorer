use std::collections::HashMap;

use super::core::{hmac_proof, presence_payload, public_fingerprint};
use super::identity::ShareIdentity;
use super::legacy_direct_request::{LegacyDirectDecisionState, LegacyDirectDeliveryState};
use super::profile_persistence::ProfilePersistence;
use super::profiles::{ProfileRevision, ShareProfiles};
use super::types::{DirectGrant, DirectGrantState, PeerPresence};

#[test]
fn verified_receive_survives_reload_and_new_nonce_updates_same_selector() {
    let identity = identity();
    let mut profiles = ShareProfiles::default();
    let first = presence(&identity, 2, "nonce-a", 200);
    assert!(profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &first, 100)
        .unwrap());
    let selector = profiles.legacy_direct_requests[0].selector.clone();
    let first_event = profiles.legacy_direct_requests[0].evidence.event_id.clone();

    let mut renamed = presence(&identity, 2, "nonce-b", 220);
    renamed.device_name = "Renamed".into();
    assert!(profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &renamed, 110)
        .unwrap());
    assert_eq!(profiles.legacy_direct_requests.len(), 1);
    assert_eq!(profiles.legacy_direct_requests[0].selector, selector);
    assert_ne!(
        profiles.legacy_direct_requests[0].evidence.event_id,
        first_event
    );
    assert_eq!(
        profiles.legacy_direct_requests[0].peer.device_name,
        "Renamed"
    );

    let json = serde_json::to_string(&profiles).unwrap();
    let restored: ShareProfiles = serde_json::from_str(&json).unwrap();
    restored.validate_legacy_direct_requests().unwrap();
    restored.validate_legacy_evidence(&identity).unwrap();
    assert_eq!(restored.legacy_direct_requests[0].selector, selector);
}

#[test]
fn ci_remote_task_replay_requires_revoke_before_delete_and_retains_denial() {
    let identity = identity();
    let mut profiles = ShareProfiles::default();
    let first = presence(&identity, 2, "nonce-a", 200);
    profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &first, 100)
        .unwrap();
    assert!(!profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &first, 101)
        .unwrap());
    let selector = profiles.legacy_direct_requests[0].selector.clone();
    assert!(profiles
        .delete_legacy_direct_request(&selector, 110)
        .unwrap_err()
        .contains("active authorization"));
    profiles
        .revoke_legacy_direct_request(&selector, 110)
        .unwrap();
    assert!(profiles
        .delete_legacy_direct_request(&selector, 111)
        .unwrap());
    assert_eq!(profiles.direct_grants.len(), 1);
    assert_eq!(profiles.direct_grants[0].state, DirectGrantState::Ignored);

    let reconnect = presence(&identity, 2, "nonce-b", 230);
    assert!(!profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &reconnect, 120)
        .unwrap());
    assert!(profiles.legacy_direct_requests.is_empty());

    let later = presence(&identity, 2, "nonce-c", 320);
    assert!(profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &later, 201)
        .unwrap());
    assert_eq!(profiles.legacy_direct_requests.len(), 1);
    assert_eq!(
        profiles.legacy_direct_requests[0].decision,
        LegacyDirectDecisionState::Rejected
    );
}

#[test]
fn ci_remote_task_autoaccept_revoke_and_manual_answer_retry_remain_truthful() {
    let identity = identity();
    let mut profiles = ShareProfiles::default();
    let request = presence(&identity, 2, "nonce-a", 200);
    profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &request, 100)
        .unwrap();
    let selector = profiles.legacy_direct_requests[0].selector.clone();
    let entry = profiles.legacy_direct_request(&selector).unwrap();
    assert_eq!(entry.decision, LegacyDirectDecisionState::Accepted);
    assert_eq!(
        entry.decision_delivery.state,
        LegacyDirectDeliveryState::Queued
    );
    assert_eq!(
        entry.decision_delivery_channel(),
        "legacy_signaling_untracked"
    );
    assert!(entry.authorization_active(&profiles));

    profiles
        .record_legacy_answer_attempt(&selector, 1, 111, None)
        .unwrap();
    assert_eq!(
        profiles
            .legacy_direct_request(&selector)
            .unwrap()
            .decision_delivery
            .state,
        LegacyDirectDeliveryState::AttemptedUntracked
    );
    profiles.retry_legacy_answer(&selector).unwrap();
    assert_eq!(
        profiles
            .legacy_direct_request(&selector)
            .unwrap()
            .decision_delivery
            .state,
        LegacyDirectDeliveryState::Queued
    );
    profiles
        .revoke_legacy_direct_request(&selector, 120)
        .unwrap();
    let entry = profiles.legacy_direct_request(&selector).unwrap();
    assert_eq!(entry.decision, LegacyDirectDecisionState::Revoked);
    assert_eq!(
        entry.decision_delivery.state,
        LegacyDirectDeliveryState::LocalOnlyUntracked
    );
    assert_eq!(entry.decision_delivery_channel(), "local_only_untracked");
    assert!(!entry.authorization_active(&profiles));
}

#[test]
fn ci_remote_task_identity_conflict_is_rejected_without_replacing_the_grant() {
    let identity = identity();
    let other = peer_key(3);
    let mut profiles = ShareProfiles::default();
    profiles.direct_grants.push(DirectGrant {
        device_id: "peer-device".into(),
        device_name: "Old identity".into(),
        public_key: other.clone(),
        fingerprint: public_fingerprint(other.as_bytes()),
        node_id: other,
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: Default::default(),
    });
    let request = presence(&identity, 2, "nonce-a", 200);
    profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &request, 100)
        .unwrap();
    let entry = profiles.legacy_direct_requests[0].clone();
    assert!(entry.identity_conflict);
    assert_eq!(entry.decision, LegacyDirectDecisionState::Rejected);
    assert!(profiles
        .decide_legacy_direct_request(&entry.selector, true, 110)
        .unwrap_err()
        .contains("not pending"));
    assert_eq!(profiles.direct_grants[0].public_key, peer_key(3));
}

#[test]
fn ci_remote_task_first_verified_identity_wins_in_both_arrival_orders() {
    let identity = identity();
    for seeds in [[2, 3], [3, 2]] {
        let mut profiles = ShareProfiles::default();
        for (index, seed) in seeds.into_iter().enumerate() {
            let request = presence(
                &identity,
                seed,
                &format!("nonce-{index}"),
                200 + index as i64,
            );
            profiles
                .record_verified_legacy_direct_request(
                    &identity.direct_lookup_id,
                    &request,
                    100 + index as i64,
                )
                .unwrap();
        }
        assert_eq!(profiles.legacy_direct_requests.len(), 2);
        let accepted = &profiles.legacy_direct_requests[0];
        let rejected = &profiles.legacy_direct_requests[1];
        assert!(!accepted.identity_conflict);
        assert_eq!(accepted.decision, LegacyDirectDecisionState::Accepted);
        assert!(rejected.identity_conflict);
        assert_eq!(rejected.decision, LegacyDirectDecisionState::Rejected);
        let rejected_selector = rejected.selector.clone();
        assert_eq!(profiles.direct_grants.len(), 1);
        assert_eq!(profiles.direct_grants[0].public_key, peer_key(seeds[0]));
        assert!(profiles
            .decide_legacy_direct_request(&rejected_selector, true, 110)
            .unwrap_err()
            .contains("not pending"));
    }
}

#[test]
fn ci_remote_task_generic_grant_upsert_cannot_replace_an_autoaccepted_identity() {
    let identity = identity();
    let mut profiles = ShareProfiles::default();
    let request_a = presence(&identity, 2, "nonce-a", 300);
    profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &request_a, 100)
        .unwrap();
    let selector_a = profiles.legacy_direct_requests[0].selector.clone();
    assert_eq!(profiles.legacy_answers_due(110).len(), 1);

    let request_b = presence(&identity, 3, "nonce-b", 310);
    assert!(profiles
        .set_direct_grant(&request_b, DirectGrantState::Accepted)
        .unwrap_err()
        .contains("conflicts"));
    assert_eq!(profiles.legacy_answers_due(119).len(), 1);
    assert!(profiles
        .legacy_direct_request(&selector_a)
        .unwrap()
        .authorization_active(&profiles));
}

#[test]
fn ci_remote_task_load_reconciles_autoaccepted_history_when_its_grant_was_lost() {
    let identity = identity();
    let mut profiles = ShareProfiles::default();
    let request_a = presence(&identity, 2, "nonce-a", 300);
    profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &request_a, 100)
        .unwrap();
    let selector = profiles.legacy_direct_requests[0].selector.clone();
    profiles.direct_grants.clear();
    assert!(profiles.validate_legacy_direct_requests().is_err());

    let mut storage = MemoryStorage {
        raw: Some(serde_json::to_string(&profiles).unwrap()),
        ..MemoryStorage::default()
    };
    let restored = ShareProfiles::load_checked_with(None, &mut storage).unwrap();
    let entry = restored.legacy_direct_request(&selector).unwrap();
    assert_eq!(entry.decision, LegacyDirectDecisionState::Revoked);
    assert_eq!(
        entry.decision_delivery.state,
        LegacyDirectDeliveryState::LocalOnlyUntracked
    );
    assert!(!entry.authorization_active(&restored));
    assert!(restored.legacy_answers_due(120).is_empty());
}

#[test]
fn far_future_presence_cannot_pin_inbox_or_tombstone_capacity() {
    let identity = identity();
    let mut profiles = ShareProfiles::default();
    for index in 0..100 {
        let request = presence(
            &identity,
            2,
            &format!("far-future-{index}"),
            100 + super::legacy_direct_request::MAX_LEGACY_PRESENCE_FUTURE_SECS + 1,
        );
        assert!(profiles
            .record_verified_legacy_direct_request(&identity.direct_lookup_id, &request, 100)
            .unwrap_err()
            .contains("future window"));
    }
    assert!(profiles.legacy_direct_requests.is_empty());
    assert!(profiles.legacy_direct_request_tombstones.is_empty());

    let valid = presence(
        &identity,
        2,
        "valid",
        100 + super::legacy_direct_request::MAX_LEGACY_PRESENCE_FUTURE_SECS,
    );
    assert!(profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &valid, 100)
        .unwrap());
}

#[test]
fn identity_rotation_disables_even_unlinked_direct_and_exec_grants() {
    let key = peer_key(3);
    let mut profiles = ShareProfiles::default();
    profiles.direct_grants.push(DirectGrant {
        device_id: "unlinked-device".into(),
        device_name: "Unlinked".into(),
        public_key: key.clone(),
        fingerprint: public_fingerprint(key.as_bytes()),
        node_id: key,
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: super::exec_policy::ExecGrant {
            enabled: true,
            ..Default::default()
        },
    });

    assert_eq!(profiles.invalidate_all_direct_grants(100), 1);
    assert_eq!(profiles.direct_grants[0].state, DirectGrantState::Ignored);
    assert!(!profiles.direct_grants[0].exec.enabled);
    assert_eq!(profiles.direct_grants[0].exec.policy_revision, 1);
    assert_eq!(profiles.invalidate_all_direct_grants(101), 0);
    assert_eq!(profiles.direct_grants[0].exec.policy_revision, 1);
}

#[test]
fn corrupt_evidence_and_future_schema_fail_closed_while_v6_defaults_empty() {
    let identity = identity();
    let mut profiles = ShareProfiles::default();
    let request = presence(&identity, 2, "nonce-a", 200);
    profiles
        .record_verified_legacy_direct_request(&identity.direct_lookup_id, &request, 100)
        .unwrap();
    profiles.legacy_direct_requests[0].evidence.proof = "corrupt".into();
    assert!(profiles.validate_legacy_direct_requests().is_err());

    let mut storage = MemoryStorage {
        raw: Some(r#"{"schema_version":6}"#.into()),
        ..MemoryStorage::default()
    };
    let migrated = ShareProfiles::load_checked_with(None, &mut storage).unwrap();
    assert_eq!(migrated.schema_version, 7);
    assert!(migrated.legacy_direct_requests.is_empty());

    storage.raw = Some(r#"{"schema_version":8}"#.into());
    assert!(ShareProfiles::load_checked_with(None, &mut storage).is_err());
}

fn identity() -> ShareIdentity {
    let secret = iroh::SecretKey::from_bytes(&[1; 32]);
    let public = secret.public().to_string();
    ShareIdentity {
        device_id: "local-device".into(),
        device_name: "Local".into(),
        direct_lookup_id: "local-lookup".into(),
        public_key: public.clone(),
        fingerprint: public_fingerprint(public.as_bytes()),
        node_id: public,
        iroh_secret: secret,
        direct_secret: [7; 32],
    }
}

fn presence(identity: &ShareIdentity, seed: u8, nonce: &str, expires_at: i64) -> PeerPresence {
    let public = peer_key(seed);
    let mut presence = PeerPresence {
        kind: "direct".into(),
        relation_id: identity.direct_lookup_id.clone(),
        device_id: "peer-device".into(),
        device_name: "Peer".into(),
        public_key: public.clone(),
        fingerprint: public_fingerprint(public.as_bytes()),
        node_id: public,
        relay_url: "https://relay.invalid".into(),
        candidates: vec!["127.0.0.1:9000".into()],
        expires_at,
        nonce: nonce.into(),
        proof: String::new(),
    };
    let payload = presence_payload(
        "direct",
        &presence.relation_id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
        &presence.relay_url,
        &presence.candidates,
        presence.expires_at,
        &presence.nonce,
    );
    presence.proof = hmac_proof(&identity.direct_secret(), &payload);
    presence
}

fn peer_key(seed: u8) -> String {
    iroh::SecretKey::from_bytes(&[seed; 32])
        .public()
        .to_string()
}

#[derive(Default)]
struct MemoryStorage {
    raw: Option<String>,
    secrets: HashMap<String, String>,
}

impl ProfilePersistence for MemoryStorage {
    fn load_profiles(&mut self) -> Result<Option<String>, String> {
        Ok(self.raw.clone())
    }

    fn save_profiles(
        &mut self,
        contents: &str,
        _expected: &ProfileRevision,
    ) -> Result<ProfileRevision, String> {
        self.raw = Some(contents.into());
        Ok(ProfileRevision::from_contents(contents))
    }

    fn save_secret(&mut self, account: &str, secret: &str) -> Result<(), String> {
        self.secrets.insert(account.into(), secret.into());
        Ok(())
    }

    fn delete_secret(&mut self, account: &str) -> Result<(), String> {
        self.secrets.remove(account);
        Ok(())
    }
}
