pub(super) fn pending_tracked(
    profiles: &crate::share::ShareProfiles,
    now: i64,
) -> Vec<&crate::share::DirectRequestEntry> {
    profiles
        .direct_requests
        .iter()
        .filter(|entry| is_pending_incoming(entry, now))
        .collect()
}

pub(super) fn is_pending_incoming(entry: &crate::share::DirectRequestEntry, now: i64) -> bool {
    entry.direction == crate::share::DirectRequestDirection::Incoming
        && entry.record.decision.state == crate::share::DirectDecisionState::Pending
        && entry.record.request.expires_at >= now
}

pub(in crate::cli) fn tracked_accept_eligible(
    profiles: &crate::share::ShareProfiles,
    entry: &crate::share::DirectRequestEntry,
    now: i64,
) -> bool {
    is_pending_incoming(entry, now)
        && !profiles.tracked_identity_conflict(&entry.record.request.request_id)
}

pub(in crate::cli) fn legacy_accept_eligible(
    entry: &crate::share::LegacyDirectRequestEntry,
    now: i64,
) -> bool {
    entry.is_pending(now) && !entry.identity_conflict
}

pub(in crate::cli) fn tracked_conflict_resolution_commands(
    profiles: &crate::share::ShareProfiles,
    entry: &crate::share::DirectRequestEntry,
) -> Vec<String> {
    if !profiles.tracked_identity_conflict(&entry.record.request.request_id) {
        return Vec::new();
    }
    let mut commands = blocker_resolution_commands(
        profiles,
        &entry.record.request.requester,
        Some(entry.record.request.request_id.as_str()),
        None,
    );
    match entry.record.decision.state {
        crate::share::DirectDecisionState::Pending => {
            push_unique(
                &mut commands,
                format!(
                    "se share request reject {}",
                    entry.record.request.request_id
                ),
            );
            push_unique(
                &mut commands,
                format!(
                    "se share request delete {}",
                    entry.record.request.request_id
                ),
            );
        }
        crate::share::DirectDecisionState::Accepted => push_unique(
            &mut commands,
            format!("se share grants revoke {}", entry.record.request.request_id),
        ),
        _ => {}
    }
    commands
}

pub(in crate::cli) fn legacy_conflict_resolution_commands(
    profiles: &crate::share::ShareProfiles,
    entry: &crate::share::LegacyDirectRequestEntry,
) -> Vec<String> {
    if !entry.identity_conflict {
        return Vec::new();
    }
    let mut commands =
        blocker_resolution_commands(profiles, &entry.peer, None, Some(&entry.selector));
    match entry.decision {
        crate::share::LegacyDirectDecisionState::Pending => {
            push_unique(
                &mut commands,
                format!("se share request reject {}", entry.selector),
            );
            push_unique(
                &mut commands,
                format!("se share request delete {}", entry.selector),
            );
        }
        crate::share::LegacyDirectDecisionState::Accepted => push_unique(
            &mut commands,
            format!("se share grants revoke {}", entry.selector),
        ),
        _ => {}
    }
    commands
}

fn blocker_resolution_commands(
    profiles: &crate::share::ShareProfiles,
    peer: &crate::share::DirectPeerIdentity,
    current_tracked: Option<&str>,
    current_legacy: Option<&str>,
) -> Vec<String> {
    let mut commands = Vec::new();
    for grant in profiles.direct_grants.iter().filter(|grant| {
        grant.state == crate::share::DirectGrantState::Accepted
            && peer_conflicts_with_grant(peer, grant)
    }) {
        push_unique(
            &mut commands,
            format!("se share grants revoke {}", grant.fingerprint),
        );
    }
    for sibling in profiles.direct_requests.iter().filter(|sibling| {
        sibling.direction == crate::share::DirectRequestDirection::Incoming
            && current_tracked != Some(sibling.record.request.request_id.as_str())
            && peer_identities_conflict(peer, &sibling.record.request.requester)
    }) {
        let selector = sibling.record.request.request_id.as_str();
        match sibling.record.decision.state {
            crate::share::DirectDecisionState::Pending => {
                push_unique(&mut commands, format!("se share request reject {selector}"));
                push_unique(&mut commands, format!("se share request delete {selector}"));
            }
            crate::share::DirectDecisionState::Accepted => {
                push_unique(&mut commands, format!("se share grants revoke {selector}"))
            }
            _ => {}
        }
    }
    for sibling in profiles.legacy_direct_requests.iter().filter(|sibling| {
        current_legacy != Some(sibling.selector.as_str())
            && peer_identities_conflict(peer, &sibling.peer)
    }) {
        match sibling.decision {
            crate::share::LegacyDirectDecisionState::Pending => {
                push_unique(
                    &mut commands,
                    format!("se share request reject {}", sibling.selector),
                );
                push_unique(
                    &mut commands,
                    format!("se share request delete {}", sibling.selector),
                );
            }
            crate::share::LegacyDirectDecisionState::Accepted => push_unique(
                &mut commands,
                format!("se share grants revoke {}", sibling.selector),
            ),
            _ => {}
        }
    }
    commands
}

fn peer_conflicts_with_grant(
    peer: &crate::share::DirectPeerIdentity,
    grant: &crate::share::DirectGrant,
) -> bool {
    peer.device_id == grant.device_id
        && (peer.public_key != grant.public_key
            || peer.node_id != grant.node_id
            || peer.fingerprint != grant.fingerprint)
}

fn peer_identities_conflict(
    left: &crate::share::DirectPeerIdentity,
    right: &crate::share::DirectPeerIdentity,
) -> bool {
    left.device_id == right.device_id
        && (left.public_key != right.public_key
            || left.node_id != right.node_id
            || left.fingerprint != right.fingerprint)
}

fn push_unique(commands: &mut Vec<String>, command: String) {
    if !commands.contains(&command) {
        commands.push(command);
    }
}

pub(crate) fn legacy_retryable(entry: &crate::share::LegacyDirectRequestEntry) -> bool {
    matches!(
        entry.decision_delivery.state,
        crate::share::LegacyDirectDeliveryState::AttemptedUntracked
            | crate::share::LegacyDirectDeliveryState::FailedUntracked
    )
}

pub(super) fn tracked_retryable(entry: &crate::share::DirectRequestEntry, now: i64) -> bool {
    !entry.manually_retryable_outboxes(now).is_empty()
}

pub(crate) fn tracked_deletable(entry: &crate::share::DirectRequestEntry, now: i64) -> bool {
    entry.direction == crate::share::DirectRequestDirection::Outgoing
        || entry.record.decision.state == crate::share::DirectDecisionState::Pending
        || entry.removable_from_history(now)
}

pub(super) fn pending_legacy(
    profiles: &crate::share::ShareProfiles,
    now: i64,
) -> Vec<&crate::share::LegacyDirectRequestEntry> {
    profiles
        .legacy_direct_requests
        .iter()
        .filter(|request| request.is_pending(now))
        .collect()
}

pub(super) fn matching_tracked<'a>(
    profiles: &'a crate::share::ShareProfiles,
    selector: Option<&str>,
    predicate: impl Fn(&crate::share::DirectRequestEntry) -> bool,
) -> Vec<&'a crate::share::DirectRequestEntry> {
    profiles
        .direct_requests
        .iter()
        .filter(|entry| predicate(entry))
        .filter(|entry| {
            selector.is_none_or(|selector| tracked_selector_matches(entry, selector.trim()))
        })
        .collect()
}

pub(super) fn matching_legacy<'a>(
    requests: &'a [&'a crate::share::LegacyDirectRequestEntry],
    selector: Option<&str>,
) -> Vec<&'a crate::share::LegacyDirectRequestEntry> {
    requests
        .iter()
        .copied()
        .filter(|request| {
            selector.is_none_or(|selector| legacy_selector_matches(request, selector.trim()))
        })
        .collect()
}

pub(super) fn verify_optional_fingerprint(
    supplied: Option<&str>,
    expected: &str,
    selector: &impl std::fmt::Display,
) -> Result<(), String> {
    if supplied.is_some_and(|value| !value.trim().eq_ignore_ascii_case(expected)) {
        Err(format!(
            "fingerprint mismatch for {selector}: expected {expected}"
        ))
    } else {
        Ok(())
    }
}

pub(super) fn no_pending_error(
    selector: Option<&str>,
    profiles: &crate::share::ShareProfiles,
    legacy: &[&crate::share::LegacyDirectRequestEntry],
    now: i64,
) -> String {
    let available = pending_tracked(profiles, now).len() + legacy.len();
    match selector {
        Some(selector) => format!(
            "pending incoming request not found: {selector}; {available} pending. Run `se share request` to list valid selectors"
        ),
        None => "no pending incoming requests; `se share request` shows the inbox".to_string(),
    }
}

pub(super) fn no_acceptable_error(
    selector: Option<&str>,
    profiles: &crate::share::ShareProfiles,
    now: i64,
) -> String {
    let tracked = pending_tracked(profiles, now)
        .into_iter()
        .filter(|entry| {
            selector.is_none_or(|selector| tracked_selector_matches(entry, selector.trim()))
        })
        .filter(|entry| profiles.tracked_identity_conflict(&entry.record.request.request_id))
        .collect::<Vec<_>>();
    let legacy = pending_legacy(profiles, now)
        .into_iter()
        .filter(|entry| {
            selector.is_none_or(|selector| legacy_selector_matches(entry, selector.trim()))
        })
        .filter(|entry| entry.identity_conflict)
        .collect::<Vec<_>>();
    if tracked.is_empty() && legacy.is_empty() {
        return no_pending_error(selector, profiles, &pending_legacy(profiles, now), now);
    }
    let mut commands = Vec::new();
    for entry in tracked {
        for command in tracked_conflict_resolution_commands(profiles, entry) {
            push_unique(&mut commands, command);
        }
    }
    for entry in legacy {
        for command in legacy_conflict_resolution_commands(profiles, entry) {
            push_unique(&mut commands, command);
        }
    }
    let commands = commands
        .iter()
        .map(|command| format!("`{command}`"))
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "identity-conflicting request(s) cannot be accepted; resolve the conflicting claim first: {commands}"
    )
}

pub(super) fn ambiguous_pending_error(
    tracked: &[&crate::share::DirectRequestEntry],
    legacy: &[&crate::share::LegacyDirectRequestEntry],
) -> String {
    let mut selectors = tracked
        .iter()
        .map(|entry| entry.record.request.request_id.as_str())
        .collect::<Vec<_>>();
    selectors.extend(legacy.iter().map(|request| request.selector.as_str()));
    format!(
        "multiple pending incoming requests; choose one selector shown by `se share request`: {}",
        selectors.join(", ")
    )
}

pub(super) fn legacy_selector_matches(
    entry: &crate::share::LegacyDirectRequestEntry,
    selector: &str,
) -> bool {
    exact_or_prefix(selector, &entry.selector)
        || peer_selector_matches(
            selector,
            &entry.peer.device_id,
            &entry.peer.device_name,
            &entry.peer.fingerprint,
        )
}

fn tracked_selector_matches(entry: &crate::share::DirectRequestEntry, selector: &str) -> bool {
    let request = &entry.record.request;
    let peer = match entry.direction {
        crate::share::DirectRequestDirection::Incoming => &request.requester,
        crate::share::DirectRequestDirection::Outgoing => &request.target,
    };
    exact_or_prefix(selector, request.request_id.as_str())
        || peer_selector_matches(
            selector,
            &peer.device_id,
            &peer.device_name,
            &peer.fingerprint,
        )
        || entry
            .contact_id
            .as_deref()
            .is_some_and(|contact_id| exact_or_prefix(selector, contact_id))
}

fn peer_selector_matches(
    selector: &str,
    device_id: &str,
    device_name: &str,
    fingerprint: &str,
) -> bool {
    device_name.eq_ignore_ascii_case(selector)
        || exact_or_prefix(selector, device_id)
        || exact_or_prefix(selector, fingerprint)
}

fn exact_or_prefix(selector: &str, candidate: &str) -> bool {
    candidate.eq_ignore_ascii_case(selector)
        || (selector.len() >= 4
            && candidate
                .to_ascii_lowercase()
                .starts_with(&selector.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::{exact_or_prefix, peer_selector_matches};

    #[test]
    fn visible_exact_values_and_unambiguous_length_prefixes_match() {
        assert!(peer_selector_matches(
            "257b936f",
            "device-a",
            "Silas Asus",
            "257b936f2d590cc01a9008579ca2352b"
        ));
        assert!(peer_selector_matches(
            "silas asus",
            "device-a",
            "Silas Asus",
            "257b936f2d590cc01a9008579ca2352b"
        ));
        assert!(exact_or_prefix(
            "01234567",
            "01234567-89ab-4def-8123-456789abcdef"
        ));
        assert!(!exact_or_prefix(
            "012",
            "01234567-89ab-4def-8123-456789abcdef"
        ));
    }
}
