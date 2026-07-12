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
            current.expected_node_id = updated.expected_node_id.clone();
            current.remote_device_id = updated.remote_device_id.clone();
            current.remote_public_key = updated.remote_public_key.clone();
            current.last_seen = updated.last_seen;
            current.status = updated.status.clone();
            current.last_error = updated.last_error.clone();
            current.presence = updated.presence.clone();
            current.access_state = updated.access_state.clone();
            current.request_sent_at = updated.request_sent_at;
            current.accepted_at = updated.accepted_at;
            current.accepted_public_key = updated.accepted_public_key.clone();
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
            if !current
                .members
                .iter()
                .any(|member| member.device_id == updated_member.device_id)
            {
                current.members.push(updated_member.clone());
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
            let blocked = current_member.blocked;
            *current_member = updated_member.clone();
            current_member.blocked = blocked;
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
        }
    }
}
