pub(super) fn request_candidates() -> Vec<clap_complete::CompletionCandidate> {
    load_profiles()
        .map(|profiles| request_candidates_from_profiles(&profiles))
        .unwrap_or_default()
}

pub(super) fn pending_request_candidates() -> Vec<clap_complete::CompletionCandidate> {
    let now = crate::share::core_now_secs();
    load_profiles()
        .map(|profiles| pending_from_profiles(&profiles, now))
        .unwrap_or_default()
}

pub(super) fn acceptable_request_candidates() -> Vec<clap_complete::CompletionCandidate> {
    let now = crate::share::core_now_secs();
    load_profiles()
        .map(|profiles| acceptable_from_profiles(&profiles, now))
        .unwrap_or_default()
}

pub(super) fn retry_request_candidates() -> Vec<clap_complete::CompletionCandidate> {
    let now = crate::share::core_now_secs();
    load_profiles()
        .map(|profiles| retry_from_profiles(&profiles, now))
        .unwrap_or_default()
}

pub(super) fn deletable_request_candidates() -> Vec<clap_complete::CompletionCandidate> {
    let now = crate::share::core_now_secs();
    load_profiles()
        .map(|profiles| deletable_from_profiles(&profiles, now))
        .unwrap_or_default()
}

fn retry_from_profiles(
    profiles: &crate::share::ShareProfiles,
    now: i64,
) -> Vec<clap_complete::CompletionCandidate> {
    let tracked = profiles
        .direct_requests
        .iter()
        .filter(|entry| !entry.manually_retryable_outboxes(now).is_empty())
        .map(tracked_candidate);
    let legacy = profiles
        .legacy_direct_requests
        .iter()
        .filter(|entry| super::share::request_selection::legacy_retryable(entry))
        .map(legacy_candidate);
    tracked.chain(legacy).collect()
}

fn deletable_from_profiles(
    profiles: &crate::share::ShareProfiles,
    now: i64,
) -> Vec<clap_complete::CompletionCandidate> {
    let tracked = profiles
        .direct_requests
        .iter()
        .filter(|entry| super::share::request_selection::tracked_deletable(entry, now))
        .map(tracked_candidate);
    let legacy = profiles
        .legacy_direct_requests
        .iter()
        .filter(|entry| !entry.authorization_active(profiles))
        .map(legacy_candidate);
    tracked.chain(legacy).collect()
}

fn request_candidates_from_profiles(
    profiles: &crate::share::ShareProfiles,
) -> Vec<clap_complete::CompletionCandidate> {
    profiles
        .direct_requests
        .iter()
        .map(tracked_candidate)
        .chain(profiles.legacy_direct_requests.iter().map(legacy_candidate))
        .collect()
}

fn pending_from_profiles(
    profiles: &crate::share::ShareProfiles,
    now: i64,
) -> Vec<clap_complete::CompletionCandidate> {
    let tracked = profiles
        .direct_requests
        .iter()
        .filter(|entry| {
            entry.direction == crate::share::DirectRequestDirection::Incoming
                && entry.record.decision.state == crate::share::DirectDecisionState::Pending
                && entry.record.request.expires_at >= now
        })
        .map(tracked_candidate);
    let legacy = profiles
        .legacy_direct_requests
        .iter()
        .filter(|entry| entry.is_pending(now))
        .map(legacy_candidate);
    tracked.chain(legacy).collect()
}

fn acceptable_from_profiles(
    profiles: &crate::share::ShareProfiles,
    now: i64,
) -> Vec<clap_complete::CompletionCandidate> {
    let tracked = profiles
        .direct_requests
        .iter()
        .filter(|entry| {
            super::share::request_selection::tracked_accept_eligible(profiles, entry, now)
        })
        .map(tracked_candidate);
    let legacy = profiles
        .legacy_direct_requests
        .iter()
        .filter(|entry| super::share::request_selection::legacy_accept_eligible(entry, now))
        .map(legacy_candidate);
    tracked.chain(legacy).collect()
}

fn tracked_candidate(
    entry: &crate::share::DirectRequestEntry,
) -> clap_complete::CompletionCandidate {
    let request = &entry.record.request;
    let peer = match entry.direction {
        crate::share::DirectRequestDirection::Incoming => &request.requester,
        crate::share::DirectRequestDirection::Outgoing => &request.target,
    };
    candidate(
        request.request_id.as_str(),
        format!(
            "{} · {} · decision={} · delivery={}",
            direction(entry.direction),
            peer.device_name,
            entry.record.decision.state.code(),
            entry.record.delivery.state.code(),
        ),
    )
}

fn legacy_candidate(
    entry: &crate::share::LegacyDirectRequestEntry,
) -> clap_complete::CompletionCandidate {
    candidate(
        &entry.selector,
        format!(
            "legacy · {} · decision={} · delivery={}",
            entry.peer.device_name,
            entry.decision.code(),
            entry.decision_delivery.state.code(),
        ),
    )
}

fn load_profiles() -> Option<crate::share::ShareProfiles> {
    crate::share::ShareProfiles::load_checked(Some(super::share::default_home())).ok()
}

fn candidate(value: &str, help: String) -> clap_complete::CompletionCandidate {
    let help = help.replace(['\t', '\r', '\n'], " ");
    clap_complete::CompletionCandidate::new(value).help(Some(clap::builder::StyledStr::from(help)))
}

fn direction(value: crate::share::DirectRequestDirection) -> &'static str {
    match value {
        crate::share::DirectRequestDirection::Incoming => "incoming",
        crate::share::DirectRequestDirection::Outgoing => "outgoing",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        acceptable_from_profiles, deletable_from_profiles, pending_from_profiles,
        request_candidates_from_profiles, retry_from_profiles,
    };

    #[test]
    fn legacy_history_pending_retry_and_delete_candidates_use_visible_selector() {
        let mut profiles = crate::share::ShareProfiles::default();
        profiles.legacy_direct_requests.push(entry());
        assert_values(
            request_candidates_from_profiles(&profiles),
            &["legacy-selector"],
        );
        assert_values(pending_from_profiles(&profiles, 100), &["legacy-selector"]);
        assert_values(
            acceptable_from_profiles(&profiles, 100),
            &["legacy-selector"],
        );
        assert_values(
            deletable_from_profiles(&profiles, 100),
            &["legacy-selector"],
        );
        assert_values(retry_from_profiles(&profiles, 100), &[]);

        let entry = &mut profiles.legacy_direct_requests[0];
        entry.decision = crate::share::LegacyDirectDecisionState::Rejected;
        entry.decision_source = Some(crate::share::LegacyDirectDecisionSource::User);
        entry.decision_revision = 1;
        entry.decision_delivery.state = crate::share::LegacyDirectDeliveryState::AttemptedUntracked;
        entry.decision_delivery.decision_revision = 1;
        entry.decision_delivery.attempt_count = 1;
        entry.decision_delivery.last_attempt_at = Some(110);
        assert_values(pending_from_profiles(&profiles, 100), &[]);
        assert_values(acceptable_from_profiles(&profiles, 100), &[]);
        assert_values(retry_from_profiles(&profiles, 100), &["legacy-selector"]);
        assert_values(
            deletable_from_profiles(&profiles, 100),
            &["legacy-selector"],
        );
    }

    #[test]
    fn accept_completion_excludes_conflicts_but_reject_completion_keeps_them() {
        let mut profiles = crate::share::ShareProfiles::default();
        let mut legacy = entry();
        legacy.identity_conflict = true;
        profiles.legacy_direct_requests.push(legacy);

        assert_values(pending_from_profiles(&profiles, 100), &["legacy-selector"]);
        assert_values(acceptable_from_profiles(&profiles, 100), &[]);
    }

    #[test]
    fn tracked_conflict_disappears_from_accept_completion_until_peer_claim_is_rejected() {
        let mut profiles = crate::share::ShareProfiles::default();
        profiles
            .direct_requests
            .push(tracked_entry(3, "01234567-89ab-4def-8123-456789abcdef"));
        profiles
            .direct_requests
            .push(tracked_entry(4, "11234567-89ab-4def-8123-456789abcdef"));

        assert_values(
            pending_from_profiles(&profiles, 100),
            &[
                "01234567-89ab-4def-8123-456789abcdef",
                "11234567-89ab-4def-8123-456789abcdef",
            ],
        );
        assert_values(acceptable_from_profiles(&profiles, 100), &[]);

        profiles.direct_requests[0].record.decision.state =
            crate::share::DirectDecisionState::Rejected;
        assert_values(
            acceptable_from_profiles(&profiles, 100),
            &["11234567-89ab-4def-8123-456789abcdef"],
        );
    }

    #[test]
    fn active_old_grant_blocks_only_accept_completion_until_revoked() {
        let mut profiles = crate::share::ShareProfiles::default();
        profiles
            .direct_requests
            .push(tracked_entry(3, "21234567-89ab-4def-8123-456789abcdef"));
        let blocker = tracked_entry(4, "31234567-89ab-4def-8123-456789abcdef")
            .record
            .request
            .requester;
        profiles.direct_grants.push(crate::share::DirectGrant {
            device_id: blocker.device_id,
            device_name: blocker.device_name,
            public_key: blocker.public_key,
            fingerprint: blocker.fingerprint,
            node_id: blocker.node_id,
            state: crate::share::DirectGrantState::Accepted,
            updated_at: 50,
            exec: Default::default(),
        });

        assert_values(
            pending_from_profiles(&profiles, 100),
            &["21234567-89ab-4def-8123-456789abcdef"],
        );
        assert_values(acceptable_from_profiles(&profiles, 100), &[]);
        profiles.direct_grants[0].state = crate::share::DirectGrantState::Ignored;
        assert_values(
            acceptable_from_profiles(&profiles, 100),
            &["21234567-89ab-4def-8123-456789abcdef"],
        );
    }

    fn entry() -> crate::share::LegacyDirectRequestEntry {
        crate::share::LegacyDirectRequestEntry {
            selector: "legacy-selector".into(),
            lookup_id: "lookup".into(),
            peer: crate::share::DirectPeerIdentity {
                device_id: "device".into(),
                device_name: "Peer".into(),
                node_id: "node".into(),
                public_key: "public".into(),
                fingerprint: "fingerprint".into(),
            },
            evidence: crate::share::LegacyDirectPresenceEvidence {
                event_id: "event".into(),
                relay_url: String::new(),
                candidates: Vec::new(),
                expires_at: 200,
                nonce: "nonce".into(),
                proof: "proof".into(),
            },
            first_received_at: 90,
            last_received_at: 90,
            decision: crate::share::LegacyDirectDecisionState::Pending,
            decision_source: None,
            decision_changed_at: 90,
            decision_revision: 0,
            decision_delivery: Default::default(),
            identity_conflict: false,
        }
    }

    fn tracked_entry(secret_byte: u8, request_id: &str) -> crate::share::DirectRequestEntry {
        let requester_secret = iroh::SecretKey::from_bytes(&[secret_byte; 32]);
        let target_secret = iroh::SecretKey::from_bytes(&[7; 32]);
        let target =
            crate::share::DirectPeerIdentity::from_secret("target", "Target", &target_secret);
        let request = crate::share::SignedDirectRequest::sign_with_nonce(
            crate::share::DirectRequestId::parse(request_id).unwrap(),
            "lookup",
            crate::share::DirectPeerIdentity::from_secret(
                "shared-device",
                "Requester",
                &requester_secret,
            ),
            crate::share::DirectPeerIdentity::pinned_target(target.node_id, target.fingerprint),
            10,
            1_000,
            format!("nonce-{secret_byte}"),
            None,
            &[9; 32],
            &requester_secret,
        )
        .unwrap();
        crate::share::DirectRequestEntry {
            direction: crate::share::DirectRequestDirection::Incoming,
            contact_id: None,
            local_lookup_id: Some("lookup".into()),
            record: crate::share::DirectRequestRecord::new(request),
            request_receipt: None,
            decision: None,
            decision_receipt: None,
            retries: Default::default(),
        }
    }

    fn assert_values(candidates: Vec<clap_complete::CompletionCandidate>, expected: &[&str]) {
        let values = candidates
            .iter()
            .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(values, expected);
    }
}
