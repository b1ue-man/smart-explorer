use crate::share::{DirectContact, RoomProfile, ShareProfiles, ShareStatus};

/// Rebase only user-owned configuration fields onto the latest daemon-owned
/// profile. Runtime presence and lifecycle updates are intentionally left on
/// `latest`, so a GUI poll cannot overwrite a worker update with a stale copy.
pub(super) fn merge_user_edits(
    latest: &mut ShareProfiles,
    before: &ShareProfiles,
    edited: &ShareProfiles,
) {
    if edited.auto_connect != before.auto_connect {
        latest.auto_connect = edited.auto_connect;
    }
    if edited.default_direct_exports != before.default_direct_exports {
        latest.default_direct_exports = edited.default_direct_exports.clone();
    }

    for edited_contact in &edited.direct_contacts {
        let Some(before_contact) = before
            .direct_contacts
            .iter()
            .find(|contact| contact.id == edited_contact.id)
        else {
            continue;
        };
        let Some(latest_contact) = latest
            .direct_contacts
            .iter_mut()
            .find(|contact| contact.id == edited_contact.id)
        else {
            continue;
        };
        merge_contact(latest_contact, before_contact, edited_contact);
    }

    for edited_room in &edited.rooms {
        let Some(before_room) = before.rooms.iter().find(|room| room.id == edited_room.id) else {
            continue;
        };
        let Some(latest_room) = latest
            .rooms
            .iter_mut()
            .find(|room| room.id == edited_room.id)
        else {
            continue;
        };
        merge_room(latest_room, before_room, edited_room);
    }
}

fn merge_contact(latest: &mut DirectContact, before: &DirectContact, edited: &DirectContact) {
    if edited.display_name != before.display_name {
        latest.display_name = edited.display_name.clone();
    }
    if edited.auto_connect != before.auto_connect {
        latest.auto_connect = edited.auto_connect;
    }
    if edited.auto_open != before.auto_open {
        latest.auto_open = edited.auto_open;
    }

    let trust_was_reset = before.presence.is_some()
        && edited.presence.is_none()
        && edited.remote_device_id.is_none()
        && edited.remote_public_key.is_none();
    if trust_was_reset {
        latest.remote_device_id = None;
        latest.remote_public_key = None;
        latest.presence = None;
        latest.status = ShareStatus::Waiting;
    }
}

fn merge_room(latest: &mut RoomProfile, before: &RoomProfile, edited: &RoomProfile) {
    if edited.name != before.name {
        latest.name = edited.name.clone();
    }
    if edited.auto_join != before.auto_join {
        latest.auto_join = edited.auto_join;
    }
    if edited.exports != before.exports {
        latest.exports = edited.exports.clone();
    }
    for edited_member in &edited.members {
        let Some(before_member) = before
            .members
            .iter()
            .find(|member| member.device_id == edited_member.device_id)
        else {
            continue;
        };
        let Some(latest_member) = latest
            .members
            .iter_mut()
            .find(|member| member.device_id == edited_member.device_id)
        else {
            continue;
        };
        if edited_member.blocked != before_member.blocked {
            latest_member.blocked = edited_member.blocked;
        }
        if before_member.presence.is_some() && edited_member.presence.is_none() {
            latest_member.presence = None;
            latest_member.status = ShareStatus::Waiting;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::merge_user_edits;
    use crate::share::{DirectAccessState, DirectContact, ShareProfiles, ShareStatus};

    #[test]
    fn gui_edit_rebases_without_reverting_worker_runtime_state() {
        let mut before = ShareProfiles::default();
        before.direct_contacts.push(contact());
        let mut edited = before.clone();
        edited.direct_contacts[0].auto_connect = false;

        let mut latest = before.clone();
        latest.direct_contacts[0].status = ShareStatus::Available;
        latest.direct_contacts[0].last_seen = Some(42);
        latest.direct_contacts[0].access_state = DirectAccessState::Accepted;

        merge_user_edits(&mut latest, &before, &edited);

        assert!(!latest.direct_contacts[0].auto_connect);
        assert_eq!(latest.direct_contacts[0].status, ShareStatus::Available);
        assert_eq!(latest.direct_contacts[0].last_seen, Some(42));
        assert_eq!(
            latest.direct_contacts[0].access_state,
            DirectAccessState::Accepted
        );
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
