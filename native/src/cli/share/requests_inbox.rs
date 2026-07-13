use super::request_selection::{
    legacy_accept_eligible, pending_legacy, pending_tracked, tracked_accept_eligible,
};

pub(super) fn run(json: bool) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    let now = crate::share::core_now_secs();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&value(&profiles, now))
                .map_err(|error| error.to_string())?
        );
    } else {
        for line in text(&profiles, now) {
            println!("{line}");
        }
    }
    Ok(())
}

fn value(profiles: &crate::share::ShareProfiles, now: i64) -> serde_json::Value {
    let tracked = pending_tracked(profiles, now);
    let legacy = pending_legacy(profiles, now);
    let acceptable = acceptable_selectors(profiles, &tracked, &legacy, now);
    let count = tracked.len() + legacy.len();
    let next_command = next_command(profiles, &tracked, &legacy, &acceptable);
    serde_json::json!({
        "count": count,
        "acceptable_count": acceptable.len(),
        "blocked_count": count.saturating_sub(acceptable.len()),
        "requests": tracked.iter().map(|entry| {
            super::lifecycle_output::request_value(entry, profiles)
        }).collect::<Vec<_>>(),
        "legacy_requests": legacy.iter().map(|entry| {
            super::requests_legacy::value(entry, profiles)
        }).collect::<Vec<_>>(),
        "next_command": next_command,
        "history_count": profiles.direct_requests.len() + profiles.legacy_direct_requests.len(),
        "history_command": "se share request list",
    })
}

fn text(profiles: &crate::share::ShareProfiles, now: i64) -> Vec<String> {
    let tracked = pending_tracked(profiles, now);
    let legacy = pending_legacy(profiles, now);
    let acceptable = acceptable_selectors(profiles, &tracked, &legacy, now);
    let mut lines = vec![
        format!("pending_requests\t{}", tracked.len() + legacy.len()),
        format!("acceptable_requests\t{}", acceptable.len()),
        format!(
            "request_history\t{}",
            profiles.direct_requests.len() + profiles.legacy_direct_requests.len()
        ),
    ];
    for entry in &tracked {
        let request = &entry.record.request;
        let conflict = profiles.tracked_identity_conflict(&request.request_id);
        lines.push(format!(
            "pending_request\t{}\tdevice_name={}\tdevice_id={}\tfingerprint={}\tdelivery={}\tdecision={}\tauthorization=inactive\tidentity_conflict={conflict}",
            request.request_id,
            clean(&request.requester.device_name),
            clean(&request.requester.device_id),
            clean(&request.requester.fingerprint),
            entry.record.delivery.state.code(),
            entry.record.decision.state.code(),
        ));
        if conflict {
            append_resolution(
                &mut lines,
                super::request_selection::tracked_conflict_resolution_commands(profiles, entry),
            );
        }
    }
    for request in &legacy {
        lines.push(format!(
            "pending_legacy_request\t{}\tdevice_name={}\tdevice_id={}\tfingerprint={}\tdelivery=received\tdelivery_scope=local_persisted\tdecision=pending\tdecision_channel=not_applicable\tdecision_delivery=not_started\tauthorization=inactive\treceipt=unsupported\tidentity_conflict={}",
            clean(&request.selector),
            clean(&request.peer.device_name),
            clean(&request.peer.device_id),
            clean(&request.peer.fingerprint),
            request.identity_conflict,
        ));
        if request.identity_conflict {
            append_resolution(
                &mut lines,
                super::request_selection::legacy_conflict_resolution_commands(profiles, request),
            );
        }
    }
    if let Some(command) = next_command(profiles, &tracked, &legacy, &acceptable) {
        lines.push(format!("next\t{command}"));
    } else if acceptable.len() > 1 {
        lines.extend(
            acceptable
                .iter()
                .map(|selector| format!("accept\tse share request accept {selector}")),
        );
    }
    lines.push("history\tse share request list".into());
    lines
}

fn acceptable_selectors(
    profiles: &crate::share::ShareProfiles,
    tracked: &[&crate::share::DirectRequestEntry],
    legacy: &[&crate::share::LegacyDirectRequestEntry],
    now: i64,
) -> Vec<String> {
    tracked
        .iter()
        .filter(|entry| tracked_accept_eligible(profiles, entry, now))
        .map(|entry| entry.record.request.request_id.to_string())
        .chain(
            legacy
                .iter()
                .filter(|entry| legacy_accept_eligible(entry, now))
                .map(|entry| entry.selector.clone()),
        )
        .collect()
}

fn next_command(
    profiles: &crate::share::ShareProfiles,
    tracked: &[&crate::share::DirectRequestEntry],
    legacy: &[&crate::share::LegacyDirectRequestEntry],
    acceptable: &[String],
) -> Option<String> {
    if acceptable.len() == 1 {
        return Some("se share request accept".into());
    }
    if tracked.len() + legacy.len() != 1 {
        return None;
    }
    tracked
        .first()
        .filter(|entry| profiles.tracked_identity_conflict(&entry.record.request.request_id))
        .and_then(|entry| {
            super::request_selection::tracked_conflict_resolution_commands(profiles, entry)
                .into_iter()
                .next()
        })
        .or_else(|| {
            legacy
                .first()
                .filter(|entry| entry.identity_conflict)
                .and_then(|entry| {
                    super::request_selection::legacy_conflict_resolution_commands(profiles, entry)
                        .into_iter()
                        .next()
                })
        })
}

fn append_resolution(lines: &mut Vec<String>, commands: Vec<String>) {
    for command in commands {
        let action = if command.starts_with("se share request reject ") {
            "reject"
        } else if command.starts_with("se share request delete ") {
            "delete"
        } else if command.starts_with("se share grants revoke ") {
            "revoke"
        } else {
            "resolve"
        };
        lines.push(format!("{action}\t{command}"));
    }
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::{text, value};

    #[test]
    fn one_conflicted_legacy_request_never_advertises_accept() {
        let mut profiles = crate::share::ShareProfiles::default();
        let mut entry = legacy_entry();
        entry.identity_conflict = true;
        profiles.legacy_direct_requests.push(entry);

        let json = value(&profiles, 100);
        assert_eq!(json["acceptable_count"], 0);
        assert_eq!(json["blocked_count"], 1);
        assert!(json["next_command"]
            .as_str()
            .unwrap()
            .starts_with("se share request reject "));
        assert_eq!(
            json["legacy_requests"][0]["resolution_commands"],
            serde_json::json!([
                "se share request reject legacy-selector",
                "se share request delete legacy-selector"
            ])
        );
        let lines = text(&profiles, 100);
        assert!(!lines.iter().any(|line| line.starts_with("accept\t")));
        assert!(lines.iter().any(|line| line.starts_with("reject\t")));
        assert!(lines.iter().any(|line| line.starts_with("delete\t")));
    }

    #[test]
    fn tracked_conflicts_show_only_exact_resolution_commands() {
        let mut profiles = crate::share::ShareProfiles::default();
        profiles
            .direct_requests
            .push(tracked_entry(3, "01234567-89ab-4def-8123-456789abcdef"));
        profiles
            .direct_requests
            .push(tracked_entry(4, "11234567-89ab-4def-8123-456789abcdef"));

        let json = value(&profiles, 100);
        assert_eq!(json["acceptable_count"], 0);
        assert_eq!(json["blocked_count"], 2);
        assert!(json["next_command"].is_null());
        let lines = text(&profiles, 100);
        assert!(!lines.iter().any(|line| line.starts_with("accept\t")));
        assert!(lines.iter().any(|line| {
            line == "reject\tse share request reject 01234567-89ab-4def-8123-456789abcdef"
        }));
        assert!(lines.iter().any(|line| {
            line == "delete\tse share request delete 11234567-89ab-4def-8123-456789abcdef"
        }));
    }

    #[test]
    fn active_old_grant_emits_revoke_then_new_request_becomes_acceptable() {
        let mut profiles = crate::share::ShareProfiles::default();
        profiles
            .direct_requests
            .push(tracked_entry(3, "21234567-89ab-4def-8123-456789abcdef"));
        let blocker = tracked_entry(4, "31234567-89ab-4def-8123-456789abcdef")
            .record
            .request
            .requester;
        let blocker_fingerprint = blocker.fingerprint.clone();
        profiles.direct_grants.push(grant(blocker));

        let before = value(&profiles, 100);
        assert_eq!(before["acceptable_count"], 0);
        assert_eq!(
            before["next_command"],
            format!("se share grants revoke {blocker_fingerprint}")
        );
        assert_eq!(
            before["requests"][0]["resolution_commands"][0],
            format!("se share grants revoke {blocker_fingerprint}")
        );

        profiles.direct_grants[0].state = crate::share::DirectGrantState::Ignored;
        let after = value(&profiles, 100);
        assert_eq!(after["acceptable_count"], 1);
        assert_eq!(after["next_command"], "se share request accept");
    }

    #[test]
    fn shown_cross_protocol_reject_command_unblocks_the_tracked_request() {
        let mut profiles = crate::share::ShareProfiles::default();
        profiles
            .direct_requests
            .push(tracked_entry(3, "41234567-89ab-4def-8123-456789abcdef"));
        let mut legacy = legacy_entry();
        legacy.peer = tracked_entry(4, "51234567-89ab-4def-8123-456789abcdef")
            .record
            .request
            .requester;
        legacy.identity_conflict = true;
        profiles.legacy_direct_requests.push(legacy);

        let before = value(&profiles, 100);
        assert_eq!(before["acceptable_count"], 0);
        assert!(before["requests"][0]["resolution_commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command.as_str() == Some("se share request reject legacy-selector")));

        profiles.legacy_direct_requests[0].decision =
            crate::share::LegacyDirectDecisionState::Rejected;
        profiles.legacy_direct_requests[0].identity_conflict = false;
        let after = value(&profiles, 100);
        assert_eq!(after["acceptable_count"], 1);
        assert_eq!(after["next_command"], "se share request accept");
    }

    fn legacy_entry() -> crate::share::LegacyDirectRequestEntry {
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

    fn grant(peer: crate::share::DirectPeerIdentity) -> crate::share::DirectGrant {
        crate::share::DirectGrant {
            device_id: peer.device_id,
            device_name: peer.device_name,
            public_key: peer.public_key,
            fingerprint: peer.fingerprint,
            node_id: peer.node_id,
            state: crate::share::DirectGrantState::Accepted,
            updated_at: 50,
            exec: Default::default(),
        }
    }
}
