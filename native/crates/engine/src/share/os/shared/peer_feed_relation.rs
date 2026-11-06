//! What this device stores about its relation to one other device, without
//! runtime data (A5 of plan 2026-11-06-akku-rechte): a refused feed starts
//! again only when this changes. Presence, routes, status, names and
//! `last_seen` move with every presence event of any device and are left
//! out, as `relation_rights::profiles_differ_beyond_runtime` leaves them
//! out of a configuration transition. The parts are kept as values (no
//! hash), so equal contents always compare equal.
use crate::share::{DirectContact, DirectGrant, RoomProfile, ShareProfiles, ShareStatus};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Relation {
    contacts: Vec<DirectContact>,
    grants: Vec<DirectGrant>,
    /// Every auto-join room the device is a member of, with only that member.
    rooms: Vec<RoomProfile>,
}

impl Relation {
    pub(super) fn of(profiles: &ShareProfiles, device_id: &str, public_key: &str) -> Self {
        let contacts = profiles
            .direct_contacts
            .iter()
            .filter(|contact| {
                contact.remote_device_id.as_deref() == Some(device_id)
                    && contact.remote_public_key.as_deref() == Some(public_key)
            })
            .map(without_contact_runtime)
            .collect();
        let grants = profiles
            .direct_grants
            .iter()
            .filter(|grant| grant.device_id == device_id && grant.public_key == public_key)
            .map(|grant| {
                let mut grant = grant.clone();
                grant.device_name.clear();
                grant
            })
            .collect();
        let rooms = profiles
            .rooms
            .iter()
            .filter(|room| room.auto_join)
            .filter_map(|room| room_of(room, device_id, public_key))
            .collect();
        Self {
            contacts,
            grants,
            rooms,
        }
    }
}

fn without_contact_runtime(contact: &DirectContact) -> DirectContact {
    let mut contact = contact.clone();
    contact.status = ShareStatus::default();
    contact.last_seen = None;
    contact.last_error = None;
    contact.presence = None;
    contact.lan_candidates.clear();
    contact.lan_seen_at = None;
    contact.lan_uplink = None;
    contact
}

fn room_of(room: &RoomProfile, device_id: &str, public_key: &str) -> Option<RoomProfile> {
    let member = room
        .members
        .iter()
        .find(|member| member.device_id == device_id && member.public_key == public_key)?;
    let mut member = member.clone();
    member.device_name.clear();
    member.relay_url.clear();
    member.candidates.clear();
    member.last_seen = None;
    member.status = ShareStatus::default();
    member.presence = None;
    Some(RoomProfile {
        id: room.id.clone(),
        name: String::new(),
        room_id: room.room_id.clone(),
        auto_join: room.auto_join,
        last_seen: None,
        status: ShareStatus::default(),
        members: vec![member],
        exports: room.exports.clone(),
        policy: room.policy.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::Relation;
    use crate::share::{DirectContact, PeerPresence, ShareProfiles};

    #[test]
    fn akku_task_relation_ignores_presence_and_follows_access() {
        let mut profiles = ShareProfiles::default();
        let contact: DirectContact = serde_json::from_value(serde_json::json!({
            "id": "c1",
            "display_name": "Laptop",
            "lookup_id": "l1",
            "expected_fingerprint": "f",
            "remote_device_id": "device",
            "remote_public_key": "key",
            "auto_connect": true,
            "auto_open": false,
            "last_seen": null,
        }))
        .expect("contact");
        profiles.direct_contacts.push(contact);
        let before = Relation::of(&profiles, "device", "key");
        assert_eq!(Relation::of(&profiles, "other", "key"), Relation::default());

        let contact = &mut profiles.direct_contacts[0];
        contact.last_seen = Some(1_700_000_000);
        contact.lan_candidates.push("192.0.2.1:4433".into());
        contact.presence = Some(PeerPresence {
            kind: "direct".into(),
            relation_id: "l1".into(),
            device_id: "device".into(),
            device_name: "Laptop".into(),
            public_key: "key".into(),
            fingerprint: "f".into(),
            node_id: String::new(),
            relay_url: String::new(),
            candidates: Vec::new(),
            expires_at: 1_700_000_100,
            nonce: "n".into(),
            proof: "p".into(),
        });
        assert_eq!(Relation::of(&profiles, "device", "key"), before);

        profiles.direct_contacts[0].auto_connect = false;
        assert_ne!(Relation::of(&profiles, "device", "key"), before);
    }
}
