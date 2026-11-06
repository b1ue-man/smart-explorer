//! The devices `peer_feed` keeps a feed for (direct contacts with a pinned
//! identity, members of auto-join rooms) and the events that report one of
//! them arriving.
use crate::share::auto_forward;
use crate::share::{DirectContact, ShareEvent, ShareProfiles};
use std::collections::{BTreeMap, BTreeSet};

/// Device ID, public key, name.
pub(super) type Identity = (String, String, String);

pub(super) fn identities(profiles: &ShareProfiles) -> BTreeMap<String, Identity> {
    let mut known = BTreeMap::new();
    for contact in &profiles.direct_contacts {
        if let (Some(device), Some(key)) = (&contact.remote_device_id, &contact.remote_public_key) {
            let identity = (device.clone(), key.clone(), contact.display_name.clone());
            known.insert(auto_forward::device_key(device, key), identity);
        }
    }
    for room in profiles.rooms.iter().filter(|room| room.auto_join) {
        for member in room.members.iter().filter(|member| !member.blocked) {
            let key = auto_forward::device_key(&member.device_id, &member.public_key);
            known.entry(key).or_insert_with(|| {
                let name = member.device_name.clone();
                (member.device_id.clone(), member.public_key.clone(), name)
            });
        }
    }
    known
}

/// Device IDs whose arrival the events report (design 5.2 retry triggers):
/// a presence, a room join or roster, a LAN sighting. Departures are no
/// reason to connect.
pub(super) fn arrived_devices(profiles: &ShareProfiles, events: &[ShareEvent]) -> BTreeSet<String> {
    let contact = |contact_id: &str| {
        profiles
            .direct_contacts
            .iter()
            .find(|contact: &&DirectContact| contact.id == contact_id)
            .and_then(|contact| contact.remote_device_id.clone())
    };
    let mut devices = BTreeSet::new();
    for event in events {
        match event {
            ShareEvent::DirectAvailable { presence, .. }
            | ShareEvent::RoomJoined { presence, .. } => {
                devices.insert(presence.device_id.clone());
            }
            ShareEvent::RoomRoster { members, .. } => {
                devices.extend(members.iter().map(|member| member.device_id.clone()));
            }
            ShareEvent::LanPeerSeen { contact_id, .. } => {
                devices.extend(contact(contact_id.as_str()));
            }
            _ => {}
        }
    }
    devices
}

/// A mapped service of the device has a listener (on Android only after the
/// person's tap) or a tunnel: a low-power pause would cut it off.
pub(super) fn tunnel_in_use(identity: &Identity) -> bool {
    auto_forward::views(&identity.0, &identity.1)
        .iter()
        .any(|mapped| mapped.address.is_some() || mapped.tunnel.is_some())
}
