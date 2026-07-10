use super::fs::ShareExportConfig;
use super::types::{
    DirectContact, DirectGrant, PeerPresence, RoomMember, RoomProfile, ShareAuthState,
};

pub(super) fn configuration_changed(
    current: &ShareAuthState,
    direct: &[DirectContact],
    direct_grants: &[DirectGrant],
    rooms: &[RoomProfile],
    default_direct_exports: &ShareExportConfig,
) -> bool {
    &current.default_direct_exports != default_direct_exports
        || !same_contacts(&current.direct_contacts, direct)
        || !same_grants(&current.direct_grants, direct_grants)
        || !same_rooms(&current.rooms, rooms)
}

fn same_contacts(left: &[DirectContact], right: &[DirectContact]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.id == right.id
                && left.lookup_id == right.lookup_id
                && left.expected_fingerprint == right.expected_fingerprint
                && left.expected_node_id == right.expected_node_id
                && left.remote_device_id == right.remote_device_id
                && left.remote_public_key == right.remote_public_key
                && left.auto_connect == right.auto_connect
                && left.access_state == right.access_state
                && same_presence(left.presence.as_ref(), right.presence.as_ref())
        })
}

fn same_grants(left: &[DirectGrant], right: &[DirectGrant]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.device_id == right.device_id
                && left.public_key == right.public_key
                && left.fingerprint == right.fingerprint
                && left.node_id == right.node_id
                && left.state == right.state
        })
}

fn same_rooms(left: &[RoomProfile], right: &[RoomProfile]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.id == right.id
                && left.room_id == right.room_id
                && left.auto_join == right.auto_join
                && left.exports == right.exports
                && same_members(&left.members, &right.members)
        })
}

fn same_members(left: &[RoomMember], right: &[RoomMember]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.device_id == right.device_id
                && left.public_key == right.public_key
                && left.fingerprint == right.fingerprint
                && left.node_id == right.node_id
                && left.blocked == right.blocked
        })
}

fn same_presence(left: Option<&PeerPresence>, right: Option<&PeerPresence>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.relation_id == right.relation_id
                && left.device_id == right.device_id
                && left.public_key == right.public_key
                && left.fingerprint == right.fingerprint
                && left.node_id == right.node_id
        }
        _ => false,
    }
}
