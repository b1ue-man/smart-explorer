use super::*;

fn refusal() -> Refusal {
    Refusal { identity_conflict: false,
    policy_denied: false,
    key_denied: false }
}

#[test]
fn review_task_legacy_new_peer_waits_reconfirm_accepts_and_key_denial_wins() {
    use LegacyDirectDecisionState as State;
    assert!(authenticated_decision(State::Pending, None, None, refusal(), DirectRequestPolicy::Ask).is_none());
    let reconfirm = authenticated_decision(
        State::Pending, None, Some(DirectGrantState::Reconfirm), refusal(), DirectRequestPolicy::Ask,
    ).unwrap();
    assert_eq!(reconfirm.decision, State::Accepted);
    assert!(reconfirm.install_grant);
    let mut denied = refusal();
    denied.key_denied = true;
    let result = authenticated_decision(
        State::Pending, None, Some(DirectGrantState::Accepted), denied, DirectRequestPolicy::AutoAccept,
    ).unwrap();
    assert_eq!(result.decision, State::Rejected);
    assert!(!result.install_grant);
}

#[test]
fn review_task_legacy_current_explicit_grant_supersedes_old_revocation_history() {
    let peer = DirectPeerIdentity::from_secret("a", "A", &iroh::SecretKey::from_bytes(&[4; 32]));
    let mut entry = LegacyDirectRequestEntry {
        selector: "legacy-a".into(),
        lookup_id: "lookup".into(), peer,
        evidence: crate::share::legacy_direct_request::LegacyDirectPresenceEvidence {
            event_id: "event".into(),
            relay_url: String::new(),
            candidates: Vec::new(),
            expires_at: 30,
            nonce: "nonce".into(),
            proof: "proof".into(),
        },
        first_received_at: 1,
        last_received_at: 10,
        decision: LegacyDirectDecisionState::Revoked,
        decision_source: Some(LegacyDirectDecisionSource::User),
        decision_changed_at: 5,
        decision_revision: 2,
        decision_delivery: Default::default(),
        identity_conflict: false,
    };
    let decision = authenticated_decision(
        entry.decision, entry.decision_source, Some(DirectGrantState::Accepted), refusal(), DirectRequestPolicy::Ask,
    );
    apply_authenticated_decision(&mut entry, decision, 11);
    assert_eq!(entry.decision, LegacyDirectDecisionState::Accepted);
    assert_eq!(entry.decision_source, Some(LegacyDirectDecisionSource::ExistingGrant));
    assert_eq!(entry.decision_revision, 3);
    assert_eq!(entry.decision_delivery.state, LegacyDirectDeliveryState::Queued);
}
