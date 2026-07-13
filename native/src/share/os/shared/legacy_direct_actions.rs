use super::identity::IdentityRepairAction;
use super::identity::ShareIdentity;
use super::legacy_direct_request::{LegacyDirectDecisionState, LegacyDirectRequestEntry};
use super::profiles::ShareProfiles;

pub fn decide_legacy_direct_request(
    default_home: Option<String>,
    identity: &ShareIdentity,
    selector: &str,
    expected_fingerprint: &str,
    accepted: bool,
) -> Result<LegacyDirectRequestEntry, String> {
    ShareIdentity::with_current_locked(identity.device_name.clone(), |current| {
        super::identity_store::with_matching_identity_generation(identity, current, |locked| {
            decide_legacy_direct_request_locked(
                default_home,
                locked,
                selector,
                expected_fingerprint,
                accepted,
            )
        })
    })
}

fn decide_legacy_direct_request_locked(
    default_home: Option<String>,
    identity: &ShareIdentity,
    selector: &str,
    expected_fingerprint: &str,
    accepted: bool,
) -> Result<LegacyDirectRequestEntry, String> {
    let now = super::core::now_secs();
    let profiles = ShareProfiles::load_checked(default_home.clone())?;
    let expected = profiles
        .legacy_direct_request(selector)
        .cloned()
        .ok_or_else(|| format!("legacy request not found: {selector}"))?;
    if !expected
        .peer
        .fingerprint
        .eq_ignore_ascii_case(expected_fingerprint.trim())
    {
        return Err(format!(
            "fingerprint mismatch for {selector}: expected {}",
            expected.peer.fingerprint
        ));
    }
    expected.verify_for_local_identity(identity, now)?;
    let event_id = expected.evidence.event_id.clone();
    let committed = ShareProfiles::mutate_persisted(default_home, |candidate| {
        let current = candidate
            .legacy_direct_request(selector)
            .ok_or_else(|| format!("legacy request disappeared: {selector}"))?;
        if current.evidence.event_id != event_id || current.peer != expected.peer {
            return Err(format!("legacy request changed concurrently: {selector}"));
        }
        current.verify_for_local_identity(identity, now)?;
        candidate.decide_legacy_direct_request(selector, accepted, now)
    })?;
    committed
        .legacy_direct_request(selector)
        .cloned()
        .ok_or_else(|| format!("persisted legacy request is missing: {selector}"))
}

pub fn delete_legacy_direct_request(
    default_home: Option<String>,
    selector: &str,
) -> Result<bool, String> {
    let now = super::core::now_secs();
    let mut deleted = false;
    ShareProfiles::mutate_persisted(default_home, |profiles| {
        deleted = profiles.delete_legacy_direct_request(selector, now)?;
        Ok(())
    })?;
    Ok(deleted)
}

pub fn revoke_legacy_direct_request(
    default_home: Option<String>,
    selector: &str,
) -> Result<LegacyDirectRequestEntry, String> {
    let now = super::core::now_secs();
    let committed = ShareProfiles::mutate_persisted(default_home, |profiles| {
        profiles.revoke_legacy_direct_request(selector, now)
    })?;
    committed
        .legacy_direct_request(selector)
        .cloned()
        .ok_or_else(|| format!("persisted legacy request is missing: {selector}"))
}

pub fn retry_legacy_direct_answer(
    default_home: Option<String>,
    selector: &str,
) -> Result<LegacyDirectRequestEntry, String> {
    let committed = ShareProfiles::mutate_persisted(default_home, |profiles| {
        profiles.retry_legacy_answer(selector)
    })?;
    committed
        .legacy_direct_request(selector)
        .cloned()
        .ok_or_else(|| format!("persisted legacy request is missing: {selector}"))
}

pub(crate) fn mark_legacy_answer_attempt(
    default_home: Option<String>,
    selector: &str,
    decision_revision: u64,
    error: Option<String>,
) -> Result<ShareProfiles, String> {
    let now = super::core::now_secs();
    ShareProfiles::mutate_persisted(default_home, |profiles| {
        profiles
            .record_legacy_answer_attempt(selector, decision_revision, now, error.clone())
            .map(|_| ())
    })
}

pub fn refresh_legacy_request_expiry(
    default_home: Option<String>,
    identity: &ShareIdentity,
) -> Result<ShareProfiles, String> {
    let now = super::core::now_secs();
    let current = reconcile_legacy_identity(default_home.clone(), identity)?;
    current.validate_legacy_evidence(identity)?;
    if !current.legacy_direct_requests.iter().any(|entry| {
        entry.decision == LegacyDirectDecisionState::Pending && entry.evidence.expires_at < now
    }) {
        return Ok(current);
    }
    ShareProfiles::mutate_persisted(default_home, |profiles| {
        profiles.validate_legacy_evidence(identity)?;
        profiles.expire_legacy_direct_requests(now);
        Ok(())
    })
}

pub fn reconcile_legacy_identity(
    default_home: Option<String>,
    identity: &ShareIdentity,
) -> Result<ShareProfiles, String> {
    let now = super::core::now_secs();
    let current = ShareProfiles::load_checked(default_home.clone())?;
    if !current
        .legacy_direct_requests
        .iter()
        .any(|entry| entry.lookup_id != identity.direct_lookup_id)
    {
        return Ok(current);
    }
    let committed = ShareProfiles::mutate_persisted(default_home, |profiles| {
        profiles
            .reconcile_legacy_identity(&identity.direct_lookup_id, now)
            .map(|_| ())
    })?;
    committed.validate_legacy_evidence(identity)?;
    Ok(committed)
}

pub(crate) fn invalidate_direct_grants_after_identity_rotation(
    default_home: Option<String>,
    identity: &ShareIdentity,
    action: IdentityRepairAction,
) -> Result<ShareProfiles, String> {
    let now = super::core::now_secs();
    let committed = ShareProfiles::mutate_persisted(default_home, |profiles| {
        profiles
            .invalidate_direct_grants_after_identity_rotation(
                &identity.direct_lookup_id,
                now,
                action,
            )
            .map(|_| ())
    })?;
    committed.validate_legacy_evidence(identity)?;
    Ok(committed)
}
