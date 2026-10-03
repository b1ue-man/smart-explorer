use crate::share::ShareProfiles;

/// Rebase daemon-owned runtime fields without replacing concurrently edited
/// user configuration or lifecycle ledger entries.
pub(crate) fn merge_worker_updates(
    latest: &mut ShareProfiles,
    before: &ShareProfiles,
    worker: &ShareProfiles,
) {
    for updated in &worker.direct_contacts {
        let Some(previous) = before
            .direct_contacts
            .iter()
            .find(|contact| contact.id == updated.id)
        else {
            continue;
        };
        let Some(current) = latest
            .direct_contacts
            .iter_mut()
            .find(|contact| contact.id == updated.id)
        else {
            continue;
        };
        if runtime_contact_changed(previous, updated) {
            // A late event must not overwrite a user's concurrently replaced
            // pins or restore authorization after an explicit withdrawal.
            if current.lookup_id != previous.lookup_id
                || current.expected_fingerprint != previous.expected_fingerprint
                || current.expected_node_id != previous.expected_node_id
                || current.remote_device_id != previous.remote_device_id
                || current.remote_public_key != previous.remote_public_key
            {
                continue;
            }
            current.expected_node_id = updated.expected_node_id.clone();
            current.remote_device_id = updated.remote_device_id.clone();
            current.remote_public_key = updated.remote_public_key.clone();
            current.last_seen = updated.last_seen;
            current.status = updated.status.clone();
            current.last_error = updated.last_error.clone();
            current.presence = updated.presence.clone();
            if current.access_state == previous.access_state
                && current.request_sent_at == previous.request_sent_at
                && current.accepted_at == previous.accepted_at
                && current.accepted_public_key == previous.accepted_public_key
            {
                current.access_state = updated.access_state.clone();
                current.request_sent_at = updated.request_sent_at;
                current.accepted_at = updated.accepted_at;
                current.accepted_public_key = updated.accepted_public_key.clone();
            }
            current.relation.signed_presence |= updated.relation.signed_presence;
            current.lan_candidates = updated.lan_candidates.clone();
            current.lan_seen_at = updated.lan_seen_at;
            current.lan_uplink = updated.lan_uplink;
        }
    }

    for updated in &worker.rooms {
        let Some(previous) = before.rooms.iter().find(|room| room.id == updated.id) else {
            continue;
        };
        let Some(current) = latest.rooms.iter_mut().find(|room| room.id == updated.id) else {
            continue;
        };
        if updated.last_seen != previous.last_seen || updated.status != previous.status {
            current.last_seen = updated.last_seen;
            current.status = updated.status.clone();
        }
        merge_members(current, previous, updated);
    }
}

fn runtime_contact_changed(
    previous: &crate::share::DirectContact,
    updated: &crate::share::DirectContact,
) -> bool {
    previous.expected_node_id != updated.expected_node_id
        || previous.remote_device_id != updated.remote_device_id
        || previous.remote_public_key != updated.remote_public_key
        || previous.last_seen != updated.last_seen
        || previous.status != updated.status
        || previous.last_error != updated.last_error
        || previous.presence != updated.presence
        || previous.access_state != updated.access_state
        || previous.request_sent_at != updated.request_sent_at
        || previous.accepted_at != updated.accepted_at
        || previous.accepted_public_key != updated.accepted_public_key
        || previous.lan_candidates != updated.lan_candidates
        || previous.lan_seen_at != updated.lan_seen_at
        || previous.lan_uplink != updated.lan_uplink
        || previous.relation.signed_presence != updated.relation.signed_presence
}

fn merge_members(
    current: &mut crate::share::RoomProfile,
    previous: &crate::share::RoomProfile,
    updated: &crate::share::RoomProfile,
) {
    for updated_member in &updated.members {
        let previous_member = previous
            .members
            .iter()
            .find(|member| member.device_id == updated_member.device_id);
        if previous_member.is_none() {
            if !current.members.iter().any(|member| {
                member.device_id == updated_member.device_id
                    || member.public_key == updated_member.public_key
                    || !member.node_id.is_empty() && member.node_id == updated_member.node_id
            }) {
                if let Some(presence) = &updated_member.presence {
                    current.upsert_member_from_presence(
                        presence.clone(),
                        updated_member.last_seen.unwrap_or_default(),
                    );
                }
            }
            continue;
        }
        if previous_member == Some(updated_member) {
            continue;
        }
        if let Some(current_member) = current
            .members
            .iter_mut()
            .find(|member| member.device_id == updated_member.device_id)
        {
            let Some(previous_member) = previous_member else {
                continue;
            };
            if current_member.public_key != previous_member.public_key
                || current_member.node_id != previous_member.node_id
                || current_member.fingerprint != previous_member.fingerprint
            {
                continue;
            }
            if updated_member.public_key != previous_member.public_key
                || updated_member.fingerprint != previous_member.fingerprint
                || !previous_member.node_id.is_empty()
                    && updated_member.node_id != previous_member.node_id
            {
                continue;
            }
            current_member.node_id = updated_member.node_id.clone();
            current_member.relation.signed_presence |= updated_member.relation.signed_presence;
            current_member.device_name = updated_member.device_name.clone();
            current_member.relay_url = updated_member.relay_url.clone();
            current_member.candidates = updated_member.candidates.clone();
            current_member.last_seen = updated_member.last_seen;
            current_member.status = updated_member.status.clone();
            current_member.presence = updated_member.presence.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::merge_worker_updates;
    use crate::share::{DirectAccessState, DirectContact, ShareProfiles, ShareStatus};

    #[test]
    fn worker_runtime_update_preserves_concurrent_user_configuration() {
        let mut before = ShareProfiles::default();
        before.direct_contacts.push(contact());
        let mut worker = before.clone();
        worker.direct_contacts[0].status = ShareStatus::Available;
        worker.direct_contacts[0].last_seen = Some(77);

        let mut latest = before.clone();
        latest.direct_contacts[0].display_name = "Renamed".into();
        latest.direct_contacts[0].auto_connect = false;

        merge_worker_updates(&mut latest, &before, &worker);

        assert_eq!(latest.direct_contacts[0].display_name, "Renamed");
        assert!(!latest.direct_contacts[0].auto_connect);
        assert_eq!(latest.direct_contacts[0].status, ShareStatus::Available);
        assert_eq!(latest.direct_contacts[0].last_seen, Some(77));
    }

    #[test]
    fn review_task_late_worker_cannot_restore_withdrawn_access_or_clear_signature() {
        let mut before = ShareProfiles::default();
        before.direct_contacts.push(contact());
        let mut worker = before.clone();
        worker.direct_contacts[0].access_state = DirectAccessState::Accepted;
        worker.direct_contacts[0].accepted_at = Some(77);
        worker.direct_contacts[0].relation.signed_presence = true;
        let mut latest = before.clone();
        latest.direct_contacts[0].access_state = DirectAccessState::Ignored;
        merge_worker_updates(&mut latest, &before, &worker);
        assert_eq!(
            latest.direct_contacts[0].access_state,
            DirectAccessState::Ignored
        );
        assert_eq!(latest.direct_contacts[0].accepted_at, None);
        assert!(latest.direct_contacts[0].relation.signed_presence);
        let mut unsigned_worker = latest.clone();
        unsigned_worker.direct_contacts[0].relation.signed_presence = false;
        unsigned_worker.direct_contacts[0].last_seen = Some(80);
        let previous = latest.clone();
        merge_worker_updates(&mut latest, &previous, &unsigned_worker);
        assert!(latest.direct_contacts[0].relation.signed_presence);
    }

    #[test]
    fn review_task_new_worker_room_member_uses_current_confirmation_policy() {
        let mut before = ShareProfiles::default();
        before.rooms.push(crate::share::RoomProfile {
            id: "profile".into(),
            name: "Room".into(),
            room_id: "room".into(),
            auto_join: true,
            last_seen: None,
            status: ShareStatus::Waiting,
            members: Vec::new(),
            exports: Default::default(),
            policy: crate::share::RoomPolicy::new_room(),
        });
        let presence = crate::share::PeerPresence {
            kind: "room".into(),
            relation_id: "room".into(),
            device_id: "peer".into(),
            device_name: "Peer".into(),
            public_key: "key".into(),
            fingerprint: "fp".into(),
            node_id: "key".into(),
            relay_url: String::new(),
            candidates: Vec::new(),
            expires_at: 100,
            nonce: "verified.ps1.signature".into(),
            proof: String::new(),
        };
        let mut worker = before.clone();
        worker.rooms[0].upsert_member_from_presence(presence, 2);
        let mut latest = before.clone();
        latest.rooms[0].policy.confirm_new_members = true;
        merge_worker_updates(&mut latest, &before, &worker);
        assert_eq!(latest.rooms[0].members.len(), 1);
        assert!(latest.rooms[0].members[0].blocked);
        assert!(!latest.rooms[0].members[0].is_admitted());
        assert!(!latest.rooms[0].members[0].exec.enabled);
    }

    fn contact() -> DirectContact {
        DirectContact {
            id: "contact-a".into(),
            display_name: "Device A".into(),
            lookup_id: "lookup-a".into(),
            expected_fingerprint: "fingerprint".into(),
            expected_node_id: "node".into(),
            remote_device_id: None,
            remote_public_key: None,
            auto_connect: true,
            auto_open: false,
            last_seen: None,
            status: ShareStatus::Offline,
            last_error: None,
            presence: None,
            access_state: DirectAccessState::Pending,
            request_sent_at: None,
            accepted_at: None,
            accepted_public_key: None,
            lan_candidates: Vec::new(),
            lan_seen_at: None,
            lan_uplink: None,
            relation: Default::default(),
        }
    }
}
