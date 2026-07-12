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

pub(super) fn pending_legacy(
    requests: Vec<crate::share::PeerPresence>,
    now: i64,
) -> Vec<crate::share::PeerPresence> {
    requests
        .into_iter()
        .filter(|request| request.expires_at >= now)
        .collect()
}

pub(super) fn select_tracked<'a>(
    profiles: &'a crate::share::ShareProfiles,
    selector: Option<&str>,
    predicate: impl Fn(&crate::share::DirectRequestEntry) -> bool,
    kind: &str,
) -> Result<&'a crate::share::DirectRequestEntry, String> {
    let matches = matching_tracked(profiles, selector, predicate);
    match matches.as_slice() {
        [] => Err(format!(
            "{kind} not found: {}",
            selector.unwrap_or("<only item>")
        )),
        [entry] => Ok(*entry),
        many => Err(format!(
            "{kind} selector is ambiguous; use one request UUID: {}",
            many.iter()
                .map(|entry| entry.record.request.request_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
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
    requests: &'a [crate::share::PeerPresence],
    selector: Option<&str>,
) -> Vec<&'a crate::share::PeerPresence> {
    requests
        .iter()
        .filter(|request| {
            selector.is_none_or(|selector| {
                peer_selector_matches(
                    selector.trim(),
                    &request.device_id,
                    &request.device_name,
                    &request.fingerprint,
                )
            })
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
    legacy: &[crate::share::PeerPresence],
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

pub(super) fn ambiguous_pending_error(
    tracked: &[&crate::share::DirectRequestEntry],
    legacy: &[&crate::share::PeerPresence],
) -> String {
    let mut selectors = tracked
        .iter()
        .map(|entry| entry.record.request.request_id.as_str())
        .collect::<Vec<_>>();
    selectors.extend(legacy.iter().map(|request| request.device_id.as_str()));
    format!(
        "multiple pending incoming requests; choose one selector shown by `se share request`: {}",
        selectors.join(", ")
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
