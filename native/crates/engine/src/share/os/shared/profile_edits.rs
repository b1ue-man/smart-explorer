use crate::share::{DirectContact, RoomProfile, ShareProfiles, ShareStatus};

#[path = "profile_export_edits.rs"]
mod export_edits;

/// Rebase only user-owned configuration fields onto the latest daemon-owned
/// profile. Runtime presence and lifecycle updates are intentionally left on
/// `latest`, so a GUI poll cannot overwrite a worker update with a stale copy.
pub fn merge_user_edits(
    latest: &mut ShareProfiles,
    before: &ShareProfiles,
    edited: &ShareProfiles,
) -> Result<(), String> {
    let mut candidate = latest.clone();
    apply_user_edits(&mut candidate, before, edited)?;
    *latest = candidate;
    Ok(())
}

fn apply_user_edits(
    latest: &mut ShareProfiles,
    before: &ShareProfiles,
    edited: &ShareProfiles,
) -> Result<(), String> {
    if edited.auto_connect != before.auto_connect {
        latest.auto_connect = edited.auto_connect;
    }
    export_edits::merge(
        &mut latest.default_direct_exports,
        &before.default_direct_exports,
        &edited.default_direct_exports,
    )?;

    for edited_contact in &edited.direct_contacts {
        let Some(before_contact) = before
            .direct_contacts
            .iter()
            .find(|contact| contact.id == edited_contact.id)
        else {
            continue;
        };
        let trust_was_reset = before_contact.presence.is_some()
            && edited_contact.presence.is_none()
            && edited_contact.remote_device_id.is_none()
            && edited_contact.remote_public_key.is_none();
        if trust_was_reset
            || before_contact.relation.share_back != edited_contact.relation.share_back
        {
            let current = latest
                .direct_contacts
                .iter()
                .find(|contact| contact.id == edited_contact.id)
                .ok_or_else(|| "Direktkontakt wurde entfernt; bitte neu laden".to_string())?;
            if current.lookup_id != before_contact.lookup_id
                || current.expected_fingerprint != before_contact.expected_fingerprint
                || current.expected_node_id != before_contact.expected_node_id
                || before_contact.remote_device_id.is_some()
                    && current.remote_device_id != before_contact.remote_device_id
                || before_contact.remote_public_key.is_some()
                    && current.remote_public_key != before_contact.remote_public_key
            {
                return Err(
                    "Identitaet des Direktkontakts wurde geaendert; bitte neu laden".into(),
                );
            }
        }
        if before_contact.relation.share_back != edited_contact.relation.share_back {
            latest.set_contact_share_back(
                &edited_contact.id,
                edited_contact.relation.share_back,
                crate::share::core_now_secs(),
            )?;
        }
        if trust_was_reset {
            if let Some(peer) = ShareProfiles::contact_remote_identity(before_contact) {
                latest.withdraw_direct_key(&peer, crate::share::core_now_secs());
            }
        }
        let Some(latest_contact) = latest
            .direct_contacts
            .iter_mut()
            .find(|contact| contact.id == edited_contact.id)
        else {
            if edited_contact.display_name != before_contact.display_name
                || edited_contact.auto_connect != before_contact.auto_connect
                || edited_contact.auto_open != before_contact.auto_open
                || trust_was_reset
            {
                return Err("Direktkontakt wurde entfernt; bitte neu laden".into());
            }
            continue;
        };
        merge_contact(latest_contact, before_contact, edited_contact);
    }

    for edited_grant in &edited.direct_grants {
        let Some(before_grant) = before.direct_grants.iter().find(|grant| {
            grant.device_id == edited_grant.device_id
                && grant.public_key == edited_grant.public_key
                && grant.node_id == edited_grant.node_id
                && grant.fingerprint == edited_grant.fingerprint
        }) else {
            continue;
        };
        if before_grant.write != edited_grant.write {
            let expected = crate::share::DirectPeerIdentity {
                device_id: before_grant.device_id.clone(),
                device_name: before_grant.device_name.clone(),
                public_key: before_grant.public_key.clone(),
                node_id: before_grant.node_id.clone(),
                fingerprint: before_grant.fingerprint.clone(),
            };
            latest.set_direct_peer_write(
                &expected,
                edited_grant.write,
                crate::share::core_now_secs(),
            )?;
        }
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
            if edited_room.name != before_room.name
                || edited_room.auto_join != before_room.auto_join
                || edited_room.exports != before_room.exports
                || edited_room.policy != before_room.policy
            {
                return Err("Raum wurde entfernt; bitte neu laden".into());
            }
            continue;
        };
        merge_room(latest_room, before_room, edited_room)?;
    }
    Ok(())
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

fn merge_room(
    latest: &mut RoomProfile,
    before: &RoomProfile,
    edited: &RoomProfile,
) -> Result<(), String> {
    if latest.room_id != before.room_id {
        return Err("Raumidentitaet wurde geaendert; bitte neu laden".into());
    }
    if edited.name != before.name {
        latest.name = edited.name.clone();
    }
    if edited.auto_join != before.auto_join {
        latest.auto_join = edited.auto_join;
    }
    export_edits::merge(&mut latest.exports, &before.exports, &edited.exports)?;
    if edited.policy.members_may_write != before.policy.members_may_write {
        latest.policy.members_may_write = edited.policy.members_may_write;
    }
    if edited.policy.confirm_new_members != before.policy.confirm_new_members {
        latest.policy.confirm_new_members = edited.policy.confirm_new_members;
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
            .iter()
            .find(|member| member.device_id == edited_member.device_id)
        else {
            if edited_member.blocked != before_member.blocked {
                return Err("Raummitglied wurde entfernt; bitte neu laden".into());
            }
            continue;
        };
        if latest_member.public_key != before_member.public_key
            || latest_member.node_id != before_member.node_id
            || latest_member.fingerprint != before_member.fingerprint
        {
            if edited_member.blocked != before_member.blocked {
                return Err("Raummitglied wurde ersetzt; bitte neu laden".into());
            }
            continue;
        }
        if edited_member.blocked != before_member.blocked {
            if edited_member.blocked {
                latest.set_member_blocked(
                    &edited_member.device_id,
                    true,
                    crate::share::core_now_secs(),
                );
            } else if !latest.admit_member(&edited_member.device_id) {
                latest.set_member_blocked(
                    &edited_member.device_id,
                    false,
                    crate::share::core_now_secs(),
                );
            }
        }
        if before_member.presence.is_some() && edited_member.presence.is_none() {
            let Some(latest_member) = latest
                .members
                .iter_mut()
                .find(|member| member.device_id == edited_member.device_id)
            else {
                continue;
            };
            latest_member.presence = None;
            latest_member.status = ShareStatus::Waiting;
        }
    }
    Ok(())
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

        merge_user_edits(&mut latest, &before, &edited).unwrap();

        assert!(!latest.direct_contacts[0].auto_connect);
        assert_eq!(latest.direct_contacts[0].status, ShareStatus::Available);
        assert_eq!(latest.direct_contacts[0].last_seen, Some(42));
        assert_eq!(
            latest.direct_contacts[0].access_state,
            DirectAccessState::Accepted
        );
    }

    #[test]
    fn review_task_fc1_gui_rebase_keeps_concurrent_exports_and_all_denials() {
        use crate::share::{ExportAccess, SharedRoot};
        let mut before = ShareProfiles::default();
        before
            .default_direct_exports
            .roots
            .push(SharedRoot::new("A", "/a"));
        before
            .default_direct_exports
            .set_connection_access("sftp://u@a:22/docs", Some(ExportAccess::ReadOnly))
            .unwrap();
        let mut edited = before.clone();
        edited.auto_connect = false;
        edited.default_direct_exports.roots[0].access = ExportAccess::ReadWrite;
        edited
            .default_direct_exports
            .set_connection_access("sftp://u@a:22/docs", None)
            .unwrap();
        let mut latest = before.clone();
        latest.default_direct_exports.roots[0].allow_system_writes = true;
        latest
            .default_direct_exports
            .roots
            .push(SharedRoot::new("B", "/b"));
        latest
            .default_direct_exports
            .set_connection_access("sftp://u@b:22/docs", Some(ExportAccess::ReadWrite))
            .unwrap();
        for index in 0..70 {
            latest.record_removed_direct_peer(
                &crate::share::DirectPeerIdentity {
                    device_id: format!("d{index}"),
                    device_name: String::new(),
                    public_key: format!("k{index}"),
                    node_id: format!("n{index}"),
                    fingerprint: format!("f{index}"),
                },
                index,
            );
        }
        merge_user_edits(&mut latest, &before, &edited).unwrap();
        assert!(!latest.auto_connect);
        assert_eq!(latest.removed_direct_peers.len(), 70);
        assert_eq!(latest.default_direct_exports.roots.len(), 2);
        assert_eq!(
            latest.default_direct_exports.roots[0].access,
            ExportAccess::ReadWrite
        );
        assert!(latest.default_direct_exports.roots[0].allow_system_writes);
        assert_eq!(latest.default_direct_exports.roots[1].path, "/b");
        assert_eq!(
            latest
                .default_direct_exports
                .connection_access("sftp://u@a:22/docs"),
            None
        );
        assert_eq!(
            latest
                .default_direct_exports
                .connection_access("sftp://u@b:22/docs"),
            Some(ExportAccess::ReadWrite)
        );
    }

    #[test]
    fn review_task_fc1_gui_write_to_a_replaced_key_rolls_back_every_edit() {
        let mut before = ShareProfiles::default();
        before.direct_grants.push(crate::share::DirectGrant {
            device_id: "d".into(),
            device_name: "Peer".into(),
            public_key: "old-key".into(),
            node_id: "node".into(),
            fingerprint: "old-fp".into(),
            state: crate::share::DirectGrantState::Accepted,
            updated_at: 1,
            exec: Default::default(),
            write: false,
        });
        let mut edited = before.clone();
        edited.auto_connect = false;
        edited.direct_grants[0].write = true;
        let mut latest = before.clone();
        latest.direct_grants[0].public_key = "new-key".into();
        assert!(merge_user_edits(&mut latest, &before, &edited).is_err());
        assert!(latest.auto_connect);
        assert_eq!(latest.direct_grants[0].public_key, "new-key");
        assert!(!latest.direct_grants[0].write);
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
